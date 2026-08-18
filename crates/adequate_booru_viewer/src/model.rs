use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{Display, Formatter},
};

use crate::wire;

const POST_MAGIC: &[u8; 4] = b"BBP1";

pub fn media_extension(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let (_, extension) = path.rsplit('/').next()?.rsplit_once('.')?;
    (!extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then_some(extension)
}

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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    #[default]
    General,
    Artist,
    Copyright,
    Character,
    Meta,
}

impl TagKind {
    pub const PALETTE_ORDER: [Self; 5] = [
        Self::Artist,
        Self::Copyright,
        Self::Character,
        Self::General,
        Self::Meta,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "regular",
            Self::Artist => "artist",
            Self::Copyright => "copyright",
            Self::Character => "character",
            Self::Meta => "meta",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::General => 0,
            Self::Artist => 1,
            Self::Copyright => 3,
            Self::Character => 4,
            Self::Meta => 5,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::General),
            1 => Some(Self::Artist),
            3 => Some(Self::Copyright),
            4 => Some(Self::Character),
            5 => Some(Self::Meta),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TagHint {
    pub tag: Tag,
    pub kind: TagKind,
}

impl TagHint {
    pub fn new(tag: Tag, kind: TagKind) -> Self {
        Self { tag, kind }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    Newest,
    Score,
    #[serde(rename = "favorites")]
    FavCount,
}

impl Sort {
    pub const ALL: [Self; 3] = [Self::Newest, Self::Score, Self::FavCount];

    pub fn label(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Score => "score",
            Self::FavCount => "fav count",
        }
    }

    pub fn danbooru_order(self) -> &'static str {
        match self {
            Self::Newest => "order:id_desc",
            Self::Score => "order:score",
            Self::FavCount => "order:favcount",
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
    #[cfg(test)]
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
        self.sort_atoms();
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
        self.sort_atoms();
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
        self.sort_atoms();
        true
    }

    pub fn move_atom(
        &mut self,
        parent: &[usize],
        child: usize,
        target: &[usize],
    ) -> Option<Vec<usize>> {
        if parent == target || self.group(target).is_none() {
            return None;
        }
        let term = self
            .group(parent)?
            .children
            .get(child)
            .and_then(QueryExpr::term)?;
        let target = target_after_child_removal(parent, child, target)?;
        if !self.remove_child(parent, child) {
            return None;
        }
        self.push_atom(&target, term.atom, term.polarity)
            .then_some(target)
    }

    pub fn remove_atom(&mut self, atom: &QueryAtom) {
        self.root.remove_atom(atom);
        self.sort_atoms();
    }

    pub fn toggle_not(&mut self, path: &[usize]) -> bool {
        let Some(expr) = self.root.expr_mut(path) else {
            return false;
        };
        expr.toggle_not();
        self.sort_atoms();
        true
    }

    pub fn sort_atoms(&mut self) {
        self.root.sort_atoms();
    }

    pub fn clamp_group_path(&self, path: &[usize]) -> Vec<usize> {
        if self.group(path).is_some() {
            path.to_vec()
        } else {
            Vec::new()
        }
    }

