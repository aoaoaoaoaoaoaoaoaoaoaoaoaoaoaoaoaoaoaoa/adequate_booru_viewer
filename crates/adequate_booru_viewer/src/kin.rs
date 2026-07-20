//! Danbooru parent trees: sparse durable edges and one dense, mmap-backed
//! direct-parent accelerator. Danbooru bounds a hierarchy to four levels, so
//! family identity is a fixed-depth pointer chase rather than a graph query.
#![expect(
    unsafe_code,
    reason = "memmap2 requires unsafe construction; Atlas owns the file and serializes every mapping access"
)]

use anyhow::{Context as _, Result, bail};
use memmap2::{MmapMut, MmapOptions};
use redb::{Database, ReadableDatabase as _, ReadableTable as _, TableDefinition};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use crate::model::{FamilyBadge, FamilyNode, FamilyTree, Kin, PostId, PostRecord, decode_record};

pub(crate) const PARENTS: TableDefinition<'_, u64, u32> = TableDefinition::new("kin.parents.v1");
pub(crate) const CHILDREN: TableDefinition<'_, u64, &[u8]> =
    TableDefinition::new("kin.children.v1");
pub(crate) const CHILD_HINTS: TableDefinition<'_, u64, u8> =
    TableDefinition::new("kin.child_hints.v1");
pub(crate) const HYDRATED: TableDefinition<'_, u64, u8> = TableDefinition::new("kin.hydrated.v1");
pub(crate) const META: TableDefinition<'_, &str, u64> = TableDefinition::new("kin.meta.v1");

const GENERATION: &str = "generation";
const BACKFILL_BEFORE: &str = "danbooru.backfill.before";
const BACKFILL_DONE: &str = "danbooru.backfill.done";
const MAGIC: &[u8; 8] = b"ABVKIN1\0";
const HEADER: usize = 4096;
const GENERATION_AT: usize = 8;
const SLOTS_AT: usize = 16;
const SLOT_BYTES: usize = size_of::<u32>();
const SLOT_QUANTUM: usize = 1 << 20;
const PAGE: usize = 4096;
const MAX_VALID_DEPTH: usize = 4;
const MAX_DEFENSIVE_DEPTH: usize = 64;
const MAX_FAMILY_NODES: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backfill {
    Running(Option<PostId>),
    Complete,
}

#[derive(Debug)]
pub struct Mutation {
    pub generation: u64,
    pub patches: Vec<(PostId, Option<PostId>)>,
}

/// Runtime family-key accelerator. The redb edge tables are authoritative;
/// this file is a generation-sealed projection and may always be rebuilt.
pub struct Atlas {
    file: File,
    map: MmapMut,
    slots: usize,
    generation: u64,
    path: PathBuf,
}

impl Atlas {
    pub fn open(db: &Database, path: impl Into<PathBuf>, minimum_slots: usize) -> Result<Self> {
        let path = path.into();
        let tx = db.begin_read().context("begin kin atlas read")?;
        let parents = tx.open_table(PARENTS).context("open kin parents")?;
        let meta = tx.open_table(META).context("open kin meta")?;
        let generation = meta
            .get(GENERATION)
            .context("read kin generation")?
            .map_or(0, |value| value.value());
        let minimum_slots = rounded_slots(minimum_slots);
        if let Some(mut atlas) = Self::open_valid(&path, generation)? {
            if atlas.slots < minimum_slots {
                atlas.grow(minimum_slots)?;
            }
            return Ok(atlas);
        }
        Self::rebuild(&path, generation, minimum_slots, &parents)
    }

    pub fn parent(&self, id: PostId) -> Option<PostId> {
        let slot = id.0 as usize;
        if slot >= self.slots {
            return None;
        }
        let offset = slot_offset(slot);
        let parent = read_u32(&self.map, offset);
        (parent != 0).then_some(PostId(parent))
    }

    pub fn root(&self, id: PostId) -> PostId {
        let mut trail = [id.0; MAX_VALID_DEPTH + 1];
        let mut len = 1;
        let mut cursor = id;
        for _ in 0..MAX_VALID_DEPTH {
            let Some(parent) = self.parent(cursor) else {
                return cursor;
            };
            if trail[..len].contains(&parent.0) {
                return PostId(
                    *trail[..len]
                        .iter()
                        .chain([&parent.0])
                        .min()
                        .unwrap_or(&id.0),
                );
            }
            trail[len] = parent.0;
            len += 1;
            cursor = parent;
        }
        self.root_slow(cursor, &trail[..len])
    }

