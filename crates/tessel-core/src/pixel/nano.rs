use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

/// Nano-Unit: 2D pixel coordinate (Top-Left origin: `(0, 0)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PixelCoord {
    pub x: u32,
    pub y: u32,
}

impl PixelCoord {
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }
}

/// Nano-Unit: Stable layer identifier inside a `PixelCanvas`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u32);

/// Nano-Unit: Zero-based animation frame index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FrameIndex(pub usize);

/// Nano-Unit: Single pixel mutation entry (`None` = transparent/erased).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelEdit {
    pub coord: PixelCoord,
    pub color: Option<PaletteIndex>,
}

/// Nano-Unit: 2D symmetry axes for pixel tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Symmetry2D {
    pub x: bool,
    pub y: bool,
}

/// Nano-Unit: Active 2D pixel editing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PixelTool {
    #[default]
    Pencil,
    Eraser,
    Fill,
    Line,
    Rect,
    Eyedropper,
}

impl PixelTool {
    pub const ALL: [Self; 6] = [
        Self::Pencil,
        Self::Eraser,
        Self::Fill,
        Self::Line,
        Self::Rect,
        Self::Eyedropper,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Pencil => "Pencil",
            Self::Eraser => "Eraser",
            Self::Fill => "Fill",
            Self::Line => "Line",
            Self::Rect => "Rect",
            Self::Eyedropper => "Picker",
        }
    }
}
