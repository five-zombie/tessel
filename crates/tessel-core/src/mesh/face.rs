use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

/// Sub-Unit: Polygon face (triangle or quad) in CCW front-face order with optional per-corner UVs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub vertices: Vec<usize>,
    pub material: PaletteIndex,
    /// Optional per-corner UV coordinates preserved from imported `.obj`/`.glb` meshes.
    #[serde(default)]
    pub uvs: Vec<[f32; 2]>,
}

impl Face {
    pub fn new(vertices: Vec<usize>, material: PaletteIndex) -> Self {
        Self {
            vertices,
            material,
            uvs: Vec::new(),
        }
    }

    pub fn with_uvs(vertices: Vec<usize>, material: PaletteIndex, uvs: Vec<[f32; 2]>) -> Self {
        Self {
            vertices,
            material,
            uvs,
        }
    }
}
