use anyhow::{Context as _, Result};
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, ReadableTableMetadata as _,
    TableDefinition,
};
use roaring::RoaringBitmap;
use std::{io::Cursor, mem::size_of, path::Path, sync::Arc};

use crate::model::{
    CLIP_DIM, Embedding, PostId, PostRecord, Query, RatingClass, SearchHit, Sort, Tag,
    decode_record, encode_record,
};

const POSTS: TableDefinition<'_, u64, &[u8]> = TableDefinition::new("posts");
const TAG_POSTS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("tag_posts");
const RATING_POSTS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("rating_posts");
const SCORE_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("score_posts");
const FAV_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("fav_posts");
const JINA_IMAGE: TableDefinition<'_, u64, &[u8]> =
    TableDefinition::new("jina_clip_v1_image_embeddings");
const META: TableDefinition<'_, &str, u64> = TableDefinition::new("meta");

const SMALL_SORT: u64 = 50_000;
const DANBOORU_CRAWL_BEFORE: &str = "danbooru.crawl.before";
const RATING_BACKFILL_V1: &str = "rating.index.v1.backfilled";

#[derive(Clone, Debug, Default)]
pub struct SoftHit {
    pub hit: SearchHit,
    pub pool: usize,
    pub embedded: usize,
    pub missing: Vec<PostRecord>,
}

