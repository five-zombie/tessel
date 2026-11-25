use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

pub const CHUNK_SIZE: usize = 16;
const CHUNK_LEN: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// Sub-Unit: Fixed `16x16x16` voxel block (`0` = empty, `1..=255` = `PaletteIndex`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelChunk {
    cells: Vec<u8>,
    occupied: u16,
}

impl VoxelChunk {
    pub fn new() -> Self {
        Self {
            cells: vec![0u8; CHUNK_LEN],
            occupied: 0,
        }
    }

    #[inline]
    fn idx(x: usize, y: usize, z: usize) -> usize {
        (y * CHUNK_SIZE + z) * CHUNK_SIZE + x
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> Option<PaletteIndex> {
        let raw = *self.cells.get(Self::idx(x, y, z))?;
        PaletteIndex::new(raw).ok()
    }

    pub fn set(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        color: Option<PaletteIndex>,
    ) -> Option<PaletteIndex> {
        let i = Self::idx(x, y, z);
        let cell = self.cells.get_mut(i)?;
        let prev_raw = *cell;
        let next_raw = color.map_or(0, PaletteIndex::get);
        if prev_raw != next_raw {
            if prev_raw == 0 && next_raw != 0 {
                self.occupied = self.occupied.saturating_add(1);
            } else if prev_raw != 0 && next_raw == 0 {
                self.occupied = self.occupied.saturating_sub(1);
            }
            *cell = next_raw;
        }
        PaletteIndex::new(prev_raw).ok()
    }

    pub const fn occupied_count(&self) -> u16 {
        self.occupied
    }

    pub const fn is_empty(&self) -> bool {
        self.occupied == 0
    }
}

impl Default for VoxelChunk {
    fn default() -> Self {
        Self::new()
    }
}
