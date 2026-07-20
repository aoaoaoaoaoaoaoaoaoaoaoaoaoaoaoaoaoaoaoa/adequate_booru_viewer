use anyhow::{Context as _, Result};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use ureq::Agent;

use crate::model::{
    Harvest, Kin, PostId, PostRecord, Query, Rating, Sort, Tag, TagHint, TagKind, narrow_post_id,
};

const POST_LIMIT: &str = "200";

pub fn post_url(id: PostId) -> String {
    format!("https://danbooru.donmai.us/posts/{id}")
}

pub trait Booru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<Harvest>>;
}

#[derive(Clone)]
pub struct Danbooru {
    agent: Agent,
}

impl Danbooru {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .max_idle_age(Duration::from_secs(90))
            .user_agent("adequate_booru_viewer/0.1 anonymous-readonly")
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub fn crawl_page(&self, before: Option<PostId>) -> Result<Vec<Harvest>> {
        self.fetch("order:id_desc", before.map(|id| format!("b{}", id.0)))
    }

    pub fn single(&self, id: PostId) -> Result<Vec<Harvest>> {
        self.fetch(&format!("id:{id}"), None)
    }

    pub fn posts_by_ids(&self, ids: &[PostId]) -> Result<Vec<Harvest>> {
        let ids = ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.fetch(&format!("id:{ids}"), None)
    }

    pub fn kin_page(&self, before: Option<PostId>) -> Result<Vec<Kin>> {
        self.fetch_kin("order:id_desc", before.map(|id| format!("b{}", id.0)))
    }

    pub fn kin_single(&self, id: PostId) -> Result<Option<Kin>> {
        self.fetch_kin(&format!("id:{id}"), None)
            .map(|mut posts| posts.pop())
    }

    pub fn kin_children(&self, parent: PostId) -> Result<Vec<Kin>> {
        self.fetch_kin(&format!("parent:{parent}"), None)
    }

    fn fetch_kin(&self, tags: &str, page: Option<String>) -> Result<Vec<Kin>> {
        let mut request = self
            .agent
            .get("https://danbooru.donmai.us/posts.json")
            .query("limit", POST_LIMIT)
            .query("tags", tags)
            .query("only", "id,parent_id,has_children");
        if let Some(page) = page {
            request = request.query("page", page);
        }
        let mut response = request.call().context("GET Danbooru kin page")?;
        response
            .body_mut()
            .read_json::<Vec<DanbooruKin>>()
            .context("decode Danbooru kin JSON")?
            .into_iter()
            .map(Kin::try_from)
            .collect()
    }

    fn fetch(&self, tags: &str, page: Option<String>) -> Result<Vec<Harvest>> {
        let mut request = self
            .agent
            .get("https://danbooru.donmai.us/posts.json")
            .query("limit", POST_LIMIT)
            .query("tags", tags);
        if let Some(page) = page {
            request = request.query("page", page);
        }
        let mut response = request.call().context("GET Danbooru posts")?;
        let wire = response
            .body_mut()
            .read_json::<Vec<DanbooruPost>>()
            .context("decode Danbooru posts JSON")?;
        wire.into_iter().map(Harvest::try_from).collect()
    }
}

