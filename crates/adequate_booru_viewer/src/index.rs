use anyhow::{Context as _, Result, bail};
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, ReadableTableMetadata as _,
    TableDefinition, TableError,
};
use roaring::RoaringBitmap;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap},
    path::Path,
    sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, Instant},
};

use crate::model::{
    BoolOp, Corpus, FamilyTree, GalleryTopology, Harvest, Kin, PostId, PostRecord, Query,
    QueryAtom, QueryExpr, RatingClass, SearchHit, SearchTail, Sort, Tag, TagKind, TagPattern,
    TagPatternMatch, decode_record, encode_record, narrow_post_id, record_day,
};
use crate::posting::{self, Batch as FactBatch, Lane as PostingLane};
use crate::trace::startup;
use crate::{
    date::{CreatedDay, DateRange},
    kin,
};

const POSTS: TableDefinition<'_, u64, &[u8]> = TableDefinition::new("posts");
const TAG_CHUNKS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("tag_chunks.v1");
const TAG_KINDS: TableDefinition<'_, &str, u8> = TableDefinition::new("tag_kinds.v1");
const RATING_CHUNKS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("rating_chunks.v1");
const POSTING_FACTS: TableDefinition<'_, u64, &[u8]> = TableDefinition::new("posting_facts.v1");
const SCORE_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("score_posts");
const FAV_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("fav_posts");
const META: TableDefinition<'_, &str, u64> = TableDefinition::new("meta");

const SMALL_SORT: u64 = 50_000;
const DANBOORU_CRAWL_BEFORE: &str = "danbooru.crawl.before";
const POSTING_FACT_NEXT_SEQ: &str = "posting_facts.v1.next_seq";
const CHUNK_BITS: u32 = 16;

#[derive(Clone, Copy, Debug)]
pub struct FactMergeBudget {
    pub batches: usize,
    pub bytes: usize,
}

impl FactMergeBudget {
    pub const STEADY: Self = Self {
        batches: 128,
        bytes: 16 * 1024 * 1024,
    };
}

#[derive(Clone, Debug, Default)]
pub struct FactMerge {
    pub batches: usize,
    pub bytes: usize,
    pub groups: usize,
}

#[derive(Clone, Debug)]
pub struct TagSuggestion {
    pub kind: TagKind,
    pub tag: String,
    pub posts: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    pub posts: u64,
    pub tag_chunks: u64,
    pub pending_fact_batches: u64,
    pub newest: Option<PostId>,
    pub crawl_before: Option<PostId>,
    pub ratings: Vec<(RatingClass, u64)>,
}

impl CacheStats {
    pub fn rough_crawl_percent(&self) -> Option<f32> {
        let newest = self.newest?.0;
        let before = self.crawl_before?.0;
        if newest == 0 || before > newest {
            return None;
        }
        let covered = newest - before + 1;
        Some(100.0 * covered as f32 / newest as f32)
    }
}

/// How long non-durable commits may accumulate before an anchor commit
/// fsyncs them. Everything in the index is re-crawlable; a crash costs at
/// most this much repeated work.
const ANCHOR_GAP: Duration = Duration::from_secs(30);

/// Decoded postings the vault may hold; eviction scans are O(cap) and rare.
const VAULT_CAP: usize = 96;
const RECORD_VAULT_CAP: usize = 4096;
const SORT_HEAD_CAP: usize = 262_144;

/// Cache of decoded posting bitmaps for the merged tables. Hot tags skip the
/// redb chunk scan and roaring deserialization on every query; the merge loop
/// evicts exactly the keys it rewrites. Pending (unmerged) deltas are applied
/// on top after the cache, so reads stay exact.
///
/// FUTURE DIRECTION — the Lucene-grade design: move postings out of redb
/// values into an mmap'd segment file in `CRoaring`'s *frozen* format
/// (`croaring`'s `BitmapView`). Frozen bitmaps are used in place with zero
/// deserialization, and set operations touch only the containers they visit,
/// turning query cost from O(serialized bytes of every atom touched) into
/// O(containers actually needed). Reach for it when the index outgrows this
/// cache; it obsoletes the vault entirely.
struct Vault {
    slots: HashMap<posting::Key, (Arc<RoaringBitmap>, u64)>,
    clock: u64,
}

impl Vault {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
            clock: 0,
        }
    }

    fn get(&mut self, lane: PostingLane, key: &str) -> Option<Arc<RoaringBitmap>> {
        self.clock += 1;
        let clock = self.clock;
        self.slots
            .get_mut(&posting::Key::new(lane, key))
            .map(|(bitmap, stamp)| {
                *stamp = clock;
                Arc::clone(bitmap)
            })
    }

    fn put(&mut self, lane: PostingLane, key: &str, bitmap: RoaringBitmap) -> Arc<RoaringBitmap> {
        if self.slots.len() >= VAULT_CAP
            && let Some(coldest) = self
                .slots
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(key, _)| key.clone())
        {
            let _evicted = self.slots.remove(&coldest);
        }
        self.clock += 1;
        let bitmap = Arc::new(bitmap);
        let _old = self.slots.insert(
            posting::Key::new(lane, key),
            (Arc::clone(&bitmap), self.clock),
        );
        bitmap
    }

    fn evict(&mut self, lane: PostingLane, key: &str) {
        let _old = self.slots.remove(&posting::Key::new(lane, key));
    }
}

struct RecordVault {
    slots: HashMap<PostId, (PostRecord, u64)>,
    clock: u64,
}

impl RecordVault {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
            clock: 0,
        }
    }

    fn get(&mut self, id: PostId) -> Option<PostRecord> {
        self.clock += 1;
        let clock = self.clock;
        self.slots.get_mut(&id).map(|(post, stamp)| {
            *stamp = clock;
            post.clone()
        })
    }

    fn put(&mut self, post: PostRecord) {
        if self.slots.len() >= RECORD_VAULT_CAP
            && let Some(coldest) = self
                .slots
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(id, _)| *id)
        {
            let _evicted = self.slots.remove(&coldest);
        }
        self.clock += 1;
        let _old = self.slots.insert(post.id, (post, self.clock));
    }

    fn evict(&mut self, id: PostId) {
        let _old = self.slots.remove(&id);
    }
}

#[derive(Default)]
struct SortHeadVault {
    score: Option<Arc<Vec<u32>>>,
    favs: Option<Arc<Vec<u32>>>,
}

impl SortHeadVault {
    fn get(&self, sort: Sort) -> Option<Arc<Vec<u32>>> {
        match sort {
            Sort::Score => self.score.clone(),
            Sort::FavCount => self.favs.clone(),
            Sort::Newest => None,
        }
    }

    fn put(&mut self, sort: Sort, ids: Arc<Vec<u32>>) {
        match sort {
            Sort::Score => self.score = Some(ids),
            Sort::FavCount => self.favs = Some(ids),
            Sort::Newest => {}
        }
    }

    fn clear(&mut self) {
        self.score = None;
        self.favs = None;
    }
}

#[derive(Default)]
struct SortKeyVault {
    score: Option<Arc<Vec<u64>>>,
    favs: Option<Arc<Vec<u64>>>,
}

impl SortKeyVault {
    fn get(&self, sort: Sort) -> Option<Arc<Vec<u64>>> {
        match sort {
            Sort::Score => self.score.clone(),
            Sort::FavCount => self.favs.clone(),
            Sort::Newest => None,
        }
    }

    fn put(&mut self, sort: Sort, keys: Arc<Vec<u64>>) {
        match sort {
            Sort::Score => self.score = Some(keys),
            Sort::FavCount => self.favs = Some(keys),
            Sort::Newest => {}
        }
    }

    fn refresh(&mut self, posts: &[&PostRecord]) {
        if let Some(keys) = &mut self.score {
            let keys = Arc::make_mut(keys);
            for post in posts {
                set_sort_key(
                    keys,
                    post.id,
                    post.indexable().then(|| sort_key_i32(post.score, post.id)),
                );
            }
        }
        if let Some(keys) = &mut self.favs {
            let keys = Arc::make_mut(keys);
            for post in posts {
                set_sort_key(
                    keys,
                    post.id,
                    post.indexable().then(|| sort_key_u32(post.favs, post.id)),
                );
            }
        }
    }
}

/// One dynamic binary rank dimension laid over immutable provider lanes.
///
/// For `F` favorites, a new favorite snapshot costs `O(F log_B N + F log F)`
/// once: `F` keyed post reads and two complete orderings. Warm retrieval scans
/// only the favorite prefixes surviving the arbitrary query, then resumes the
/// provider lane for ordinary posts. Space is `O(F)`; no dense `O(N)` rank
/// projection is created merely because one favorite exists.
struct FavoriteRanks {
    source: Arc<RoaringBitmap>,
    available: Arc<RoaringBitmap>,
    score: Vec<u32>,
    fav_count: Vec<u32>,
}

impl FavoriteRanks {
    fn ids(&self, sort: Sort) -> &[u32] {
        match sort {
            Sort::Score => &self.score,
            Sort::FavCount => &self.fav_count,
            Sort::Newest => unreachable!("newest iterates the favorite bitmap"),
        }
    }
}

#[derive(Default)]
struct FavoriteRankVault {
    ranks: Option<Arc<FavoriteRanks>>,
}

impl FavoriteRankVault {
    fn get(&self, source: &Arc<RoaringBitmap>) -> Option<Arc<FavoriteRanks>> {
        self.ranks
            .as_ref()
            .filter(|ranks| Arc::ptr_eq(&ranks.source, source))
            .map(Arc::clone)
    }

    fn put(&mut self, ranks: FavoriteRanks) -> Arc<FavoriteRanks> {
        let ranks = Arc::new(ranks);
        self.ranks = Some(Arc::clone(&ranks));
        ranks
    }

    fn invalidate(&mut self, posts: &[&PostRecord]) {
        if self
            .ranks
            .as_ref()
            .is_some_and(|ranks| posts.iter().any(|post| ranks.source.contains(post.id.0)))
        {
            self.ranks = None;
        }
    }
}

