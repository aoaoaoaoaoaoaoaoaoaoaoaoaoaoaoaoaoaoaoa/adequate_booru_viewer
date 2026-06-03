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

    pub fn blocks_index(&self) -> bool {
        self.0 == "animated"
    }
}

impl Display for Tag {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RatingClass {
    General,
    Sensitive,
    Questionable,
    Explicit,
}

impl RatingClass {
    pub const ALL: [Self; 4] = [
        Self::General,
        Self::Sensitive,
        Self::Questionable,
        Self::Explicit,
    ];

    pub fn parse_metatag(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        let body = normalized.strip_prefix("rating:")?;
        Self::parse_code(body)
    }

    pub fn parse_code(raw: &str) -> Option<Self> {
        match raw {
            "g" | "general" => Some(Self::General),
            "s" | "sensitive" | "safe" => Some(Self::Sensitive),
            "q" | "questionable" => Some(Self::Questionable),
            "e" | "explicit" => Some(Self::Explicit),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::General => "g",
            Self::Sensitive => "s",
            Self::Questionable => "q",
            Self::Explicit => "e",
        }
    }

    pub fn term(self) -> String {
        format!("rating:{}", self.key())
    }
}

impl Display for RatingClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "rating:{}", self.key())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    include: Vec<Tag>,
    exclude: Vec<Tag>,
    ratings: Vec<RatingClass>,
    excluded_ratings: Vec<RatingClass>,
}

