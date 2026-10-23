//! Adoption of the roots releases through 1.3.16 kept under
//! [`LEGACY_TRIPLE`](super::LEGACY_TRIPLE).
//!
//! Config, data, and state form one installation, adopted all or none; the
//! cache is disposable and follows alone when its successor is vacant. Every
//! move refuses to replace anything, so an entry that appears after the survey
//! fails the move instead of vanishing.

use std::{
    fmt,
    fs::{self, File, TryLockError},
    io,
    path::{Path, PathBuf},
};

/// A legacy root and the root that succeeds it.
pub(super) struct Root {
    pub(super) legacy: PathBuf,
    pub(super) successor: PathBuf,
}

/// Why legacy roots stay where they are.
pub enum Strand {
    /// The successor holds files or is no plain directory; adoption would
    /// merge or replace.
    Occupied { legacy: PathBuf, successor: PathBuf },
    /// Another process holds this index open; any move would split its state.
    Busy { index: PathBuf },
    /// The path could not be inspected; nothing moved.
    Unreadable { path: PathBuf, cause: io::Error },
    /// The move failed; every earlier move was reverted.
    Refused {
        legacy: PathBuf,
        successor: PathBuf,
        cause: io::Error,
    },
    /// An earlier move could not be reverted after a later refusal.
    Unreverted {
        legacy: PathBuf,
        successor: PathBuf,
        cause: io::Error,
    },
}

impl fmt::Display for Strand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occupied { legacy, successor } => write!(
                formatter,
                "{} not adopted: {} already holds files; move it aside and relaunch",
                legacy.display(),
                successor.display()
            ),
            Self::Busy { index } => write!(
                formatter,
                "legacy data not adopted: {} is open in another process; quit it and relaunch",
                index.display()
            ),
            Self::Unreadable { path, cause } => write!(
                formatter,
                "legacy data not adopted: cannot inspect {}: {cause}",
                path.display()
            ),
            Self::Refused {
                legacy,
                successor,
                cause,
            } => write!(
                formatter,
                "{} not adopted: moving it to {} failed: {cause}",
                legacy.display(),
                successor.display()
            ),
            Self::Unreverted {
                legacy,
                successor,
                cause,
            } => write!(
                formatter,
                "{} holds legacy data that could not be moved back to {}: {cause}; move it back by hand",
                successor.display(),
                legacy.display()
            ),
        }
    }
}

/// What lies at a path, judged without following a symlink.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Lot {
    Absent,
    /// A directory tree holding nothing but directories.
    Vacant,
    /// Files, a symlink, or anything else that is no plain directory.
    Occupied,
}

/// Adopt the installation all or none, then the cache alone, holding both
/// indexes' locks throughout. Nothing is locked or reported unless some legacy
/// root holds files.
pub(super) fn adopt(installation: &[Root], cache: &Root, indexes: [&Path; 2]) -> Vec<Strand> {
    if !installation
        .iter()
        .chain([cache])
        .any(|root| !matches!(survey(&root.legacy), Ok(Lot::Absent | Lot::Vacant)))
    {
        return Vec::new();
    }
    let mut locks = Vec::with_capacity(indexes.len());
    for index in indexes {
        match seize(index) {
            Ok(lock) => locks.extend(lock),
            Err(strand) => return vec![strand],
        }
    }
    let strands = adopt_installation(installation);
    if let (Ok(Lot::Occupied), Ok(lot @ (Lot::Absent | Lot::Vacant))) =
        (survey(&cache.legacy), survey(&cache.successor))
    {
        // A cache left behind is disposable, not stranded.
        let _settled = settle(cache, lot);
    }
    drop(locks);
    strands
}

fn adopt_installation(installation: &[Root]) -> Vec<Strand> {
    let mut moves = Vec::with_capacity(installation.len());
    let mut strands = Vec::new();
    for root in installation {
        match (survey(&root.legacy), survey(&root.successor)) {
            (Err(cause), _) => strands.push(Strand::Unreadable {
                path: root.legacy.clone(),
                cause,
            }),
            (_, Err(cause)) => strands.push(Strand::Unreadable {
                path: root.successor.clone(),
                cause,
            }),
            (Ok(Lot::Absent | Lot::Vacant), Ok(_)) => {}
            (Ok(Lot::Occupied), Ok(Lot::Occupied)) => strands.push(Strand::Occupied {
                legacy: root.legacy.clone(),
                successor: root.successor.clone(),
            }),
            (Ok(Lot::Occupied), Ok(lot)) => moves.push((root, lot)),
        }
    }
    if !strands.is_empty() {
        return strands;
    }
    let mut settled: Vec<&Root> = Vec::with_capacity(moves.len());
    for (root, lot) in moves {
        if let Err(cause) = settle(root, lot) {
            let mut strands = vec![Strand::Refused {
                legacy: root.legacy.clone(),
                successor: root.successor.clone(),
                cause,
            }];
            for root in settled.into_iter().rev() {
                if let Err(cause) = displace(&root.successor, &root.legacy) {
                    strands.push(Strand::Unreverted {
                        legacy: root.legacy.clone(),
                        successor: root.successor.clone(),
                        cause,
                    });
                }
            }
            return strands;
        }
        settled.push(root);
    }
    Vec::new()
}

