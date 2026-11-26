use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Non-zero palette index (`1..=255`). Index `0` is reserved for empty voxel / transparent pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PaletteIndex(u8);

impl PaletteIndex {
    /// Default active studio palette color index (`4`).
    pub const DEFAULT_ACTIVE: Self = Self(4);

    pub fn new(raw: u8) -> Result<Self, CoreError> {
        if raw == 0 {
            return Err(CoreError::InvalidPaletteIndex);
        }
        Ok(Self(raw))
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

/// 8-bit sRGBA color value object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}
