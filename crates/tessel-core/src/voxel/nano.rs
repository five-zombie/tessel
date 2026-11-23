use glam::IVec3;
use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

use super::chunk::CHUNK_SIZE;

/// Nano-Unit: 3D integer voxel coordinate (Right-Handed Y-Up, 1 voxel = 0.1m default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VoxelCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl VoxelCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn to_ivec3(self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }

    pub const fn from_ivec3(v: IVec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }

    pub fn chunk_and_local(self) -> (ChunkCoord, (usize, usize, usize)) {
        let s = CHUNK_SIZE as i32;
        let cx = self.x.div_euclid(s);
        let cy = self.y.div_euclid(s);
        let cz = self.z.div_euclid(s);
        let lx = self.x.rem_euclid(s) as usize;
        let ly = self.y.rem_euclid(s) as usize;
        let lz = self.z.rem_euclid(s) as usize;
        (ChunkCoord::new(cx, cy, cz), (lx, ly, lz))
    }
}

/// Nano-Unit: Sparse chunk coordinate (`16x16x16` block index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// Nano-Unit: Single voxel mutation (`None` = erase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelEdit {
    pub coord: VoxelCoord,
    pub color: Option<PaletteIndex>,
}

/// Nano-Unit: 3D mirror symmetry flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Symmetry3D {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

/// Nano-Unit: Active voxel editing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VoxelTool {
    #[default]
    Place,
    Erase,
    Paint,
    BoxFill,
    Eyedropper,
}

impl VoxelTool {
    pub const ALL: [Self; 5] = [
        Self::Place,
        Self::Erase,
        Self::Paint,
        Self::BoxFill,
        Self::Eyedropper,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Place => "Place",
            Self::Erase => "Erase",
            Self::Paint => "Paint",
            Self::BoxFill => "Box",
            Self::Eyedropper => "Picker",
        }
    }
}

/// Nano-Unit: Result of a 3D DDA raycast against a `VoxelVolume`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelHit {
    pub voxel: VoxelCoord,
    pub normal: IVec3,
}

impl VoxelHit {
    pub fn adjacent(self) -> VoxelCoord {
        VoxelCoord::new(
            self.voxel.x + self.normal.x,
            self.voxel.y + self.normal.y,
            self.voxel.z + self.normal.z,
        )
    }
}