#[derive(Clone)]
pub struct Index {
    db: Arc<Database>,
    kin: Arc<RwLock<kin::Atlas>>,
    tag_bank: Arc<RwLock<Option<BTreeMap<Tag, TagKind>>>>,
    anchor: Arc<Mutex<Instant>>,
    vault: Arc<Mutex<Vault>>,
    records: Arc<Mutex<RecordVault>>,
    sort_heads: Arc<Mutex<SortHeadVault>>,
    sort_keys: Arc<Mutex<SortKeyVault>>,
    favorite_ranks: Arc<Mutex<FavoriteRankVault>>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        startup("index.open.enter");
        let db = Database::create(path).with_context(|| format!("open redb {}", path.display()))?;
        startup("index.redb.create.done");
        Self::prime_database(&db)?;
        let minimum_slots = newest_slot(&db)?;
        let atlas = kin::Atlas::open(&db, path.with_extension("kin.u32"), minimum_slots)?;
        let index = Self {
            db: Arc::new(db),
            kin: Arc::new(RwLock::new(atlas)),
            tag_bank: Arc::new(RwLock::new(None)),
            anchor: Arc::new(Mutex::new(Instant::now())),
            vault: Arc::new(Mutex::new(Vault::new())),
            records: Arc::new(Mutex::new(RecordVault::new())),
            sort_heads: Arc::new(Mutex::new(SortHeadVault::default())),
            sort_keys: Arc::new(Mutex::new(SortKeyVault::default())),
            favorite_ranks: Arc::new(Mutex::new(FavoriteRankVault::default())),
        };
        startup("index.prime.done");
        Ok(index)
    }

    #[allow(
        dead_code,
        reason = "the duplicated binary module does not see the benchmark/test consumers of the library API"
    )]
    pub fn absorb(&self, posts: &[PostRecord]) -> Result<()> {
        let posts = posts.iter().collect::<Vec<_>>();
        let tx = self.begin_quick_write("begin index write")?;
        Self::absorb_into(&tx, &posts)?;
        tx.commit().context("commit index write")?;
        self.patch_tag_bank(&posts);
        self.evict_records(&posts);
        self.clear_sort_heads();
        self.refresh_sort_keys(&posts);
        self.invalidate_favorite_ranks(&posts);
        Ok(())
    }

    pub fn absorb_harvest(&self, harvest: &[Harvest]) -> Result<()> {
        self.absorb_harvest_with(harvest, None)
    }

    pub fn absorb_crawl_harvest(&self, harvest: &[Harvest], before: Option<PostId>) -> Result<()> {
        self.absorb_harvest_with(harvest, Some((before, false)))
    }

    pub fn absorb_kin_crawl(
        &self,
        facts: &[Kin],
        before: Option<PostId>,
        complete: bool,
    ) -> Result<()> {
        let mut atlas = write(&self.kin);
        let tx = self.begin_quick_write("begin kin crawl write")?;
        let mutation = kin::absorb(&tx, facts)?;
        kin::advance_backfill(&tx, before, complete)?;
        tx.commit().context("commit kin crawl write")?;
        atlas.apply(&mutation)
    }

    pub fn kin_backfill(&self) -> Result<kin::Backfill> {
        kin::backfill(&self.db)
    }

    pub fn family_hydrated(&self, id: PostId) -> Result<bool> {
        let root = read(&self.kin).root(id);
        kin::hydrated(&self.db, root)
    }

    pub fn absorb_family(&self, facts: &[Kin], root: PostId) -> Result<()> {
        let mut atlas = write(&self.kin);
        let tx = self.begin_quick_write("begin family hydration write")?;
        let mutation = kin::absorb(&tx, facts)?;
        kin::seal_hydrated(&tx, root)?;
        tx.commit().context("commit family hydration")?;
        atlas.apply(&mutation)
    }

    pub fn missing_posts(&self, ids: impl IntoIterator<Item = PostId>) -> Result<Vec<PostId>> {
        let tx = self.db.begin_read().context("begin missing-post read")?;
        let posts = tx.open_table(POSTS).context("open missing-post table")?;
        ids.into_iter()
            .filter_map(|id| match posts.get(u64::from(id.0)) {
                Ok(Some(_)) => None,
                Ok(None) => Some(Ok(id)),
                Err(err) => Some(Err(err).context("read missing family post")),
            })
            .collect()
    }

    pub fn family_tree(&self, focus: PostId) -> Result<FamilyTree> {
        let atlas = read(&self.kin);
        let tx = self.db.begin_read().context("begin family read")?;
        let posts = tx.open_table(POSTS).context("open family posts")?;
        let children = tx
            .open_table(kin::CHILDREN)
            .context("open family children")?;
        let hints = tx
            .open_table(kin::CHILD_HINTS)
            .context("open family hints")?;
        kin::family_tree(&posts, &children, &hints, &atlas, focus)
    }

    fn absorb_harvest_with(
        &self,
        harvest: &[Harvest],
        crawl: Option<(Option<PostId>, bool)>,
    ) -> Result<()> {
        let posts = harvest.iter().map(|item| &item.post).collect::<Vec<_>>();
        let facts = harvest.iter().map(|item| item.kin).collect::<Vec<_>>();
        let mut atlas = write(&self.kin);
        let tx = self.begin_quick_write("begin harvested index write")?;
        Self::absorb_into(&tx, &posts)?;
        let mutation = kin::absorb(&tx, &facts)?;
        if let Some((before, complete)) = crawl {
            let mut table = tx.open_table(META).context("open meta")?;
            if let Some(before) = before {
                let _old = table
                    .insert(DANBOORU_CRAWL_BEFORE, u64::from(before.0))
                    .context("write Danbooru crawl cursor")?;
            }
            drop(table);
            if complete {
                kin::advance_backfill(&tx, before, true)?;
            }
        }
        tx.commit().context("commit harvested index write")?;
        atlas.apply(&mutation)?;
        self.patch_tag_bank(&posts);
        self.evict_records(&posts);
        self.clear_sort_heads();
        self.refresh_sort_keys(&posts);
        self.invalidate_favorite_ranks(&posts);
        Ok(())
    }

    fn evict_records(&self, posts: &[&PostRecord]) {
        let mut records = lock(&self.records);
        for post in posts {
            records.evict(post.id);
        }
    }

    fn clear_sort_heads(&self) {
        lock(&self.sort_heads).clear();
    }

    fn refresh_sort_keys(&self, posts: &[&PostRecord]) {
        lock(&self.sort_keys).refresh(posts);
    }

    fn invalidate_favorite_ranks(&self, posts: &[&PostRecord]) {
        lock(&self.favorite_ranks).invalidate(posts);
    }

    fn patch_tag_bank(&self, posts: &[&PostRecord]) {
        let mut slot = write(&self.tag_bank);
        let Some(bank) = slot.as_mut() else {
            return;
        };
        for post in posts.iter().filter(|post| post.indexable()) {
            for hint in &post.tag_hints {
                let _old = bank.insert(hint.tag.clone(), hint.kind);
            }
        }
    }

    fn absorb_into(tx: &redb::WriteTransaction, posts: &[&PostRecord]) -> Result<()> {
        {
            let mut post_table = tx.open_table(POSTS).context("open posts")?;
            let mut score_table = tx.open_table(SCORE_POSTS).context("open score_posts")?;
            let mut fav_table = tx.open_table(FAV_POSTS).context("open fav_posts")?;
            let mut tag_kinds = tx.open_table(TAG_KINDS).context("open tag kind table")?;
            let mut facts = FactBatch::default();

            for post in posts {
                let indexable = post.indexable();
                let old = {
                    post_table
                        .get(u64::from(post.id.0))
                        .context("read old post")?
                        .map(|guard| decode_record(guard.value()))
                        .transpose()?
                };
                if let Some(old) = old.as_ref() {
                    stage_record_delta(&mut facts, Some(old), indexable.then_some(post));
                    remove_record_core(&mut post_table, &mut score_table, &mut fav_table, old)?;
                }

                if old.is_none() {
                    stage_record_delta(&mut facts, None, indexable.then_some(post));
                }

                if !indexable {
                    continue;
                }

                let encoded = encode_record(post);
                write_tag_kinds(&mut tag_kinds, post)?;
                let _old_post = post_table
                    .insert(u64::from(post.id.0), encoded.as_slice())
                    .context("upsert post")?;
                let _old_score = score_table
                    .insert(sort_key_i32(post.score, post.id), post.id.0)
                    .context("upsert score lane")?;
                let _old_fav = fav_table
                    .insert(sort_key_u32(post.favs, post.id), post.id.0)
                    .context("upsert favorite lane")?;
            }
            if !facts.is_empty() {
                append_facts(tx, &facts)?;
            }
        }
        Ok(())
    }

    pub fn crawl_before(&self) -> Result<Option<PostId>> {
        let tx = self.db.begin_read().context("begin crawl cursor read")?;
        let table = tx.open_table(META).context("open meta")?;
        table
            .get(DANBOORU_CRAWL_BEFORE)
            .context("read Danbooru crawl cursor")?
            .map(|guard| narrow_post_id(guard.value()))
            .transpose()
    }

    pub fn tag_suggestions(&self, prefix: &str, limit: usize) -> Result<Vec<TagSuggestion>> {
        let Some(prefix) = normalize_prefix(prefix) else {
            return Ok(Vec::new());
        };
        let tx = self.db.begin_read().context("begin tag suggestion read")?;
        let chunks = tx.open_table(TAG_CHUNKS).context("open tag chunks")?;
        let kinds = tx.open_table(TAG_KINDS).context("open tag kind table")?;
        let facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
        let pending = pending_facts(&facts)?;
        let mut candidates = BTreeSet::new();
        let candidate_cap = limit.saturating_mul(32).max(limit);
        collect_chunked_tag_names(&chunks, &prefix, candidate_cap, &mut candidates)?;
        for (key, _) in pending.groups() {
            if key.lane == PostingLane::Tag && key.key.starts_with(&prefix) {
                let _inserted = candidates.insert(key.key.clone());
            }
            if candidates.len() >= candidate_cap {
                break;
            }
        }
        let mut hits = Vec::with_capacity(candidates.len());
        for tag in candidates {
            let Some(tag_atom) = Tag::forge(&tag) else {
                continue;
            };
            hits.push(TagSuggestion {
                kind: read_tag_kind(&kinds, &tag_atom)?,
                posts: tag_post_count(&chunks, &pending, &tag)?,
                tag,
            });
        }
        hits.sort_unstable_by(|a, b| b.posts.cmp(&a.posts).then_with(|| a.tag.cmp(&b.tag)));
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn tag_pattern_matches(
        &self,
        patterns: &[TagPattern],
    ) -> Result<Vec<(TagPattern, Vec<TagPatternMatch>)>> {
        self.ensure_tag_bank()?;
        let bank = read(&self.tag_bank);
        let bank = bank.as_ref().context("tag bank was not initialized")?;
        patterns
            .iter()
            .map(|pattern| {
                let matcher = pattern.matcher()?;
                let matches = bank
                    .iter()
                    .filter(|(tag, kind)| {
                        **kind == TagKind::General && matcher.is_match(tag.as_str())
                    })
                    .map(|(tag, kind)| TagPatternMatch {
                        tag: tag.clone(),
                        kind: *kind,
                    })
                    .collect::<Vec<_>>();
                Ok((pattern.clone(), matches))
            })
            .collect()
    }

    fn ensure_tag_bank(&self) -> Result<()> {
        let mut slot = write(&self.tag_bank);
        if slot.is_some() {
            return Ok(());
        }
        let tx = self.db.begin_read().context("begin tag bank read")?;
        let kinds = tx.open_table(TAG_KINDS).context("open tag kind table")?;
        let mut bank = BTreeMap::new();
        for row in kinds.iter().context("iterate tag bank")? {
            let (tag, kind) = row.context("read tag bank row")?;
            let tag = Tag::forge(tag.value()).context("indexed tag key is empty")?;
            let kind = TagKind::from_code(kind.value()).context("invalid indexed tag kind")?;
            let _old = bank.insert(tag, kind);
        }
        *slot = Some(bank);
        Ok(())
    }

    pub fn tag_kind(&self, tag: &Tag) -> Result<TagKind> {
        let tx = self.db.begin_read().context("begin tag kind read")?;
        let kinds = tx.open_table(TAG_KINDS).context("open tag kind table")?;
        read_tag_kind(&kinds, tag)
    }

    pub fn tag_kinds(&self, tags: &[Tag]) -> Result<BTreeMap<Tag, TagKind>> {
        let tx = self.db.begin_read().context("begin tag kind batch read")?;
        let kinds = tx.open_table(TAG_KINDS).context("open tag kind table")?;
        let mut out = BTreeMap::new();
        for tag in tags {
            let _old = out.insert(tag.clone(), read_tag_kind(&kinds, tag)?);
        }
        Ok(out)
    }

    pub fn stats(&self) -> Result<CacheStats> {
        startup("index.stats.enter");
        let tx = self.db.begin_read().context("begin cache stats read")?;
        startup("index.stats.tx");
        let posts = tx.open_table(POSTS).context("open posts")?;
        let tag_chunks = tx.open_table(TAG_CHUNKS).context("open tag chunks")?;
        let rating_chunks = tx.open_table(RATING_CHUNKS).context("open rating chunks")?;
        let facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
        let pending = pending_facts(&facts)?;
        let meta = tx.open_table(META).context("open meta")?;
        startup("index.stats.tables");
        let posts_len = posts.len().context("count posts")?;
        startup("index.stats.posts.len");
        let newest = posts
            .range(0_u64..=u64::MAX)
            .context("range newest post id")?
            .next_back()
            .map(|row| {
                let (id, _) = row.context("read newest post id")?;
                narrow_post_id(id.value())
            })
            .transpose()?;
        startup("index.stats.newest");
        let crawl_before = meta
            .get(DANBOORU_CRAWL_BEFORE)
            .context("read Danbooru crawl cursor")?
            .map(|guard| narrow_post_id(guard.value()))
            .transpose()?;
        startup("index.stats.crawl.before");
        let ratings = RatingClass::ALL
            .into_iter()
            .map(|rating| {
                let posts =
                    posting_count(&rating_chunks, &pending, PostingLane::Rating, rating.key())?;
                Ok((rating, posts))
            })
            .collect::<Result<Vec<_>>>()?;
        startup("index.stats.rating.bitmaps");
        Ok(CacheStats {
            posts: posts_len,
            tag_chunks: tag_chunks.len().context("count tag chunks")?,
            pending_fact_batches: facts.len().context("count posting fact batches")?,
            newest,
            crawl_before,
            ratings,
        })
    }

    pub fn merge_pending_facts(&self, budget: FactMergeBudget) -> Result<FactMerge> {
        let tx = self.begin_quick_write("begin posting fact merge")?;
        let pending = {
            let facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
            collect_pending_fact_rows(&facts, budget)?
        };
        if pending.is_empty() {
            return Ok(FactMerge::default());
        }
        let mut batch = FactBatch::default();
        let mut bytes = 0_usize;
        for (_, encoded) in &pending {
            bytes = bytes.saturating_add(encoded.len());
            batch.assimilate(FactBatch::decode(encoded)?);
        }
        let groups = batch.groups().count();
        {
            let mut tag_chunks = tx.open_table(TAG_CHUNKS).context("open tag chunks")?;
            let mut rating_chunks = tx.open_table(RATING_CHUNKS).context("open rating chunks")?;
            for (key, delta) in batch.groups() {
                match key.lane {
                    PostingLane::Tag => {
                        apply_delta_chunks(&mut tag_chunks, &key.key, delta)?;
                    }
                    PostingLane::Rating => {
                        apply_delta_chunks(&mut rating_chunks, &key.key, delta)?;
                    }
                }
                lock(&self.vault).evict(key.lane, &key.key);
            }
        }
        {
            let mut facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
            for (seq, _) in &pending {
                let _old = facts
                    .remove(*seq)
                    .with_context(|| format!("remove merged posting fact batch {seq}"))?;
            }
        }
        tx.commit().context("commit posting fact merge")?;
        Ok(FactMerge {
            batches: pending.len(),
            bytes,
            groups,
        })
    }

    #[allow(
        dead_code,
        reason = "the duplicated binary module does not see the retrieval benchmark's POSTS-only facade"
    )]
    pub fn search(
        &self,
        query: &Query,
        sort: Sort,
        dates: DateRange,
        limit: usize,
    ) -> Result<SearchHit> {
        let local_favorites = Arc::new(RoaringBitmap::new());
        self.search_topology(
            query,
            &local_favorites,
            sort,
            dates,
            GalleryTopology::Ungrouped,
            limit,
        )
    }

    pub fn search_topology(
        &self,
        query: &Query,
        local_favorites: &Arc<RoaringBitmap>,
        sort: Sort,
        dates: DateRange,
        topology: GalleryTopology,
        limit: usize,
    ) -> Result<SearchHit> {
        self.search_corpus(
            query,
            local_favorites,
            Corpus::All,
            sort,
            dates,
            topology,
            limit,
        )
    }

    pub fn search_corpus(
        &self,
        query: &Query,
        local_favorites: &Arc<RoaringBitmap>,
        corpus: Corpus,
        sort: Sort,
        dates: DateRange,
        topology: GalleryTopology,
        limit: usize,
    ) -> Result<SearchHit> {
        startup("index.search.enter");
        let dates = dates.normalized();
        let tx = self.db.begin_read().context("begin index read")?;
        startup("index.search.tx");
        let posts = tx.open_table(POSTS).context("open posts")?;
        let window = date_window(&posts, dates)?;
        if window.is_some_and(DateWindow::empty) {
            return Ok(SearchHit {
                horizon: limit,
                ..SearchHit::default()
            });
        }

        let favorite_ranks = if local_favorites.is_empty() {
            None
        } else {
            Some(self.favorite_ranks(&posts, local_favorites)?)
        };
        let available_favorites = favorite_ranks.as_ref().map_or_else(
            || Arc::clone(local_favorites),
            |ranks| Arc::clone(&ranks.available),
        );
        let candidate = self.candidate_set(&tx, query, &posts, &available_favorites, corpus)?;
        startup("index.search.candidate");
        let candidates = match &candidate {
            None => window.map_or_else(
                || posts.len().context("count posts"),
                |window| Ok(window.len()),
            ),
            Some(Candidate::Finite(bitmap)) => Ok(candidate_len(bitmap.as_ref(), window)),
            Some(Candidate::Cofinite(excluded)) => window.map_or_else(
                || {
                    posts
                        .len()
                        .context("count posts")
                        .map(|posts| posts.saturating_sub(excluded.len()))
                },
                |window| {
                    Ok(window
                        .len()
                        .saturating_sub(candidate_len(excluded.as_ref(), Some(window))))
                },
            ),
        }?;
        startup("index.search.candidates.len");

        let ids = match topology {
            GalleryTopology::Ungrouped => self.favorite_ordered_ids(
                &tx,
                &posts,
                &candidate,
                favorite_ranks.as_deref(),
                sort,
                window,
                limit,
            )?,
            GalleryTopology::Grouped => {
                let atlas = read(&self.kin);
                self.favorite_ordered_family_ids(
                    &tx,
                    &posts,
                    &atlas,
                    &candidate,
                    favorite_ranks.as_deref(),
                    sort,
                    window,
                    limit,
                )?
            }
        };
        startup("index.search.ids");

        let tail = if limit > 0 && ids.len() == limit {
            SearchTail::Open
        } else {
            SearchTail::Exhausted
        };
        let mut hydrated = Vec::with_capacity(ids.len());
        for id in ids {
            let id = PostId(id);
            if let Some(post) = lock(&self.records).get(id) {
                if post.indexable() {
                    hydrated.push(post);
                }
                continue;
            }
            if let Some(post) = posts
                .get(u64::from(id.0))
                .context("hydrate post")?
                .map(|guard| decode_record(guard.value()))
                .transpose()?
            {
                // Records predating an admission ban wash out immediately;
                // their next crawl removes the stale posting facts.
                if post.indexable() {
                    lock(&self.records).put(post.clone());
                    hydrated.push(post);
                }
            }
        }
        startup("index.search.posts.loaded");
        let mut families = BTreeMap::new();
        if topology == GalleryTopology::Grouped {
            let atlas = read(&self.kin);
            let children = tx
                .open_table(kin::CHILDREN)
                .context("open result-family children")?;
            let hints = tx
                .open_table(kin::CHILD_HINTS)
                .context("open result-family hints")?;
            let mut badges = BTreeMap::new();
            for post in &hydrated {
                let root = atlas.root(post.id);
                let badge = if let Some(badge) = badges.get(&root).copied() {
                    Some(badge)
                } else {
                    let badge = kin::family_badge(&posts, &children, &hints, &atlas, post.id)?;
                    if let Some(badge) = badge {
                        let _old = badges.insert(root, badge);
                    }
                    badge
                };
                if let Some(badge) = badge {
                    let _old = families.insert(post.id, badge);
                }
            }
        }
        Ok(SearchHit {
            posts: hydrated,
            candidates,
            families,
            horizon: limit,
            tail,
        })
    }

    fn favorite_ranks(
        &self,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        source: &Arc<RoaringBitmap>,
    ) -> Result<Arc<FavoriteRanks>> {
        if let Some(ranks) = lock(&self.favorite_ranks).get(source) {
            return Ok(ranks);
        }
        let mut available = RoaringBitmap::new();
        let mut score = Vec::with_capacity(source.len() as usize);
        let mut fav_count = Vec::with_capacity(source.len() as usize);
        for id in source.iter() {
            let Some(post) = posts
                .get(u64::from(id))
                .context("read local favorite post")?
                .map(|guard| decode_record(guard.value()))
                .transpose()?
                .filter(PostRecord::indexable)
            else {
                continue;
            };
            let _inserted = available.insert(id);
            score.push((sort_key_i32(post.score, post.id), id));
            fav_count.push((sort_key_u32(post.favs, post.id), id));
        }
        score.sort_unstable_by(|left, right| right.cmp(left));
        fav_count.sort_unstable_by(|left, right| right.cmp(left));
        let ranks = FavoriteRanks {
            source: Arc::clone(source),
            available: Arc::new(available),
            score: score.into_iter().map(|(_, id)| id).collect(),
            fav_count: fav_count.into_iter().map(|(_, id)| id).collect(),
        };
        let mut vault = lock(&self.favorite_ranks);
        if let Some(raced) = vault.get(source) {
            return Ok(raced);
        }
        Ok(vault.put(ranks))
    }

    fn favorite_ordered_ids(
        &self,
        tx: &redb::ReadTransaction,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        candidate: &Option<Candidate>,
        favorite_ranks: Option<&FavoriteRanks>,
        sort: Sort,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let Some(favorite_ranks) = favorite_ranks else {
            return self.post_ids(tx, posts, candidate, sort, window, limit);
        };
        let (favored, ordinary) = favorite_partition(candidate, &favorite_ranks.available);
        let mut ids = favorite_ids(favorite_ranks, &favored, sort, window, limit);
        if ids.len() < limit {
            ids.extend(self.post_ids(
                tx,
                posts,
                &Some(ordinary),
                sort,
                window,
                limit - ids.len(),
            )?);
        }
        Ok(ids)
    }

    fn post_ids(
        &self,
        tx: &redb::ReadTransaction,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        candidate: &Option<Candidate>,
        sort: Sort,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let ids = match (candidate, sort) {
            (None, Sort::Newest) => newest_ids(posts, window, limit)?,
            (Some(Candidate::Finite(bitmap)), Sort::Newest) => {
                newest_bitmap_ids(bitmap.as_ref(), window, limit)
            }
            (Some(candidate @ Candidate::Cofinite(_)), Sort::Newest) => {
                newest_ids_filtered(posts, candidate, window, limit)?
            }
            (None, Sort::Score | Sort::FavCount) => {
                self.ranked_ids(tx, sort, None, window, limit)?
            }
            (Some(candidate @ Candidate::Finite(bitmap)), Sort::Score)
                if bitmap.len() > SMALL_SORT =>
            {
                self.ranked_ids(tx, sort, Some(candidate), window, limit)?
            }
            (Some(candidate @ Candidate::Finite(bitmap)), Sort::FavCount)
                if bitmap.len() > SMALL_SORT =>
            {
                self.ranked_ids(tx, sort, Some(candidate), window, limit)?
            }
            (Some(candidate @ Candidate::Cofinite(_)), Sort::Score | Sort::FavCount) => {
                self.ranked_ids(tx, sort, Some(candidate), window, limit)?
            }
            (Some(Candidate::Finite(bitmap)), Sort::Score | Sort::FavCount) => {
                self.local_sorted_ids(tx, bitmap.as_ref(), sort, window, limit)?
            }
        };
        Ok(ids)
    }

    fn ranked_ids(
        &self,
        tx: &redb::ReadTransaction,
        sort: Sort,
        candidate: Option<&Candidate>,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let head = self.sort_head(tx, sort)?;
        if let Some(ids) = head_ids(&head, candidate, window, limit) {
            return Ok(ids);
        }
        match sort {
            Sort::Score => lane_ids(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                candidate,
                window,
                limit,
            ),
            Sort::FavCount => lane_ids(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                candidate,
                window,
                limit,
            ),
            Sort::Newest => unreachable!("newest is not a ranked sort lane"),
        }
    }

    fn sort_head(&self, tx: &redb::ReadTransaction, sort: Sort) -> Result<Arc<Vec<u32>>> {
        if let Some(ids) = lock(&self.sort_heads).get(sort) {
            return Ok(ids);
        }
        let ids = Arc::new(match sort {
            Sort::Score => lane_head(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                SORT_HEAD_CAP,
            )?,
            Sort::FavCount => lane_head(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                SORT_HEAD_CAP,
            )?,
            Sort::Newest => unreachable!("newest has no sort-head cache"),
        });
        let mut heads = lock(&self.sort_heads);
        if let Some(raced) = heads.get(sort) {
            return Ok(raced);
        }
        heads.put(sort, Arc::clone(&ids));
        Ok(ids)
    }

    fn local_sorted_ids(
        &self,
        tx: &redb::ReadTransaction,
        bitmap: &RoaringBitmap,
        sort: Sort,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        match sort {
            Sort::Score | Sort::FavCount => {
                let keys = self.sort_keys(tx, sort)?;
                Ok(local_sorted_ids_from_keys(bitmap, &keys, window, limit))
            }
            Sort::Newest => unreachable!("finite newest search iterates candidate IDs directly"),
        }
    }

    fn favorite_ordered_family_ids(
        &self,
        tx: &redb::ReadTransaction,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        atlas: &kin::Atlas,
        candidate: &Option<Candidate>,
        favorite_ranks: Option<&FavoriteRanks>,
        sort: Sort,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let mut page = FamilyPage::new(atlas, limit);
        let Some(favorite_ranks) = favorite_ranks else {
            self.extend_family_page(tx, posts, candidate, sort, window, &mut page)?;
            return Ok(page.ids);
        };
        let (favored, ordinary) = favorite_partition(candidate, &favorite_ranks.available);
        match sort {
            Sort::Newest => offer_family_stream(
                favorite_ranks.available.iter().rev(),
                Some(&favored),
                window,
                &mut page,
            ),
            Sort::Score | Sort::FavCount => offer_family_stream(
                favorite_ranks.ids(sort).iter().copied(),
                Some(&favored),
                window,
                &mut page,
            ),
        }
        if !page.full() {
            self.extend_family_page(tx, posts, &Some(ordinary), sort, window, &mut page)?;
        }
        Ok(page.ids)
    }

    fn extend_family_page(
        &self,
        tx: &redb::ReadTransaction,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        candidate: &Option<Candidate>,
        sort: Sort,
        window: Option<DateWindow>,
        page: &mut FamilyPage<'_>,
    ) -> Result<()> {
        match (candidate, sort) {
            (None, Sort::Newest) => newest_family_ids(posts, None, window, page),
            (Some(Candidate::Finite(bitmap)), Sort::Newest) => {
                newest_bitmap_family_ids(bitmap.as_ref(), window, page);
                Ok(())
            }
            (Some(candidate @ Candidate::Cofinite(_)), Sort::Newest) => {
                newest_family_ids(posts, Some(candidate), window, page)
            }
            (Some(Candidate::Finite(bitmap)), Sort::Score | Sort::FavCount)
                if bitmap.len() <= SMALL_SORT =>
            {
                for id in self.local_sorted_family_ids(
                    tx,
                    bitmap.as_ref(),
                    page.atlas,
                    sort,
                    window,
                    page.remaining(),
                )? {
                    if page.offer(id) {
                        break;
                    }
                }
                Ok(())
            }
            (candidate, Sort::Score | Sort::FavCount) => {
                self.extend_ranked_family_page(tx, sort, candidate.as_ref(), window, page)
            }
        }
    }

    fn extend_ranked_family_page(
        &self,
        tx: &redb::ReadTransaction,
        sort: Sort,
        candidate: Option<&Candidate>,
        window: Option<DateWindow>,
        page: &mut FamilyPage<'_>,
    ) -> Result<()> {
        let head = self.sort_head(tx, sort)?;
        offer_family_stream(head.iter().copied(), candidate, window, page);
        if page.full() {
            return Ok(());
        }
        match sort {
            Sort::Score => lane_family_ids(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                candidate,
                window,
                page,
            ),
            Sort::FavCount => lane_family_ids(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                candidate,
                window,
                page,
            ),
            Sort::Newest => unreachable!("newest is not a ranked family lane"),
        }
    }

    fn local_sorted_family_ids(
        &self,
        tx: &redb::ReadTransaction,
        bitmap: &RoaringBitmap,
        atlas: &kin::Atlas,
        sort: Sort,
        window: Option<DateWindow>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let keys = self.sort_keys(tx, sort)?;
        let mut strongest = HashMap::<u32, (u64, u32)>::new();
        let ids: Box<dyn Iterator<Item = u32> + '_> = match window {
            Some(window) => Box::new(bitmap.range(window.bounds())),
            None => Box::new(bitmap.iter()),
        };
        for id in ids {
            let Some(&key) = keys.get(id as usize) else {
                continue;
            };
            if key == 0 {
                continue;
            }
            let root = atlas.root(PostId(id)).0;
            let candidate = (key, id);
            let _best = strongest
                .entry(root)
                .and_modify(|best| *best = (*best).max(candidate))
                .or_insert(candidate);
        }
        let mut heap = BinaryHeap::<Reverse<(u64, u32)>>::with_capacity(limit + 1);
        for item in strongest.into_values() {
            if heap.len() < limit {
                heap.push(Reverse(item));
            } else if let Some(mut cold) = heap.peek_mut()
                && item > cold.0
            {
                *cold = Reverse(item);
            }
        }
        Ok(finish_sorted_heap(heap))
    }

    fn sort_keys(&self, tx: &redb::ReadTransaction, sort: Sort) -> Result<Arc<Vec<u64>>> {
        if let Some(keys) = lock(&self.sort_keys).get(sort) {
            return Ok(keys);
        }
        let keys = Arc::new(match sort {
            Sort::Score => {
                lane_sort_keys(&tx.open_table(SCORE_POSTS).context("open score_posts")?)?
            }
            Sort::FavCount => lane_sort_keys(&tx.open_table(FAV_POSTS).context("open fav_posts")?)?,
            Sort::Newest => unreachable!("newest has no dense sort-key cache"),
        });
        let mut vault = lock(&self.sort_keys);
        if let Some(raced) = vault.get(sort) {
            return Ok(raced);
        }
        vault.put(sort, Arc::clone(&keys));
        Ok(keys)
    }

    fn prime_database(db: &Database) -> Result<()> {
        if Self::schema_ready(db)? {
            startup("index.prime.schema.ready");
            return Ok(());
        }
        let mut tx = db.begin_write().context("begin schema prime")?;
        tx.set_quick_repair(true);
        {
            let _posts = tx.open_table(POSTS).context("prime posts")?;
            let _tags = tx.open_table(TAG_CHUNKS).context("prime tag chunks")?;
            let _tag_kinds = tx.open_table(TAG_KINDS).context("prime tag kinds")?;
            let _ratings = tx
                .open_table(RATING_CHUNKS)
                .context("prime rating chunks")?;
            let _facts = tx
                .open_table(POSTING_FACTS)
                .context("prime posting facts")?;
            let _score = tx.open_table(SCORE_POSTS).context("prime score_posts")?;
            let _favs = tx.open_table(FAV_POSTS).context("prime fav_posts")?;
            let _meta = tx.open_table(META).context("prime meta")?;
            let _parents = tx.open_table(kin::PARENTS).context("prime kin parents")?;
            let _children = tx.open_table(kin::CHILDREN).context("prime kin children")?;
            let _hints = tx
                .open_table(kin::CHILD_HINTS)
                .context("prime kin child hints")?;
            let _hydrated = tx
                .open_table(kin::HYDRATED)
                .context("prime hydrated families")?;
            let _kin_meta = tx.open_table(kin::META).context("prime kin meta")?;
        }
        tx.commit().context("commit schema prime")
    }

    fn schema_ready(db: &Database) -> Result<bool> {
        let tx = db.begin_read().context("begin schema read")?;
        macro_rules! open {
            ($table:expr) => {
                match tx.open_table($table) {
                    Ok(table) => drop(table),
                    Err(TableError::TableDoesNotExist(_)) => return Ok(false),
                    Err(err) => return Err(err).context("open schema table"),
                }
            };
        }
        open!(POSTS);
        open!(TAG_CHUNKS);
        open!(TAG_KINDS);
        open!(RATING_CHUNKS);
        open!(POSTING_FACTS);
        open!(SCORE_POSTS);
        open!(FAV_POSTS);
        open!(META);
        open!(kin::PARENTS);
        open!(kin::CHILDREN);
        open!(kin::CHILD_HINTS);
        open!(kin::HYDRATED);
        open!(kin::META);
        Ok(true)
    }

    fn candidate_set(
        &self,
        tx: &redb::ReadTransaction,
        query: &Query,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        local_favorites: &Arc<RoaringBitmap>,
        corpus: Corpus,
    ) -> Result<Option<Candidate>> {
        if corpus == Corpus::LocalFavorites {
            if !query.is_empty() {
                bail!("local-favorites corpus cannot carry provider predicates");
            }
            return Ok(Some(Candidate::Finite(BitmapCow::Shared(Arc::clone(
                local_favorites,
            )))));
        }
        let remote = if query.is_empty() {
            None
        } else {
            let patterns = query.tag_patterns().into_iter().collect::<Vec<_>>();
            let pattern_matches = if patterns.is_empty() {
                HashMap::new()
            } else {
                self.tag_pattern_matches(&patterns)?
                    .into_iter()
                    .map(|(pattern, matches)| {
                        (
                            pattern,
                            matches.into_iter().map(|matched| matched.tag).collect(),
                        )
                    })
                    .collect::<HashMap<_, Vec<_>>>()
            };
            let tag_chunks = tx.open_table(TAG_CHUNKS).context("open tag chunks")?;
            let rating_chunks = tx.open_table(RATING_CHUNKS).context("open rating chunks")?;
            let facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
            let pending = pending_facts(&facts)?;
            Some(
                BitmapEval {
                    posts,
                    tags: &tag_chunks,
                    ratings: &rating_chunks,
                    pending: &pending,
                    vault: &self.vault,
                    pattern_matches: &pattern_matches,
                    universe: None,
                }
                .eval(query.root())?,
            )
        };
        Ok(remote)
    }

    /// Write transactions default to non-durable commits — the index is a
    /// cache, and fsync per crawl page is the single largest write cost. An
    /// anchor commit goes durable every [`ANCHOR_GAP`] to bound replay.
    fn begin_quick_write(&self, context: &'static str) -> Result<redb::WriteTransaction> {
        let mut tx = self.db.begin_write().context(context)?;
        tx.set_quick_repair(true);
        let anchor_due = {
            let mut anchor = match self.anchor.lock() {
                Ok(anchor) => anchor,
                Err(poisoned) => poisoned.into_inner(),
            };
            anchor.elapsed() >= ANCHOR_GAP && {
                *anchor = Instant::now();
                true
            }
        };
        if !anchor_due && let Err(err) = tx.set_durability(redb::Durability::None) {
            return Err(anyhow::anyhow!("set commit durability: {err}"));
        }
        Ok(tx)
    }
}

