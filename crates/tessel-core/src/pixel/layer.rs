use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

use super::nano::{LayerId, PixelCoord};

/// Sub-Unit: Single indexed 2D pixel layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixelLayer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    width: u32,
    height: u32,
    pixels: Vec<Option<PaletteIndex>>,
}

impl PixelLayer {
    pub fn new(id: LayerId, name: impl Into<String>, width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            width,
            height,
            pixels: vec![None; len],
        }
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    fn index(&self, coord: PixelCoord) -> Option<usize> {
        if coord.x < self.width && coord.y < self.height {
            Some((coord.y as usize) * (self.width as usize) + (coord.x as usize))
        } else {
            None
        }
    }

    pub fn get(&self, coord: PixelCoord) -> Option<PaletteIndex> {
        self.index(coord).and_then(|idx| self.pixels[idx])
    }

    pub fn set(
        &mut self,
        coord: PixelCoord,
        color: Option<PaletteIndex>,
    ) -> Option<Option<PaletteIndex>> {
        let idx = self.index(coord)?;
        let prev = self.pixels[idx];
        self.pixels[idx] = color;
        Some(prev)
    }

    pub fn pixels(&self) -> &[Option<PaletteIndex>] {
        &self.pixels
    }
}
