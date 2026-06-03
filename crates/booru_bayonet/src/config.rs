use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

use crate::model::Sort;

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
    pub include: Vec<String>,
    pub exclude: Vec<String>,
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
