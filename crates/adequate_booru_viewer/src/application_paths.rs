use anyhow::{Context as _, Result};
pub use eternalist_apps::ApplicationPaths;
use eternalist_apps::ProductIdentity;
use std::path::PathBuf;

pub const PRODUCT: ProductIdentity =
    ProductIdentity::declare(abv_contract::PRODUCT_IDENTIFIER, abv_contract::PRODUCT_NAME);

/// Resolve the platform directories and create every root the viewer writes.
pub fn claim() -> Result<ApplicationPaths> {
    let paths = ApplicationPaths::claim(PRODUCT)?;
    paths.prepare()?;
    for path in [paths.media_dir(), paths.model_dir(), paths.debug_dir()] {
        std::fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    }
    Ok(paths)
}

/// The viewer's files beneath the platform directories.
pub trait ViewerPaths {
    fn session_state_path(&self) -> PathBuf;
    fn index_path(&self) -> PathBuf;
    fn favorites_path(&self) -> PathBuf;
    fn config_path(&self) -> PathBuf;
    fn filter_library_path(&self) -> PathBuf;
    fn media_dir(&self) -> PathBuf;
    fn model_dir(&self) -> PathBuf;
    fn debug_dir(&self) -> PathBuf;
}

impl ViewerPaths for ApplicationPaths {
    fn session_state_path(&self) -> PathBuf {
        // Filename is a durable compatibility boundary predating the type's
        // rectified name.
        self.state.join("slate.toml")
    }

    fn index_path(&self) -> PathBuf {
        self.local_data.join("index.redb")
    }

    fn favorites_path(&self) -> PathBuf {
        self.local_data.join("favorites.roar")
    }

    fn config_path(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    fn filter_library_path(&self) -> PathBuf {
        self.local_data.join("filters.toml")
    }

    fn media_dir(&self) -> PathBuf {
        self.cache.join("media")
    }

    fn model_dir(&self) -> PathBuf {
        self.local_data.join("models")
    }

    fn debug_dir(&self) -> PathBuf {
        self.state.join("debug")
    }
}
