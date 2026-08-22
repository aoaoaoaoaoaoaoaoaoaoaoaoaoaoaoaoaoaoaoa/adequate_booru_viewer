use anyhow::{Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
};

use crate::{
    date::DateRange,
    model::{Corpus, GalleryTopology, PostId, Query, QueryAtom, RatingClass, Sort, TagPolarity},
};

/// Small human-edited application configuration.
///
/// Saved filters are product data owned separately by [`FilterLibrary`]. View
/// ephemera live in [`SessionState`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Configuration {
    pub prefetch_on_hover: bool,
    pub mirror: MirrorConfig,
    pub danbooru: DanbooruConfig,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            prefetch_on_hover: true,
            mirror: MirrorConfig::default(),
            danbooru: DanbooruConfig::default(),
        }
    }
}

impl eternalist_apps::configuration::Configuration for Configuration {}

/// Durable user-owned saved-filter collection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilterLibrary {
    pub unfiled: Vec<SavedFilter>,
    pub folders: Vec<Folder>,
}

impl Serialize for FilterLibrary {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        FilterLibraryOut {
            filter: &self.unfiled,
            folder: &self.folders,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FilterLibrary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = FilterLibraryIn::deserialize(deserializer)?;
        let legacy = wire.saved.is_some() || wire.shelves.is_some();
        let current = wire.filter.is_some() || wire.folder.is_some();
        if legacy && current {
            return Err(D::Error::custom(
                "legacy `saved`/`shelves` cannot be combined with `filter`/`folder`",
            ));
        }
        Ok(if legacy {
            LegacyFilterLibrary {
                saved: wire.saved.unwrap_or_default(),
                shelves: wire.shelves.unwrap_or_default(),
            }
            .into()
        } else {
            Self {
                unfiled: wire.filter.unwrap_or_default(),
                folders: wire.folder.unwrap_or_default(),
            }
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FilterLibraryIn {
    filter: Option<Vec<SavedFilter>>,
    folder: Option<Vec<Folder>>,
    saved: Option<Vec<LegacySavedFilter>>,
    shelves: Option<Vec<LegacyFolder>>,
}

#[derive(Serialize)]
struct FilterLibraryOut<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    filter: &'a Vec<SavedFilter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    folder: &'a Vec<Folder>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyFilterLibrary {
    saved: Vec<LegacySavedFilter>,
    shelves: Vec<LegacyFolder>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacySavedFilter {
    name: FilterName,
    tree: Query,
    #[serde(default)]
    active_group: Vec<usize>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyFolder {
    name: String,
    filters: Vec<LegacySavedFilter>,
}

impl From<LegacyFilterLibrary> for FilterLibrary {
    fn from(legacy: LegacyFilterLibrary) -> Self {
        Self {
            unfiled: legacy
                .saved
                .into_iter()
                .map(LegacySavedFilter::migrate)
                .collect(),
            folders: legacy
                .shelves
                .into_iter()
                .map(|folder| Folder {
                    name: folder.name,
                    filters: folder
                        .filters
                        .into_iter()
                        .map(LegacySavedFilter::migrate)
                        .collect(),
                })
                .collect(),
        }
    }
}

impl LegacySavedFilter {
    fn migrate(self) -> SavedFilter {
        let Self {
            name,
            tree,
            active_group: _,
        } = self;
        SavedFilter::new(name, tree)
    }
}

/// Optional authenticated Danbooru surface. The API key remains outside the
/// config file; its path is durable user intent, while readiness is runtime
/// state established by loading the credential file.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DanbooruConfig {
    pub account: Option<DanbooruAccountConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DanbooruAccountConfig {
    pub login: String,
    pub api_key_file: PathBuf,
}

impl DanbooruAccountConfig {
    /// Relative secret paths are rooted beside `config.toml`. Deliberately no
    /// shell-style tilde or environment expansion: config has one literal,
    /// portable path law rather than a partial shell language.
    pub fn resolve_api_key(&self, config_dir: &Path) -> PathBuf {
        if self.api_key_file.is_absolute() {
            self.api_key_file.clone()
        } else {
            config_dir.join(&self.api_key_file)
        }
    }
}

impl FilterLibrary {
    /// The shipped first-launch library: one deletable `general rating` filter,
    /// so a new user is not dropped straight into the full firehose.
    pub(crate) fn first_run() -> Self {
        let mut library = Self::default();
        if let Some(filter) = safe_default_filter() {
            library.unfiled.push(filter);
        }
        library
    }

    pub fn load(path: &Path, first_run: bool) -> Result<Self> {
        if !path.exists() {
            return Ok(if first_run {
                Self::first_run()
            } else {
                Self::default()
            });
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read filter library {}", path.display()))?;
        let document = toml::from_str::<toml::Table>(&text)
            .with_context(|| format!("parse filter library {}", path.display()))?;
        let legacy = document.contains_key("saved") || document.contains_key("shelves");
        let library = document
            .try_into::<Self>()
            .with_context(|| format!("decode filter library {}", path.display()))?;
        if legacy {
            library.save(path)?;
        }
        Ok(library)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_toml(self, path, "serialize filter library")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyConfiguration {
    prefetch_on_hover: bool,
    mirror: MirrorConfig,
    danbooru: DanbooruConfig,
    filters: LegacyFilterLibrary,
}

impl Default for LegacyConfiguration {
    fn default() -> Self {
        Self {
            prefetch_on_hover: true,
            mirror: MirrorConfig::default(),
            danbooru: DanbooruConfig::default(),
            filters: LegacyFilterLibrary::default(),
        }
    }
}

/// Split the former combined configuration without risking saved-filter loss.
pub fn migrate_legacy_configuration(config: &Path, filters: &Path) -> Result<bool> {
    let text = match std::fs::read_to_string(config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("read configuration {}", config.display()));
        }
    };
    let Ok(document) = toml::from_str::<toml::Table>(&text) else {
        return Ok(false);
    };
    if !document.contains_key("filters") {
        return Ok(false);
    }
    let Ok(legacy) = toml::from_str::<LegacyConfiguration>(&text) else {
        return Ok(false);
    };
    if !filters.exists() {
        FilterLibrary::from(legacy.filters).save(filters)?;
    }
    save_toml(
        &Configuration {
            prefetch_on_hover: legacy.prefetch_on_hover,
            mirror: legacy.mirror,
            danbooru: legacy.danbooru,
        },
        config,
        "serialize migrated configuration",
    )?;
    Ok(true)
}

/// The canonical name of the seeded first-run filter; the app activates it on a
/// first launch (see `Bayonet::open`).
pub const SAFE_DEFAULT_FILTER: &str = "general rating";

fn safe_default_filter() -> Option<SavedFilter> {
    let name = FilterName::forge(SAFE_DEFAULT_FILTER)?;
    let mut tree = Query::default();
    let _added = tree.push_atom(
        &[],
        QueryAtom::Rating(RatingClass::General),
        TagPolarity::Positive,
    );
    Some(SavedFilter::new(name, tree))
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MirrorConfig {
    pub policy: MirrorPolicy,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorPolicy {
    #[default]
    Active,
    Paused,
}

impl MirrorPolicy {
    pub fn active(self) -> bool {
        self == Self::Active
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryConfig {
    pub tree: Query,
    pub active_group: Vec<usize>,
}

/// The selected filter-library object. Saved filters name user-authored query
/// trees; local favorites names a built-in corpus and therefore cannot be
/// represented by a query flag or collide with a saved filter named
/// `favorites`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FilterSelection {
    #[default]
    Scratch,
    Saved {
        name: FilterName,
    },
    LocalFavorites,
}

impl FilterSelection {
    pub fn saved(&self) -> Option<&FilterName> {
        match self {
            Self::Saved { name } => Some(name),
            Self::Scratch | Self::LocalFavorites => None,
        }
    }

    pub fn corpus(&self) -> Corpus {
        match self {
            Self::Scratch | Self::Saved { .. } => Corpus::All,
            Self::LocalFavorites => Corpus::LocalFavorites,
        }
    }
}

/// An ordered user-facing filter folder. Open/closed state belongs to
/// [`SessionState`].
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Folder {
    pub name: String,
    #[serde(rename = "filter", skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<SavedFilter>,
}

/// The semantic view within one open image-viewer session. Geometry and
/// animation remain runtime ephemera.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerView {
    #[default]
    Image,
    Tree,
}

/// Reconstructible identity for one open image-viewer session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerSession {
    pub post: PostId,
    pub gallery_anchor: PostId,
    #[serde(default)]
    pub view: ViewerView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_focus: Option<PostId>,
    /// Result depth already paid for when this image was reached. This is a
    /// reconstruction bound, never positional identity.
    #[serde(default)]
    pub result_horizon: usize,
}

/// Persistent workbench state (XDG state dir): the snapshot the app keeps of
/// itself: scratch query, selections, sliders, folder collapse. Nothing here
/// is user-authored; losing it must never lose user intent.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionState {
    pub closed_folders: std::collections::BTreeSet<String>,
    /// Per-Panel fold state; presence overrides the Panel's compiled-in
    /// default. Absent ⇒ that default. Mirrors `closed_folders`, but mixed
    /// Panel defaults require an explicit boolean rather than a bare set.
    #[serde(alias = "shutters")]
    pub panel_folds: std::collections::BTreeMap<String, bool>,
    pub filter: FilterSelection,
    pub query: QueryConfig,
    pub sort: Sort,
    pub gallery: GalleryTopology,
    pub dates: DateRange,
    pub images_per_row: u16,
    pub water: WaterMode,
    pub viewer_tags_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer: Option<ViewerSession>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            closed_folders: std::collections::BTreeSet::new(),
            panel_folds: std::collections::BTreeMap::new(),
            filter: FilterSelection::Scratch,
            query: QueryConfig::default(),
            sort: Sort::Score,
            gallery: GalleryTopology::Ungrouped,
            dates: DateRange::default(),
            images_per_row: 5,
            water: WaterMode::Wet,
            viewer_tags_open: false,
            viewer: None,
        }
    }
}

impl SessionState {
    /// State is disposable: any read or parse failure decays to defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_toml(self, path, "serialize session state")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedFilter {
    pub name: FilterName,
    #[serde(with = "query_text")]
    pub query: Query,
}

impl SavedFilter {
    pub fn new(name: FilterName, query: Query) -> Self {
        Self { name, query }
    }
}

mod query_text {
    use super::*;

    pub fn serialize<S: Serializer>(query: &Query, serializer: S) -> Result<S::Ok, S::Error> {
        crate::filter_expression::render(query)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Query, D::Error> {
        let source = String::deserialize(deserializer)?;
        crate::filter_expression::parse(&source).map_err(D::Error::custom)
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

impl eternalist_apps::CabinetKey for FilterName {
    fn forge(raw: &str) -> Option<Self> {
        Self::forge(raw)
    }

    fn as_str(&self) -> &str {
        self.as_str()
    }
}

impl eternalist_apps::CabinetEntry for SavedFilter {
    type Key = FilterName;

    fn key(&self) -> &FilterName {
        &self.name
    }

    fn rename(&mut self, name: FilterName) {
        self.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoolOp, QueryAtom, Tag, TagPolarity};

    #[test]
    fn filter_library_roundtrips_without_session_folds() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let choice = query.push_group(&[], BoolOp::Or).context("push OR")?;
        assert!(query.push_atom(&choice, tag("bikini")?, TagPolarity::Positive));
        assert!(query.push_atom(&choice, tag("nude")?, TagPolarity::Positive));

        let library = FilterLibrary {
            unfiled: vec![SavedFilter::new(
                FilterName::forge("beach").context("filter name")?,
                query.clone(),
            )],
            folders: vec![Folder {
                name: "trips".to_owned(),
                filters: vec![SavedFilter::new(
                    FilterName::forge("shore").context("filter name")?,
                    query.clone(),
                )],
            }],
        };
        let text = toml::to_string_pretty(&library)?;
        let roundtrip = toml::from_str::<FilterLibrary>(&text)?;
        assert_eq!(library, roundtrip);
        assert!(text.contains("[[filter]]"));
        assert!(text.contains("query = \"solo AND (bikini OR nude)\""));
        assert!(text.contains("[[folder]]"));
        assert!(text.contains("[[folder.filter]]"));
        assert!(!text.contains("active_group"));
        assert!(!text.contains(".tree"));
        Ok(())
    }

    #[test]
    fn first_run_seeds_only_the_absent_filter_library() -> Result<()> {
        let seeded = FilterLibrary::first_run();
        assert_eq!(seeded.unfiled.len(), 1);
        assert_eq!(seeded.unfiled[0].name.as_str(), SAFE_DEFAULT_FILTER);
        assert!(toml::to_string(&seeded.unfiled[0])?.contains("rating:g"));
        assert!(FilterLibrary::default().unfiled.is_empty());
        Ok(())
    }

    #[test]
    fn legacy_configuration_migration_preserves_saved_filters() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let configuration_path = directory.path().join("config.toml");
        let library_path = directory.path().join("filters.toml");
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let legacy = LegacyConfiguration {
            prefetch_on_hover: false,
            mirror: MirrorConfig {
                policy: MirrorPolicy::Paused,
            },
            danbooru: DanbooruConfig {
                account: Some(DanbooruAccountConfig {
                    login: "blade".to_owned(),
                    api_key_file: PathBuf::from("secrets/danbooru.token"),
                }),
            },
            filters: LegacyFilterLibrary {
                saved: vec![SavedFilter::new(
                    FilterName::forge("beach").context("filter name")?,
                    query.clone(),
                )]
                .into_iter()
                .map(|filter| LegacySavedFilter {
                    name: filter.name,
                    tree: filter.query,
                    active_group: Vec::new(),
                })
                .collect(),
                shelves: Vec::new(),
            },
        };
        std::fs::write(&configuration_path, toml::to_string_pretty(&legacy)?)?;

        assert!(migrate_legacy_configuration(
            &configuration_path,
            &library_path
        )?);
        let configuration: Configuration =
            toml::from_str(&std::fs::read_to_string(&configuration_path)?)?;
        let library = FilterLibrary::load(&library_path, false)?;

        assert!(!configuration.prefetch_on_hover);
        assert_eq!(configuration.mirror.policy, MirrorPolicy::Paused);
        assert_eq!(configuration.danbooru, legacy.danbooru);
        assert_eq!(library.unfiled.len(), 1);
        assert_eq!(library.unfiled[0].name.as_str(), "beach");
        assert_eq!(library.unfiled[0].query, query);
        Ok(())
    }

    #[test]
    fn session_state_roundtrips_workbench_identity() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let session_state = SessionState {
            closed_folders: std::collections::BTreeSet::from(["trips".to_owned()]),
            panel_folds: std::collections::BTreeMap::from([("gallery-controls".to_owned(), true)]),
            filter: FilterSelection::Saved {
                name: FilterName::forge("beach").context("filter name")?,
            },
            query: QueryConfig {
                tree: query.clone(),
                active_group: Vec::new(),
            },
            sort: Sort::Newest,
            gallery: GalleryTopology::Grouped,
            dates: DateRange {
                first: crate::date::CreatedDay::parse("2024-01-01"),
                last: crate::date::CreatedDay::parse("2024-12-31"),
            },
            images_per_row: 7,
            water: WaterMode::ReallyWet,
            viewer_tags_open: true,
            viewer: Some(ViewerSession {
                post: PostId(44),
                gallery_anchor: PostId(42),
                view: ViewerView::Tree,
                tree_focus: Some(PostId(43)),
                result_horizon: 1_440,
            }),
        };
        let text = toml::to_string_pretty(&session_state)?;
        let roundtrip = toml::from_str::<SessionState>(&text)?;
        assert_eq!(roundtrip.query.tree, query);
        assert!(roundtrip.closed_folders.contains("trips"));
        assert_eq!(roundtrip.panel_folds.get("gallery-controls"), Some(&true));
        assert_eq!(roundtrip.images_per_row, 7);
        assert_eq!(roundtrip.dates, session_state.dates);
        assert_eq!(roundtrip.water, WaterMode::ReallyWet);
        assert!(roundtrip.viewer_tags_open);
        assert_eq!(roundtrip.viewer, session_state.viewer);

        let legacy = toml::from_str::<SessionState>("[shutters]\ngallery-controls = true\n")?;
        assert_eq!(legacy.panel_folds.get("gallery-controls"), Some(&true));

        let favorites = SessionState {
            filter: FilterSelection::LocalFavorites,
            ..SessionState::default()
        };
        let favorites = toml::from_str::<SessionState>(&toml::to_string_pretty(&favorites)?)?;
        assert_eq!(favorites.filter, FilterSelection::LocalFavorites);
        assert!(favorites.query.tree.is_empty());
        Ok(())
    }

    fn tag(raw: &str) -> Result<QueryAtom> {
        Tag::forge(raw).map(QueryAtom::Tag).context("forge tag")
    }
}