impl Booru for Danbooru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<Harvest>> {
        self.fetch(&query.remote_seed(sort), Some(page.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct DanbooruPost {
    id: u64,
    #[serde(default)]
    parent_id: Option<u64>,
    #[serde(default)]
    has_children: bool,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    score: i32,
    #[serde(default)]
    fav_count: u32,
    #[serde(default)]
    image_width: u32,
    #[serde(default)]
    image_height: u32,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    file_ext: String,
    #[serde(default)]
    tag_string: String,
    #[serde(default)]
    tag_string_general: String,
    #[serde(default)]
    tag_string_artist: String,
    #[serde(default)]
    tag_string_copyright: String,
    #[serde(default)]
    tag_string_character: String,
    #[serde(default)]
    tag_string_meta: String,
    #[serde(default)]
    preview_file_url: Option<String>,
    #[serde(default)]
    large_file_url: Option<String>,
    #[serde(default)]
    file_url: Option<String>,
    #[serde(default)]
    media_asset: Option<DanbooruMediaAsset>,
}

#[derive(Debug, Deserialize)]
struct DanbooruKin {
    id: u64,
    #[serde(default)]
    parent_id: Option<u64>,
    #[serde(default)]
    has_children: bool,
}

#[derive(Debug, Deserialize)]
struct DanbooruMediaAsset {
    #[serde(default)]
    variants: Vec<DanbooruVariant>,
}

#[derive(Debug, Deserialize)]
struct DanbooruVariant {
    #[serde(rename = "type")]
    kind: String,
    url: String,
}

impl TryFrom<DanbooruPost> for Harvest {
    type Error = anyhow::Error;

    fn try_from(post: DanbooruPost) -> Result<Self> {
        let (tags, tag_hints) = tag_inventory(&post);
        let variants = Variants::from(post.media_asset.as_ref());
        let flash = post.file_ext.eq_ignore_ascii_case("swf");
        let (preview_url, thumb_360_url, thumb_720_url, large_url, file_url) = if flash {
            (None, None, None, None, None)
        } else {
            (
                post.preview_file_url.or(variants.thumb_180),
                variants.thumb_360,
                variants.thumb_720,
                post.large_file_url,
                post.file_url,
            )
        };
        let id = narrow_post_id(post.id)?;
        let parent = post.parent_id.map(narrow_post_id).transpose()?;
        let has_children = post.has_children;
        let post = PostRecord {
            id,
            rating: Rating::parse(&post.rating),
            score: post.score,
            favs: post.fav_count,
            width: post.image_width,
            height: post.image_height,
            created_at: post.created_at,
            tags,
            tag_hints,
            preview_url,
            thumb_360_url,
            thumb_720_url,
            large_url,
            file_url,
        };
        Ok(Self {
            post,
            kin: Kin {
                id,
                parent,
                has_children,
            },
        })
    }
}

impl TryFrom<DanbooruKin> for Kin {
    type Error = anyhow::Error;

    fn try_from(post: DanbooruKin) -> Result<Self> {
        Ok(Self {
            id: narrow_post_id(post.id)?,
            parent: post.parent_id.map(narrow_post_id).transpose()?,
            has_children: post.has_children,
        })
    }
}

fn tag_inventory(post: &DanbooruPost) -> (Vec<Tag>, Vec<TagHint>) {
    let mut tags = BTreeSet::new();
    let mut hints = BTreeMap::new();
    for (kind, lane) in [
        (TagKind::General, post.tag_string_general.as_str()),
        (TagKind::Artist, post.tag_string_artist.as_str()),
        (TagKind::Copyright, post.tag_string_copyright.as_str()),
        (TagKind::Character, post.tag_string_character.as_str()),
        (TagKind::Meta, post.tag_string_meta.as_str()),
    ] {
        for tag in lane.split_whitespace().filter_map(Tag::forge) {
            let _inserted = tags.insert(tag.clone());
            let _old = hints.insert(tag, kind);
        }
    }
    for tag in post.tag_string.split_whitespace().filter_map(Tag::forge) {
        let _inserted = tags.insert(tag.clone());
        let _kind = hints.entry(tag).or_insert(TagKind::General);
    }
    let tag_hints = hints
        .into_iter()
        .map(|(tag, kind)| TagHint::new(tag, kind))
        .collect();
    (tags.into_iter().collect(), tag_hints)
}

#[derive(Default)]
struct Variants {
    thumb_180: Option<String>,
    thumb_360: Option<String>,
    thumb_720: Option<String>,
}

impl From<Option<&DanbooruMediaAsset>> for Variants {
    fn from(asset: Option<&DanbooruMediaAsset>) -> Self {
        let mut out = Self::default();
        let Some(asset) = asset else {
            return out;
        };
        for variant in &asset.variants {
            match variant.kind.as_str() {
                "180x180" => out.thumb_180 = Some(variant.url.clone()),
                "360x360" => out.thumb_360 = Some(variant.url.clone()),
                "720x720" => out.thumb_720 = Some(variant.url.clone()),
                _ => {}
            }
        }
        out
    }
}
