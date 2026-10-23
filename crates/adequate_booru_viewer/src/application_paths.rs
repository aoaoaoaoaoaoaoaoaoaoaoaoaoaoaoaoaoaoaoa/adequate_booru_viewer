use anyhow::{Context as _, Result};
use directories::ProjectDirs;
pub use eternalist_apps::ApplicationPaths;
use eternalist_apps::ProductIdentity;
use std::{
    fs,
    path::{Path, PathBuf},
};

mod adoption;

use adoption::Root;
pub use adoption::Strand;

pub const PRODUCT: ProductIdentity =
    ProductIdentity::declare(abv_contract::PRODUCT_IDENTIFIER, abv_contract::PRODUCT_NAME);

/// The `ProjectDirs` triple that rooted every release through 1.3.16. Releases
/// 1.3.17–1.3.19 derived the roots from [`PRODUCT`] instead, moved nothing, and
/// created empty successors beside the stranded originals.
pub const LEGACY_TRIPLE: (&str, &str, &str) = ("moe", "swarm", "adequate_booru_viewer");

const INDEX: &str = "index.redb";

/// The platform directories, claimed for one launch.
pub struct Claim {
    pub paths: ApplicationPaths,
    /// Legacy roots this launch left in place.
    pub strands: Vec<Strand>,
}

impl Claim {
    /// Adopt the legacy roots, then create every root the viewer writes.
    ///
    /// Adopt before anything touches a root: a successor that fills first
    /// strands its legacy predecessor.
    pub fn adopting() -> Result<Self> {
        let paths = ApplicationPaths::claim(PRODUCT)?;
        let strands = adopt_legacy_roots(&paths);
        prepare(&paths)?;
        Ok(Self { paths, strands })
    }

    /// Create every root the viewer writes, leaving any legacy roots untouched.
    pub fn sparing() -> Result<Self> {
        let paths = ApplicationPaths::claim(PRODUCT)?;
        prepare(&paths)?;
        Ok(Self {
            paths,
            strands: Vec::new(),
        })
    }
}

fn prepare(paths: &ApplicationPaths) -> Result<()> {
    paths.prepare()?;
    for path in [paths.media_dir(), paths.model_dir(), paths.debug_dir()] {
        fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    }
    Ok(())
}

fn adopt_legacy_roots(paths: &ApplicationPaths) -> Vec<Strand> {
    let (qualifier, organization, application) = LEGACY_TRIPLE;
    let Some(legacy) = ProjectDirs::from(qualifier, organization, application) else {
        return Vec::new();
    };
    let root = |legacy: &Path, successor: &Path| Root {
        legacy: legacy.to_path_buf(),
        successor: successor.to_path_buf(),
    };
    let mut installation = vec![
        root(legacy.config_dir(), &paths.config),
        root(legacy.data_local_dir(), &paths.local_data),
    ];
    // Without a platform state directory, legacy state lives beneath legacy
    // data and travels with it.
    installation.extend(legacy.state_dir().map(|state| root(state, &paths.state)));
    // macOS keeps config and data in one directory.
    installation.dedup_by(|later, earlier| later.legacy == earlier.legacy);
    adoption::adopt(
        &installation,
        &root(legacy.cache_dir(), &paths.cache),
        [&legacy.data_local_dir().join(INDEX), &paths.index_path()],
    )
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
        self.local_data.join(INDEX)
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
