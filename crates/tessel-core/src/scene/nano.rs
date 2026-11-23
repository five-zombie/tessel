use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Nano-Unit: Unique stable identifier for a scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Nano-Unit: Active creative workspace mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AppMode {
    Pixel,
    #[default]
    Voxel,
    LowPoly,
    Character,
}

impl AppMode {
    pub const ALL: [Self; 4] = [Self::Pixel, Self::Voxel, Self::LowPoly, Self::Character];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Pixel => "Pixel",
            Self::Voxel => "Voxel",
            Self::LowPoly => "Low-Poly",
            Self::Character => "Character",
        }
    }
}

/// Nano-Unit: Domain kind hosted by a scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Group,
    PixelCanvas,
    VoxelVolume,
    LowPolyMesh,
    RigRoot,
}

/// Nano-Unit: Right-Handed Y-Up local transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}
