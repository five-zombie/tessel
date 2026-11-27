use serde::{Deserialize, Serialize};

use super::nano::Rgba8;

fn default_ior() -> f32 {
    1.5
}

/// Sub-Unit: Single palette color with lightweight PBR & optical material hints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub name: String,
    pub color: Rgba8,
    pub roughness: f32,
    pub metallic: f32,
    pub emissive: f32,
    /// Glass/dielectric transmission factor (`0.0..=1.0`, default `0.0`).
    #[serde(default)]
    pub transmission: f32,
    /// Index of Refraction (`1.0..=2.5`, default `1.5`).
    #[serde(default = "default_ior")]
    pub ior: f32,
}

impl PaletteEntry {
    pub fn matte(name: impl Into<String>, color: Rgba8) -> Self {
        Self {
            name: name.into(),
            color,
            roughness: 0.85,
            metallic: 0.0,
            emissive: 0.0,
            transmission: 0.0,
            ior: default_ior(),
        }
    }
}