/// Move a legacy root onto its absent or vacant successor.
fn settle(root: &Root, lot: Lot) -> io::Result<()> {
    if let Some(parent) = root.successor.parent() {
        fs::create_dir_all(parent)?;
    }
    if lot == Lot::Vacant {
        vacate(&root.successor)?;
    }
    displace(&root.legacy, &root.successor)
}

fn survey(path: &Path) -> io::Result<Lot> {
    let entry = match fs::symlink_metadata(path) {
        Ok(entry) => entry,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Lot::Absent),
        Err(error) => return Err(error),
    };
    Ok(if entry.is_dir() && hollow(path)? {
        Lot::Vacant
    } else {
        Lot::Occupied
    })
}

fn hollow(directory: &Path) -> io::Result<bool> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !hollow(&entry.path())? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Remove a hollow directory tree. Only `remove_dir` deletes, and it refuses
/// anything nonempty, so a file that appeared since the survey survives and
/// fails the removal; a symlink is never followed.
fn vacate(directory: &Path) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            return Err(io::ErrorKind::DirectoryNotEmpty.into());
        }
        vacate(&entry.path())?;
    }
    fs::remove_dir(directory)
}

/// Rename `from` to `to`, a symlink as the link itself, refusing to replace
/// whatever exists at `to`.
#[cfg(unix)]
fn displace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    // renameat2(RENAME_NOREPLACE) on Linux, renameatx_np(RENAME_EXCL) on macOS.
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

/// Rename `from` to `to`, a symlink as the link itself, refusing to replace
/// whatever exists at `to`.
#[cfg(windows)]
fn displace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<u16>>()
    };
    let (from, to) = (wide(from), wide(to));
    // SAFETY: both arguments are NUL-terminated UTF-16 paths that outlive the
    // call. Without MOVEFILE_REPLACE_EXISTING the move refuses an existing
    // destination.
    #[expect(unsafe_code, reason = "std offers no move that refuses to replace")]
    let moved = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Take the exclusive lock redb holds on an open index (`File::try_lock`:
/// `flock(2)` on Linux, `LockFileEx` on Windows) and keep it until dropped.
fn seize(index: &Path) -> Result<Option<File>, Strand> {
    let file = match File::open(index) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => {
            return Err(Strand::Unreadable {
                path: index.to_path_buf(),
                cause,
            });
        }
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Err(Strand::Busy {
            index: index.to_path_buf(),
        }),
        // redb itself proceeds unlocked where the platform has no file locks.
        Err(TryLockError::Error(cause)) if cause.kind() == io::ErrorKind::Unsupported => Ok(None),
        Err(TryLockError::Error(cause)) => Err(Strand::Unreadable {
            path: index.to_path_buf(),
            cause,
        }),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{Root, Strand, adopt};
    use std::{fs, io, os::unix::fs::symlink, path::Path};
    use tempfile::TempDir;

    /// Config, data, and state legacy roots, each holding a file.
    fn installation(base: &Path) -> io::Result<[Root; 3]> {
        let roots = ["config", "data", "state"].map(|kind| Root {
            legacy: base.join("legacy").join(kind),
            successor: base.join("successor").join(kind),
        });
        for root in &roots {
            fs::create_dir_all(&root.legacy)?;
            fs::write(root.legacy.join("held"), b"legacy")?;
        }
        Ok(roots)
    }

    fn run(base: &Path, roots: &[Root]) -> Vec<Strand> {
        let cache = Root {
            legacy: base.join("legacy/cache"),
            successor: base.join("successor/cache"),
        };
        adopt(
            roots,
            &cache,
            [&base.join("legacy.redb"), &base.join("successor.redb")],
        )
    }

    // rename(2) silently replaces a symlink at its destination, orphaning
    // whatever the link named, even when the target looks empty. One occupied
    // successor must also hold back the members that could move, or legacy
    // config would run against fresh data.
    #[test]
    fn symlinked_successor_holds_back_the_installation() -> io::Result<()> {
        let base = TempDir::new()?;
        let roots = installation(base.path())?;
        fs::create_dir_all(base.path().join("elsewhere"))?;
        fs::create_dir_all(base.path().join("successor"))?;
        symlink("../elsewhere", &roots[1].successor)?;
        let strands = run(base.path(), &roots);
        assert!(
            matches!(strands.as_slice(), [Strand::Occupied { successor, .. }] if *successor == roots[1].successor)
        );
        assert!(fs::symlink_metadata(&roots[1].successor)?.is_symlink());
        for root in &roots {
            assert!(root.legacy.join("held").is_file());
        }
        assert!(!roots[0].successor.exists() && !roots[2].successor.exists());
        Ok(())
    }

    // A refusal after earlier moves must move them back, or the installation
    // splits across both layouts.
    #[test]
    fn refusal_reverts_earlier_moves() -> io::Result<()> {
        let base = TempDir::new()?;
        let mut roots = installation(base.path())?;
        // rename(2) refuses to move a directory beneath itself.
        roots[2].successor = roots[2].legacy.join("beneath");
        let strands = run(base.path(), &roots);
        assert!(matches!(strands.as_slice(), [Strand::Refused { .. }]));
        for root in &roots {
            assert!(root.legacy.join("held").is_file());
        }
        assert!(!roots[0].successor.exists() && !roots[1].successor.exists());
        Ok(())
    }
}