#[derive(Clone, Debug)]
enum Candidate {
    Finite(BitmapCow),
    Cofinite(BitmapCow),
}

#[derive(Clone, Debug)]
enum BitmapCow {
    Shared(Arc<RoaringBitmap>),
    Owned(RoaringBitmap),
}

impl BitmapCow {
    fn as_ref(&self) -> &RoaringBitmap {
        match self {
            Self::Shared(bitmap) => bitmap,
            Self::Owned(bitmap) => bitmap,
        }
    }

    fn into_owned(self) -> RoaringBitmap {
        match self {
            Self::Shared(bitmap) => (*bitmap).clone(),
            Self::Owned(bitmap) => bitmap,
        }
    }

    fn len(&self) -> u64 {
        self.as_ref().len()
    }

    fn contains(&self, id: u32) -> bool {
        self.as_ref().contains(id)
    }
}

impl Candidate {
    fn contains(&self, id: u32) -> bool {
        match self {
            Self::Finite(bitmap) => bitmap.contains(id),
            Self::Cofinite(excluded) => !excluded.contains(id),
        }
    }

    fn complement(self) -> Self {
        match self {
            Self::Finite(bitmap) => Self::Cofinite(bitmap),
            Self::Cofinite(excluded) => Self::Finite(excluded),
        }
    }

