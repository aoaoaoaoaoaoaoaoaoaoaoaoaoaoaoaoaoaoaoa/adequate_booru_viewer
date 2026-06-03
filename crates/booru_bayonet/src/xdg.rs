use anyhow::{Context as _, Result, bail};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Lair {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Lair {
    pub fn claim() -> Result<Self> {
        let Some(dirs) = ProjectDirs::from("moe", "swarm", "booru_bayonet") else {
            bail!("could not resolve platform project directories");
        };
        let lair = Self {
            config: dirs.config_dir().to_path_buf(),
            data: dirs.data_local_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
        };
        lair.mkdir()?;
        Ok(lair)
    }

    pub fn index_path(&self) -> PathBuf {
        self.data.join("index.redb")
    }

    pub fn media_dir(&self) -> PathBuf {
        self.cache.join("media")
    }

    pub fn model_dir(&self) -> PathBuf {
        self.data.join("models")
    }

    fn mkdir(&self) -> Result<()> {
        for path in [
            &self.config,
            &self.data,
            &self.cache,
            &self.media_dir(),
            &self.model_dir(),
        ] {
            std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        }
        Ok(())
    }
}

pub fn compact_path(path: &Path) -> String {
    path.display().to_string()
}