impl Query {
    pub fn parse(raw: &str) -> Self {
        let mut query = Self {
            include: Vec::new(),
            exclude: Vec::new(),
            ratings: Vec::new(),
            excluded_ratings: Vec::new(),
        };
        for token in raw.split_whitespace() {
            let (polarity, body) = match token.strip_prefix('-') {
                Some(body) => (TagPolarity::Negative, body),
                None => (
                    TagPolarity::Positive,
                    token.strip_prefix('+').unwrap_or(token),
                ),
            };
            if let Some(rating) = RatingClass::parse_metatag(body) {
                query.set_rating(rating, polarity);
            } else if let Some(tag) = Tag::forge(body) {
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

    pub fn ratings(&self) -> &[RatingClass] {
        &self.ratings
    }

    pub fn excluded_ratings(&self) -> &[RatingClass] {
        &self.excluded_ratings
    }

    pub fn is_empty(&self) -> bool {
        self.include.is_empty()
            && self.exclude.is_empty()
            && self.ratings.is_empty()
            && self.excluded_ratings.is_empty()
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

    pub fn set_rating(&mut self, rating: RatingClass, polarity: TagPolarity) {
        self.remove_rating(rating);
        match polarity {
            TagPolarity::Positive => self.ratings.push(rating),
            TagPolarity::Negative => self.excluded_ratings.push(rating),
        }
        self.normalize();
    }

    pub fn remove(&mut self, tag: &Tag) {
        self.include.retain(|candidate| candidate != tag);
        self.exclude.retain(|candidate| candidate != tag);
    }

    pub fn remove_rating(&mut self, rating: RatingClass) {
        self.ratings.retain(|candidate| *candidate != rating);
        self.excluded_ratings
            .retain(|candidate| *candidate != rating);
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

    pub fn include_terms(&self) -> Vec<String> {
        self.include
            .iter()
            .map(Tag::as_str)
            .map(ToOwned::to_owned)
            .chain(self.ratings.iter().map(|rating| rating.term()))
            .collect()
    }

    pub fn exclude_terms(&self) -> Vec<String> {
        self.exclude
            .iter()
            .map(Tag::as_str)
            .map(ToOwned::to_owned)
            .chain(self.excluded_ratings.iter().map(|rating| rating.term()))
            .collect()
    }

    fn terms(&self) -> Vec<String> {
        self.include_terms()
            .into_iter()
            .chain(
                self.exclude_terms()
                    .into_iter()
                    .map(|term| format!("-{term}")),
            )
            .collect()
    }

    pub fn remote_seed(&self, sort: Sort) -> String {
        let mut terms = Vec::with_capacity(3);
        if let Some(rating) = self.ratings.first() {
            terms.push(rating.term());
        }
        let remaining = 2_usize.saturating_sub(terms.len());
        terms.extend(self.include.iter().take(remaining).map(ToString::to_string));
        terms.push(sort.danbooru_order().to_owned());
        terms.join(" ")
    }

    fn normalize(&mut self) {
        self.include.sort();
        self.include.dedup();
        self.exclude.sort();
        self.exclude.dedup();
        self.ratings.sort();
        self.ratings.dedup();
        self.excluded_ratings.sort();
        self.excluded_ratings.dedup();
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

    pub fn class(&self) -> Option<RatingClass> {
        match self {
            Self::General => Some(RatingClass::General),
            Self::Sensitive => Some(RatingClass::Sensitive),
            Self::Questionable => Some(RatingClass::Questionable),
            Self::Explicit => Some(RatingClass::Explicit),
            Self::Unknown(_) => None,
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
    #[serde(default)]
    pub thumb_360_url: Option<String>,
    #[serde(default)]
    pub thumb_720_url: Option<String>,
    pub large_url: Option<String>,
    pub file_url: Option<String>,
}

impl PostRecord {
    pub fn indexable(&self) -> bool {
        !self.tags.iter().any(Tag::blocks_index)
    }

    pub fn blade_url(&self) -> Option<&str> {
        self.preview_url
            .as_deref()
            .or(self.thumb_360_url.as_deref())
            .or(self.thumb_720_url.as_deref())
            .or(self.large_url.as_deref())
            .or(self.file_url.as_deref())
    }

    pub fn thumb_url(&self, edge: f32) -> Option<&str> {
        if edge > 390.0 {
            self.thumb_720_url
                .as_deref()
                .or(self.thumb_360_url.as_deref())
                .or(self.preview_url.as_deref())
                .or(self.large_url.as_deref())
                .or(self.file_url.as_deref())
        } else if edge > 190.0 {
            self.thumb_360_url
                .as_deref()
                .or(self.preview_url.as_deref())
                .or(self.thumb_720_url.as_deref())
                .or(self.large_url.as_deref())
                .or(self.file_url.as_deref())
        } else {
            self.blade_url()
        }
    }

    pub fn full_url(&self) -> Option<&str> {
        self.large_url
            .as_deref()
            .or(self.file_url.as_deref())
            .or(self.preview_url.as_deref())
    }

    pub fn clip_url(&self) -> Option<&str> {
        self.thumb_720_url
            .as_deref()
            .or(self.large_url.as_deref())
            .or(self.thumb_360_url.as_deref())
            .or(self.preview_url.as_deref())
            .or(self.file_url.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_metatags_parse_as_query_predicates() {
        let query = Query::parse("rating:q -rating:e 1girl");
        assert_eq!(query.ratings(), &[RatingClass::Questionable]);
        assert_eq!(query.excluded_ratings(), &[RatingClass::Explicit]);
        assert_eq!(
            query.tags().iter().map(Tag::as_str).collect::<Vec<_>>(),
            vec!["1girl"]
        );
        assert_eq!(query.to_text(), "1girl rating:q -rating:e");
    }

    #[test]
    fn remote_seed_uses_only_one_rating_metatag() {
        let query = Query::parse("rating:q rating:e solo 1girl");
        assert_eq!(query.remote_seed(Sort::Score), "rating:q 1girl order:score");
    }

    #[test]
    fn animated_posts_are_not_indexable() {
        let post = PostRecord {
            id: PostId(1),
            rating: Rating::General,
            score: 0,
            favs: 0,
            width: 1,
            height: 1,
            created_at: String::new(),
            tags: vec![Tag("animated".to_owned())],
            preview_url: None,
            thumb_360_url: None,
            thumb_720_url: None,
            large_url: None,
            file_url: None,
        };
        assert!(!post.indexable());
    }
}
