use serde::{Deserialize, Serialize};

use super::texture::MeshTexture;

/// Hard cap on non-destructive paint layers per mesh to prevent unbounded memory/undo growth.
pub const MAX_PAINT_LAYERS: usize = 32;

/// Sub-Unit: Single non-destructive RGBA paint layer on a `LowPolyMesh`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshPaintLayer {
    pub id: u32,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub texture: MeshTexture,
}

impl MeshPaintLayer {
    pub fn new(id: u32, name: impl Into<String>, opacity: f32, texture: MeshTexture) -> Self {
        let safe_opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: safe_opacity,
            texture,
        }
    }
}

/// Deterministically composites visible `MeshPaintLayer` entries from bottom to top
/// using Porter-Duff source-over alpha blending weighted by each layer's `opacity`.
pub fn composite_paint_layers(layers: &[MeshPaintLayer]) -> Option<MeshTexture> {
    let first = layers.first()?;
    let w = first.texture.width;
    let h = first.texture.height;
    let pixel_count = (w as usize).checked_mul(h as usize)?;
    let byte_len = pixel_count.checked_mul(4)?;
    if pixel_count == 0 {
        return None;
    }

    let mut out = vec![0.0_f32; byte_len];

    for layer in layers {
        if !layer.visible {
            continue;
        }
        let op = if layer.opacity.is_finite() {
            layer.opacity.clamp(0.0, 1.0)
        } else {
            continue;
        };
        if op <= 0.0 {
            continue;
        }
        let lw = layer.texture.width as usize;
        let lh = layer.texture.height as usize;
        let Some(expected_bytes) = lw.checked_mul(lh).and_then(|px| px.checked_mul(4)) else {
            continue;
        };
        if lw == 0 || lh == 0 || layer.texture.rgba.len() < expected_bytes {
            continue;
        }
        let same_size = lw == w as usize && lh == h as usize;

        for py in 0..(h as usize) {
            let sy = if same_size {
                py
            } else {
                (py * lh / h as usize).min(lh - 1)
            };
            for px in 0..(w as usize) {
                let sx = if same_size {
                    px
                } else {
                    (px * lw / w as usize).min(lw - 1)
                };
                let src_off = (sy * lw + sx) * 4;
                let sa = (f32::from(layer.texture.rgba[src_off + 3]) / 255.0) * op;
                if sa <= 1e-6 {
                    continue;
                }
                let sr = f32::from(layer.texture.rgba[src_off]) / 255.0;
                let sg = f32::from(layer.texture.rgba[src_off + 1]) / 255.0;
                let sb = f32::from(layer.texture.rgba[src_off + 2]) / 255.0;

                let dst_off = (py * w as usize + px) * 4;
                let dr = out[dst_off];
                let dg = out[dst_off + 1];
                let db = out[dst_off + 2];
                let da = out[dst_off + 3];

                let out_a = sa + da * (1.0 - sa);
                if out_a > 1e-6 {
                    let inv_a = 1.0 / out_a;
                    out[dst_off] = (sr * sa + dr * da * (1.0 - sa)) * inv_a;
                    out[dst_off + 1] = (sg * sa + dg * da * (1.0 - sa)) * inv_a;
                    out[dst_off + 2] = (sb * sa + db * da * (1.0 - sa)) * inv_a;
                    out[dst_off + 3] = out_a;
                }
            }
        }
    }

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for px in out.chunks_exact(4) {
        rgba.push((px[0].clamp(0.0, 1.0) * 255.0).round() as u8);
        rgba.push((px[1].clamp(0.0, 1.0) * 255.0).round() as u8);
        rgba.push((px[2].clamp(0.0, 1.0) * 255.0).round() as u8);
        rgba.push((px[3].clamp(0.0, 1.0) * 255.0).round() as u8);
    }

    Some(MeshTexture {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Rgba8;

    #[test]
    fn composite_blends_opacity_and_respects_visibility() {
        let base = MeshPaintLayer::new(
            1,
            "Base",
            1.0,
            MeshTexture::solid(16, 16, Rgba8::rgb(0, 0, 200)),
        );
        let mut top = MeshPaintLayer::new(
            2,
            "Wear",
            0.5,
            MeshTexture::solid(16, 16, Rgba8::rgb(200, 0, 0)),
        );

        let comp = composite_paint_layers(&[base.clone(), top.clone()]).unwrap();
        // 50% red over 100% blue -> ~(100, 0, 100, 255)
        assert!((i16::from(comp.rgba[0]) - 100).abs() <= 1);
        assert_eq!(comp.rgba[1], 0);
        assert!((i16::from(comp.rgba[2]) - 100).abs() <= 1);
        assert_eq!(comp.rgba[3], 255);

        top.visible = false;
        let hidden = composite_paint_layers(&[base, top]).unwrap();
        assert_eq!(&hidden.rgba[0..4], &[0, 0, 200, 255]);
    }
}
