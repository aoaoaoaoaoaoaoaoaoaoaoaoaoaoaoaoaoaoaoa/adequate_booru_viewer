//! Tester-independent vocabulary shared across ABV's native UI boundary.

use std::{borrow::Cow, fmt};

pub const UI_FINGERPRINT: &str = "abv.ui/2";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Water {
    Dry,
    Wet,
    ReallyWet,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_targets_have_disjoint_identity() {
        assert_ne!(Target::Water(Water::Dry), Target::Water(Water::Wet));
        assert_eq!(Target::UiRecess.wire(), "recess:ui");
    }

    #[test]
    fn command_chrome_has_stable_identity() {
        assert_eq!(Target::Help.wire(), "application.help");
        assert_eq!(Target::CommandGuide.wire(), "application.command-guide");
        assert_eq!(
            Target::Panel("reference-query").wire(),
            "panel/reference-query"
        );
        assert_ne!(Target::Help.wire(), Target::CommandGuide.wire());
        assert_ne!(
            Target::TagEntry.wire(),
            Target::Panel("reference-query").wire()
        );
    }
}
