//! Tester-independent vocabulary shared across ABV's native UI boundary.

use std::{borrow::Cow, fmt};

use egui_tester_witness::encode_identity as identity;

/// Reverse-DNS product identifier; the sole source of the viewer's platform and crash identity.
pub const PRODUCT_IDENTIFIER: &str = "moe.eternalist.adequate-booru-viewer";
/// Product display name.
pub const PRODUCT_NAME: &str = "Adequate Booru Viewer";

/// The identifier's last label: the platform directory name on Linux.
#[must_use]
pub fn product_directory() -> &'static str {
    PRODUCT_IDENTIFIER
        .rsplit('.')
        .next()
        .unwrap_or(PRODUCT_IDENTIFIER)
}
pub const UI_FINGERPRINT: &str = "abv.ui/6";
/// Witness scope of the Filter Library's cabinet.
pub const FILTER_CABINET: &str = "filters";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Water {
    Dry,
    Wet,
    ReallyWet,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ViewerControl {
    Tree,
    Tags,
    Copy,
    Save,
    Favorite,
    Close,
    Danbooru,
    Previous,
    Parent,
    Children,
    Next,
}

impl ViewerControl {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Tags => "tags",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::Favorite => "favorite",
            Self::Close => "close",
            Self::Danbooru => "danbooru",
            Self::Previous => "previous",
            Self::Parent => "parent",
            Self::Children => "children",
            Self::Next => "next",
        }
    }
}

impl Water {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Dry => "dry",
            Self::Wet => "wet",
            Self::ReallyWet => "really",
        }
    }
}

/// A tag-drawer row's control.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TagRowAction {
    Require,
    Exclude,
    Remove,
    Definition,
}

impl TagRowAction {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Require => "require",
            Self::Exclude => "exclude",
            Self::Remove => "remove",
            Self::Definition => "definition",
        }
    }
}

/// The entry whose completion list offers a candidate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Completion {
    Query,
    TagPush,
}

impl Completion {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::TagPush => "tag-push",
        }
    }
}

/// A query group's Boolean operator.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Operator {
    And,
    Or,
    Xor,
}

impl Operator {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::And => "and",
            Self::Or => "or",
            Self::Xor => "xor",
        }
    }
}

/// A gallery sort plate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Sort {
    Newest,
    Score,
    FavCount,
}

impl Sort {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Score => "score",
            Self::FavCount => "fav-count",
        }
    }
}

/// A bound of the gallery's date range.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DateBound {
    From,
    Until,
}

impl DateBound {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::Until => "until",
        }
    }
}

/// The viewer's Danbooru tag-push controls.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TagPush {
    Field,
    Stage,
    Updated,
    Fault,
    Export,
}

impl TagPush {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Stage => "stage",
            Self::Updated => "updated",
            Self::Fault => "fault",
            Self::Export => "export",
        }
    }
}

/// A gallery tile's wire, before its post id.
const TILE: &str = "abv.gallery.tile/";