    fn materialize(self, universe: &RoaringBitmap) -> RoaringBitmap {
        match self {
            Self::Finite(bitmap) => bitmap.into_owned(),
            Self::Cofinite(excluded) => {
                let mut bitmap = universe.clone();
                bitmap -= excluded.as_ref();
                bitmap
            }
        }
    }
}

fn favorite_partition(
    candidate: &Option<Candidate>,
    local_favorites: &Arc<RoaringBitmap>,
) -> (Candidate, Candidate) {
    let favored = Candidate::Finite(BitmapCow::Shared(Arc::clone(local_favorites)));
    let ordinary = Candidate::Cofinite(BitmapCow::Shared(Arc::clone(local_favorites)));
    match candidate {
        Some(candidate) => (
            conjunction(vec![candidate.clone(), favored]),
            conjunction(vec![candidate.clone(), ordinary]),
        ),
        None => (favored, ordinary),
    }
}

#[derive(Clone, Copy, Debug)]
struct DateWindow {
    lo: u32,
    hi: u32,
}

impl DateWindow {
    fn empty(self) -> bool {
        self.lo > self.hi
    }

    fn contains(self, id: u32) -> bool {
        self.lo <= id && id <= self.hi
    }

    fn len(self) -> u64 {
        if self.empty() {
            0
        } else {
            u64::from(self.hi - self.lo) + 1
        }
    }

