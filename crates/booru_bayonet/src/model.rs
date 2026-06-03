use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter},
};

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    root: QueryExpr,
}

impl Query {
    pub fn parse(raw: &str) -> Self {
        Self {
            root: QueryExpr::Group {
                group: BoolGroup {
                    op: BoolOp::And,
                    children: Self::parse_terms(raw)
                        .into_iter()
                        .map(QueryTerm::into_expr)
                        .collect(),
                },
            },
        }
    }

    pub fn parse_terms(raw: &str) -> Vec<QueryTerm> {
        raw.split_whitespace()
            .filter_map(|token| {
                let (polarity, body) = match token.strip_prefix('-') {
                    Some(body) => (TagPolarity::Negative, body),
                    None => (
                        TagPolarity::Positive,
                        token.strip_prefix('+').unwrap_or(token),
                    ),
                };
                QueryAtom::parse(body).map(|atom| QueryTerm { atom, polarity })
            })
            .collect()
    }

    pub fn root(&self) -> &QueryExpr {
        &self.root
    }

    pub fn is_empty(&self) -> bool {
        matches!(
            &self.root,
            QueryExpr::Group { group } if group.op == BoolOp::And && group.children.is_empty()
        )
    }

    pub fn key(&self) -> String {
        self.to_text()
    }

