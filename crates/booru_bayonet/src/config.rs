use anyhow::{Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use std::{
    fmt::{Display, Formatter},
    path::Path,
    sync::Arc,
};

use crate::model::{Query, Sort};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub query: QueryConfig,
    pub filters: FilterConfig,
    pub view: ViewConfig,
    pub soft: SoftConfig,
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialize config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("replace {} with {}", path.display(), tmp.display()))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryConfig {
    pub tree: Query,
    pub active_group: Vec<usize>,
}

impl QueryConfig {
    pub fn query(&self) -> Query {
        self.tree.clone()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilterConfig {
    pub active: Option<FilterName>,
    pub saved: Vec<SavedFilter>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

impl Serialize for FilterName {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FilterName {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::forge(&raw).ok_or_else(|| D::Error::custom("filter name is empty"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewConfig {
    pub sort: Sort,
    pub tile_scale: f32,
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            sort: Sort::Score,
            tile_scale: 1.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SoftConfig {
    pub prompt: String,
    pub alpha: f32,
}

impl Default for SoftConfig {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            alpha: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoolOp, QueryAtom, Tag, TagPolarity};

    #[test]
    fn config_roundtrips_boolean_query_tree() -> Result<()> {
        let mut query = Query::default();
        assert!(query.push_atom(&[], tag("solo")?, TagPolarity::Positive));
        let choice = query.push_group(&[], BoolOp::Or).context("push OR")?;
        assert!(query.push_atom(&choice, tag("bikini")?, TagPolarity::Positive));
        assert!(query.push_atom(&choice, tag("nude")?, TagPolarity::Positive));

        let config = Config {
            query: QueryConfig {
                tree: query.clone(),
                active_group: choice.clone(),
            },
            filters: FilterConfig {
                active: Some(FilterName::forge("beach").context("active filter name")?),
                saved: vec![SavedFilter::new(
                    FilterName::forge("beach").context("filter name")?,
                    query.clone(),
                    choice.clone(),
                )],
            },
            view: ViewConfig::default(),
            soft: SoftConfig::default(),
        };
        let text = toml::to_string_pretty(&config)?;
        let roundtrip = toml::from_str::<Config>(&text)?;
        assert_eq!(roundtrip.query.query(), query);
        assert_eq!(roundtrip.query.active_group, choice);
        assert_eq!(
            roundtrip.filters.active.as_ref().map(FilterName::as_str),
            Some("beach")
        );
        assert_eq!(roundtrip.filters.saved[0].name.as_str(), "beach");
        assert_eq!(roundtrip.filters.saved[0].active_group, choice);
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