    fn bounds(self) -> std::ops::RangeInclusive<u32> {
        self.lo..=self.hi
    }
}

/// Which end of the date run a binary search is hunting for.
#[derive(Clone, Copy)]
enum Edge {
    Lower, // the smallest id whose day is ≥ the target
    Upper, // the largest id whose day is ≤ the target
}

/// Resolve a date range to the contiguous id window covering it. Danbooru ids
/// climb monotonically with `created_at`, so the posts whose day falls in
/// [first, last] form one unbroken id run; we bracket its ends by binary
/// searching the POSTS keyspace, peeking each probe's day. No chronology side
/// table — the ordering of POSTS itself *is* the chronology. (A backdated post
/// can land a hair off; the candidate intersection still filters the rest.)
fn date_window(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    dates: DateRange,
) -> Result<Option<DateWindow>> {
    let dates = dates.normalized();
    if !dates.active() {
        return Ok(None);
    }
    let empty = DateWindow { lo: 1, hi: 0 };
    let Some(max_id) = posts
        .last()
        .context("read last post")?
        .map(|(id, _)| id.value())
    else {
        return Ok(Some(empty)); // no posts at all
    };
    let first_day = dates.first.map_or(0, CreatedDay::get);
    let last_day = dates.last.map_or(u32::MAX, CreatedDay::get);
    if first_day > last_day {
        return Ok(Some(empty));
    }
    let lo = day_bound(posts, max_id, first_day, Edge::Lower)?;
    let hi = day_bound(posts, max_id, last_day, Edge::Upper)?;
    Ok(Some(match (lo, hi) {
        (Some(lo), Some(hi)) if lo <= hi => DateWindow { lo, hi },
        _ => empty,
    }))
}

