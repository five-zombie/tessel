use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Non-zero palette index (`1..=255`). Index `0` is reserved for empty voxel / transparent pixel.
/// Backed by `NonZeroU8` so `Option<PaletteIndex>` is 1 byte and `0` is rejected on deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PaletteIndex(NonZeroU8);

impl PaletteIndex {
    /// Default active studio palette color index (`4`).
    pub const DEFAULT_ACTIVE: Self = Self(NonZeroU8::MIN.saturating_add(3));

    pub fn new(raw: u8) -> Result<Self, CoreError> {
        NonZeroU8::new(raw)
            .map(Self)
            .ok_or(CoreError::InvalidPaletteIndex)
    }

    pub const fn get(self) -> u8 {
        self.0.get()
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