#[derive(Clone, Debug)]
pub struct TagSuggestion {
    pub tag: String,
    pub posts: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    pub posts: u64,
    pub tags: u64,
    pub embeddings: u64,
    pub rating_indexed: bool,
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

#[derive(Clone)]
pub struct Index {
    db: Arc<Database>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path).with_context(|| format!("open redb {}", path.display()))?;
        let index = Self { db: Arc::new(db) };
        index.prime()?;
        Ok(index)
    }

    pub fn absorb(&self, posts: &[PostRecord]) -> Result<()> {
        let tx = self.db.begin_write().context("begin index write")?;
        {
            let mut post_table = tx.open_table(POSTS).context("open posts")?;
            let mut tag_table = tx.open_table(TAG_POSTS).context("open tag_posts")?;
            let mut rating_table = tx.open_table(RATING_POSTS).context("open rating_posts")?;
            let mut score_table = tx.open_table(SCORE_POSTS).context("open score_posts")?;
            let mut fav_table = tx.open_table(FAV_POSTS).context("open fav_posts")?;
            let mut jina_table = tx.open_table(JINA_IMAGE).context("open Jina image table")?;

            for post in posts {
                let indexable = post.indexable();
                let old = {
                    post_table
                        .get(u64::from(post.id.0))
                        .context("read old post")?
                        .map(|guard| decode_record(guard.value()))
                        .transpose()?
                };
                if let Some(old) = old {
                    remove_record(
                        &mut post_table,
                        &mut tag_table,
                        &mut rating_table,
                        &mut score_table,
                        &mut fav_table,
                        &mut jina_table,
                        &old,
                        !indexable,
                    )?;
                }

                if !indexable {
                    continue;
                }

                let encoded = encode_record(post)?;
                let _old_post = post_table
                    .insert(u64::from(post.id.0), encoded.as_slice())
                    .context("upsert post")?;
                let _old_score = score_table
                    .insert(sort_key_i32(post.score, post.id), post.id.0)
                    .context("upsert score lane")?;
                let _old_fav = fav_table
                    .insert(sort_key_u32(post.favs, post.id), post.id.0)
                    .context("upsert favorite lane")?;

                for tag in &post.tags {
                    bitmap_insert(&mut tag_table, tag.as_str(), post.id)?;
                }
                if let Some(rating) = post.rating.class() {
                    bitmap_insert(&mut rating_table, rating.key(), post.id)?;
                }
            }
        }
        tx.commit().context("commit index write")
    }

    pub fn put_embedding(&self, id: PostId, embedding: &Embedding) -> Result<()> {
        let tx = self.db.begin_write().context("begin embedding write")?;
        {
            let mut table = tx.open_table(JINA_IMAGE).context("open Jina image table")?;
            let bytes = encode_embedding(embedding);
            let _old = table
                .insert(u64::from(id.0), bytes.as_slice())
                .context("upsert Jina image embedding")?;
        }
        tx.commit().context("commit embedding write")
    }

    pub fn has_embedding(&self, id: PostId) -> Result<bool> {
        let tx = self.db.begin_read().context("begin embedding read")?;
        let table = tx.open_table(JINA_IMAGE).context("open Jina image table")?;
        table
            .get(u64::from(id.0))
            .context("read Jina image embedding")
            .map(|guard| guard.is_some())
    }

    pub fn crawl_before(&self) -> Result<Option<PostId>> {
        let tx = self.db.begin_read().context("begin crawl cursor read")?;
        let table = tx.open_table(META).context("open meta")?;
        table
            .get(DANBOORU_CRAWL_BEFORE)
            .context("read Danbooru crawl cursor")?
            .map(|guard| narrow_meta_post_id(guard.value()))
            .transpose()
    }

    pub fn set_crawl_before(&self, before: PostId) -> Result<()> {
        let tx = self.db.begin_write().context("begin crawl cursor write")?;
        {
            let mut table = tx.open_table(META).context("open meta")?;
            let _old = table
                .insert(DANBOORU_CRAWL_BEFORE, u64::from(before.0))
                .context("write Danbooru crawl cursor")?;
        }
        tx.commit().context("commit crawl cursor write")
    }

    pub fn tag_suggestions(&self, prefix: &str, limit: usize) -> Result<Vec<TagSuggestion>> {
        let Some(prefix) = normalize_prefix(prefix) else {
            return Ok(Vec::new());
        };
        let tx = self.db.begin_read().context("begin tag suggestion read")?;
        let table = tx.open_table(TAG_POSTS).context("open tag_posts")?;
        let mut hits = Vec::new();
        for row in table
            .range(prefix.as_str()..)
            .context("range tag suggestions")?
        {
            let (tag, bytes) = row.context("read tag suggestion")?;
            let tag = tag.value();
            if !tag.starts_with(prefix.as_str()) {
                break;
            }
            hits.push(TagSuggestion {
                tag: tag.to_owned(),
                posts: bitmap_decode(bytes.value())?.len(),
            });
            if hits.len() >= limit.saturating_mul(16).max(limit) {
                break;
            }
        }
        hits.sort_unstable_by(|a, b| b.posts.cmp(&a.posts).then_with(|| a.tag.cmp(&b.tag)));
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn stats(&self) -> Result<CacheStats> {
        let tx = self.db.begin_read().context("begin cache stats read")?;
        let posts = tx.open_table(POSTS).context("open posts")?;
        let tags = tx.open_table(TAG_POSTS).context("open tag_posts")?;
        let rating_table = tx.open_table(RATING_POSTS).context("open rating_posts")?;
        let embeddings = tx.open_table(JINA_IMAGE).context("open Jina image table")?;
        let meta = tx.open_table(META).context("open meta")?;
        let posts_len = posts.len().context("count posts")?;
        let newest = posts
            .range(0_u64..=u64::MAX)
            .context("range newest post id")?
            .next_back()
            .map(|row| {
                let (id, _) = row.context("read newest post id")?;
                narrow_meta_post_id(id.value())
            })
            .transpose()?;
        let crawl_before = meta
            .get(DANBOORU_CRAWL_BEFORE)
            .context("read Danbooru crawl cursor")?
            .map(|guard| narrow_meta_post_id(guard.value()))
            .transpose()?;
        let rating_indexed = meta
            .get(RATING_BACKFILL_V1)
            .context("read rating backfill marker")?
            .is_some();
        let ratings = RatingClass::ALL
            .into_iter()
            .map(|rating| {
                let posts =
                    read_rating_bitmap(&rating_table, rating)?.map_or(0, |bitmap| bitmap.len());
                Ok((rating, posts))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(CacheStats {
            posts: posts_len,
            tags: tags.len().context("count tags")?,
            embeddings: embeddings.len().context("count Jina image embeddings")?,
            rating_indexed,
            newest,
            crawl_before,
            ratings,
        })
    }

    pub fn backfill_ratings_if_needed(&self) -> Result<Option<u64>> {
        let (lanes, posts) = {
            let tx = self.db.begin_read().context("begin rating backfill read")?;
            let meta = tx.open_table(META).context("open meta")?;
            if meta
                .get(RATING_BACKFILL_V1)
                .context("read rating backfill marker")?
                .is_some()
            {
                return Ok(None);
            }
            let posts = tx.open_table(POSTS).context("open posts")?;
            rating_lanes(&posts)?
        };

        let tx = self
            .db
            .begin_write()
            .context("begin rating backfill write")?;
        {
            let mut ratings = tx.open_table(RATING_POSTS).context("open rating_posts")?;
            let mut meta = tx.open_table(META).context("open meta")?;
            for (rating, mut bitmap) in lanes {
                if let Some(existing) = read_rating_bitmap(&ratings, rating)? {
                    bitmap |= existing;
                }
                if !bitmap.is_empty() {
                    let bytes = bitmap_encode(&bitmap)?;
                    let _old = ratings
                        .insert(rating.key(), bytes.as_slice())
                        .with_context(|| format!("write {} rating backfill", rating.key()))?;
                }
            }
            let _old = meta
                .insert(RATING_BACKFILL_V1, posts)
                .context("write rating backfill marker")?;
        }
        tx.commit().context("commit rating backfill")?;
        Ok(Some(posts))
    }

    pub fn purge_unindexable(&self) -> Result<u64> {
        let tx = self.db.begin_write().context("begin unindexable purge")?;
        let mut purged = 0_u64;
        {
            let mut post_table = tx.open_table(POSTS).context("open posts")?;
            let mut tag_table = tx.open_table(TAG_POSTS).context("open tag_posts")?;
            let mut rating_table = tx.open_table(RATING_POSTS).context("open rating_posts")?;
            let mut score_table = tx.open_table(SCORE_POSTS).context("open score_posts")?;
            let mut fav_table = tx.open_table(FAV_POSTS).context("open fav_posts")?;
            let mut jina_table = tx.open_table(JINA_IMAGE).context("open Jina image table")?;
            if let Some(blocked) = read_raw_bitmap(&tag_table, "animated")? {
                for id in blocked {
                    let post = {
                        post_table
                            .get(u64::from(id))
                            .context("read unindexable post")?
                            .map(|guard| decode_record(guard.value()))
                            .transpose()?
                    };
                    let Some(post) = post else {
                        continue;
                    };
                    if post.indexable() {
                        continue;
                    }
                    remove_record(
                        &mut post_table,
                        &mut tag_table,
                        &mut rating_table,
                        &mut score_table,
                        &mut fav_table,
                        &mut jina_table,
                        &post,
                        true,
                    )?;
                    purged += 1;
                }
            }
        }
        tx.commit().context("commit unindexable purge")?;
        Ok(purged)
    }

    pub fn search(&self, query: &Query, sort: Sort, limit: usize) -> Result<SearchHit> {
        let tx = self.db.begin_read().context("begin index read")?;
        let posts = tx.open_table(POSTS).context("open posts")?;

        let candidate = Self::candidate_bitmap(&tx, query)?;
        let candidates = candidate
            .as_ref()
            .map_or_else(|| posts.len().unwrap_or_default(), RoaringBitmap::len);

        let ids = match (&candidate, sort) {
            (None, Sort::Newest) => newest_ids(&posts, limit)?,
            (Some(bitmap), Sort::Newest) => bitmap.iter().rev().take(limit).collect::<Vec<_>>(),
            (None, Sort::Score) => lane_ids(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                None,
                limit,
            )?,
            (None, Sort::Favorites) => lane_ids(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                None,
                limit,
            )?,
            (Some(bitmap), Sort::Score) if bitmap.len() > SMALL_SORT => lane_ids(
                &tx.open_table(SCORE_POSTS).context("open score_posts")?,
                Some(bitmap),
                limit,
            )?,
            (Some(bitmap), Sort::Favorites) if bitmap.len() > SMALL_SORT => lane_ids(
                &tx.open_table(FAV_POSTS).context("open fav_posts")?,
                Some(bitmap),
                limit,
            )?,
            (Some(bitmap), Sort::Score | Sort::Favorites) => {
                local_sorted_ids(&posts, bitmap, sort, limit)?
            }
        };

        let mut hydrated = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(post) = posts
                .get(u64::from(id))
                .context("hydrate post")?
                .map(|guard| decode_record(guard.value()))
                .transpose()?
            {
                hydrated.push(post);
            }
        }
        Ok(SearchHit {
            posts: hydrated,
            candidates,
        })
    }

    pub fn search_soft(
        &self,
        query: &Query,
        sort: Sort,
        needle: &Embedding,
        alpha: f32,
        limit: usize,
        pool: usize,
        backlog: usize,
    ) -> Result<SoftHit> {
        let mut hit = self.search(query, sort, pool.max(limit))?;
        let candidates = hit.candidates;
        let pool_len = hit.posts.len();
        let mut missing = Vec::with_capacity(backlog);
        let mut embedded = 0_usize;
        {
            let tx = self.db.begin_read().context("begin soft rank read")?;
            let embeddings = tx.open_table(JINA_IMAGE).context("open Jina image table")?;
            let mut scored = Vec::with_capacity(pool_len);
            for (rank, post) in hit.posts.drain(..).enumerate() {
                let base = base_rank(&post, sort, rank, pool_len);
                let sim = embeddings
                    .get(u64::from(post.id.0))
                    .context("read Jina image embedding")?
                    .map(|guard| decode_embedding(guard.value()).map(|image| needle.cosine(&image)))
                    .transpose()?;
                if sim.is_some() {
                    embedded += 1;
                } else if missing.len() < backlog && post.blade_url().is_some() {
                    missing.push(post.clone());
                }
                scored.push((base + alpha * sim.unwrap_or_default(), rank, post));
            }
            scored.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            hit.posts = scored
                .into_iter()
                .take(limit)
                .map(|(_, _, post)| post)
                .collect();
        }
        hit.candidates = candidates;
        Ok(SoftHit {
            hit,
            pool: pool_len,
            embedded,
            missing,
        })
    }

    fn prime(&self) -> Result<()> {
        let tx = self.db.begin_write().context("begin schema prime")?;
        {
            let _posts = tx.open_table(POSTS).context("prime posts")?;
            let _tags = tx.open_table(TAG_POSTS).context("prime tag_posts")?;
            let _ratings = tx.open_table(RATING_POSTS).context("prime rating_posts")?;
            let _score = tx.open_table(SCORE_POSTS).context("prime score_posts")?;
            let _favs = tx.open_table(FAV_POSTS).context("prime fav_posts")?;
            let _jina = tx
                .open_table(JINA_IMAGE)
                .context("prime Jina image table")?;
            let _meta = tx.open_table(META).context("prime meta")?;
        }
        tx.commit().context("commit schema prime")
    }

    fn candidate_bitmap(
        tx: &redb::ReadTransaction,
        query: &Query,
    ) -> Result<Option<RoaringBitmap>> {
        let tags = query.tags();
        let excluded = query.excluded_tags();
        let ratings = query.ratings();
        let excluded_ratings = query.excluded_ratings();
        if query.is_empty() {
            return Ok(None);
        }

        let tag_table = tx.open_table(TAG_POSTS).context("open tag_posts")?;
        let rating_table = tx.open_table(RATING_POSTS).context("open rating_posts")?;
        let mut positive = tags
            .iter()
            .map(|tag| read_bitmap(&tag_table, tag))
            .collect::<Result<Vec<_>>>()?;
        if positive.iter().any(Option::is_none) {
            return Ok(Some(RoaringBitmap::new()));
        }
        positive.sort_by_key(|bitmap| bitmap.as_ref().map_or(0, RoaringBitmap::len));

        let mut iter = positive.into_iter().flatten();
        let mut acc = if let Some(rating_bitmap) = rating_union(&rating_table, ratings)? {
            rating_bitmap
        } else if let Some(first) = iter.next() {
            first
        } else {
            let posts = tx.open_table(POSTS).context("open posts")?;
            all_post_ids(&posts)?
        };
        for bitmap in iter {
            acc &= bitmap;
        }
        for tag in excluded {
            if let Some(bitmap) = read_bitmap(&tag_table, tag)? {
                acc -= bitmap;
            }
        }
        for rating in excluded_ratings {
            if let Some(bitmap) = read_rating_bitmap(&rating_table, *rating)? {
                acc -= bitmap;
            }
        }
        Ok(Some(acc))
    }
}

fn remove_record(
    post_table: &mut redb::Table<'_, u64, &[u8]>,
    tag_table: &mut redb::Table<'_, &str, &[u8]>,
    rating_table: &mut redb::Table<'_, &str, &[u8]>,
    score_table: &mut redb::Table<'_, u64, u32>,
    fav_table: &mut redb::Table<'_, u64, u32>,
    jina_table: &mut redb::Table<'_, u64, &[u8]>,
    post: &PostRecord,
    purge_embedding: bool,
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
    for tag in &post.tags {
        bitmap_remove(tag_table, tag.as_str(), post.id)?;
    }
    if let Some(rating) = post.rating.class() {
        bitmap_remove(rating_table, rating.key(), post.id)?;
    }
    if purge_embedding {
        let _old_embedding = jina_table
            .remove(u64::from(post.id.0))
            .context("remove Jina image embedding")?;
    }
    Ok(())
}

fn bitmap_insert(table: &mut redb::Table<'_, &str, &[u8]>, key: &str, id: PostId) -> Result<()> {
    let mut bitmap = table
        .get(key)
        .with_context(|| format!("read bitmap {key}"))?
        .map(|guard| bitmap_decode(guard.value()))
        .transpose()?
        .unwrap_or_default();
    let _inserted = bitmap.insert(id.0);
    let bytes = bitmap_encode(&bitmap)?;
    let _old = table
        .insert(key, bytes.as_slice())
        .with_context(|| format!("upsert bitmap {key}"))?;
    Ok(())
}

fn bitmap_remove(table: &mut redb::Table<'_, &str, &[u8]>, key: &str, id: PostId) -> Result<()> {
    let Some(mut bitmap) = table
        .get(key)
        .with_context(|| format!("read bitmap {key}"))?
        .map(|guard| bitmap_decode(guard.value()))
        .transpose()?
    else {
        return Ok(());
    };
    let _removed = bitmap.remove(id.0);
    if bitmap.is_empty() {
        let _old = table
            .remove(key)
            .with_context(|| format!("remove empty bitmap {key}"))?;
    } else {
        let bytes = bitmap_encode(&bitmap)?;
        let _old = table
            .insert(key, bytes.as_slice())
            .with_context(|| format!("rewrite bitmap {key}"))?;
    }
    Ok(())
}

fn rating_lanes(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
) -> Result<([(RatingClass, RoaringBitmap); 4], u64)> {
    let mut lanes = RatingClass::ALL.map(|rating| (rating, RoaringBitmap::new()));
    let mut posts_seen = 0_u64;
    for row in posts
        .range(0_u64..=u64::MAX)
        .context("range posts for rating backfill")?
    {
        let (_, post) = row.context("read post for rating backfill")?;
        let post = decode_record(post.value())?;
        posts_seen += 1;
        if let Some(rating) = post.rating.class() {
            for (lane, bitmap) in &mut lanes {
                if *lane == rating {
                    let _inserted = bitmap.insert(post.id.0);
                    break;
                }
            }
        }
    }
    Ok((lanes, posts_seen))
}

fn read_bitmap(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    tag: &Tag,
) -> Result<Option<RoaringBitmap>> {
    read_raw_bitmap(table, tag.as_str())
}

fn read_raw_bitmap(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<Option<RoaringBitmap>> {
    table
        .get(key)
        .context("read bitmap")?
        .map(|guard| bitmap_decode(guard.value()))
        .transpose()
}

fn read_rating_bitmap(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    rating: RatingClass,
) -> Result<Option<RoaringBitmap>> {
    table
        .get(rating.key())
        .context("read rating bitmap")?
        .map(|guard| bitmap_decode(guard.value()))
        .transpose()
}

fn rating_union(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    ratings: &[RatingClass],
) -> Result<Option<RoaringBitmap>> {
    if ratings.is_empty() {
        return Ok(None);
    }
    let mut acc = None::<RoaringBitmap>;
    for rating in ratings {
        let Some(bitmap) = read_rating_bitmap(table, *rating)? else {
            continue;
        };
        match &mut acc {
            Some(acc) => *acc |= bitmap,
            None => acc = Some(bitmap),
        }
    }
    Ok(Some(acc.unwrap_or_default()))
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
    candidate: Option<&RoaringBitmap>,
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
        if candidate.is_none_or(|bitmap| bitmap.contains(id)) {
            ids.push(id);
            if ids.len() == limit {
                break;
            }
        }
    }
    Ok(ids)
}

fn local_sorted_ids(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    bitmap: &RoaringBitmap,
    sort: Sort,
    limit: usize,
) -> Result<Vec<u32>> {
    let mut keyed = Vec::with_capacity(bitmap.len().min(limit as u64) as usize);
    for id in bitmap {
        if let Some(post) = posts
            .get(u64::from(id))
            .context("read candidate post")?
            .map(|guard| decode_record(guard.value()))
            .transpose()?
        {
            let key = match sort {
                Sort::Newest => u64::from(post.id.0),
                Sort::Score => sort_key_i32(post.score, post.id),
                Sort::Favorites => sort_key_u32(post.favs, post.id),
            };
            keyed.push((key, id));
        }
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

fn narrow_meta_post_id(id: u64) -> Result<PostId> {
    crate::model::narrow_post_id(id)
}

fn normalize_prefix(prefix: &str) -> Option<String> {
    let prefix = prefix.trim().to_ascii_lowercase().replace(' ', "_");
    (!prefix.is_empty()).then_some(prefix)
}

fn base_rank(post: &PostRecord, sort: Sort, rank: usize, pool_len: usize) -> f32 {
    match sort {
        Sort::Newest => ordinal_rank(rank, pool_len),
        Sort::Score => signed_log_rank(post.score),
        Sort::Favorites => unsigned_log_rank(post.favs),
    }
}

fn ordinal_rank(rank: usize, pool_len: usize) -> f32 {
    if pool_len <= 1 {
        return 1.0;
    }
    1.0 - rank as f32 / (pool_len - 1) as f32
}

fn signed_log_rank(score: i32) -> f32 {
    let magnitude = (1.0 + score.unsigned_abs() as f32).ln() / 8.0;
    if score < 0 { -magnitude } else { magnitude }.clamp(-1.0, 1.0)
}

fn unsigned_log_rank(count: u32) -> f32 {
    ((1.0 + count as f32).ln() / 8.0).clamp(0.0, 1.0)
}

fn bitmap_encode(bitmap: &RoaringBitmap) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(bitmap.serialized_size());
    bitmap
        .serialize_into(&mut bytes)
        .context("serialize bitmap")?;
    Ok(bytes)
}

fn bitmap_decode(bytes: &[u8]) -> Result<RoaringBitmap> {
    RoaringBitmap::deserialize_from(Cursor::new(bytes)).context("deserialize bitmap")
}

fn encode_embedding(embedding: &Embedding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CLIP_DIM * size_of::<f32>());
    for value in embedding.as_slice() {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_embedding(bytes: &[u8]) -> Result<Embedding> {
    if bytes.len() != CLIP_DIM * size_of::<f32>() {
        anyhow::bail!(
            "expected {} embedding bytes, got {}",
            CLIP_DIM * size_of::<f32>(),
            bytes.len()
        );
    }
    let values = bytes
        .chunks_exact(size_of::<f32>())
        .map(|chunk| {
            let chunk = chunk.try_into().context("decode f32 embedding lane")?;
            Ok(f32::from_le_bytes(chunk))
        })
        .collect::<Result<Vec<_>>>()?;
    Embedding::from_normalized(values)
}
