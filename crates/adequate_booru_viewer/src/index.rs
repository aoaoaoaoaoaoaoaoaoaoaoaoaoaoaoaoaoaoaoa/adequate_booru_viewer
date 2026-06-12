use anyhow::{Context as _, Result};
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, ReadableTableMetadata as _,
    TableDefinition, TableError,
};
use roaring::RoaringBitmap;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::model::{
    BoolOp, PostId, PostRecord, Query, QueryAtom, QueryExpr, RatingClass, SearchHit, Sort, Tag,
    TagKind, decode_record, decode_sort_keys, encode_record, narrow_post_id,
};
use crate::posting::{self, Batch as FactBatch, Lane as PostingLane};
use crate::trace::startup;

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
const QUICK_REPAIR_V1: &str = "redb.quick_repair.v1";
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
    slots: std::collections::HashMap<posting::Key, (Arc<RoaringBitmap>, u64)>,
    clock: u64,
}

impl Vault {
    fn new() -> Self {
        Self {
            slots: std::collections::HashMap::new(),
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
    slots: std::collections::HashMap<PostId, (PostRecord, u64)>,
    clock: u64,
}

impl RecordVault {
    fn new() -> Self {
        Self {
            slots: std::collections::HashMap::new(),
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
    newest: Option<Arc<Vec<u32>>>,
    score: Option<Arc<Vec<u32>>>,
    favs: Option<Arc<Vec<u32>>>,
}

impl SortHeadVault {
    fn get(&self, sort: Sort) -> Option<Arc<Vec<u32>>> {
        match sort {
            Sort::Newest => self.newest.clone(),
            Sort::Score => self.score.clone(),
            Sort::Favorites => self.favs.clone(),
        }
    }

    fn put(&mut self, sort: Sort, ids: Arc<Vec<u32>>) {
        match sort {
            Sort::Newest => self.newest = Some(ids),
            Sort::Score => self.score = Some(ids),
            Sort::Favorites => self.favs = Some(ids),
        }
    }

    fn clear(&mut self) {
        self.newest = None;
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
            Sort::Favorites => self.favs.clone(),
            Sort::Newest => None,
        }
    }

    fn put(&mut self, sort: Sort, keys: Arc<Vec<u64>>) {
        match sort {
            Sort::Score => self.score = Some(keys),
            Sort::Favorites => self.favs = Some(keys),
            Sort::Newest => {}
        }
    }

    fn refresh(&mut self, posts: &[PostRecord]) {
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

#[derive(Clone)]
pub struct Index {
    db: Arc<Database>,
    anchor: Arc<Mutex<Instant>>,
    vault: Arc<Mutex<Vault>>,
    records: Arc<Mutex<RecordVault>>,
    sort_heads: Arc<Mutex<SortHeadVault>>,
    sort_keys: Arc<Mutex<SortKeyVault>>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        startup("index.open.enter");
        let db = Database::create(path).with_context(|| format!("open redb {}", path.display()))?;
        startup("index.redb.create.done");
        let index = Self {
            db: Arc::new(db),
            anchor: Arc::new(Mutex::new(Instant::now())),
            vault: Arc::new(Mutex::new(Vault::new())),
            records: Arc::new(Mutex::new(RecordVault::new())),
            sort_heads: Arc::new(Mutex::new(SortHeadVault::default())),
            sort_keys: Arc::new(Mutex::new(SortKeyVault::default())),
        };
        index.prime()?;
        startup("index.prime.done");
        Ok(index)
    }

    pub fn absorb(&self, posts: &[PostRecord]) -> Result<()> {
        let tx = self.begin_quick_write("begin index write")?;
        Self::absorb_into(&tx, posts)?;
        tx.commit().context("commit index write")?;
        self.evict_records(posts);
        self.clear_sort_heads();
        self.refresh_sort_keys(posts);
        Ok(())
    }

    /// One transaction per crawl page: posts and the advanced cursor land
    /// together, halving commits on the hottest write path.
    pub fn absorb_crawl(&self, posts: &[PostRecord], before: Option<PostId>) -> Result<()> {
        let tx = self.begin_quick_write("begin crawl write")?;
        Self::absorb_into(&tx, posts)?;
        if let Some(before) = before {
            let mut table = tx.open_table(META).context("open meta")?;
            let _old = table
                .insert(DANBOORU_CRAWL_BEFORE, u64::from(before.0))
                .context("write Danbooru crawl cursor")?;
        }
        tx.commit().context("commit crawl write")?;
        self.evict_records(posts);
        self.clear_sort_heads();
        self.refresh_sort_keys(posts);
        Ok(())
    }

    fn evict_records(&self, posts: &[PostRecord]) {
        let mut records = lock_record_vault(&self.records);
        for post in posts {
            records.evict(post.id);
        }
    }

    fn clear_sort_heads(&self) {
        lock_sort_heads(&self.sort_heads).clear();
    }

    fn refresh_sort_keys(&self, posts: &[PostRecord]) {
        lock_sort_keys(&self.sort_keys).refresh(posts);
    }

    fn absorb_into(tx: &redb::WriteTransaction, posts: &[PostRecord]) -> Result<()> {
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
                lock_vault(&self.vault).evict(key.lane, &key.key);
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

    pub fn search(&self, query: &Query, sort: Sort, limit: usize) -> Result<SearchHit> {
        startup("index.search.enter");
        let tx = self.db.begin_read().context("begin index read")?;
        startup("index.search.tx");
        let posts = tx.open_table(POSTS).context("open posts")?;

        let candidate = self.candidate_set(&tx, query)?;
        startup("index.search.candidate");
        let posts_len = posts.len().context("count posts")?;
        let candidates = candidate
            .as_ref()
            .map_or(posts_len, |candidate| candidate.len(posts_len));
        startup("index.search.candidates.len");

        let ids = match (&candidate, sort) {
            (None, Sort::Newest) => self.newest_ranked_ids(&posts, None, limit)?,
            (Some(Candidate::Finite(bitmap)), Sort::Newest) => {
                bitmap.as_ref().iter().rev().take(limit).collect::<Vec<_>>()
            }
            (Some(candidate @ Candidate::Cofinite(_)), Sort::Newest) => {
                self.newest_ranked_ids(&posts, Some(candidate), limit)?
            }
            (None, Sort::Score | Sort::Favorites) => self.ranked_ids(&tx, sort, None, limit)?,
            (Some(candidate @ Candidate::Finite(bitmap)), Sort::Score)
                if bitmap.len() > SMALL_SORT =>
            {
                self.ranked_ids(&tx, sort, Some(candidate), limit)?
            }
            (Some(candidate @ Candidate::Finite(bitmap)), Sort::Favorites)
                if bitmap.len() > SMALL_SORT =>
            {
                self.ranked_ids(&tx, sort, Some(candidate), limit)?
            }
            (Some(candidate @ Candidate::Cofinite(_)), Sort::Score | Sort::Favorites) => {
                self.ranked_ids(&tx, sort, Some(candidate), limit)?
            }
            (Some(Candidate::Finite(bitmap)), Sort::Score | Sort::Favorites) => {
                self.local_sorted_ids(&tx, &posts, bitmap.as_ref(), sort, limit)?
            }
        };
        startup("index.search.ids");

        let mut hydrated = Vec::with_capacity(ids.len());
        for id in ids {
            let id = PostId(id);
            if let Some(post) = lock_record_vault(&self.records).get(id) {
                hydrated.push(post);
                continue;
            }
            if let Some(post) = posts
                .get(u64::from(id.0))
                .context("hydrate post")?
                .map(|guard| decode_record(guard.value()))
                .transpose()?
            {
                // Media-less records predating the ingestion ban wash out here
                // until a re-crawl purges them.
                if post.blade_url().is_some() {
                    lock_record_vault(&self.records).put(post.clone());
                    hydrated.push(post);
                }
            }
        }
        startup("index.search.posts.loaded");
        Ok(SearchHit {
            posts: hydrated,
            candidates,
        })
    }

    fn ranked_ids(
        &self,
        tx: &redb::ReadTransaction,
        sort: Sort,
        candidate: Option<&Candidate>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let head = self.sort_head(tx, sort)?;
        if let Some(ids) = head_ids(&head, candidate, limit) {
            return Ok(ids);
        }
        match sort {
            Sort::Score => lane_ids(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                candidate,
                limit,
            ),
            Sort::Favorites => lane_ids(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                candidate,
                limit,
            ),
            Sort::Newest => unreachable!("newest is not a ranked sort lane"),
        }
    }

    fn newest_ranked_ids(
        &self,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        candidate: Option<&Candidate>,
        limit: usize,
    ) -> Result<Vec<u32>> {
        let head = self.newest_head(posts)?;
        if let Some(ids) = head_ids(&head, candidate, limit) {
            return Ok(ids);
        }
        match candidate {
            Some(candidate) => newest_ids_filtered(posts, candidate, limit),
            None => newest_ids(posts, limit),
        }
    }

    fn newest_head(
        &self,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    ) -> Result<Arc<Vec<u32>>> {
        if let Some(ids) = lock_sort_heads(&self.sort_heads).get(Sort::Newest) {
            return Ok(ids);
        }
        let ids = Arc::new(post_head(posts, SORT_HEAD_CAP)?);
        let mut heads = lock_sort_heads(&self.sort_heads);
        if let Some(raced) = heads.get(Sort::Newest) {
            return Ok(raced);
        }
        heads.put(Sort::Newest, Arc::clone(&ids));
        Ok(ids)
    }

    fn sort_head(&self, tx: &redb::ReadTransaction, sort: Sort) -> Result<Arc<Vec<u32>>> {
        if let Some(ids) = lock_sort_heads(&self.sort_heads).get(sort) {
            return Ok(ids);
        }
        let ids = Arc::new(match sort {
            Sort::Score => lane_head(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                SORT_HEAD_CAP,
            )?,
            Sort::Favorites => lane_head(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                SORT_HEAD_CAP,
            )?,
            Sort::Newest => unreachable!("newest has no sort-head cache"),
        });
        let mut heads = lock_sort_heads(&self.sort_heads);
        if let Some(raced) = heads.get(sort) {
            return Ok(raced);
        }
        heads.put(sort, Arc::clone(&ids));
        Ok(ids)
    }

    fn local_sorted_ids(
        &self,
        tx: &redb::ReadTransaction,
        posts: &impl redb::ReadableTable<u64, &'static [u8]>,
        bitmap: &RoaringBitmap,
        sort: Sort,
        limit: usize,
    ) -> Result<Vec<u32>> {
        match sort {
            Sort::Score | Sort::Favorites => {
                let keys = self.sort_keys(tx, sort)?;
                Ok(local_sorted_ids_from_keys(bitmap, &keys, limit))
            }
            Sort::Newest => local_sorted_ids(posts, bitmap, sort, limit),
        }
    }

    fn sort_keys(&self, tx: &redb::ReadTransaction, sort: Sort) -> Result<Arc<Vec<u64>>> {
        if let Some(keys) = lock_sort_keys(&self.sort_keys).get(sort) {
            return Ok(keys);
        }
        let keys = Arc::new(match sort {
            Sort::Score => {
                lane_sort_keys(&tx.open_table(SCORE_POSTS).context("open score_posts")?)?
            }
            Sort::Favorites => {
                lane_sort_keys(&tx.open_table(FAV_POSTS).context("open fav_posts")?)?
            }
            Sort::Newest => unreachable!("newest has no dense sort-key cache"),
        });
        let mut vault = lock_sort_keys(&self.sort_keys);
        if let Some(raced) = vault.get(sort) {
            return Ok(raced);
        }
        vault.put(sort, Arc::clone(&keys));
        Ok(keys)
    }

    fn prime(&self) -> Result<()> {
        if self.schema_ready()? && self.quick_repair_marked()? {
            startup("index.prime.schema.ready");
            return Ok(());
        }
        let tx = self.begin_quick_write("begin schema prime")?;
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
            let mut meta = tx.open_table(META).context("prime meta")?;
            let _old = meta
                .insert(QUICK_REPAIR_V1, 1)
                .context("write quick repair marker")?;
        }
        tx.commit().context("commit schema prime")
    }

    fn schema_ready(&self) -> Result<bool> {
        let tx = self.db.begin_read().context("begin schema read")?;
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
        Ok(true)
    }

    fn quick_repair_marked(&self) -> Result<bool> {
        let tx = self.db.begin_read().context("begin quick repair read")?;
        let meta = match tx.open_table(META) {
            Ok(meta) => meta,
            Err(TableError::TableDoesNotExist(_)) => return Ok(false),
            Err(err) => return Err(err).context("open meta for quick repair marker"),
        };
        meta.get(QUICK_REPAIR_V1)
            .context("read quick repair marker")
            .map(|guard| guard.is_some())
    }

    fn candidate_set(
        &self,
        tx: &redb::ReadTransaction,
        query: &Query,
    ) -> Result<Option<Candidate>> {
        if query.is_empty() {
            return Ok(None);
        }

        let tag_chunks = tx.open_table(TAG_CHUNKS).context("open tag chunks")?;
        let rating_chunks = tx.open_table(RATING_CHUNKS).context("open rating chunks")?;
        let facts = tx.open_table(POSTING_FACTS).context("open posting facts")?;
        let pending = pending_facts(&facts)?;
        let posts = tx.open_table(POSTS).context("open posts")?;
        BitmapEval {
            posts: &posts,
            tags: &tag_chunks,
            ratings: &rating_chunks,
            pending: &pending,
            vault: &self.vault,
            universe: None,
        }
        .eval(query.root())
        .map(Some)
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
    fn len(&self, universe: u64) -> u64 {
        match self {
            Self::Finite(bitmap) => bitmap.len(),
            Self::Cofinite(excluded) => universe.saturating_sub(excluded.len()),
        }
    }

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

    fn atom(&self, atom: &QueryAtom) -> Result<Candidate> {
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
    let old_tags = indexed_tags(old);
    let new_tags = indexed_tags(new);
    for tag in old_tags.difference(&new_tags) {
        facts.del(PostingLane::Tag, tag, id);
    }
    for tag in new_tags.difference(&old_tags) {
        facts.add(PostingLane::Tag, tag, id);
    }

    let old_rating = indexed_rating(old);
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

fn indexed_tags(post: Option<&PostRecord>) -> BTreeSet<String> {
    post.filter(|post| post.indexable())
        .map(|post| post.tags.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
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

fn lock_vault(vault: &Mutex<Vault>) -> std::sync::MutexGuard<'_, Vault> {
    match vault.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_record_vault(vault: &Mutex<RecordVault>) -> std::sync::MutexGuard<'_, RecordVault> {
    match vault.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_sort_heads(vault: &Mutex<SortHeadVault>) -> std::sync::MutexGuard<'_, SortHeadVault> {
    match vault.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_sort_keys(vault: &Mutex<SortKeyVault>) -> std::sync::MutexGuard<'_, SortKeyVault> {
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
    let cached = lock_vault(vault).get(lane, key);
    let base = if let Some(base) = cached {
        base
    } else {
        let decoded = read_chunk_bitmap(chunks, key)?;
        lock_vault(vault).put(lane, key, decoded)
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
    limit: usize,
) -> Result<Vec<u32>> {
    table
        .range(0_u64..=u64::MAX)
        .context("range posts")?
        .rev()
        .take(limit)
        .map(|row| {
            row.map(|(id, _)| id.value() as u32)
                .context("read newest row")
        })
        .collect()
}

fn post_head(table: &impl redb::ReadableTable<u64, &'static [u8]>, cap: usize) -> Result<Vec<u32>> {
    table
        .range(0_u64..=u64::MAX)
        .context("range newest head")?
        .rev()
        .take(cap)
        .map(|row| {
            row.map(|(id, _)| id.value() as u32)
                .context("read newest-head row")
        })
        .collect()
}

fn newest_ids_filtered(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    candidate: &Candidate,
    limit: usize,
) -> Result<Vec<u32>> {
    let mut ids = Vec::with_capacity(limit);
    for row in table
        .range(0_u64..=u64::MAX)
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
    limit: usize,
) -> Result<Vec<u32>> {
    let mut ids = Vec::with_capacity(limit);
    for row in table
        .range(0_u64..=u64::MAX)
        .context("range sort lane")?
        .rev()
    {
        let (_, id) = row.context("read sort row")?;
        let id = id.value();
        if candidate.is_none_or(|candidate| candidate.contains(id)) {
            ids.push(id);
            if ids.len() == limit {
                break;
            }
        }
    }
    Ok(ids)
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

fn head_ids(head: &[u32], candidate: Option<&Candidate>, limit: usize) -> Option<Vec<u32>> {
    let mut ids = Vec::with_capacity(limit);
    for id in head {
        if candidate.is_none_or(|candidate| candidate.contains(*id)) {
            ids.push(*id);
            if ids.len() == limit {
                return Some(ids);
            }
        }
    }
    (candidate.is_none() && ids.len() == limit).then_some(ids)
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

fn local_sorted_ids_from_keys(bitmap: &RoaringBitmap, keys: &[u64], limit: usize) -> Vec<u32> {
    if limit == 0 {
        return Vec::new();
    }
    let mut heap = BinaryHeap::<Reverse<(u64, u32)>>::with_capacity(limit + 1);
    for id in bitmap {
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
    let mut keyed = heap
        .into_iter()
        .map(|Reverse(item)| item)
        .collect::<Vec<_>>();
    keyed.sort_unstable_by(|a, b| b.cmp(a));
    keyed.into_iter().map(|(_, id)| id).collect()
}

fn local_sorted_ids(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    bitmap: &RoaringBitmap,
    sort: Sort,
    limit: usize,
) -> Result<Vec<u32>> {
    let mut keyed = Vec::with_capacity(bitmap.len() as usize);
    for id in bitmap {
        let Some(guard) = posts.get(u64::from(id)).context("read candidate post")? else {
            continue;
        };
        let (score, favs) = decode_sort_keys(guard.value())?;
        let key = match sort {
            Sort::Newest => u64::from(id),
            Sort::Score => sort_key_i32(score, PostId(id)),
            Sort::Favorites => sort_key_u32(favs, PostId(id)),
        };
        keyed.push((key, id));
    }
    keyed.sort_unstable_by(|a, b| b.cmp(a));
    Ok(keyed.into_iter().take(limit).map(|(_, id)| id).collect())
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

#[cfg(test)]
mod tests;
