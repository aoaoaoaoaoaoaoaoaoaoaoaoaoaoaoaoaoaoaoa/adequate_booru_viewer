use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Display, Formatter},
    path::Path,
    sync::Arc,
};

use crate::{
    date::DateRange,
    model::{Query, Sort},
};

/// User-authored intent only: everything here is something a person could
/// reasonably write into the file by hand. View ephemera live in [`Slate`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub prefetch_on_hover: bool,
    pub filters: FilterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefetch_on_hover: true,
            filters: FilterConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Arc<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Self>(&text)
                .map(Arc::new)
                .with_context(|| format!("parse {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Arc::new(Self::default())),
            Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_toml(self, path, "serialize config")
    }
}

fn save_toml(value: &impl Serialize, path: &Path, what: &'static str) -> Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(value).context(what)?;
    let tmp = path.with_extension("toml.tmp");
    {
        // fsync before rename: without it, a crash can atomically install an
        // empty file — the one failure mode rename was meant to prevent.
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("replace {} with {}", path.display(), tmp.display()))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryConfig {
    pub tree: Query,
    pub active_group: Vec<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilterConfig {
    pub saved: Vec<SavedFilter>,
    pub shelves: Vec<Shelf>,
}

/// A filter folder; ordered, like everything in the library.
///
/// `open` is view state, not configuration: it lives in the [`Slate`] and is
/// never serialized into config.toml.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shelf {
    pub name: String,
    #[serde(skip, default = "shelf_open_default")]
    pub open: bool,
    pub filters: Vec<SavedFilter>,
}

impl Default for Shelf {
    fn default() -> Self {
        Self {
            name: String::new(),
            open: true,
            filters: Vec::new(),
        }
    }
}

fn shelf_open_default() -> bool {
    true
}

/// Persistent workbench state (XDG state dir): the snapshot the app keeps of
/// itself — scratch query, selections, sliders, folder collapse. Nothing here
/// is user-authored; losing it must never lose user intent.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Slate {
    pub closed_folders: std::collections::BTreeSet<String>,
    /// Per-section fold state, keyed by panel id; presence overrides the
    /// section's compiled-in default. Absent ⇒ that default. Mirrors
    /// `closed_folders` but for the left-rail recesses, which carry mixed
    /// defaults so a bare set cannot say which way a section was thrown.
    pub shutters: std::collections::BTreeMap<String, bool>,
    pub active_filter: Option<FilterName>,
    pub query: QueryConfig,
    pub sort: Sort,
    pub dates: DateRange,
    pub images_per_row: u16,
    pub water: WaterMode,
    pub viewer_tags_open: bool,
}

impl Default for Slate {
    fn default() -> Self {
        Self {
            closed_folders: std::collections::BTreeSet::new(),
            shutters: std::collections::BTreeMap::new(),
            active_filter: None,
            query: QueryConfig::default(),
            sort: Sort::Score,
            dates: DateRange::default(),
            images_per_row: 5,
            water: WaterMode::Wet,
            viewer_tags_open: false,
        }
    }
}

impl Slate {
    /// State is disposable: any read or parse failure decays to defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_toml(self, path, "serialize slate")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedFilter {
    pub name: FilterName,
    pub tree: Query,
    pub active_group: Vec<usize>,
}

