use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::palette::{Palette, PaletteIndex};

use super::layer::PixelLayer;
use super::nano::{FrameIndex, LayerId, PixelCoord};

/// Sub-Unit: One animation frame containing ordered pixel layers (bottom to top).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixelFrame {
    pub layers: Vec<PixelLayer>,
    pub active_layer: usize,
}

impl PixelFrame {
    pub fn new(first_layer_id: LayerId, width: u32, height: u32) -> Self {
        Self {
            layers: vec![PixelLayer::new(first_layer_id, "Layer 1", width, height)],
            active_layer: 0,
        }
    }

    pub fn layer(&self, id: LayerId) -> Option<&PixelLayer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut PixelLayer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn active_layer_id(&self) -> Option<LayerId> {
        self.layers.get(self.active_layer).map(|l| l.id)
    }
}

/// Unit Anchor: 2D sprite & texture canvas with layers and frame-by-frame animation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixelCanvas {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<PixelFrame>,
    pub active_frame: usize,
    pub onion_skin: bool,
    pub fps: u16,
    next_layer_id: u32,
}

impl PixelCanvas {
    pub fn new(width: u32, height: u32) -> Result<Self, CoreError> {
        if width == 0 || height == 0 || width > 512 || height > 512 {
            return Err(CoreError::OutOfBounds);
        }
        Ok(Self {
            width,
            height,
            frames: vec![PixelFrame::new(LayerId(1), width, height)],
            active_frame: 0,
            onion_skin: false,
            fps: 8,
            next_layer_id: 2,
        })
    }

    pub fn allocate_layer_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id = self.next_layer_id.saturating_add(1);
        id
    }

    /// Rewinds `next_layer_id` when undoing the most recently allocated layer so Redo reproduces the exact ID.
    pub fn rollback_layer_id(&mut self, id: LayerId) {
        if self.next_layer_id == id.0.saturating_add(1) {
            self.next_layer_id = id.0;
        }
    }

    pub fn current_frame(&self) -> Option<&PixelFrame> {
        self.frames.get(self.active_frame)
    }

    pub fn current_frame_mut(&mut self) -> Option<&mut PixelFrame> {
        self.frames.get_mut(self.active_frame)
    }

    pub fn frame(&self, index: FrameIndex) -> Option<&PixelFrame> {
        self.frames.get(index.0)
    }

    pub fn frame_mut(&mut self, index: FrameIndex) -> Option<&mut PixelFrame> {
        self.frames.get_mut(index.0)
    }

    /// Samples the topmost visible non-empty pixel at `coord` in the active frame.
    pub fn sample_active_pixel(&self, coord: PixelCoord) -> Option<PaletteIndex> {
        let frame = self.current_frame()?;
        for layer in frame.layers.iter().rev() {
            if !layer.visible {
                continue;
            }
            if let Some(idx) = layer.get(coord) {
                return Some(idx);
            }
        }
        None
    }

    /// Composites all visible layers of `frame_idx` into tightly packed RGBA8 bytes (`width * height * 4`).
    pub fn composite_frame_rgba(&self, frame_idx: usize, palette: &Palette) -> Vec<u8> {
        const MAX_COMPOSITE_DIM: usize = 512;
        let w = (self.width as usize).min(MAX_COMPOSITE_DIM);
        let h = (self.height as usize).min(MAX_COMPOSITE_DIM);
        let pixel_count = w.saturating_mul(h);
        let mut out = vec![0u8; pixel_count.saturating_mul(4)];
        let Some(frame) = self.frames.get(frame_idx) else {
            return out;
        };

        for layer in &frame.layers {
            if !layer.visible || !layer.opacity.is_finite() || layer.opacity <= 0.0 {
                continue;
            }
            let layer_alpha = layer.opacity.clamp(0.0, 1.0);
            for (i, cell) in layer.pixels().iter().take(pixel_count).enumerate() {
                let Some(idx) = cell else {
                    continue;
                };
                let Some(entry) = palette.get(*idx) else {
                    continue;
                };
                let src_a = (f32::from(entry.color.a) / 255.0) * layer_alpha;
                if src_a <= 0.0 {
                    continue;
                }
                let base = i * 4;
                if base + 3 >= out.len() {
                    break;
                }
                let dst_a = f32::from(out[base + 3]) / 255.0;
                let out_a = src_a + dst_a * (1.0 - src_a);
                if out_a <= 0.0 {
                    continue;
                }
                let blend = |src: u8, dst: u8| -> u8 {
                    let s = f32::from(src) / 255.0;
                    let d = f32::from(dst) / 255.0;
                    let c = (s * src_a + d * dst_a * (1.0 - src_a)) / out_a;
                    (c.clamp(0.0, 1.0) * 255.0).round() as u8
                };
                out[base] = blend(entry.color.r, out[base]);
                out[base + 1] = blend(entry.color.g, out[base + 1]);
                out[base + 2] = blend(entry.color.b, out[base + 2]);
                out[base + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        out
    }

    /// Composites the active frame with optional previous-frame blue-tinted onion skinning.
    pub fn composite_viewport_rgba(&self, palette: &Palette) -> Vec<u8> {
        let mut rgba = self.composite_frame_rgba(self.active_frame, palette);
        if self.onion_skin && self.active_frame > 0 {
            let prev = self.composite_frame_rgba(self.active_frame - 1, palette);
            for (dst, src) in rgba
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(prev.as_chunks::<4>().0)
            {
                if dst[3] == 0 && src[3] > 0 {
                    dst[0] = (u16::from(src[0]) / 2) as u8;
                    dst[1] = (u16::from(src[1]) * 3 / 4) as u8;
                    dst[2] = src[2].saturating_add(60);
                    dst[3] = (u16::from(src[3]) / 3) as u8;
                }
            }
        }
        rgba
    }
}
