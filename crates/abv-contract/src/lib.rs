//! Tester-independent vocabulary shared across ABV's native UI boundary.

use std::{borrow::Cow, fmt};

pub const UI_FINGERPRINT: &str = "abv.ui/1";

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
    UiRecess,
    Water(Water),
    Filter(Cow<'static, str>),
    LocalFavorites,
}

impl Target {
    #[must_use]
    pub fn wire(&self) -> Cow<'static, str> {
        match self {
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
}
