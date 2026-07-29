use anyhow::{Context as _, Result, bail};
use roaring::RoaringBitmap;
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::model::PostId;

const MAGIC: &[u8; 8] = b"ABVFAV01";

/// Durable user-owned post identity, independent of the disposable booru
/// index. Snapshots are `Arc`-cheap because every retrieval owns the exact
/// favorite universe against which it was ordered.
#[derive(Clone, Debug)]
pub struct LocalFavorites {
    path: PathBuf,
    posts: Arc<RoaringBitmap>,
}

impl LocalFavorites {
    pub fn load(path: PathBuf) -> Result<Self> {
        let posts = if path.exists() {
            read_bitmap(&path)?
        } else {
            RoaringBitmap::new()
        };
        Ok(Self {
            path,
            posts: Arc::new(posts),
        })
    }

    pub fn contains(&self, id: PostId) -> bool {
        self.posts.contains(id.0)
    }

    pub fn snapshot(&self) -> Arc<RoaringBitmap> {
        Arc::clone(&self.posts)
    }

    /// Flip one post and durably commit before exposing the new snapshot.
    pub fn toggle(&mut self, id: PostId) -> Result<bool> {
        let mut next = (*self.posts).clone();
        let favorite = if next.remove(id.0) {
            false
        } else {
            let _inserted = next.insert(id.0);
            true
        };
        write_bitmap(&self.path, &next)?;
        self.posts = Arc::new(next);
        Ok(favorite)
    }
}

fn read_bitmap(path: &Path) -> Result<RoaringBitmap> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut input = BufReader::new(file);
    let mut magic = [0_u8; MAGIC.len()];
    input
        .read_exact(&mut magic)
        .with_context(|| format!("read {} header", path.display()))?;
    if &magic != MAGIC {
        bail!("{} is not an ABV favorites file", path.display());
    }
    let posts = RoaringBitmap::deserialize_from(&mut input)
        .with_context(|| format!("decode {}", path.display()))?;
    let mut tail = [0_u8; 1];
    if input
        .read(&mut tail)
        .with_context(|| format!("verify {}", path.display()))?
        != 0
    {
        bail!("{} has trailing bytes", path.display());
    }
    Ok(posts)
}

fn write_bitmap(path: &Path, posts: &RoaringBitmap) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("favorites path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let next = path.with_extension("roar.next");
    {
        let file = File::create(&next).with_context(|| format!("create {}", next.display()))?;
        let mut output = BufWriter::new(file);
        output
            .write_all(MAGIC)
            .with_context(|| format!("write {} header", next.display()))?;
        posts
            .serialize_into(&mut output)
            .with_context(|| format!("encode {}", next.display()))?;
        output
            .flush()
            .with_context(|| format!("flush {}", next.display()))?;
        output
            .get_ref()
            .sync_all()
            .with_context(|| format!("sync {}", next.display()))?;
    }
    fs::rename(&next, path)
        .with_context(|| format!("replace {} with {}", path.display(), next.display()))?;
    #[cfg(unix)]
    File::open(parent)
        .with_context(|| format!("open {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("sync {}", parent.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn toggle_round_trips_canonically() -> Result<()> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("clock before Unix epoch")?
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("abv-favorites-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).context("create test directory")?;
        let path = dir.join("favorites.roar");

        let mut favorites = LocalFavorites::load(path.clone())?;
        assert!(favorites.toggle(PostId(42))?);
        assert!(favorites.contains(PostId(42)));
        assert!(LocalFavorites::load(path.clone())?.contains(PostId(42)));
        assert!(!favorites.toggle(PostId(42))?);
        assert!(!LocalFavorites::load(path)?.contains(PostId(42)));

        fs::remove_dir_all(dir).context("remove test directory")
    }
}
