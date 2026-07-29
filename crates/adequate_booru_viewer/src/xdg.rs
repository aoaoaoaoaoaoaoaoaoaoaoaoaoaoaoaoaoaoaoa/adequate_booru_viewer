use anyhow::{Context as _, Result, bail};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Lair {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl Lair {
    pub fn claim() -> Result<Self> {
        let Some(dirs) = ProjectDirs::from("moe", "swarm", "adequate_booru_viewer") else {
            bail!("could not resolve platform project directories");
        };
        // XDG_STATE_HOME on Linux; macOS/Windows fold state into data.
        let state = dirs
            .state_dir()
            .map_or_else(|| dirs.data_local_dir().join("state"), Path::to_path_buf);
        let lair = Self {
            config: dirs.config_dir().to_path_buf(),
            data: dirs.data_local_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            state,
        };
        lair.mkdir()?;
        Ok(lair)
    }

    pub fn slate_path(&self) -> PathBuf {
        self.state.join("slate.toml")
    }

    pub fn index_path(&self) -> PathBuf {
        self.data.join("index.redb")
    }

    pub fn favorites_path(&self) -> PathBuf {
        self.data.join("favorites.roar")
    }

    pub fn config_path(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn media_dir(&self) -> PathBuf {
        self.cache.join("media")
    }

    pub fn model_dir(&self) -> PathBuf {
        self.data.join("models")
    }

    pub fn debug_dir(&self) -> PathBuf {
        self.state.join("debug")
    }

    fn mkdir(&self) -> Result<()> {
        for path in [
            &self.config,
            &self.data,
            &self.cache,
            &self.state,
            &self.media_dir(),
            &self.model_dir(),
            &self.debug_dir(),
        ] {
            std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        }
        Ok(())
    }
}
