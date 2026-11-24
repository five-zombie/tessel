use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

use super::chunk::{VoxelChunk, CHUNK_SIZE};
use super::nano::{ChunkCoord, VoxelCoord};

/// Unit Anchor: Sparse chunked 3D voxel volume with deterministic ordering & dirty tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "VoxelVolumeRaw")]
pub struct VoxelVolume {
    #[serde(with = "chunks_serde")]
    chunks: BTreeMap<ChunkCoord, VoxelChunk>,
    #[serde(skip)]
    dirty_chunks: BTreeSet<ChunkCoord>,
    voxel_count: usize,
}

#[derive(Deserialize)]
struct VoxelVolumeRaw {
    #[serde(with = "chunks_serde")]
    chunks: BTreeMap<ChunkCoord, VoxelChunk>,
    #[serde(default)]
    #[allow(dead_code)]
    voxel_count: usize,
}

impl From<VoxelVolumeRaw> for VoxelVolume {
    fn from(raw: VoxelVolumeRaw) -> Self {
        let voxel_count = raw
            .chunks
            .values()
            .map(|c| usize::from(c.occupied_count()))
            .sum();
        Self {
            chunks: raw.chunks,
            dirty_chunks: BTreeSet::new(),
            voxel_count,
        }
    }
}

mod chunks_serde {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{ChunkCoord, VoxelChunk};

    pub fn serialize<S>(
        map: &BTreeMap<ChunkCoord, VoxelChunk>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let entries: Vec<(&ChunkCoord, &VoxelChunk)> = map.iter().collect();
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ChunkCoord, VoxelChunk>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries: Vec<(ChunkCoord, VoxelChunk)> = Vec::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

impl VoxelVolume {
    pub fn new() -> Self {
        Self {
            chunks: BTreeMap::new(),
            dirty_chunks: BTreeSet::new(),
            voxel_count: 0,
        }
    }

    pub fn get(&self, coord: VoxelCoord) -> Option<PaletteIndex> {
        let (chunk_coord, (lx, ly, lz)) = coord.chunk_and_local();
        self.chunks
            .get(&chunk_coord)
            .and_then(|c| c.get(lx, ly, lz))
    }

    pub fn set(&mut self, coord: VoxelCoord, color: Option<PaletteIndex>) -> Option<PaletteIndex> {
        let (chunk_coord, (lx, ly, lz)) = coord.chunk_and_local();
        let prev = match color {
            Some(idx) => {
                let chunk = self.chunks.entry(chunk_coord).or_default();
                chunk.set(lx, ly, lz, Some(idx))
            }
            None => {
                let prev = self
                    .chunks
                    .get_mut(&chunk_coord)
                    .and_then(|c| c.set(lx, ly, lz, None));
                if self
                    .chunks
                    .get(&chunk_coord)
                    .is_some_and(VoxelChunk::is_empty)
                {
                    self.chunks.remove(&chunk_coord);
                }
                prev
            }
        };

        if prev != color {
            match (prev, color) {
                (None, Some(_)) => self.voxel_count = self.voxel_count.saturating_add(1),
                (Some(_), None) => self.voxel_count = self.voxel_count.saturating_sub(1),
                _ => {}
            }
            self.mark_dirty_with_neighbors(chunk_coord, lx, ly, lz);
        }
        prev
    }

    fn mark_dirty_with_neighbors(&mut self, c: ChunkCoord, lx: usize, ly: usize, lz: usize) {
        self.dirty_chunks.insert(c);
        let max_i = CHUNK_SIZE - 1;
        if lx == 0 {
            self.dirty_chunks.insert(ChunkCoord::new(c.x - 1, c.y, c.z));
        } else if lx == max_i {
            self.dirty_chunks.insert(ChunkCoord::new(c.x + 1, c.y, c.z));
        }
        if ly == 0 {
            self.dirty_chunks.insert(ChunkCoord::new(c.x, c.y - 1, c.z));
        } else if ly == max_i {
            self.dirty_chunks.insert(ChunkCoord::new(c.x, c.y + 1, c.z));
        }
        if lz == 0 {
            self.dirty_chunks.insert(ChunkCoord::new(c.x, c.y, c.z - 1));
        } else if lz == max_i {
            self.dirty_chunks.insert(ChunkCoord::new(c.x, c.y, c.z + 1));
        }
    }

    pub fn voxel_count(&self) -> usize {
        self.voxel_count
    }

    pub fn is_empty(&self) -> bool {
        self.voxel_count == 0
    }

    pub fn chunks(&self) -> &BTreeMap<ChunkCoord, VoxelChunk> {
        &self.chunks
    }

    pub fn dirty_chunks(&self) -> &BTreeSet<ChunkCoord> {
        &self.dirty_chunks
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_chunks.clear();
    }

    /// Computes inclusive min/max bounds of all occupied voxels.
    pub fn bounds(&self) -> Option<(VoxelCoord, VoxelCoord)> {
        let mut min_v = [i32::MAX; 3];
        let mut max_v = [i32::MIN; 3];
        let mut found = false;

        for (&cc, chunk) in &self.chunks {
            let ox = cc.x * (CHUNK_SIZE as i32);
            let oy = cc.y * (CHUNK_SIZE as i32);
            let oz = cc.z * (CHUNK_SIZE as i32);
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        if chunk.get(x, y, z).is_some() {
                            let gx = ox + x as i32;
                            let gy = oy + y as i32;
                            let gz = oz + z as i32;
                            min_v[0] = min_v[0].min(gx);
                            min_v[1] = min_v[1].min(gy);
                            min_v[2] = min_v[2].min(gz);
                            max_v[0] = max_v[0].max(gx);
                            max_v[1] = max_v[1].max(gy);
                            max_v[2] = max_v[2].max(gz);
                            found = true;
                        }
                    }
                }
            }
        }

        found.then_some((
            VoxelCoord::new(min_v[0], min_v[1], min_v[2]),
            VoxelCoord::new(max_v[0], max_v[1], max_v[2]),
        ))
    }

    /// Iterates all occupied `(VoxelCoord, PaletteIndex)` in deterministic order.
    pub fn iter_voxels(&self) -> Vec<(VoxelCoord, PaletteIndex)> {
        let mut out = Vec::with_capacity(self.voxel_count);
        for (&cc, chunk) in &self.chunks {
            let ox = cc.x * (CHUNK_SIZE as i32);
            let oy = cc.y * (CHUNK_SIZE as i32);
            let oz = cc.z * (CHUNK_SIZE as i32);
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        if let Some(color) = chunk.get(x, y, z) {
                            out.push((
                                VoxelCoord::new(ox + x as i32, oy + y as i32, oz + z as i32),
                                color,
                            ));
                        }
                    }
                }
            }
        }
        out
    }
}

impl Default for VoxelVolume {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_recomputes_stale_voxel_count() {
        let mut vol = VoxelVolume::new();
        let c = PaletteIndex::new(3).unwrap();
        vol.set(VoxelCoord::new(0, 0, 0), Some(c));
        vol.set(VoxelCoord::new(1, 0, 0), Some(c));
        let loaded = VoxelVolume::from(VoxelVolumeRaw {
            chunks: vol.chunks,
            voxel_count: 999_999,
        });
        assert_eq!(loaded.voxel_count(), 2);
    }
}