/// Binary search the POSTS keyspace for the `edge` of the run of posts on day
/// `target`, peeking each probe's day. Probe ranges are clamped to the live
/// `[lo, hi]` value window so the search always makes progress on a sparse,
/// gap-riddled keyspace.
fn day_bound(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    max_id: u64,
    target: u32,
    edge: Edge,
) -> Result<Option<u32>> {
    let (mut lo, mut hi) = (0_u64, max_id);
    let mut answer = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let probe = match edge {
            Edge::Lower => posts.range(mid..=hi).context("probe up")?.next(),
            Edge::Upper => posts.range(lo..=mid).context("probe down")?.next_back(),
        }
        .transpose()
        .context("read probe post")?;
        let Some((id, blob)) = probe else {
            // No live post on this side of the midpoint; collapse toward it.
            match edge {
                Edge::Lower => match mid.checked_sub(1) {
                    Some(next) => hi = next,
                    None => break,
                },
                Edge::Upper => lo = mid + 1,
            }
            continue;
        };
        let pid = id.value();
        let day = record_day(blob.value()).map(CreatedDay::get);
        let hit = match edge {
            Edge::Lower => day.is_some_and(|day| day >= target),
            Edge::Upper => day.is_some_and(|day| day <= target),
        };
        match (edge, hit) {
            (Edge::Lower, true) | (Edge::Upper, false) => {
                if hit {
                    answer = Some(pid);
                }
                match pid.checked_sub(1) {
                    Some(next) => hi = next,
                    None => break,
                }
            }
            (Edge::Lower, false) | (Edge::Upper, true) => {
                if hit {
                    answer = Some(pid);
                }
                lo = pid + 1;
            }
        }
    }
    Ok(answer.and_then(|id| u32::try_from(id).ok()))
}

struct BitmapEval<'a, P, B>
where
    P: redb::ReadableTable<u64, &'static [u8]>,
    B: redb::ReadableTable<&'static str, &'static [u8]>,
{
    posts: &'a P,
    tags: &'a B,
    ratings: &'a B,
    pending: &'a FactBatch,
    vault: &'a Mutex<Vault>,
    pattern_matches: &'a HashMap<TagPattern, Vec<Tag>>,
    universe: Option<RoaringBitmap>,
}

impl<P, B> BitmapEval<'_, P, B>
where
    P: redb::ReadableTable<u64, &'static [u8]>,
    B: redb::ReadableTable<&'static str, &'static [u8]>,
{
    fn eval(&mut self, expr: &QueryExpr) -> Result<Candidate> {
        match expr {
            QueryExpr::Atom { atom } => self.atom(atom),
            QueryExpr::Not { child } => self.eval(child).map(Candidate::complement),
            QueryExpr::Group { group } => match group.op {
                BoolOp::And => group
                    .children
                    .iter()
                    .map(|child| self.eval(child))
                    .collect::<Result<Vec<_>>>()
                    .map(conjunction),
                BoolOp::Or => group
                    .children
                    .iter()
                    .map(|child| self.eval(child))
                    .collect::<Result<Vec<_>>>()
                    .map(disjunction),
                BoolOp::Xor => self.exactly_one(&group.children),
            },
        }
    }

    fn atom(&mut self, atom: &QueryAtom) -> Result<Candidate> {
        match atom {
            QueryAtom::Tag(tag) => read_posting_bitmap(
                self.tags,
                self.pending,
                self.vault,
                PostingLane::Tag,
                tag.as_str(),
            )
            .map(Candidate::Finite),
            QueryAtom::Rating(rating) => read_posting_bitmap(
                self.ratings,
                self.pending,
                self.vault,
                PostingLane::Rating,
                rating.key(),
            )
            .map(Candidate::Finite),
            QueryAtom::Pattern(pattern) => {
                let mut union = RoaringBitmap::new();
                let matches = self
                    .pattern_matches
                    .get(pattern)
                    .context("query tag pattern was not resolved")?;
                for tag in matches {
                    union |= read_posting_bitmap(
                        self.tags,
                        self.pending,
                        self.vault,
                        PostingLane::Tag,
                        tag.as_str(),
                    )?
                    .as_ref();
                }
                Ok(Candidate::Finite(BitmapCow::Owned(union)))
            }
        }
    }

    fn universe(&mut self) -> Result<RoaringBitmap> {
        if let Some(universe) = &self.universe {
            return Ok(universe.clone());
        }
        let universe = all_post_ids(self.posts)?;
        self.universe = Some(universe.clone());
        Ok(universe)
    }

    fn exactly_one(&mut self, children: &[QueryExpr]) -> Result<Candidate> {
        let children = children
            .iter()
            .map(|child| self.eval(child))
            .collect::<Result<Vec<_>>>()?;
        if children
            .iter()
            .all(|child| matches!(child, Candidate::Finite(_)))
        {
            return Ok(Candidate::Finite(BitmapCow::Owned(exactly_one(
                children.into_iter().filter_map(|child| match child {
                    Candidate::Finite(bitmap) => Some(bitmap),
                    Candidate::Cofinite(_) => None,
                }),
            ))));
        }
        let universe = self.universe()?;
        Ok(Candidate::Finite(BitmapCow::Owned(exactly_one(
            children
                .into_iter()
                .map(|child| BitmapCow::Owned(child.materialize(&universe))),
        ))))
    }
}

fn conjunction(children: Vec<Candidate>) -> Candidate {
    let mut finite = Vec::<BitmapCow>::new();
    let mut excluded = RoaringBitmap::new();
    for child in children {
        match child {
            Candidate::Finite(bitmap) => finite.push(bitmap),
            Candidate::Cofinite(bitmap) => excluded |= bitmap.as_ref(),
        }
    }
    finite.sort_unstable_by_key(BitmapCow::len);
    let mut finite = finite.into_iter();
    match finite.next() {
        Some(bitmap) => {
            let mut bitmap = bitmap.into_owned();
            for child in finite {
                bitmap &= child.as_ref();
            }
            bitmap -= excluded;
            Candidate::Finite(BitmapCow::Owned(bitmap))
        }
        None => Candidate::Cofinite(BitmapCow::Owned(excluded)),
    }
}

fn disjunction(children: Vec<Candidate>) -> Candidate {
    let mut finite = RoaringBitmap::new();
    let mut cofinite = None::<RoaringBitmap>;
    for child in children {
        match child {
            Candidate::Finite(bitmap) => finite |= bitmap.as_ref(),
            Candidate::Cofinite(excluded) => match &mut cofinite {
                Some(acc) => *acc &= excluded.as_ref(),
                None => cofinite = Some(excluded.into_owned()),
            },
        }
    }
    match cofinite {
        Some(mut excluded) => {
            excluded -= finite;
            Candidate::Cofinite(BitmapCow::Owned(excluded))
        }
        None => Candidate::Finite(BitmapCow::Owned(finite)),
    }
}

fn remove_record_core(
    post_table: &mut redb::Table<'_, u64, &[u8]>,
    score_table: &mut redb::Table<'_, u64, u32>,
    fav_table: &mut redb::Table<'_, u64, u32>,
    post: &PostRecord,
) -> Result<()> {
    let _old_post = post_table
        .remove(u64::from(post.id.0))
        .context("remove post record")?;
    let _old_score = score_table
        .remove(sort_key_i32(post.score, post.id))
        .context("remove score lane")?;
    let _old_fav = fav_table
        .remove(sort_key_u32(post.favs, post.id))
        .context("remove favorite lane")?;
    Ok(())
}

fn stage_record_delta(facts: &mut FactBatch, old: Option<&PostRecord>, new: Option<&PostRecord>) {
    let Some(record) = old.or(new) else {
        return;
    };
    let id = record.id;
    // Presence in POSTS is proof that the old record was admitted under the
    // then-current rules. Subtract it without reapplying today's predicate.
    let old_tags = old.map_or_else(BTreeSet::new, record_tags);
    let new_tags = indexed_tags(new);
    for tag in old_tags.difference(&new_tags) {
        facts.del(PostingLane::Tag, tag, id);
    }
    for tag in new_tags.difference(&old_tags) {
        facts.add(PostingLane::Tag, tag, id);
    }

    let old_rating = old.and_then(|post| post.rating.class());
    let new_rating = indexed_rating(new);
    if old_rating != new_rating {
        if let Some(rating) = old_rating {
            facts.del(PostingLane::Rating, rating.key(), id);
        }
        if let Some(rating) = new_rating {
            facts.add(PostingLane::Rating, rating.key(), id);
        }
    }
}

