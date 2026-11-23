use serde::{Deserialize, Serialize};

use super::nano::Rgba8;

/// Sub-Unit: Single palette color with lightweight PBR material hints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub name: String,
    pub color: Rgba8,
    pub roughness: f32,
    pub metallic: f32,
    pub emissive: f32,
}

impl PaletteEntry {
    pub fn matte(name: impl Into<String>, color: Rgba8) -> Self {
        Self {
            name: name.into(),
            color,
            roughness: 0.85,
            metallic: 0.0,
            emissive: 0.0,
        }
    }
}
