use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PostId(pub u32);

impl Display for PostId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Tag(String);

impl Tag {
    pub fn forge(raw: &str) -> Option<Self> {
        let tag = raw.trim().to_ascii_lowercase().replace(' ', "_");
        (!tag.is_empty()).then_some(Self(tag))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for Tag {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sort {
    Newest,
    Score,
    Favorites,
}

impl Sort {
    pub const ALL: [Self; 3] = [Self::Newest, Self::Score, Self::Favorites];

    pub fn label(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Score => "score",
            Self::Favorites => "favorites",
        }
    }

    pub fn danbooru_order(self) -> &'static str {
        match self {
            Self::Newest => "order:id_desc",
            Self::Score => "order:score",
            Self::Favorites => "order:favcount",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    tags: Vec<Tag>,
}

impl Query {
    pub fn parse(raw: &str) -> Self {
        let mut tags = raw
            .split_whitespace()
            .filter_map(Tag::forge)
            .collect::<Vec<_>>();
        tags.sort();
        tags.dedup();
        Self { tags }
    }

    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    pub fn key(&self) -> String {
        self.tags
            .iter()
            .map(Tag::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn remote_seed(&self, sort: Sort) -> String {
        self.tags
            .iter()
            .take(2)
            .map(ToString::to_string)
            .chain(std::iter::once(sort.danbooru_order().to_owned()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Rating {
    General,
    Sensitive,
    Questionable,
    Explicit,
    Unknown(String),
}

impl Rating {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "g" | "general" => Self::General,
            "s" | "sensitive" | "safe" => Self::Sensitive,
            "q" | "questionable" => Self::Questionable,
            "e" | "explicit" => Self::Explicit,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostRecord {
    pub id: PostId,
    pub rating: Rating,
    pub score: i32,
    pub favs: u32,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
    pub tags: Vec<Tag>,
    pub preview_url: Option<String>,
    pub large_url: Option<String>,
    pub file_url: Option<String>,
}

impl PostRecord {
    pub fn blade_url(&self) -> Option<&str> {
        self.preview_url
            .as_deref()
            .or(self.large_url.as_deref())
            .or(self.file_url.as_deref())
    }

    pub fn haystack(&self) -> String {
        self.tags
            .iter()
            .map(Tag::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchHit {
    pub posts: Vec<PostRecord>,
    pub candidates: u64,
}

pub fn encode_record(post: &PostRecord) -> Result<Vec<u8>> {
    serde_json::to_vec(post).context("serialize post record")
}

pub fn decode_record(bytes: &[u8]) -> Result<PostRecord> {
    serde_json::from_slice(bytes).context("deserialize post record")
}

pub fn narrow_post_id(id: u64) -> Result<PostId> {
    let id = u32::try_from(id).context("post id exceeds roaring bitmap range")?;
    if id == 0 {
        bail!("post id zero is invalid");
    }
    Ok(PostId(id))
}
