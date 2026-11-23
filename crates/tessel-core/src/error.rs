use thiserror::Error;

use crate::pixel::{FrameIndex, LayerId};
use crate::scene::NodeId;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("node {0:?} not found in scene")]
    NodeNotFound(NodeId),

    #[error("pixel canvas not found for node {0:?}")]
    PixelCanvasNotFound(NodeId),

    #[error("pixel frame {0:?} not found")]
    PixelFrameNotFound(FrameIndex),

    #[error("pixel layer {0:?} not found")]
    PixelLayerNotFound(LayerId),

    #[error("voxel volume not found for node {0:?}")]
    VoxelVolumeNotFound(NodeId),

    #[error("low-poly mesh not found for node {0:?}")]
    LowPolyMeshNotFound(NodeId),

    #[error("character rig is not initialized")]
    CharacterRigNotFound,

    #[error("coordinates or dimensions are out of bounds")]
    OutOfBounds,

    #[error("palette index 0 is reserved for empty cells")]
    InvalidPaletteIndex,

    #[error("command history is empty")]
    EmptyHistory,

    #[error("transaction contains no commands")]
    EmptyTransaction,
}