    pub fn apply(&mut self, mutation: &Mutation) -> Result<()> {
        let required = mutation
            .patches
            .iter()
            .flat_map(|(child, parent)| [child.0, parent.unwrap_or(*child).0])
            .max()
            .map_or(0, |id| id as usize + 1);
        if required > self.slots {
            self.grow(rounded_slots(required))?;
        }
        let mut pages = BTreeSet::new();
        for &(child, parent) in &mutation.patches {
            let offset = slot_offset(child.0 as usize);
            write_u32(&mut self.map, offset, parent.map_or(0, |id| id.0));
            let _inserted = pages.insert(offset / PAGE * PAGE);
        }
        for page in pages {
            let len = PAGE.min(self.map.len().saturating_sub(page));
            self.map
                .flush_range(page, len)
                .with_context(|| format!("flush kin atlas page in {}", self.path.display()))?;
        }
        self.generation = mutation.generation;
        write_u64(&mut self.map, GENERATION_AT, self.generation);
        self.map
            .flush_range(0, HEADER)
            .with_context(|| format!("flush kin atlas header in {}", self.path.display()))
    }

    fn open_valid(path: &Path, generation: u64) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("open kin atlas {}", path.display()))?;
        if file.metadata().context("stat kin atlas")?.len() < HEADER as u64 {
            return Ok(None);
        }
        // SAFETY: the file remains owned by the atlas for at least as long as
        // the mapping, and all access is serialized by Index's RwLock.
        let map = unsafe { MmapOptions::new().map_mut(&file) }.context("mmap kin atlas")?;
        if map.get(..MAGIC.len()) != Some(MAGIC) || read_u64(&map, GENERATION_AT) != generation {
            return Ok(None);
        }
        let slots = usize::try_from(read_u64(&map, SLOTS_AT)).context("kin slot count")?;
        if map.len() != atlas_len(slots)? {
            return Ok(None);
        }
        Ok(Some(Self {
            file,
            map,
            slots,
            generation,
            path: path.to_path_buf(),
        }))
    }

    fn rebuild(
        path: &Path,
        generation: u64,
        minimum_slots: usize,
        parents: &impl redb::ReadableTable<u64, u32>,
    ) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("rebuild kin atlas {}", path.display()))?;
        file.set_len(atlas_len(minimum_slots)? as u64)
            .context("size kin atlas")?;
        // SAFETY: see `open_valid`; the newly-sized file is exclusively owned.
        let mut map = unsafe { MmapOptions::new().map_mut(&file) }.context("mmap new kin atlas")?;
        map[..MAGIC.len()].copy_from_slice(MAGIC);
        write_u64(&mut map, GENERATION_AT, generation);
        write_u64(&mut map, SLOTS_AT, minimum_slots as u64);
        for row in parents
            .range(0_u64..=u64::MAX)
            .context("range kin parents")?
        {
            let (child, parent) = row.context("read kin parent")?;
            let child = usize::try_from(child.value()).context("kin child id")?;
            if child < minimum_slots {
                write_u32(&mut map, slot_offset(child), parent.value());
            }
        }
        map.flush().context("flush rebuilt kin atlas")?;
        Ok(Self {
            file,
            map,
            slots: minimum_slots,
            generation,
            path: path.to_path_buf(),
        })
    }

    fn grow(&mut self, slots: usize) -> Result<()> {
        if slots <= self.slots {
            return Ok(());
        }
        self.map.flush().context("flush kin atlas before growth")?;
        let spare = MmapOptions::new()
            .len(HEADER)
            .map_anon()
            .context("allocate kin atlas remap sentinel")?;
        let old = std::mem::replace(&mut self.map, spare);
        drop(old);
        self.file
            .set_len(atlas_len(slots)? as u64)
            .context("grow kin atlas")?;
        // SAFETY: the old mapping has been dropped and the resized file stays
        // alive in `self.file`.
        self.map =
            unsafe { MmapOptions::new().map_mut(&self.file) }.context("remap grown kin atlas")?;
        self.slots = slots;
        self.map[..MAGIC.len()].copy_from_slice(MAGIC);
        write_u64(&mut self.map, GENERATION_AT, self.generation);
        write_u64(&mut self.map, SLOTS_AT, slots as u64);
        self.map.flush().context("flush grown kin atlas")
    }

    fn root_slow(&self, mut cursor: PostId, prefix: &[u32]) -> PostId {
        let mut seen = prefix.iter().copied().collect::<HashSet<_>>();
        let mut least = prefix.iter().copied().min().unwrap_or(cursor.0);
        for _ in MAX_VALID_DEPTH..MAX_DEFENSIVE_DEPTH {
            let Some(parent) = self.parent(cursor) else {
                return cursor;
            };
            least = least.min(parent.0);
            if !seen.insert(parent.0) {
                return PostId(least);
            }
            cursor = parent;
        }
        PostId(least)
    }
}

