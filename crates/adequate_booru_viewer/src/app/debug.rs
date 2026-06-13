use super::*;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

impl Bayonet {
    pub fn debug_dump_path(&self) -> Result<PathBuf> {
        let dir = self.lair.debug_dir();
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before unix epoch")?;
        Ok(dir.join(format!(
            "abv-water-{}-{:03}.abvdump",
            now.as_secs(),
            now.subsec_millis()
        )))
    }

    pub fn report_debug_dump(&mut self, result: Result<&Path>) {
        self.status = match result {
            Ok(path) => format!("debug dump {}", path.display()),
            Err(err) => format!("debug dump failed: {err:#}"),
        };
    }

    pub fn purge_debug_dumps(&mut self) {
        self.status = match purge_debug_dumps(&self.lair.debug_dir()) {
            Ok(purge) => format!(
                "purged {} debug dumps ({:.1} MiB)",
                purge.files,
                purge.bytes as f64 / (1024.0 * 1024.0)
            ),
            Err(err) => format!("purge debug dumps failed: {err:#}"),
        };
    }
}

struct DumpPurge {
    files: u64,
    bytes: u64,
}

fn purge_debug_dumps(dir: &Path) -> Result<DumpPurge> {
    let mut purge = DumpPurge { files: 0, bytes: 0 };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(purge),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("abvdump") {
            continue;
        }
        let meta = entry
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?;
        if !meta.is_file() {
            continue;
        }
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        purge.files += 1;
        purge.bytes += meta.len();
    }
    Ok(purge)
}