impl SavedFilter {
    pub fn new(name: FilterName, tree: Query, active_group: Vec<usize>) -> Self {
        let active_group = tree.clamp_group_path(&active_group);
        Self {
            name,
            tree,
            active_group,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaterMode {
    Dry,
    #[default]
    Wet,
    ReallyWet,
}

impl WaterMode {
    pub fn wet(self) -> bool {
        self != Self::Dry
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct FilterName(String);

impl FilterName {
    pub fn forge(raw: &str) -> Option<Self> {
        let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        (!name.is_empty()).then_some(Self(name))
    }

    pub fn neutral() -> Self {
        Self("neutral".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for FilterName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for FilterName {
    type Error = &'static str;

    fn try_from(raw: String) -> std::result::Result<Self, Self::Error> {
        Self::forge(&raw).ok_or("filter name is empty")
    }
}

impl From<FilterName> for String {
    fn from(name: FilterName) -> Self {
        name.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoolOp, QueryAtom, Tag, TagPolarity};

    #[test]
    fn config_roundtrips_filter_library() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let choice = query.push_group(&[], BoolOp::Or).context("push OR")?;
        assert!(query.push_atom(&choice, tag("bikini")?, TagPolarity::Positive));
        assert!(query.push_atom(&choice, tag("nude")?, TagPolarity::Positive));

        let config = Config {
            prefetch_on_hover: false,
            filters: FilterConfig {
                saved: vec![SavedFilter::new(
                    FilterName::forge("beach").context("filter name")?,
                    query.clone(),
                    choice.clone(),
                )],
                shelves: vec![Shelf {
                    name: "trips".to_owned(),
                    open: false,
                    filters: Vec::new(),
                }],
            },
        };
        let text = toml::to_string_pretty(&config)?;
        let roundtrip = toml::from_str::<Config>(&text)?;
        assert!(!roundtrip.prefetch_on_hover);
        assert_eq!(roundtrip.filters.saved[0].name.as_str(), "beach");
        assert_eq!(roundtrip.filters.saved[0].tree, query);
        assert_eq!(roundtrip.filters.saved[0].active_group, choice);
        // `open` is slate state, never config: it must not survive the trip.
        assert!(roundtrip.filters.shelves[0].open);
        Ok(())
    }

    /// The pinned demo fixture must deserialize through the real loaders and
    /// keep the nested `harmless screenshot` shape the take's choreography
    /// depends on. Guards the fixture against silent drift.
    #[test]
    fn demo_fixture_is_loadable() -> Result<()> {
        let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/wet");
        // Config must fail loud, so its loader is the strict path.
        let config = Config::load(&demo.join("config.toml"))?;
        let names = config
            .filters
            .shelves
            .iter()
            .map(|shelf| shelf.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["work", "play"]);
        let screenshot = &config.filters.shelves[0].filters[0];
        assert_eq!(screenshot.name.as_str(), "harmless screenshot");
        assert_eq!(screenshot.active_group, vec![0_usize, 1]);
        // Slate decays to defaults on error, which would mask a typo; parse strict.
        let slate: Slate = toml::from_str(&std::fs::read_to_string(demo.join("slate.toml"))?)?;
        assert_eq!(
            slate.active_filter.as_ref().map(FilterName::as_str),
            Some("harmless screenshot")
        );
        assert_eq!(slate.water, WaterMode::Dry);
        assert_eq!(slate.images_per_row, 7);
        assert_eq!(slate.shutters.get("reference-query"), Some(&false));
        assert!(slate.closed_folders.contains("work"));
        assert!(slate.closed_folders.contains("play"));
        Ok(())
    }

    #[test]
    fn slate_roundtrips_workbench_state() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let slate = Slate {
            closed_folders: std::collections::BTreeSet::from(["trips".to_owned()]),
            shutters: std::collections::BTreeMap::from([("gallery-controls".to_owned(), true)]),
            active_filter: FilterName::forge("beach"),
            query: QueryConfig {
                tree: query.clone(),
                active_group: Vec::new(),
            },
            sort: Sort::Newest,
            dates: DateRange {
                first: crate::date::CreatedDay::parse("2024-01-01"),
                last: crate::date::CreatedDay::parse("2024-12-31"),
            },
            images_per_row: 7,
            water: WaterMode::ReallyWet,
            viewer_tags_open: true,
        };
        let text = toml::to_string_pretty(&slate)?;
        let roundtrip = toml::from_str::<Slate>(&text)?;
        assert_eq!(roundtrip.query.tree, query);
        assert!(roundtrip.closed_folders.contains("trips"));
        assert_eq!(roundtrip.shutters.get("gallery-controls"), Some(&true));
        assert_eq!(roundtrip.images_per_row, 7);
        assert_eq!(roundtrip.dates, slate.dates);
        assert_eq!(roundtrip.water, WaterMode::ReallyWet);
        assert!(roundtrip.viewer_tags_open);
        Ok(())
    }

    #[test]
    fn slate_defaults_to_wet() {
        let slate = Slate::default();
        assert_eq!(slate.water, WaterMode::Wet);
        assert!(!slate.viewer_tags_open);
    }

    #[test]
    fn slate_without_water_field_defaults_to_wet() -> Result<()> {
        let slate = toml::from_str::<Slate>(
            r#"
sort = "score"
images_per_row = 5

[query]
active_group = []
"#,
        )?;
        assert_eq!(slate.water, WaterMode::Wet);
        assert!(!slate.viewer_tags_open);
        Ok(())
    }

    #[test]
    fn filter_names_are_compacted_and_nonempty() -> Result<()> {
        let name = FilterName::forge("  study   pose  ").context("valid filter name")?;
        assert_eq!(name.as_str(), "study pose");
        assert!(FilterName::forge(" \n\t ").is_none());
        Ok(())
    }

    fn tag(raw: &str) -> Result<QueryAtom> {
        Tag::forge(raw).map(QueryAtom::Tag).context("forge tag")
    }
}
