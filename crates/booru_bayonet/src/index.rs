use anyhow::{Context as _, Result};
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, ReadableTableMetadata as _,
    TableDefinition,
};
use roaring::RoaringBitmap;
use std::{io::Cursor, path::Path, sync::Arc};

use crate::model::{PostId, PostRecord, Query, SearchHit, Sort, Tag, decode_record, encode_record};

const POSTS: TableDefinition<'_, u64, &[u8]> = TableDefinition::new("posts");
const TAG_POSTS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("tag_posts");
const SCORE_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("score_posts");
const FAV_POSTS: TableDefinition<'_, u64, u32> = TableDefinition::new("fav_posts");

const SMALL_SORT: u64 = 50_000;

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
            let mut score_table = tx.open_table(SCORE_POSTS).context("open score_posts")?;
            let mut fav_table = tx.open_table(FAV_POSTS).context("open fav_posts")?;

            for post in posts {
                if let Some(old) = post_table
                    .get(u64::from(post.id.0))
                    .context("read old post")?
                    .map(|guard| decode_record(guard.value()))
                    .transpose()?
                {
                    let _old_score = score_table
                        .remove(sort_key_i32(old.score, old.id))
                        .context("remove old score lane")?;
                    let _old_fav = fav_table
                        .remove(sort_key_u32(old.favs, old.id))
                        .context("remove old favorite lane")?;
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
                    let mut bitmap = tag_table
                        .get(tag.as_str())
                        .context("read tag bitmap")?
                        .map(|guard| bitmap_decode(guard.value()))
                        .transpose()?
                        .unwrap_or_default();
                    let _inserted = bitmap.insert(post.id.0);
                    let bytes = bitmap_encode(&bitmap)?;
                    let _old_bitmap = tag_table
                        .insert(tag.as_str(), bytes.as_slice())
                        .context("upsert tag bitmap")?;
                }
            }
        }
        tx.commit().context("commit index write")
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

    fn prime(&self) -> Result<()> {
        let tx = self.db.begin_write().context("begin schema prime")?;
        {
            let _posts = tx.open_table(POSTS).context("prime posts")?;
            let _tags = tx.open_table(TAG_POSTS).context("prime tag_posts")?;
            let _score = tx.open_table(SCORE_POSTS).context("prime score_posts")?;
            let _favs = tx.open_table(FAV_POSTS).context("prime fav_posts")?;
        }
        tx.commit().context("commit schema prime")
    }

    fn candidate_bitmap(
        tx: &redb::ReadTransaction,
        query: &Query,
    ) -> Result<Option<RoaringBitmap>> {
        let tags = query.tags();
        if tags.is_empty() {
            return Ok(None);
        }

        let tag_table = tx.open_table(TAG_POSTS).context("open tag_posts")?;
        let mut bitmaps = tags
            .iter()
            .map(|tag| read_bitmap(&tag_table, tag))
            .collect::<Result<Vec<_>>>()?;
        if bitmaps.iter().any(Option::is_none) {
            return Ok(Some(RoaringBitmap::new()));
        }
        bitmaps.sort_by_key(|bitmap| bitmap.as_ref().map_or(0, RoaringBitmap::len));

        let mut iter = bitmaps.into_iter().flatten();
        let Some(mut acc) = iter.next() else {
            return Ok(Some(RoaringBitmap::new()));
        };
        for bitmap in iter {
            acc &= bitmap;
        }
        Ok(Some(acc))
    }
}

fn read_bitmap(
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    tag: &Tag,
) -> Result<Option<RoaringBitmap>> {
    table
        .get(tag.as_str())
        .context("read bitmap")?
        .map(|guard| bitmap_decode(guard.value()))
        .transpose()
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