pub fn absorb(tx: &redb::WriteTransaction, kin: &[Kin]) -> Result<Mutation> {
    let mut parents = tx.open_table(PARENTS).context("open kin parents")?;
    let mut children = tx.open_table(CHILDREN).context("open kin children")?;
    let mut hints = tx.open_table(CHILD_HINTS).context("open kin child hints")?;
    let mut patches = Vec::new();
    let mut changed = false;
    for fact in kin {
        let child = u64::from(fact.id.0);
        let old_parent = parents
            .get(child)
            .context("read prior kin parent")?
            .map(|value| PostId(value.value()));
        if old_parent != fact.parent {
            if let Some(parent) = old_parent {
                amend_children(&mut children, parent, fact.id, false)?;
            }
            if let Some(parent) = fact.parent {
                amend_children(&mut children, parent, fact.id, true)?;
                let _old = parents
                    .insert(child, parent.0)
                    .context("write kin parent")?;
            } else {
                let _old = parents.remove(child).context("remove kin parent")?;
            }
            patches.push((fact.id, fact.parent));
            changed = true;
        }
        let old_hint = hints.get(child).context("read child hint")?.is_some();
        if old_hint != fact.has_children {
            if fact.has_children {
                let _old = hints.insert(child, 1).context("write child hint")?;
            } else {
                let _old = hints.remove(child).context("remove child hint")?;
            }
            changed = true;
        }
    }
    drop(hints);
    drop(children);
    drop(parents);
    let mut meta = tx.open_table(META).context("open kin meta")?;
    let prior = meta
        .get(GENERATION)
        .context("read kin generation")?
        .map_or(0, |value| value.value());
    let generation = prior + u64::from(changed);
    if changed {
        let _old = meta
            .insert(GENERATION, generation)
            .context("advance kin generation")?;
    }
    Ok(Mutation {
        generation,
        patches,
    })
}

pub fn backfill(db: &Database) -> Result<Backfill> {
    let tx = db.begin_read().context("begin kin backfill read")?;
    let meta = tx.open_table(META).context("open kin meta")?;
    if meta
        .get(BACKFILL_DONE)
        .context("read kin completion")?
        .is_some()
    {
        return Ok(Backfill::Complete);
    }
    let before = meta
        .get(BACKFILL_BEFORE)
        .context("read kin cursor")?
        .map(|value| crate::model::narrow_post_id(value.value()))
        .transpose()?;
    Ok(Backfill::Running(before))
}

pub fn hydrated(db: &Database, root: PostId) -> Result<bool> {
    let tx = db.begin_read().context("begin kin hydration read")?;
    let meta = tx.open_table(META).context("open kin meta")?;
    if meta
        .get(BACKFILL_DONE)
        .context("read kin completion")?
        .is_some()
    {
        return Ok(true);
    }
    tx.open_table(HYDRATED)
        .context("open hydrated families")?
        .get(u64::from(root.0))
        .context("read family hydration")
        .map(|guard| guard.is_some())
}

pub fn seal_hydrated(tx: &redb::WriteTransaction, root: PostId) -> Result<()> {
    let mut hydrated = tx.open_table(HYDRATED).context("open hydrated families")?;
    let _old = hydrated
        .insert(u64::from(root.0), 1)
        .context("seal hydrated family")?;
    Ok(())
}

pub fn advance_backfill(
    tx: &redb::WriteTransaction,
    before: Option<PostId>,
    complete: bool,
) -> Result<()> {
    let mut meta = tx.open_table(META).context("open kin meta")?;
    if let Some(before) = before {
        let _old = meta
            .insert(BACKFILL_BEFORE, u64::from(before.0))
            .context("write kin cursor")?;
    }
    if complete {
        let _old = meta.insert(BACKFILL_DONE, 1).context("seal kin backfill")?;
    }
    Ok(())
}