    pub fn cycle_group_path(&self, active: &[usize], cycle: GroupCycle) -> Vec<usize> {
        let mut paths = Vec::new();
        self.root.group_paths(&mut Vec::new(), &mut paths);
        let active = self.clamp_group_path(active);
        let Some(slot) = paths.iter().position(|path| path == &active) else {
            return Vec::new();
        };
        let next = match cycle {
            GroupCycle::Forward => (slot + 1) % paths.len(),
            GroupCycle::Backward => slot.checked_sub(1).unwrap_or(paths.len() - 1),
        };
        paths[next].clone()
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

/// Which resident post universe a retrieval ranges over. This is orthogonal to
/// [`Query`]: provider predicates describe membership inside the indexed
/// universe; user-owned favorites select a different universe outright.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Corpus {
    #[default]
    All,
    LocalFavorites,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupCycle {
    Forward,
    Backward,
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
        if negated { format!("¬{text}") } else { text }
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

    fn group_paths(&self, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        let (_, expr) = self.denote();
        let Self::Group { group } = expr else {
            return;
        };
        out.push(path.clone());
        for (child, expr) in group.children.iter().enumerate() {
            path.push(child);
            expr.group_paths(path, out);
            let _old = path.pop();
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

    fn sort_atoms(&mut self) {
        match self {
            Self::Atom { .. } => {}
            Self::Not { child } => child.sort_atoms(),
            Self::Group { group } => {
                for child in &mut group.children {
                    child.sort_atoms();
                }
                group.sort_atoms();
            }
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

fn target_after_child_removal(
    parent: &[usize],
    child: usize,
    target: &[usize],
) -> Option<Vec<usize>> {
    let mut target = target.to_vec();
    if target.starts_with(parent)
        && let Some(slot) = target.get_mut(parent.len())
    {
        match (*slot).cmp(&child) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return None,
            std::cmp::Ordering::Greater => *slot -= 1,
        }
    }
    Some(target)
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

impl BoolGroup {
    /// Sorts atom terms across the atom-occupied slots, leaving every child's
    /// index intact: group paths held by the UI and saved filters must never
    /// shift. Display order (atoms before groups) is the renderer's concern.
    fn sort_atoms(&mut self) {
        let mut terms = self
            .children
            .iter()
            .filter_map(QueryExpr::term)
            .collect::<Vec<_>>();
        terms.sort_by(QueryTerm::cmp);
        let mut terms = terms.into_iter();
        for child in &mut self.children {
            if child.term().is_some()
                && let Some(term) = terms.next()
            {
                *child = term.into_expr();
            }
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
            Self::And => "∧",
            Self::Or => "∨",
            Self::Xor => "⊕",
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
    fn cmp(a: &Self, b: &Self) -> std::cmp::Ordering {
        a.atom
            .term()
            .cmp(&b.atom.term())
            .then_with(|| a.polarity.sort_key().cmp(&b.polarity.sort_key()))
    }

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

impl TagPolarity {
    fn sort_key(self) -> u8 {
        match self {
            Self::Positive => 0,
            Self::Negative => 1,
        }
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
    #[serde(default)]
    pub tag_hints: Vec<TagHint>,
    pub preview_url: Option<String>,
    #[serde(default)]
    pub thumb_360_url: Option<String>,
    #[serde(default)]
    pub thumb_720_url: Option<String>,
    pub large_url: Option<String>,
    pub file_url: Option<String>,
}

/// The gallery's projection over matching posts. `Grouped` collapses every
/// known parent tree to its strongest matching representative in the selected
/// sort order; it does not alter query membership.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GalleryTopology {
    #[default]
    Ungrouped,
    Grouped,
}

impl GalleryTopology {
    pub const ALL: [Self; 2] = [Self::Ungrouped, Self::Grouped];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ungrouped => "ungrouped",
            Self::Grouped => "grouped",
        }
    }
}

/// Provider metadata harvested even when its image record is inadmissible.
/// An unavailable parent must still bind visible descendants into one family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Kin {
    pub id: PostId,
    pub parent: Option<PostId>,
    pub has_children: bool,
}

#[derive(Clone, Debug)]
pub struct Harvest {
    pub post: PostRecord,
    pub kin: Kin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FamilyBadge {
    pub posts: u16,
    pub incomplete: bool,
}

#[derive(Clone, Debug)]
pub struct FamilyNode {
    pub id: PostId,
    pub parent: Option<PostId>,
    pub children: Vec<PostId>,
    pub post: Option<PostRecord>,
    pub incomplete: bool,
}

#[derive(Clone, Debug)]
pub struct FamilyTree {
    pub root: PostId,
    pub focus: PostId,
    pub nodes: BTreeMap<PostId, FamilyNode>,
}

impl FamilyTree {
    pub fn node(&self, id: PostId) -> Option<&FamilyNode> {
        self.nodes.get(&id)
    }

    pub fn post(&self, id: PostId) -> Option<&PostRecord> {
        self.node(id)?.post.as_ref()
    }

    pub fn badge(&self) -> Option<FamilyBadge> {
        let posts = self
            .nodes
            .values()
            .filter(|node| node.post.is_some())
            .count();
        let incomplete = self.nodes.values().any(|node| node.incomplete);
        (posts > 1 || incomplete).then_some(FamilyBadge {
            posts: u16::try_from(posts).unwrap_or(u16::MAX),
            incomplete,
        })
    }
}

impl PostRecord {
    /// Gold-walled and banned posts arrive with every media URL stripped;
    /// a reference workbench has no use for tombstones.
    pub fn indexable(&self) -> bool {
        !self.tags.iter().any(Tag::blocks_index)
            && !self
                .media_urls()
                .any(|url| media_extension(url).is_some_and(|ext| ext.eq_ignore_ascii_case("swf")))
            && self.blade_url().is_some()
    }

    fn media_urls(&self) -> impl Iterator<Item = &str> {
        [
            self.preview_url.as_deref(),
            self.thumb_360_url.as_deref(),
            self.thumb_720_url.as_deref(),
            self.large_url.as_deref(),
            self.file_url.as_deref(),
        ]
        .into_iter()
        .flatten()
    }

    pub fn tag_kind(&self, tag: &Tag) -> TagKind {
        self.tag_hints
            .iter()
            .find(|hint| &hint.tag == tag)
            .map_or(TagKind::General, |hint| hint.kind)
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

    /// Best available image the in-process decoder can present.
    ///
    /// Danbooru's `large_file_url` is a resized sample. Prefer the original
    /// `file_url` whenever its format belongs to the decoder's raster
    /// repertoire; non-image originals such as video fall back to the sample.
    pub fn viewer_url(&self) -> Option<&str> {
        self.file_url
            .as_deref()
            .filter(|url| decodable_raster(url))
            .or(self.large_url.as_deref())
            .or(self.preview_url.as_deref())
    }

    /// Original media bytes, with derived images only as compatibility
    /// fallbacks for records whose original URL is absent.
    pub fn original_url(&self) -> Option<&str> {
        self.file_url
            .as_deref()
            .or(self.large_url.as_deref())
            .or(self.preview_url.as_deref())
    }
}

fn decodable_raster(url: &str) -> bool {
    media_extension(url).is_some_and(|extension| {
        ["gif", "jpeg", "jpg", "png", "webp"]
            .into_iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchTail {
    #[default]
    Exhausted,
    Open,
}

#[derive(Clone, Debug, Default)]
pub struct SearchHit {
    pub posts: Vec<PostRecord>,
    pub candidates: u64,
    pub families: BTreeMap<PostId, FamilyBadge>,
    /// Requested top-N horizon which produced this hit.
    pub horizon: usize,
    /// `Open` means the selector filled its horizon exactly, so another
    /// geometric probe may expose more indexed results. `Exhausted` is proof
    /// that the selector itself ran dry before reaching the horizon.
    pub tail: SearchTail,
}

pub fn encode_record(post: &PostRecord) -> Vec<u8> {
    let mut sink = wire::Sink::with_magic(POST_MAGIC);
    sink.u32(post.id.0);
    encode_rating(&mut sink, &post.rating);
    sink.i32(post.score);
    sink.u32(post.favs);
    sink.u32(post.width);
    sink.u32(post.height);
    sink.str(&post.created_at);
    let mut tags = post.tags.iter().map(Tag::as_str).collect::<Vec<_>>();
    tags.sort_unstable();
    tags.dedup();
    sink.var(tags.len() as u64);
    for tag in tags {
        sink.str(tag);
    }
    sink.opt_str(post.preview_url.as_deref());
    sink.opt_str(post.thumb_360_url.as_deref());
    sink.opt_str(post.thumb_720_url.as_deref());
    sink.opt_str(post.large_url.as_deref());
    sink.opt_str(post.file_url.as_deref());
    encode_tag_hints(&mut sink, &post.tag_hints);
    sink.bytes()
}

pub fn decode_record(bytes: &[u8]) -> Result<PostRecord> {
    let mut blade = wire::Blade::new(bytes, POST_MAGIC)?;
    let id = PostId(blade.u32()?);
    let rating = decode_rating(&mut blade)?;
    let score = blade.i32()?;
    let favs = blade.u32()?;
    let width = blade.u32()?;
    let height = blade.u32()?;
    let created_at = blade.string()?;
    let tag_count = usize::try_from(blade.var()?).context("tag count exceeds usize")?;
    let mut tags = Vec::with_capacity(tag_count);
    for _ in 0..tag_count {
        let raw = blade.string()?;
        let tag = Tag::forge(&raw).with_context(|| format!("decode empty post tag `{raw}`"))?;
        tags.push(tag);
    }
    tags.sort();
    tags.dedup();
    let preview_url = blade.opt_string()?;
    let thumb_360_url = blade.opt_string()?;
    let thumb_720_url = blade.opt_string()?;
    let large_url = blade.opt_string()?;
    let file_url = blade.opt_string()?;
    let tag_hints = if blade.is_done() {
        Vec::new()
    } else {
        decode_tag_hints(&mut blade)?
    };
    let post = PostRecord {
        id,
        rating,
        score,
        favs,
        width,
        height,
        created_at,
        tags,
        tag_hints,
        preview_url,
        thumb_360_url,
        thumb_720_url,
        large_url,
        file_url,
    };
    blade.done()?;
    Ok(post)
}

/// Peek just a stored record's created-at day, stopping before the tag list —
/// the date-window binary search probes many records per query and only needs
/// this, so it skips the expensive tag decode. `None` if the blob is malformed
/// or the date does not parse.
pub fn record_day(bytes: &[u8]) -> Option<crate::date::CreatedDay> {
    let mut blade = wire::Blade::new(bytes, POST_MAGIC).ok()?;
    let _id = blade.u32().ok()?;
    let _rating = decode_rating(&mut blade).ok()?;
    let _score = blade.i32().ok()?;
    let _favs = blade.u32().ok()?;
    let _width = blade.u32().ok()?;
    let _height = blade.u32().ok()?;
    let created_at = blade.string().ok()?;
    crate::date::CreatedDay::parse_iso(&created_at)
}

fn encode_tag_hints(sink: &mut wire::Sink, hints: &[TagHint]) {
    let mut canonical = BTreeMap::new();
    for hint in hints {
        let _old = canonical.insert(hint.tag.as_str(), hint.kind);
    }
    sink.var(canonical.len() as u64);
    for (tag, kind) in canonical {
        sink.str(tag);
        sink.u8(kind.code());
    }
}

fn decode_tag_hints(blade: &mut wire::Blade<'_>) -> Result<Vec<TagHint>> {
    let count = usize::try_from(blade.var()?).context("tag hint count exceeds usize")?;
    let mut hints = BTreeMap::new();
    for _ in 0..count {
        let raw = blade.string()?;
        let tag = Tag::forge(&raw).with_context(|| format!("decode empty tag hint `{raw}`"))?;
        let code = blade.u8()?;
        let kind = TagKind::from_code(code).with_context(|| format!("invalid tag kind {code}"))?;
        let _old = hints.insert(tag, kind);
    }
    Ok(hints
        .into_iter()
        .map(|(tag, kind)| TagHint::new(tag, kind))
        .collect())
}

fn encode_rating(sink: &mut wire::Sink, rating: &Rating) {
    match rating {
        Rating::General => sink.u8(0),
        Rating::Sensitive => sink.u8(1),
        Rating::Questionable => sink.u8(2),
        Rating::Explicit => sink.u8(3),
        Rating::Unknown(value) => {
            sink.u8(4);
            sink.str(value);
        }
    }
}

fn decode_rating(blade: &mut wire::Blade<'_>) -> Result<Rating> {
    match blade.u8()? {
        0 => Ok(Rating::General),
        1 => Ok(Rating::Sensitive),
        2 => Ok(Rating::Questionable),
        3 => Ok(Rating::Explicit),
        4 => blade.string().map(Rating::Unknown),
        tag => bail!("invalid binary rating tag {tag}"),
    }
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
    fn remote_seed_is_bounded_to_required_positive_atoms() -> Result<()> {
        let mut query = Query::parse("rating:q rating:e solo 1girl -landscape");
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
        assert_eq!(query.remote_seed(Sort::Score), "rating:q 1girl order:score");
        Ok(())
    }

    #[test]
    fn query_tree_edits_preserve_group_identity_and_canonical_order() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(
            &[],
            QueryAtom::parse("red_hair").context("red hair")?,
            TagPolarity::Positive
        ));
        let target = query.push_group(&[], BoolOp::Xor).context("target group")?;
        assert!(query.push_atom(
            &target,
            QueryAtom::parse("zeta").context("zeta")?,
            TagPolarity::Positive
        ));
        assert!(query.push_atom(
            &target,
            QueryAtom::parse("alpha").context("alpha")?,
            TagPolarity::Positive
        ));
        let shifted = query.move_atom(&[], 0, &target).context("move atom")?;
        assert_eq!(shifted, vec![0]);
        let atoms = query
            .group(&shifted)
            .context("shifted target")?
            .children
            .iter()
            .filter_map(QueryExpr::term)
            .map(|term| term.atom.term())
            .collect::<Vec<_>>();
        assert_eq!(atoms, ["alpha", "red_hair", "zeta"]);
        assert_eq!(query.move_atom(&shifted, 0, &shifted), None);

        let nested = query
            .push_group(&shifted, BoolOp::And)
            .context("nested group")?;
        assert!(query.push_atom(
            &nested,
            QueryAtom::parse("solo").context("solo")?,
            TagPolarity::Positive
        ));
        assert_eq!(
            query.group(&nested).map(|group| group.children.len()),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn group_cycle_walks_groups_depth_first() -> Result<()> {
        let mut query = Query::default();
        let left = query.push_group(&[], BoolOp::And).context("left group")?;
        let nested = query
            .push_group(&left, BoolOp::Or)
            .context("nested group")?;
        let right = query.push_group(&[], BoolOp::Xor).context("right group")?;

        assert_eq!(query.cycle_group_path(&[], GroupCycle::Forward), left);
        assert_eq!(query.cycle_group_path(&left, GroupCycle::Forward), nested);
        assert_eq!(query.cycle_group_path(&nested, GroupCycle::Forward), right);
        assert_eq!(
            query.cycle_group_path(&right, GroupCycle::Forward),
            Vec::<usize>::new()
        );
        assert_eq!(query.cycle_group_path(&[], GroupCycle::Backward), right);
        Ok(())
    }

    #[test]
    fn media_policy_rejects_unusable_posts_and_preserves_originals() {
        let post = PostRecord {
            id: PostId(1),
            rating: Rating::General,
            score: 0,
            favs: 0,
            width: 1,
            height: 1,
            created_at: String::new(),
            tags: vec![Tag("animated".to_owned())],
            tag_hints: Vec::new(),
            preview_url: Some("https://example.test/1.jpg".to_owned()),
            thumb_360_url: None,
            thumb_720_url: None,
            large_url: None,
            file_url: None,
        };
        assert!(!post.indexable());

        let mut solo = post.clone();
        solo.tags = vec![Tag("solo".to_owned())];
        assert!(solo.indexable());

        // Gold-walled / banned: the API strips every media URL.
        solo.preview_url = None;
        assert!(!solo.indexable());

        solo.preview_url = Some("https://example.test/1.jpg".to_owned());
        solo.file_url = Some("https://example.test/1.SWF?download=1#asset".to_owned());
        assert!(!solo.indexable());

        assert_eq!(
            media_extension("https://example.test/work/image.JPEG?size=large#view"),
            Some("JPEG")
        );
        assert_eq!(media_extension("https://example.test/no-extension"), None);
        assert_eq!(media_extension("https://example.test/bad.ext%20"), None);

        let sample = "https://example.test/sample.jpg";
        let original = "https://example.test/original.PNG?download=1";
        solo.large_url = Some(sample.to_owned());
        solo.file_url = Some(original.to_owned());
        assert_eq!(solo.viewer_url(), Some(original));
        assert_eq!(solo.original_url(), Some(original));

        let video = "https://example.test/original.webm";
        solo.file_url = Some(video.to_owned());
        assert_eq!(solo.viewer_url(), Some(sample));
        assert_eq!(solo.original_url(), Some(video));
    }

    #[test]
    fn post_record_binary_codec_is_canonical() -> Result<()> {
        let alpha = Tag::forge("alpha").context("alpha")?;
        let zeta = Tag::forge("zeta").context("zeta")?;
        let post = PostRecord {
            id: PostId(42),
            rating: Rating::Questionable,
            score: -17,
            favs: 99,
            width: 1920,
            height: 1080,
            created_at: "2026-06-05T00:00:00Z".to_owned(),
            tags: vec![zeta.clone(), alpha.clone(), alpha.clone()],
            tag_hints: vec![
                TagHint::new(alpha.clone(), TagKind::Artist),
                TagHint::new(zeta.clone(), TagKind::Character),
            ],
            preview_url: Some("https://example.test/180.jpg".to_owned()),
            thumb_360_url: Some("https://example.test/360.jpg".to_owned()),
            thumb_720_url: None,
            large_url: Some("https://example.test/large.jpg".to_owned()),
            file_url: None,
        };
        let encoded = encode_record(&post);
        assert!(encoded.starts_with(POST_MAGIC));
        let decoded = decode_record(&encoded)?;
        assert_eq!(decoded.tags, vec![alpha.clone(), zeta.clone()]);
        assert_eq!(decoded.tag_kind(&alpha), TagKind::Artist);
        assert_eq!(decoded.tag_kind(&zeta), TagKind::Character);
        assert_eq!(decoded.rating.class(), Some(RatingClass::Questionable));
        assert_eq!(decoded.score, post.score);
        assert_eq!(decoded.favs, post.favs);

        Ok(())
    }
}
