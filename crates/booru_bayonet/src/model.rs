use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub const CLIP_DIM: usize = 768;

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
    include: Vec<Tag>,
    exclude: Vec<Tag>,
}

impl Query {
    pub fn parse(raw: &str) -> Self {
        let mut query = Self {
            include: Vec::new(),
            exclude: Vec::new(),
        };
        for token in raw.split_whitespace() {
            let (polarity, body) = match token.strip_prefix('-') {
                Some(body) => (TagPolarity::Negative, body),
                None => (
                    TagPolarity::Positive,
                    token.strip_prefix('+').unwrap_or(token),
                ),
            };
            if let Some(tag) = Tag::forge(body) {
                query.set(tag, polarity);
            }
        }
        query
    }

    pub fn tags(&self) -> &[Tag] {
        &self.include
    }

    pub fn excluded_tags(&self) -> &[Tag] {
        &self.exclude
    }

    pub fn key(&self) -> String {
        self.terms().join(" ")
    }

    pub fn to_text(&self) -> String {
        self.key()
    }

    pub fn set(&mut self, tag: Tag, polarity: TagPolarity) {
        self.remove(&tag);
        match polarity {
            TagPolarity::Positive => self.include.push(tag),
            TagPolarity::Negative => self.exclude.push(tag),
        }
        self.normalize();
    }

    pub fn remove(&mut self, tag: &Tag) {
        self.include.retain(|candidate| candidate != tag);
        self.exclude.retain(|candidate| candidate != tag);
    }

    pub fn polarity(&self, tag: &Tag) -> Option<TagPolarity> {
        if self.include.binary_search(tag).is_ok() {
            Some(TagPolarity::Positive)
        } else if self.exclude.binary_search(tag).is_ok() {
            Some(TagPolarity::Negative)
        } else {
            None
        }
    }

    fn terms(&self) -> Vec<String> {
        self.include
            .iter()
            .map(Tag::as_str)
            .map(ToOwned::to_owned)
            .chain(self.exclude.iter().map(|tag| format!("-{tag}")))
            .collect()
    }

    pub fn remote_seed(&self, sort: Sort) -> String {
        self.include
            .iter()
            .take(2)
            .map(ToString::to_string)
            .chain(std::iter::once(sort.danbooru_order().to_owned()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn normalize(&mut self) {
        self.include.sort();
        self.include.dedup();
        self.exclude.sort();
        self.exclude.dedup();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TagPolarity {
    Positive,
    Negative,
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

    pub fn full_url(&self) -> Option<&str> {
        self.large_url
            .as_deref()
            .or(self.file_url.as_deref())
            .or(self.preview_url.as_deref())
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchHit {
    pub posts: Vec<PostRecord>,
    pub candidates: u64,
}

#[derive(Clone, Debug)]
pub struct Embedding {
    values: Vec<f32>,
}

impl Embedding {
    pub fn forge(values: Vec<f32>) -> Result<Self> {
        if values.len() != CLIP_DIM {
            bail!(
                "expected {CLIP_DIM}-wide Jina CLIP embedding, got {}",
                values.len()
            );
        }
        let mut embedding = Self { values };
        embedding.normalize()?;
        Ok(embedding)
    }

    pub fn from_normalized(values: Vec<f32>) -> Result<Self> {
        if values.len() != CLIP_DIM {
            bail!(
                "expected {CLIP_DIM}-wide Jina CLIP embedding, got {}",
                values.len()
            );
        }
        Ok(Self { values })
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }

    pub fn cosine(&self, other: &Self) -> f32 {
        self.values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .clamp(-1.0, 1.0)
    }

    fn normalize(&mut self) -> Result<()> {
        let norm = self.values.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            bail!("degenerate Jina CLIP embedding");
        }
        for x in &mut self.values {
            *x /= norm;
        }
        Ok(())
    }
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
