//! Storage for a sealed index: the file is read-only and shared-locked, and
//! every write redb issues, including the repair an uncleanly closed index
//! demands, lands in process memory and dies with it.

use std::{
    collections::HashMap,
    fs::{File, TryLockError},
    io, iter,
    os::unix::fs::FileExt as _,
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

use anyhow::{Context as _, Result, bail};
use redb::StorageBackend;

const BLOCK: u64 = 4096;

#[derive(Debug)]
pub struct SealedFile {
    file: File,
    ink: Mutex<Ink>,
}

/// The overlay: whole blocks written since opening, the virtual length, and
/// the prefix of the file still visible beneath them.
#[derive(Debug)]
struct Ink {
    blocks: HashMap<u64, Box<[u8; BLOCK as usize]>>,
    len: u64,
    floor: u64,
}

impl SealedFile {
    /// Open an existing index file. The shared lock refuses a live writer,
    /// whose exclusive lock it would otherwise race.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        match file.try_lock_shared() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!("{} is open writable in another process", path.display())
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("lock {}", path.display()));
            }
        }
        let len = file
            .metadata()
            .with_context(|| format!("stat {}", path.display()))?
            .len();
        if len == 0 {
            bail!("{} holds no index", path.display());
        }
        Ok(Self {
            file,
            ink: Mutex::new(Ink {
                blocks: HashMap::new(),
                len,
                floor: len,
            }),
        })
    }

    fn ink(&self) -> MutexGuard<'_, Ink> {
        self.ink.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Ink {
    /// Fill `out` with the base file's bytes at `offset`, zero past the floor.
    fn base(&self, file: &File, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let visible = usize::try_from(self.floor.saturating_sub(offset))
            .unwrap_or(usize::MAX)
            .min(out.len());
        let (from_file, beyond) = out.split_at_mut(visible);
        file.read_exact_at(from_file, offset)?;
        beyond.fill(0);
        Ok(())
    }
}

impl StorageBackend for SealedFile {
    fn len(&self) -> io::Result<u64> {
        Ok(self.ink().len)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let ink = self.ink();
        let mut cursor = 0;
        for (block, within, take) in spans(offset, out.len()) {
            let chunk = &mut out[cursor..cursor + take];
            match ink.blocks.get(&block) {
                Some(bytes) => chunk.copy_from_slice(&bytes[within..within + take]),
                None => ink.base(&self.file, offset + cursor as u64, chunk)?,
            }
            cursor += take;
        }
        Ok(())
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let mut ink = self.ink();
        if len < ink.len {
            ink.floor = ink.floor.min(len);
            ink.blocks.retain(|&block, _| block * BLOCK < len);
            if let Some(bytes) = ink.blocks.get_mut(&(len / BLOCK)) {
                bytes[(len % BLOCK) as usize..].fill(0);
            }
        }
        ink.len = len;
        Ok(())
    }

    fn sync_data(&self) -> io::Result<()> {
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        let mut ink = self.ink();
        let mut cursor = 0;
        for (block, within, take) in spans(offset, data.len()) {
            if !ink.blocks.contains_key(&block) {
                let mut fresh = Box::new([0; BLOCK as usize]);
                ink.base(&self.file, block * BLOCK, &mut fresh[..])?;
                let _absent = ink.blocks.insert(block, fresh);
            }
            if let Some(bytes) = ink.blocks.get_mut(&block) {
                bytes[within..within + take].copy_from_slice(&data[cursor..cursor + take]);
            }
            cursor += take;
        }
        ink.len = ink.len.max(offset + data.len() as u64);
        Ok(())
    }
}

/// The block-local spans covering `len` bytes at `offset`: block, start
/// within it, and length.
fn spans(offset: u64, len: usize) -> impl Iterator<Item = (u64, usize, usize)> {
    let end = offset + len as u64;
    let mut at = offset;
    iter::from_fn(move || {
        (at < end).then(|| {
            let (block, within) = (at / BLOCK, (at % BLOCK) as usize);
            let take = (BLOCK as usize - within).min((end - at) as usize);
            at += take as u64;
            (block, within, take)
        })
    })
}