fn record_tags(post: &PostRecord) -> BTreeSet<String> {
    post.tags.iter().map(ToString::to_string).collect()
}

fn indexed_tags(post: Option<&PostRecord>) -> BTreeSet<String> {
    post.filter(|post| post.indexable())
        .map_or_else(BTreeSet::new, record_tags)
}

fn indexed_rating(post: Option<&PostRecord>) -> Option<RatingClass> {
    post.filter(|post| post.indexable())
        .and_then(|post| post.rating.class())
}

fn append_facts(tx: &redb::WriteTransaction, facts: &FactBatch) -> Result<()> {
    let mut table = tx.open_table(POSTING_FACTS).context("open posting facts")?;
    let mut meta = tx.open_table(META).context("open meta")?;
    let seq = meta
        .get(POSTING_FACT_NEXT_SEQ)
        .context("read posting fact sequence")?
        .map_or(1, |seq| seq.value());
    let bytes = facts.encode()?;
    let _old = table
        .insert(seq, bytes.as_slice())
        .with_context(|| format!("append posting fact batch {seq}"))?;
    let _old_seq = meta
        .insert(POSTING_FACT_NEXT_SEQ, seq.saturating_add(1))
        .context("advance posting fact sequence")?;
    Ok(())
}

fn pending_facts(table: &impl redb::ReadableTable<u64, &'static [u8]>) -> Result<FactBatch> {
    let mut out = FactBatch::default();
    for row in table
        .range(0_u64..=u64::MAX)
        .context("range pending posting facts")?
    {
        let (_, bytes) = row.context("read pending posting fact")?;
        out.assimilate(FactBatch::decode(bytes.value())?);
    }
    Ok(out)
}

fn collect_pending_fact_rows(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    budget: FactMergeBudget,
) -> Result<Vec<(u64, Vec<u8>)>> {
    let mut rows = Vec::new();
    let mut bytes = 0_usize;
    for row in table
        .range(0_u64..=u64::MAX)
        .context("range posting facts for merge")?
    {
        if rows.len() >= budget.batches.max(1) || bytes >= budget.bytes.max(1) {
            break;
        }
        let (seq, encoded) = row.context("read posting fact merge row")?;
        let encoded = encoded.value().to_vec();
        bytes = bytes.saturating_add(encoded.len());
        rows.push((seq.value(), encoded));
    }
    Ok(rows)
}

fn apply_delta_chunks(
    table: &mut redb::Table<'_, &str, &[u8]>,
    key: &str,
    delta: &posting::Delta,
) -> Result<()> {
    let mut chunks = touched_chunks(&delta.add);
    chunks.extend(touched_chunks(&delta.del));
    for chunk in chunks {
        let chunk_key = chunk_key(key, chunk);
        let mut bitmap = read_chunk_row(table, &chunk_key)?.unwrap_or_default();
        let incoming_add = restrict_chunk(&delta.add, chunk);
        let incoming_del = restrict_chunk(&delta.del, chunk);
        bitmap -= &incoming_del;
        bitmap |= &incoming_add;
        put_chunk(table, &chunk_key, &bitmap)?;
    }
    Ok(())
}

fn put_chunk(
    table: &mut redb::Table<'_, &str, &[u8]>,
    key: &str,
    bitmap: &RoaringBitmap,
) -> Result<()> {
    if bitmap.is_empty() {
        let _old = table
            .remove(key)
            .with_context(|| format!("remove empty posting chunk {key:?}"))?;
    } else {
        let bytes = posting::bitmap_encode(bitmap)?;
        let _old = table
            .insert(key, bytes.as_slice())
            .with_context(|| format!("write posting chunk {key:?}"))?;
    }
    Ok(())
}

fn touched_chunks(bitmap: &RoaringBitmap) -> BTreeSet<u32> {
    bitmap.iter().map(chunk_of).collect()
}

fn restrict_chunk(bitmap: &RoaringBitmap, chunk: u32) -> RoaringBitmap {
    let start = chunk << CHUNK_BITS;
    let end = start.saturating_add((1 << CHUNK_BITS) - 1);
    bitmap.range(start..=end).collect()
}

fn chunk_of(id: u32) -> u32 {
    id >> CHUNK_BITS
}

fn chunk_key(key: &str, chunk: u32) -> String {
    format!("{key}\0{chunk:08x}")
}

fn chunk_prefix(key: &str) -> String {
    format!("{key}\0")
}

fn write_tag_kinds(table: &mut redb::Table<'_, &str, u8>, post: &PostRecord) -> Result<()> {
    for hint in &post.tag_hints {
        let _old = table
            .insert(hint.tag.as_str(), hint.kind.code())
            .with_context(|| format!("write tag kind {}", hint.tag))?;
    }
    Ok(())
}

fn read_tag_kind(table: &impl redb::ReadableTable<&'static str, u8>, tag: &Tag) -> Result<TagKind> {
    let kind = table
        .get(tag.as_str())
        .with_context(|| format!("read tag kind {tag}"))?
        .and_then(|guard| TagKind::from_code(guard.value()))
        .unwrap_or_default();
    Ok(kind)
}

/// Posting cardinality without materializing containers: chunk cardinalities
/// come straight from the serialized headers. Keys with pending (unmerged)
/// deltas fall back to a full decode — exactness over speed, and they are few.
fn posting_count(
    chunks: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    pending: &FactBatch,
    lane: PostingLane,
    key: &str,
) -> Result<u64> {
    if pending.group(lane, key).is_some() {
        let mut bitmap = read_chunk_bitmap(chunks, key)?;
        if let Some(delta) = pending.group(lane, key) {
            bitmap -= &delta.del;
            bitmap |= &delta.add;
        }
        return Ok(bitmap.len());
    }
    let mut total = 0_u64;
    let prefix = chunk_prefix(key);
    for row in chunks
        .range(prefix.as_str()..)
        .with_context(|| format!("range chunk counts {key}"))?
    {
        let (chunk_key, bytes) = row.context("read chunk count row")?;
        if !chunk_key.value().starts_with(&prefix) {
            break;
        }
        total += match posting::serialized_cardinality(bytes.value()) {
            Some(count) => count,
            None => posting::bitmap_decode(bytes.value())?.len(),
        };
    }
    Ok(total)
}

fn tag_post_count(
    chunks: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    pending: &FactBatch,
    tag: &str,
) -> Result<u64> {
    posting_count(chunks, pending, PostingLane::Tag, tag)
}

