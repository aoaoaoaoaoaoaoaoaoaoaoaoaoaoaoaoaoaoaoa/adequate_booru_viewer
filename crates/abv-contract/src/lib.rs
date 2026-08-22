//! Tester-independent vocabulary shared across ABV's native UI boundary.

use std::{borrow::Cow, fmt};

pub const UI_FINGERPRINT: &str = "abv.ui/4";

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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Target {
    ImagesPerRow,
    Panel(&'static str),
    TagEntry,
    ViewerControl(ViewerControl),
    ViewerSurface,
    Water(Water),
    Filter(Cow<'static, str>),
    LocalFavorites,
}

impl Target {
    #[must_use]
    pub fn wire(&self) -> Cow<'static, str> {
        match self {
            Self::ImagesPerRow => Cow::Borrowed("abv.gallery.images-per-row"),
            Self::Panel(name) => Cow::Owned(format!("abv.inspector.panel/{name}")),
            Self::TagEntry => Cow::Borrowed("abv.query.tag-entry"),
            Self::ViewerControl(control) => {
                Cow::Owned(format!("abv.viewer.control.{}", control.wire()))
            }
            Self::ViewerSurface => Cow::Borrowed("abv.viewer.surface"),
            Self::Water(mode) => Cow::Owned(format!("abv.water.mode.{}", mode.wire())),
            Self::Filter(name) => Cow::Owned(format!(
                "abv.cabinet.filter.entry/{}",
                encode_identity(name)
            )),
            Self::LocalFavorites => Cow::Borrowed("abv.filter.local-favorites"),
        }
    }
}

fn encode_identity(identity: &str) -> String {
    use fmt::Write as _;

    let mut encoded = String::with_capacity(identity.len());
    for byte in identity.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _escape = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.wire())
    }
}
