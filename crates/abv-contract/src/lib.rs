//! Tester-independent vocabulary shared across ABV's native UI boundary.

use std::{borrow::Cow, fmt};

pub const UI_FINGERPRINT: &str = "abv.ui/3";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Water {
    Dry,
    Wet,
    ReallyWet,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ViewerControl {
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Target {
    CommandGuide,
    Help,
    ImagesPerRow,
    Panel(&'static str),
    TagEntry,
    ViewerControl(ViewerControl),
    ViewerSurface,
    UiRecess,
    Water(Water),
    Filter(Cow<'static, str>),
    LocalFavorites,
}

impl Target {
    #[must_use]
    pub fn wire(&self) -> Cow<'static, str> {
        match self {
            Self::CommandGuide => Cow::Borrowed("application.command-guide"),
            Self::Help => Cow::Borrowed("application.help"),
            Self::ImagesPerRow => Cow::Borrowed("gallery.images-per-row"),
            Self::Panel(name) => Cow::Owned(format!("panel/{name}")),
            Self::TagEntry => Cow::Borrowed("query.tag-entry"),
            Self::ViewerControl(control) => {
                Cow::Owned(format!("viewer.control/{}", control.wire()))
            }
            Self::ViewerSurface => Cow::Borrowed("viewer.surface"),
            Self::UiRecess => Cow::Borrowed("recess:ui"),
            Self::Water(mode) => Cow::Owned(format!("water:{}", mode.wire())),
            Self::Filter(name) => Cow::Owned(format!("cabinet.filters.entry/{name}")),
            Self::LocalFavorites => Cow::Borrowed("filter:local-favorites"),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.wire())
    }
}