fn lock<T>(vault: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match vault.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn read_posting_bitmap(
    chunks: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    pending: &FactBatch,
    vault: &Mutex<Vault>,
    lane: PostingLane,
    key: &str,
) -> Result<BitmapCow> {
    let cached = lock(vault).get(lane, key);
    let base = if let Some(base) = cached {
        base
    } else {
        let decoded = read_chunk_bitmap(chunks, key)?;
        lock(vault).put(lane, key, decoded)
    };
    if let Some(delta) = pending.group(lane, key) {
        let mut bitmap = (*base).clone();
        bitmap -= &delta.del;
        bitmap |= &delta.add;
        return Ok(BitmapCow::Owned(bitmap));
    }
    Ok(BitmapCow::Shared(base))
}

fn read_chunk_row(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<Option<RoaringBitmap>> {
    table
        .get(key)
        .context("read bitmap")?
        .map(|guard| posting::bitmap_decode(guard.value()))
        .transpose()
}

fn read_chunk_bitmap(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<RoaringBitmap> {
    let mut out = RoaringBitmap::new();
    let prefix = chunk_prefix(key);
    for row in table
        .range(prefix.as_str()..)
        .with_context(|| format!("range chunked bitmap {key}"))?
    {
        let (chunk_key, bytes) = row.context("read chunked bitmap row")?;
        if !chunk_key.value().starts_with(&prefix) {
            break;
        }
        out |= posting::bitmap_decode(bytes.value())?;
    }
    Ok(out)
}

fn collect_chunked_tag_names(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    prefix: &str,
    cap: usize,
    out: &mut BTreeSet<String>,
) -> Result<()> {
    for row in table
        .range(prefix..)
        .with_context(|| format!("range chunked tag names {prefix}"))?
    {
        let (key, _) = row.context("read chunked tag name")?;
        let key = key.value();
        if !key.starts_with(prefix) {
            break;
        }
        let Some((tag, _chunk)) = key.split_once('\0') else {
            continue;
        };
        if tag.starts_with(prefix) {
            let _inserted = out.insert(tag.to_owned());
        }
        if out.len() >= cap {
            break;
        }
    }
    Ok(())
}

fn exactly_one(children: impl IntoIterator<Item = BitmapCow>) -> RoaringBitmap {
    let mut exactly = RoaringBitmap::new();
    let mut repeated = RoaringBitmap::new();
    for child in children {
        let child = child.as_ref();
        let overlap = &exactly & child;
        repeated |= overlap;
        exactly ^= child;
        exactly -= &repeated;
    }
    exactly
}

fn newest_ids(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    window: Option<DateWindow>,
    limit: usize,
) -> Result<Vec<u32>> {
    let range = window.map_or(0_u64..=u64::MAX, |window| {
        u64::from(window.lo)..=u64::from(window.hi)
    });
    table
        .range(range)
        .context("range posts")?
        .rev()
        .take(limit)
        .map(|row| {
            row.map(|(id, _)| id.value() as u32)
                .context("read newest row")
        })
        .collect()
}

fn newest_bitmap_ids(bitmap: &RoaringBitmap, window: Option<DateWindow>, limit: usize) -> Vec<u32> {
    match window {
        Some(window) => bitmap.range(window.bounds()).rev().take(limit).collect(),
        None => bitmap.iter().rev().take(limit).collect(),
    }
}

fn favorite_ids(
    ranks: &FavoriteRanks,
    candidate: &Candidate,
    sort: Sort,
    window: Option<DateWindow>,
    limit: usize,
) -> Vec<u32> {
    let accept =
        |id: &u32| window.is_none_or(|window| window.contains(*id)) && candidate.contains(*id);
    match sort {
        Sort::Newest => ranks
            .available
            .iter()
            .rev()
            .filter(accept)
            .take(limit)
            .collect(),
        Sort::Score | Sort::FavCount => ranks
            .ids(sort)
            .iter()
            .copied()
            .filter(accept)
            .take(limit)
            .collect(),
    }
}

fn newest_ids_filtered(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    candidate: &Candidate,
    window: Option<DateWindow>,
    limit: usize,
) -> Result<Vec<u32>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let range = window.map_or(0_u64..=u64::MAX, |window| {
        u64::from(window.lo)..=u64::from(window.hi)
    });
    let mut ids = Vec::with_capacity(limit);
    for row in table
        .range(range)
        .context("range filtered newest posts")?
        .rev()
    {
        let (id, _) = row.context("read filtered newest row")?;
        let id = u32::try_from(id.value()).context("post id exceeds roaring bitmap range")?;
        if candidate.contains(id) {
            ids.push(id);
            if ids.len() == limit {
                break;
            }
        }
    }
    Ok(ids)
}

struct FamilyPage<'a> {
    atlas: &'a kin::Atlas,
    seen: RoaringBitmap,
    ids: Vec<u32>,
    limit: usize,
}

impl<'a> FamilyPage<'a> {
    fn new(atlas: &'a kin::Atlas, limit: usize) -> Self {
        Self {
            atlas,
            seen: RoaringBitmap::new(),
            ids: Vec::with_capacity(limit),
            limit,
        }
    }

    fn offer(&mut self, id: u32) -> bool {
        let root = self.atlas.root(PostId(id)).0;
        if self.seen.insert(root) {
            self.ids.push(id);
        }
        self.full()
    }

    fn full(&self) -> bool {
        self.ids.len() >= self.limit
    }

    fn remaining(&self) -> usize {
        self.limit - self.ids.len()
    }
}

fn offer_family_stream(
    ids: impl IntoIterator<Item = u32>,
    candidate: Option<&Candidate>,
    window: Option<DateWindow>,
    page: &mut FamilyPage<'_>,
) {
    if page.full() {
        return;
    }
    for id in ids {
        if window.is_none_or(|window| window.contains(id))
            && candidate.is_none_or(|candidate| candidate.contains(id))
            && page.offer(id)
        {
            break;
        }
    }
}

fn newest_family_ids(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    candidate: Option<&Candidate>,
    window: Option<DateWindow>,
    page: &mut FamilyPage<'_>,
) -> Result<()> {
    if page.full() {
        return Ok(());
    }
    let range = window.map_or(0_u64..=u64::MAX, |window| {
        u64::from(window.lo)..=u64::from(window.hi)
    });
    for row in table
        .range(range)
        .context("range family newest posts")?
        .rev()
    {
        let (id, _) = row.context("read family newest row")?;
        let id = u32::try_from(id.value()).context("family post id exceeds u32")?;
        if candidate.is_none_or(|candidate| candidate.contains(id)) && page.offer(id) {
            break;
        }
    }
    Ok(())
}

fn newest_bitmap_family_ids(
    bitmap: &RoaringBitmap,
    window: Option<DateWindow>,
    page: &mut FamilyPage<'_>,
) {
    if page.full() {
        return;
    }
    match window {
        Some(window) => {
            for id in bitmap.range(window.bounds()).rev() {
                if page.offer(id) {
                    break;
                }
            }
        }
        None => {
            for id in bitmap.iter().rev() {
                if page.offer(id) {
                    break;
                }
            }
        }
    }
}

fn all_post_ids(table: &impl redb::ReadableTable<u64, &'static [u8]>) -> Result<RoaringBitmap> {
    let mut bitmap = RoaringBitmap::new();
    for row in table.range(0_u64..=u64::MAX).context("range all posts")? {
        let (id, _) = row.context("read all-post row")?;
        let id = u32::try_from(id.value()).context("post id exceeds roaring bitmap range")?;
        let _inserted = bitmap.insert(id);
    }
    Ok(bitmap)
}

fn lane_ids(
    table: &impl redb::ReadableTable<u64, u32>,
    candidate: Option<&Candidate>,
    window: Option<DateWindow>,
    limit: usize,
) -> Result<Vec<u32>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut ids = Vec::with_capacity(limit);
    for row in table
        .range(0_u64..=u64::MAX)
        .context("range sort lane")?
        .rev()
    {
        let (_, id) = row.context("read sort row")?;
        let id = id.value();
        if window.is_none_or(|window| window.contains(id))
            && candidate.is_none_or(|candidate| candidate.contains(id))
        {
            ids.push(id);
            if ids.len() == limit {
                break;
            }
        }
    }
    Ok(ids)
}

fn lane_family_ids(
    table: &impl redb::ReadableTable<u64, u32>,
    candidate: Option<&Candidate>,
    window: Option<DateWindow>,
    page: &mut FamilyPage<'_>,
) -> Result<()> {
    if page.full() {
        return Ok(());
    }
    for row in table
        .range(0_u64..=u64::MAX)
        .context("range family sort lane")?
        .rev()
    {
        let (_, id) = row.context("read family sort row")?;
        let id = id.value();
        if window.is_none_or(|window| window.contains(id))
            && candidate.is_none_or(|candidate| candidate.contains(id))
            && page.offer(id)
        {
            break;
        }
    }
    Ok(())
}

fn lane_head(table: &impl redb::ReadableTable<u64, u32>, cap: usize) -> Result<Vec<u32>> {
    table
        .range(0_u64..=u64::MAX)
        .context("range sort head")?
        .rev()
        .take(cap)
        .map(|row| {
            let (_, id) = row.context("read sort-head row")?;
            Ok(id.value())
        })
        .collect()
}

fn head_ids(
    head: &[u32],
    candidate: Option<&Candidate>,
    window: Option<DateWindow>,
    limit: usize,
) -> Option<Vec<u32>> {
    if limit == 0 {
        return Some(Vec::new());
    }
    let mut ids = Vec::with_capacity(limit);
    for id in head {
        if window.is_none_or(|window| window.contains(*id))
            && candidate.is_none_or(|candidate| candidate.contains(*id))
        {
            ids.push(*id);
            if ids.len() == limit {
                return Some(ids);
            }
        }
    }
    (candidate.is_none() && window.is_none() && ids.len() == limit).then_some(ids)
}
fn lane_sort_keys(table: &impl redb::ReadableTable<u64, u32>) -> Result<Vec<u64>> {
    let mut keys = Vec::new();
    for row in table.range(0_u64..=u64::MAX).context("range sort keys")? {
        let (key, id) = row.context("read sort-key row")?;
        set_sort_key(&mut keys, PostId(id.value()), Some(key.value()));
    }
    Ok(keys)
}
fn set_sort_key(keys: &mut Vec<u64>, id: PostId, key: Option<u64>) {
    let slot = id.0 as usize;
    if keys.len() <= slot {
        keys.resize(slot + 1, 0);
    }
    keys[slot] = key.unwrap_or(0);
}
fn local_sorted_ids_from_keys(
    bitmap: &RoaringBitmap,
    keys: &[u64],
    window: Option<DateWindow>,
    limit: usize,
) -> Vec<u32> {
    if limit == 0 {
        return Vec::new();
    }
    let mut heap = BinaryHeap::<Reverse<(u64, u32)>>::with_capacity(limit + 1);
    match window {
        Some(window) => push_sorted_ids(bitmap.range(window.bounds()), keys, limit, &mut heap),
        None => push_sorted_ids(bitmap.iter(), keys, limit, &mut heap),
    }
    finish_sorted_heap(heap)
}

fn push_sorted_ids(
    ids: impl IntoIterator<Item = u32>,
    keys: &[u64],
    limit: usize,
    heap: &mut BinaryHeap<Reverse<(u64, u32)>>,
) {
    for id in ids {
        let Some(&key) = keys.get(id as usize) else {
            continue;
        };
        if key == 0 {
            continue;
        }
        let item = (key, id);
        if heap.len() < limit {
            heap.push(Reverse(item));
        } else if let Some(mut cold) = heap.peek_mut()
            && item > cold.0
        {
            *cold = Reverse(item);
        }
    }
}

fn finish_sorted_heap(heap: BinaryHeap<Reverse<(u64, u32)>>) -> Vec<u32> {
    let mut keyed = heap
        .into_iter()
        .map(|Reverse(item)| item)
        .collect::<Vec<_>>();
    keyed.sort_unstable_by(|a, b| b.cmp(a));
    keyed.into_iter().map(|(_, id)| id).collect()
}

fn candidate_len(bitmap: &RoaringBitmap, window: Option<DateWindow>) -> u64 {
    match window {
        Some(window) => bitmap.range(window.bounds()).count() as u64,
        None => bitmap.len(),
    }
}
fn sort_key_i32(score: i32, id: PostId) -> u64 {
    let shifted = (i64::from(score) - i64::from(i32::MIN)) as u64;
    (shifted << 32) | u64::from(id.0)
}
fn sort_key_u32(count: u32, id: PostId) -> u64 {
    (u64::from(count) << 32) | u64::from(id.0)
}
fn normalize_prefix(prefix: &str) -> Option<String> {
    let prefix = prefix.trim().to_ascii_lowercase().replace(' ', "_");
    (!prefix.is_empty()).then_some(prefix)
}

fn newest_slot(db: &Database) -> Result<usize> {
    let tx = db.begin_read().context("begin newest-slot read")?;
    let posts = tx.open_table(POSTS).context("open newest-slot posts")?;
    posts
        .last()
        .context("read newest-slot post")?
        .map_or(Ok(0), |(id, _)| {
            usize::try_from(id.value())
                .context("newest post id exceeds usize")?
                .checked_add(1)
                .context("newest post slot overflow")
        })
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