    pub fn to_text(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        self.flat_terms()
            .map(|terms| {
                terms
                    .into_iter()
                    .map(QueryTerm::into_text)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| self.root.to_text())
    }

    pub fn group(&self, path: &[usize]) -> Option<&BoolGroup> {
        self.root.expr(path)?.group()
    }

    pub fn set_group_op(&mut self, path: &[usize], op: BoolOp) -> bool {
        let Some(group) = self.root.expr_mut(path).and_then(QueryExpr::group_mut) else {
            return false;
        };
        group.op = op;
        true
    }

    pub fn push_group(&mut self, path: &[usize], op: BoolOp) -> Option<Vec<usize>> {
        let group = self.root.expr_mut(path)?.group_mut()?;
        let child = group.children.len();
        group.children.push(QueryExpr::Group {
            group: BoolGroup {
                op,
                children: Vec::new(),
            },
        });
        let mut path = path.to_vec();
        path.push(child);
        Some(path)
    }

    pub fn push_atom(&mut self, path: &[usize], atom: QueryAtom, polarity: TagPolarity) -> bool {
        let Some(group) = self.root.expr_mut(path).and_then(QueryExpr::group_mut) else {
            return false;
        };
        group
            .children
            .retain(|child| child.atom().is_none_or(|candidate| candidate != &atom));
        group
            .children
            .push(QueryTerm { atom, polarity }.into_expr());
        true
    }

    pub fn remove_child(&mut self, parent: &[usize], child: usize) -> bool {
        let Some(group) = self.root.expr_mut(parent).and_then(QueryExpr::group_mut) else {
            return false;
        };
        if child >= group.children.len() {
            return false;
        }
        let _removed = group.children.remove(child);
        true
    }

    pub fn remove_atom(&mut self, atom: &QueryAtom) {
        self.root.remove_atom(atom);
    }

    pub fn toggle_not(&mut self, path: &[usize]) -> bool {
        let Some(expr) = self.root.expr_mut(path) else {
            return false;
        };
        expr.toggle_not();
        true
    }

    pub fn clamp_group_path(&self, path: &[usize]) -> Vec<usize> {
        if self.group(path).is_some() {
            path.to_vec()
        } else {
            Vec::new()
        }
    }

    pub fn polarity(&self, tag: &Tag) -> Option<TagPolarity> {
        self.atom_polarity(&QueryAtom::Tag(tag.clone()))
    }

    pub fn atom_polarity(&self, atom: &QueryAtom) -> Option<TagPolarity> {
        self.root.atom_polarity(atom, false)
    }

    pub fn remote_seed(&self, sort: Sort) -> String {
        let atoms = self.required_positive_atoms();
        let mut terms = Vec::with_capacity(3);
        if let Some(rating) = atoms.iter().find_map(|atom| match atom {
            QueryAtom::Rating(rating) => Some(rating.term()),
            QueryAtom::Tag(_) => None,
        }) {
            terms.push(rating);
        }
        let remaining = 2_usize.saturating_sub(terms.len());
        terms.extend(atoms.iter().filter_map(QueryAtom::tag_term).take(remaining));
        terms.push(sort.danbooru_order().to_owned());
        terms.join(" ")
    }

    fn flat_terms(&self) -> Option<Vec<QueryTerm>> {
        let QueryExpr::Group { group } = &self.root else {
            return None;
        };
        if group.op != BoolOp::And {
            return None;
        }
        group.children.iter().map(QueryExpr::term).collect()
    }

    fn required_positive_atoms(&self) -> Vec<QueryAtom> {
        self.root
            .required_positive_atoms(false)
            .into_iter()
            .collect()
    }
}

impl Default for Query {
    fn default() -> Self {
        Self {
            root: QueryExpr::Group {
                group: BoolGroup {
                    op: BoolOp::And,
                    children: Vec::new(),
                },
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryExpr {
    Atom { atom: QueryAtom },
    Not { child: Box<QueryExpr> },
    Group { group: BoolGroup },
}

impl QueryExpr {
    pub fn group(&self) -> Option<&BoolGroup> {
        match self.denote().1 {
            Self::Group { group } => Some(group),
            Self::Atom { .. } | Self::Not { .. } => None,
        }
    }

    pub fn group_mut(&mut self) -> Option<&mut BoolGroup> {
        match self.denote_mut().1 {
            Self::Group { group } => Some(group),
            Self::Atom { .. } | Self::Not { .. } => None,
        }
    }

    pub fn denote(&self) -> (bool, &Self) {
        let mut negated = false;
        let mut expr = self;
        while let Self::Not { child } = expr {
            negated = !negated;
            expr = child;
        }
        (negated, expr)
    }

    pub fn atom(&self) -> Option<&QueryAtom> {
        match self.denote().1 {
            Self::Atom { atom } => Some(atom),
            Self::Group { .. } | Self::Not { .. } => None,
        }
    }

    pub fn term(&self) -> Option<QueryTerm> {
        let (negated, expr) = self.denote();
        let Self::Atom { atom } = expr else {
            return None;
        };
        Some(QueryTerm {
            atom: atom.clone(),
            polarity: if negated {
                TagPolarity::Negative
            } else {
                TagPolarity::Positive
            },
        })
    }

    pub fn to_text(&self) -> String {
        let (negated, expr) = self.denote();
        let text = match expr {
            Self::Atom { atom } => atom.term(),
            Self::Group { group } => {
                let children = group
                    .children
                    .iter()
                    .map(Self::to_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{}({children})", group.op.label())
            }
            Self::Not { child } => child.to_text(),
        };
        if negated { format!("NOT {text}") } else { text }
    }

    fn expr(&self, path: &[usize]) -> Option<&Self> {
        match path.split_first() {
            None => Some(self),
            Some((child, tail)) => {
                let (_, expr) = self.denote();
                let Self::Group { group } = expr else {
                    return None;
                };
                group.children.get(*child)?.expr(tail)
            }
        }
    }

    fn expr_mut(&mut self, path: &[usize]) -> Option<&mut Self> {
        match path.split_first() {
            None => Some(self),
            Some((child, tail)) => {
                let (_, expr) = self.denote_mut();
                let Self::Group { group } = expr else {
                    return None;
                };
                group.children.get_mut(*child)?.expr_mut(tail)
            }
        }
    }

    fn denote_mut(&mut self) -> (bool, &mut Self) {
        let mut negated = false;
        let mut expr = self;
        while let Self::Not { child } = expr {
            negated = !negated;
            expr = child;
        }
        (negated, expr)
    }

    fn toggle_not(&mut self) {
        if let Self::Not { child } = self {
            let inner = std::mem::replace(
                child,
                Box::new(Self::Group {
                    group: BoolGroup::default(),
                }),
            );
            *self = *inner;
        } else {
            let inner = std::mem::replace(
                self,
                Self::Group {
                    group: BoolGroup::default(),
                },
            );
            *self = Self::Not {
                child: Box::new(inner),
            };
        }
    }

    fn required_positive_atoms(&self, negated: bool) -> BTreeSet<QueryAtom> {
        match self {
            Self::Atom { atom } if !negated => BTreeSet::from([atom.clone()]),
            Self::Atom { .. } => BTreeSet::new(),
            Self::Not { child } => child.required_positive_atoms(!negated),
            Self::Group { .. } if negated => BTreeSet::new(),
            Self::Group { group } => match group.op {
                BoolOp::And => {
                    let mut atoms = BTreeSet::new();
                    for child in &group.children {
                        atoms.extend(child.required_positive_atoms(false));
                    }
                    atoms
                }
                BoolOp::Or | BoolOp::Xor => {
                    let mut children = group.children.iter();
                    let Some(first) = children.next() else {
                        return BTreeSet::new();
                    };
                    let mut atoms = first.required_positive_atoms(false);
                    for child in children {
                        let child = child.required_positive_atoms(false);
                        atoms = atoms.intersection(&child).cloned().collect();
                    }
                    atoms
                }
            },
        }
    }

    fn atom_polarity(&self, atom: &QueryAtom, negated: bool) -> Option<TagPolarity> {
        match self {
            Self::Atom { atom: candidate } if candidate == atom => Some(if negated {
                TagPolarity::Negative
            } else {
                TagPolarity::Positive
            }),
            Self::Atom { .. } => None,
            Self::Not { child } => child.atom_polarity(atom, !negated),
            Self::Group { group } => group
                .children
                .iter()
                .find_map(|child| child.atom_polarity(atom, negated)),
        }
    }

    fn remove_atom(&mut self, atom: &QueryAtom) {
        match self {
            Self::Atom { .. } => {}
            Self::Not { child } => child.remove_atom(atom),
            Self::Group { group } => {
                group
                    .children
                    .retain(|child| child.atom().is_none_or(|candidate| candidate != atom));
                for child in &mut group.children {
                    child.remove_atom(atom);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoolGroup {
    pub op: BoolOp,
    pub children: Vec<QueryExpr>,
}

impl Default for BoolGroup {
    fn default() -> Self {
        Self {
            op: BoolOp::And,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoolOp {
    #[default]
    And,
    Or,
    Xor,
}

impl BoolOp {
    pub const ALL: [Self; 3] = [Self::And, Self::Or, Self::Xor];

    pub fn label(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::Xor => "XOR",
        }
    }
}

impl Display for BoolOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum QueryAtom {
    Tag(Tag),
    Rating(RatingClass),
}

impl QueryAtom {
    pub fn parse(raw: &str) -> Option<Self> {
        RatingClass::parse_metatag(raw)
            .map(Self::Rating)
            .or_else(|| Tag::forge(raw).map(Self::Tag))
    }

    pub fn term(&self) -> String {
        match self {
            Self::Tag(tag) => tag.to_string(),
            Self::Rating(rating) => rating.term(),
        }
    }

    fn tag_term(&self) -> Option<String> {
        match self {
            Self::Tag(tag) => Some(tag.to_string()),
            Self::Rating(_) => None,
        }
    }
}

impl Display for QueryAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.term())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryTerm {
    pub atom: QueryAtom,
    pub polarity: TagPolarity,
}

impl QueryTerm {
    fn into_expr(self) -> QueryExpr {
        let atom = QueryExpr::Atom { atom: self.atom };
        match self.polarity {
            TagPolarity::Positive => atom,
            TagPolarity::Negative => QueryExpr::Not {
                child: Box::new(atom),
            },
        }
    }

    fn into_text(self) -> String {
        match self.polarity {
            TagPolarity::Positive => self.atom.term(),
            TagPolarity::Negative => format!("-{}", self.atom.term()),
        }
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
        let terms = Query::parse_terms("rating:q -rating:e 1girl")
            .into_iter()
            .map(|term| (term.atom.term(), term.polarity))
            .collect::<Vec<_>>();
        assert_eq!(
            terms,
            [
                ("rating:q".to_owned(), TagPolarity::Positive),
                ("rating:e".to_owned(), TagPolarity::Negative),
                ("1girl".to_owned(), TagPolarity::Positive)
            ]
        );
        assert_eq!(query.to_text(), "rating:q -rating:e 1girl");
        assert_eq!(
            query.atom_polarity(&QueryAtom::Rating(RatingClass::Explicit)),
            Some(TagPolarity::Negative)
        );
    }

    #[test]
    fn remote_seed_uses_only_one_rating_metatag() {
        let query = Query::parse("rating:q rating:e solo 1girl");
        assert_eq!(query.remote_seed(Sort::Score), "rating:q 1girl order:score");
    }

    #[test]
    fn remote_seed_does_not_conjoin_or_alternatives() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(
            &[],
            QueryAtom::parse("solo").context("solo tag")?,
            TagPolarity::Positive
        ));
        let choice = query.push_group(&[], BoolOp::Or).context("OR group")?;
        assert!(query.push_atom(
            &choice,
            QueryAtom::parse("bikini").context("bikini tag")?,
            TagPolarity::Positive
        ));
        assert!(query.push_atom(
            &choice,
            QueryAtom::parse("nude").context("nude tag")?,
            TagPolarity::Positive
        ));
        assert_eq!(query.remote_seed(Sort::Score), "solo order:score");
        Ok(())
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
