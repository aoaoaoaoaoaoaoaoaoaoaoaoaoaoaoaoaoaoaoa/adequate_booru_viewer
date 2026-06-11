use anyhow::{Context as _, Result, bail};
use roaring::RoaringBitmap;
use std::{collections::BTreeMap, io::Cursor};

use crate::{model::PostId, wire};

const FACT_MAGIC: &[u8; 4] = b"BBF1";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Lane {
    Tag,
    Rating,
}

impl Lane {
    fn code(self) -> u8 {
        match self {
            Self::Tag => 0,
            Self::Rating => 1,
        }
    }

    fn decode(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Self::Tag),
            1 => Ok(Self::Rating),
            other => bail!("invalid posting lane {other}"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key {
    pub lane: Lane,
    pub key: String,
}

impl Key {
    pub fn new(lane: Lane, key: impl Into<String>) -> Self {
        Self {
            lane,
            key: key.into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Delta {
    pub add: RoaringBitmap,
    pub del: RoaringBitmap,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.del.is_empty()
    }

    pub fn add(&mut self, id: PostId) {
        let _removed = self.del.remove(id.0);
        let _inserted = self.add.insert(id.0);
    }

    pub fn del(&mut self, id: PostId) {
        let _removed = self.add.remove(id.0);
        let _inserted = self.del.insert(id.0);
    }

    fn assimilate(&mut self, incoming: Self) {
        for id in incoming.del {
            self.del(PostId(id));
        }
        for id in incoming.add {
            self.add(PostId(id));
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Batch {
    groups: BTreeMap<Key, Delta>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn groups(&self) -> impl Iterator<Item = (&Key, &Delta)> {
        self.groups.iter()
    }

    pub fn group(&self, lane: Lane, key: &str) -> Option<&Delta> {
        self.groups.get(&Key::new(lane, key))
    }

    pub fn add(&mut self, lane: Lane, key: &str, id: PostId) {
        self.delta(lane, key).add(id);
        self.reap_empty(lane, key);
    }

    pub fn del(&mut self, lane: Lane, key: &str, id: PostId) {
        self.delta(lane, key).del(id);
        self.reap_empty(lane, key);
    }

    pub fn assimilate(&mut self, incoming: Self) {
        for (key, delta) in incoming.groups {
            self.groups.entry(key).or_default().assimilate(delta);
        }
        self.groups.retain(|_, delta| !delta.is_empty());
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut sink = wire::Sink::with_magic(FACT_MAGIC);
        sink.var(self.groups.len() as u64);
        for (key, delta) in &self.groups {
            sink.u8(key.lane.code());
            sink.str(&key.key);
            sink.bytes_raw(&bitmap_encode(&delta.add)?);
            sink.bytes_raw(&bitmap_encode(&delta.del)?);
        }
        Ok(sink.bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut blade = wire::Blade::new(bytes, FACT_MAGIC)?;
        let groups = blade.var()?;
        let mut batch = Self::default();
        for _ in 0..groups {
            let lane = Lane::decode(blade.u8()?)?;
            let key = blade.string()?;
            let add = bitmap_decode(blade.bytes_raw()?)?;
            let del = bitmap_decode(blade.bytes_raw()?)?;
            if !add.is_empty() || !del.is_empty() {
                let _old = batch.groups.insert(Key::new(lane, key), Delta { add, del });
            }
        }
        blade.done()?;
        Ok(batch)
    }

    fn delta(&mut self, lane: Lane, key: &str) -> &mut Delta {
        self.groups.entry(Key::new(lane, key)).or_default()
    }

    fn reap_empty(&mut self, lane: Lane, key: &str) {
        let key = Key::new(lane, key);
        if self.groups.get(&key).is_some_and(Delta::is_empty) {
            let _empty = self.groups.remove(&key);
        }
    }
}

pub fn bitmap_encode(bitmap: &RoaringBitmap) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(bitmap.serialized_size());
    bitmap
        .serialize_into(&mut bytes)
        .context("serialize bitmap")?;
    Ok(bytes)
}

pub fn bitmap_decode(bytes: &[u8]) -> Result<RoaringBitmap> {
    RoaringBitmap::deserialize_from(Cursor::new(bytes)).context("deserialize bitmap")
}

/// Cardinality straight from the portable-roaring header: the descriptive
/// headers carry each container's count, so no container is materialized.
/// `None` when the bytes are not the portable format the caller expects —
/// fall back to a full decode there.
pub fn serialized_cardinality(bytes: &[u8]) -> Option<u64> {
    const SERIAL_COOKIE_NO_RUNS: u32 = 12346;
    const SERIAL_COOKIE_RUNS: u32 = 12347;
    let cookie = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    let (containers, headers_at) = if cookie & 0xFFFF == SERIAL_COOKIE_RUNS {
        let containers = (cookie >> 16) as usize + 1;
        // Skip the run-container flag bitset.
        (containers, 4 + containers.div_ceil(8))
    } else if cookie == SERIAL_COOKIE_NO_RUNS {
        let containers = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
        (containers, 8)
    } else {
        return None;
    };
    let mut total = 0_u64;
    for slot in 0..containers {
        let at = headers_at + slot * 4 + 2;
        let cardinality_minus_one = u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?);
        total += u64::from(cardinality_minus_one) + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_cardinality_matches_decode() -> Result<()> {
        // Array container, bitset container, and a multi-container spread.
        for bitmap in [
            (0..50_u32).collect::<RoaringBitmap>(),
            (0..9_000).collect(),
            (0..200_000).step_by(3).collect(),
        ] {
            let bytes = bitmap_encode(&bitmap)?;
            assert_eq!(serialized_cardinality(&bytes), Some(bitmap.len()));
        }
        Ok(())
    }
}