pub fn family_tree(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    children: &impl redb::ReadableTable<u64, &'static [u8]>,
    hints: &impl redb::ReadableTable<u64, u8>,
    atlas: &Atlas,
    focus: PostId,
) -> Result<FamilyTree> {
    let mut nodes = BTreeMap::new();
    let root = walk_family(children, atlas, focus, |id, direct| {
        let post = posts
            .get(u64::from(id.0))
            .context("read family post")?
            .map(|record| decode_record(record.value()))
            .transpose()?
            .filter(PostRecord::indexable);
        let hinted = hints
            .get(u64::from(id.0))
            .context("read family child hint")?
            .is_some();
        let _old = nodes.insert(
            id,
            FamilyNode {
                id,
                parent: atlas.parent(id),
                incomplete: hinted && direct.is_empty(),
                children: direct.to_vec(),
                post,
            },
        );
        Ok(())
    })?;
    Ok(FamilyTree { root, focus, nodes })
}

pub fn family_badge(
    posts: &impl redb::ReadableTable<u64, &'static [u8]>,
    children: &impl redb::ReadableTable<u64, &'static [u8]>,
    hints: &impl redb::ReadableTable<u64, u8>,
    atlas: &Atlas,
    focus: PostId,
) -> Result<Option<FamilyBadge>> {
    let mut count = 0_usize;
    let mut incomplete = false;
    let _root = walk_family(children, atlas, focus, |id, direct| {
        count += usize::from(
            posts
                .get(u64::from(id.0))
                .context("read family member existence")?
                .is_some(),
        );
        incomplete |= direct.is_empty()
            && hints
                .get(u64::from(id.0))
                .context("read family child hint")?
                .is_some();
        Ok(())
    })?;
    Ok((count > 1 || incomplete).then_some(FamilyBadge {
        posts: u16::try_from(count).unwrap_or(u16::MAX),
        incomplete,
    }))
}

fn walk_family(
    children: &impl redb::ReadableTable<u64, &'static [u8]>,
    atlas: &Atlas,
    focus: PostId,
    mut visit: impl FnMut(PostId, &[PostId]) -> Result<()>,
) -> Result<PostId> {
    let root = atlas.root(focus);
    let mut queue = VecDeque::from([root]);
    let mut visited = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if visited.len() > MAX_FAMILY_NODES {
            bail!("family rooted at #{root} exceeds {MAX_FAMILY_NODES} nodes");
        }
        let direct = read_children(children, id)?;
        visit(id, &direct)?;
        queue.extend(direct);
    }
    Ok(root)
}

fn amend_children(
    table: &mut redb::Table<'_, u64, &'static [u8]>,
    parent: PostId,
    child: PostId,
    insert: bool,
) -> Result<()> {
    let key = u64::from(parent.0);
    let mut ids = table
        .get(key)
        .context("read kin children")?
        .map(|value| decode_children(value.value()))
        .transpose()?
        .unwrap_or_default();
    match ids.binary_search(&child) {
        Ok(slot) if !insert => {
            let _removed = ids.remove(slot);
        }
        Err(slot) if insert => ids.insert(slot, child),
        Ok(_) | Err(_) => return Ok(()),
    }
    if ids.is_empty() {
        let _old = table.remove(key).context("remove empty child list")?;
    } else {
        let encoded = encode_children(&ids);
        let _old = table
            .insert(key, encoded.as_slice())
            .context("write child list")?;
    }
    Ok(())
}

fn read_children(
    table: &impl redb::ReadableTable<u64, &'static [u8]>,
    parent: PostId,
) -> Result<Vec<PostId>> {
    table
        .get(u64::from(parent.0))
        .context("read family children")?
        .map(|value| decode_children(value.value()))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn encode_children(ids: &[PostId]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ids.len() * SLOT_BYTES);
    for id in ids {
        bytes.extend_from_slice(&id.0.to_le_bytes());
    }
    bytes
}

fn decode_children(bytes: &[u8]) -> Result<Vec<PostId>> {
    if !bytes.len().is_multiple_of(SLOT_BYTES) {
        bail!("malformed kin child list of {} bytes", bytes.len());
    }
    Ok(bytes
        .chunks_exact(SLOT_BYTES)
        .map(|word| PostId(u32::from_le_bytes([word[0], word[1], word[2], word[3]])))
        .collect())
}

fn rounded_slots(slots: usize) -> usize {
    slots.div_ceil(SLOT_QUANTUM) * SLOT_QUANTUM
}

fn atlas_len(slots: usize) -> Result<usize> {
    slots
        .checked_mul(SLOT_BYTES)
        .and_then(|bytes| HEADER.checked_add(bytes))
        .context("kin atlas length overflow")
}

fn slot_offset(slot: usize) -> usize {
    HEADER + slot * SLOT_BYTES
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
