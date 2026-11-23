use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

/// Sub-Unit: Polygon face (triangle or quad) in CCW front-face order with a palette material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Face {
    pub vertices: Vec<usize>,
    pub material: PaletteIndex,
}

impl Face {
    pub fn new(vertices: Vec<usize>, material: PaletteIndex) -> Self {
        Self { vertices, material }
    }
}
