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

/// Maximum supported width or height (in pixels) for a `PixelCanvas` (4K).
pub const MAX_PIXEL_CANVAS_DIM: u32 = 4096;

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
        if width == 0
            || height == 0
            || width > MAX_PIXEL_CANVAS_DIM
            || height > MAX_PIXEL_CANVAS_DIM
        {
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

    /// Resizes all frames and layers to `(width, height)`, preserving overlapping top-left pixels.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), CoreError> {
        if width == 0
            || height == 0
            || width > MAX_PIXEL_CANVAS_DIM
            || height > MAX_PIXEL_CANVAS_DIM
        {
            return Err(CoreError::OutOfBounds);
        }
        if self.width == width && self.height == height {
            return Ok(());
        }
        for frame in &mut self.frames {
            for layer in &mut frame.layers {
                layer.resize(width, height);
            }
        }
        self.width = width;
        self.height = height;
        Ok(())
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

    /// Composites all visible layers of `frame_idx` into `out` (`width * height * 4` RGBA8 bytes),
    /// reusing existing buffer capacity to avoid repeated multi-megabyte allocations on 4K canvases.
    pub fn composite_frame_rgba_into(
        &self,
        frame_idx: usize,
        palette: &Palette,
        out: &mut Vec<u8>,
    ) {
        const MAX_COMPOSITE_DIM: usize = MAX_PIXEL_CANVAS_DIM as usize;
        let w = (self.width as usize).min(MAX_COMPOSITE_DIM);
        let h = (self.height as usize).min(MAX_COMPOSITE_DIM);
        let pixel_count = w.saturating_mul(h);
        let byte_len = pixel_count.saturating_mul(4);
        out.clear();
        out.resize(byte_len, 0);
        let Some(frame) = self.frames.get(frame_idx) else {
            return;
        };

        // Precompute a 256-entry stack LUT once per frame to avoid repeated palette lookups per pixel.
        let mut lut: [Option<[u8; 4]>; 256] = [None; 256];
        for (&idx, entry) in palette.iter() {
            lut[idx.get() as usize] =
                Some([entry.color.r, entry.color.g, entry.color.b, entry.color.a]);
        }

        for layer in &frame.layers {
            if !layer.visible || !layer.opacity.is_finite() || layer.opacity <= 0.0 {
                continue;
            }
            let layer_alpha = layer.opacity.clamp(0.0, 1.0);
            let full_layer_opacity = layer_alpha >= 1.0 - 1e-6;
            let dst_chunks = out.as_chunks_mut::<4>().0;
            for (dst, cell) in dst_chunks
                .iter_mut()
                .zip(layer.pixels().iter().take(pixel_count))
            {
                let Some(idx) = cell else {
                    continue;
                };
                let Some(rgba) = lut[idx.get() as usize] else {
                    continue;
                };
                if rgba[3] == 0 {
                    continue;
                }
                if full_layer_opacity && rgba[3] == 255 {
                    *dst = rgba;
                    continue;
                }
                let src_a = (f32::from(rgba[3]) / 255.0) * layer_alpha;
                if src_a <= 0.0 {
                    continue;
                }
                if dst[3] == 0 {
                    dst[0] = rgba[0];
                    dst[1] = rgba[1];
                    dst[2] = rgba[2];
                    dst[3] = (src_a.clamp(0.0, 1.0) * 255.0).round() as u8;
                    continue;
                }
                let dst_a = f32::from(dst[3]) / 255.0;
                let inv_src = 1.0 - src_a;
                let out_a = src_a + dst_a * inv_src;
                if out_a <= 0.0 {
                    continue;
                }
                let blend = |src: u8, d_val: u8| -> u8 {
                    let s = f32::from(src) / 255.0;
                    let d = f32::from(d_val) / 255.0;
                    let c = (s * src_a + d * dst_a * inv_src) / out_a;
                    (c.clamp(0.0, 1.0) * 255.0).round() as u8
                };
                dst[0] = blend(rgba[0], dst[0]);
                dst[1] = blend(rgba[1], dst[1]);
                dst[2] = blend(rgba[2], dst[2]);
                dst[3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }

    /// Composites all visible layers of `frame_idx` into tightly packed RGBA8 bytes (`width * height * 4`).
    pub fn composite_frame_rgba(&self, frame_idx: usize, palette: &Palette) -> Vec<u8> {
        let mut out = Vec::new();
        self.composite_frame_rgba_into(frame_idx, palette, &mut out);
        out
    }

    /// Composites the active frame with optional previous-frame blue-tinted onion skinning into `out`,
    /// reusing `prev_scratch` when onion skinning is enabled so zero per-frame allocations occur.
    pub fn composite_viewport_rgba_with_scratch(
        &self,
        palette: &Palette,
        out: &mut Vec<u8>,
        prev_scratch: &mut Vec<u8>,
    ) {
        self.composite_frame_rgba_into(self.active_frame, palette, out);
        if self.onion_skin && self.active_frame > 0 {
            self.composite_frame_rgba_into(self.active_frame - 1, palette, prev_scratch);
            for (dst, src) in out
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(prev_scratch.as_chunks::<4>().0)
            {
                if dst[3] == 0 && src[3] > 0 {
                    dst[0] = (u16::from(src[0]) / 2) as u8;
                    dst[1] = (u16::from(src[1]) * 3 / 4) as u8;
                    dst[2] = src[2].saturating_add(60);
                    dst[3] = (u16::from(src[3]) / 3) as u8;
                }
            }
        }
    }

    /// Composites the active frame with optional previous-frame blue-tinted onion skinning into `out`.
    pub fn composite_viewport_rgba_into(&self, palette: &Palette, out: &mut Vec<u8>) {
        let mut prev_scratch = Vec::new();
        self.composite_viewport_rgba_with_scratch(palette, out, &mut prev_scratch);
    }

    /// Composites the active frame with optional previous-frame blue-tinted onion skinning.
    pub fn composite_viewport_rgba(&self, palette: &Palette) -> Vec<u8> {
        let mut rgba = Vec::new();
        self.composite_viewport_rgba_into(palette, &mut rgba);
        rgba
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{PaletteEntry, Rgba8};

    #[test]
    fn composite_frame_rgba_into_blends_opaque_and_translucent_layers_and_reuses_buffer() {
        let mut palette = Palette::default();
        let red = PaletteIndex::new(10).unwrap();
        let blue = PaletteIndex::new(11).unwrap();
        palette.set(
            red,
            PaletteEntry::matte("Red", Rgba8::rgba(200, 20, 20, 255)),
        );
        palette.set(
            blue,
            PaletteEntry::matte("Blue", Rgba8::rgba(20, 40, 220, 255)),
        );

        let mut canvas = PixelCanvas::new(2, 1).unwrap();
        let lid1 = canvas.frames[0].layers[0].id;
        let lid2 = canvas.allocate_layer_id();
        canvas.frames[0]
            .layers
            .push(PixelLayer::new(lid2, "Overlay", 2, 1));
        canvas.frames[0].layers[1].opacity = 0.5;

        canvas.frames[0]
            .layer_mut(lid1)
            .unwrap()
            .set(PixelCoord::new(0, 0), Some(red));
        canvas.frames[0]
            .layer_mut(lid2)
            .unwrap()
            .set(PixelCoord::new(0, 0), Some(blue));
        canvas.frames[0]
            .layer_mut(lid2)
            .unwrap()
            .set(PixelCoord::new(1, 0), Some(blue));

        let mut buf = Vec::with_capacity(64);
        canvas.composite_viewport_rgba_into(&palette, &mut buf);
        assert_eq!(buf.len(), 8);
        assert!(buf.capacity() >= 64);
        // Pixel 0: 50% blue over 100% red -> (110, 30, 120, 255)
        assert_eq!(&buf[0..4], &[110, 30, 120, 255]);
        // Pixel 1: 50% blue over transparent -> (20, 40, 220, 128)
        assert_eq!(&buf[4..8], &[20, 40, 220, 128]);
    }
}
