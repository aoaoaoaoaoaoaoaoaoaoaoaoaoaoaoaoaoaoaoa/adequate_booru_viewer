use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

use crate::model::{Query, Sort};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub query: QueryConfig,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

impl QueryConfig {
    pub fn query(&self) -> Query {
        if self.tree.is_empty() && (!self.include.is_empty() || !self.exclude.is_empty()) {
            Query::parse(&legacy_query_text(self))
        } else {
            self.tree.clone()
        }
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

fn legacy_query_text(config: &QueryConfig) -> String {
    config
        .include
        .iter()
        .cloned()
        .chain(config.exclude.iter().map(|tag| format!("-{tag}")))
        .collect::<Vec<_>>()
        .join(" ")
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
                include: Vec::new(),
                exclude: Vec::new(),
            },
            view: ViewConfig::default(),
            soft: SoftConfig::default(),
        };
        let text = toml::to_string_pretty(&config)?;
        let roundtrip = toml::from_str::<Config>(&text)?;
        assert_eq!(roundtrip.query.query(), query);
        assert_eq!(roundtrip.query.active_group, choice);
        Ok(())
    }

    fn tag(raw: &str) -> Result<QueryAtom> {
        Tag::forge(raw).map(QueryAtom::Tag).context("forge tag")
    }
}
