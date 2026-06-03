use anyhow::{Context as _, Result};
use serde::Deserialize;
use std::time::Duration;
use ureq::Agent;

use crate::model::{PostId, PostRecord, Query, Rating, Sort, Tag, narrow_post_id};

const POST_LIMIT: &str = "200";

pub trait Booru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<PostRecord>>;
}

#[derive(Clone)]
pub struct Danbooru {
    agent: Agent,
}

impl Danbooru {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent("booru_bayonet/0.1 anonymous-readonly")
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub fn crawl_page(&self, before: Option<PostId>) -> Result<Vec<PostRecord>> {
        self.fetch("order:id_desc", before.map(|id| format!("b{}", id.0)))
    }

    fn fetch(&self, tags: &str, page: Option<String>) -> Result<Vec<PostRecord>> {
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
        wire.into_iter().map(PostRecord::try_from).collect()
    }
}

impl Booru for Danbooru {
    fn posts(&self, query: &Query, sort: Sort, page: u32) -> Result<Vec<PostRecord>> {
        self.fetch(&query.remote_seed(sort), Some(page.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct DanbooruPost {
    id: u64,
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
    tag_string: String,
    #[serde(default)]
    preview_file_url: Option<String>,
    #[serde(default)]
    large_file_url: Option<String>,
    #[serde(default)]
    file_url: Option<String>,
}

impl TryFrom<DanbooruPost> for PostRecord {
    type Error = anyhow::Error;

    fn try_from(post: DanbooruPost) -> Result<Self> {
        let mut tags = post
            .tag_string
            .split_whitespace()
            .filter_map(Tag::forge)
            .collect::<Vec<_>>();
        tags.sort();
        tags.dedup();
        Ok(Self {
            id: narrow_post_id(post.id)?,
            rating: Rating::parse(&post.rating),
            score: post.score,
            favs: post.fav_count,
            width: post.image_width,
            height: post.image_height,
            created_at: post.created_at,
            tags,
            preview_url: post.preview_file_url,
            large_url: post.large_file_url,
            file_url: post.file_url,
        })
    }
}