/// Every semantic address ABV publishes. The wire follows
/// `owner.semantic.path[/identity...]`: runtime identities are encoded.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Target {
    ImagesPerRow,
    GalleryContent,
    /// The screw-scroll gutter beside the gallery content.
    GalleryGutter,
    Tile(u32),
    FamilyBadge(u32),
    Sort(Sort),
    /// A date bound's lever.
    DateLever(DateBound),
    Panel(&'static str),
    TagEntry,
    Completion(Completion, Cow<'static, str>),
    /// A query group, by its child-index path.
    Group(Vec<usize>),
    GroupOperator(Vec<usize>, Operator),
    Atom(Cow<'static, str>),
    ViewerControl(ViewerControl),
    ViewerSurface,
    ViewerImage,
    ViewerToolbar,
    TagDrawer,
    TagRow(TagRowAction, Cow<'static, str>),
    TagPush(TagPush),
    PendingTag(Cow<'static, str>),
    DanbooruLink(u32),
    FamilyNode(u32),
    Water(Water),
    /// A saved filter's row in the Filter Library.
    Filter(Cow<'static, str>),
    /// A saved filter's clone control.
    FilterClone(Cow<'static, str>),
    /// A Filter Library folder.
    Shelf(Cow<'static, str>),
    LocalFavorites,
}

impl Target {
    #[must_use]
    pub fn wire(&self) -> Cow<'static, str> {
        let wire = match self {
            Self::ImagesPerRow => return Cow::Borrowed("abv.gallery.images-per-row"),
            Self::GalleryContent => return Cow::Borrowed("abv.gallery.content"),
            Self::GalleryGutter => return Cow::Borrowed("abv.gallery.gutter"),
            Self::TagEntry => return Cow::Borrowed("abv.query.tag-entry"),
            Self::ViewerSurface => return Cow::Borrowed("abv.viewer.surface"),
            Self::ViewerImage => return Cow::Borrowed("abv.viewer.image"),
            Self::ViewerToolbar => return Cow::Borrowed("abv.viewer.toolbar"),
            Self::TagDrawer => return Cow::Borrowed("abv.viewer.tag-drawer"),
            Self::LocalFavorites => return Cow::Borrowed("abv.filter.local-favorites"),
            Self::Tile(id) => format!("{TILE}{id}"),
            Self::FamilyBadge(id) => format!("abv.gallery.family-badge/{id}"),
            Self::Sort(sort) => format!("abv.gallery.sort.{}", sort.wire()),
            Self::DateLever(bound) => format!("abv.gallery.date-lever.{}", bound.wire()),
            Self::Panel(name) => format!("abv.inspector.panel/{}", identity(name)),
            Self::Completion(entry, tag) => {
                format!("abv.{}.completion/{}", entry.wire(), identity(tag))
            }
            Self::Group(path) => format!("abv.query.group{}", group(path)),
            Self::GroupOperator(path, operator) => {
                format!(
                    "abv.query.group-operator.{}{}",
                    operator.wire(),
                    group(path)
                )
            }
            Self::Atom(term) => format!("abv.query.atom/{}", identity(term)),
            Self::ViewerControl(control) => format!("abv.viewer.control.{}", control.wire()),
            Self::TagRow(action, tag) => {
                format!("abv.tag-drawer.{}/{}", action.wire(), identity(tag))
            }
            Self::TagPush(part) => format!("abv.tag-push.{}", part.wire()),
            Self::PendingTag(tag) => format!("abv.tag-push.pending/{}", identity(tag)),
            Self::DanbooruLink(id) => format!("abv.viewer.danbooru-link/{id}"),
            Self::FamilyNode(id) => format!("abv.family.node/{id}"),
            Self::Water(mode) => format!("abv.water.mode.{}", mode.wire()),
            Self::Filter(name) => format!("cabinet.{FILTER_CABINET}.entry/{}", identity(name)),
            Self::FilterClone(name) => {
                format!("cabinet.{FILTER_CABINET}.clone/{}", identity(name))
            }
            Self::Shelf(name) => format!("cabinet.{FILTER_CABINET}.shelf/{}", identity(name)),
        };
        Cow::Owned(wire)
    }

    /// The post a gallery tile's wire names; `None` for any other wire.
    #[must_use]
    pub fn tile_of(wire: &str) -> Option<u32> {
        let id = wire.strip_prefix(TILE)?.parse().ok()?;
        (Self::Tile(id).wire() == wire).then_some(id)
    }
}

/// A group's identity: none for the root, else its dotted child-index path.
fn group(path: &[usize]) -> String {
    use fmt::Write as _;

    let mut identity = String::new();
    for (depth, index) in path.iter().enumerate() {
        let separator = if depth == 0 { '/' } else { '.' };
        let _step = write!(identity, "{separator}{index}");
    }
    identity
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.wire())
    }
}
