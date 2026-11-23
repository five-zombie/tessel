use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::palette::Rgba8;

pub const DEFAULT_MESH_TEX_SIZE: u32 = 256;

/// Sub-Unit: Per-mesh 2D RGBA texture used for freeform 3D texture painting on `LowPolyMesh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl MeshTexture {
    pub fn solid(width: u32, height: u32, color: Rgba8) -> Self {
        let w = width.clamp(16, 1024);
        let h = height.clamp(16, 1024);
        let pixel_count = (w as usize).saturating_mul(h as usize);
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for _ in 0..pixel_count {
            rgba.extend_from_slice(&[color.r, color.g, color.b, color.a]);
        }
        Self {
            width: w,
            height: h,
            rgba,
        }
    }

    pub fn fill(&mut self, color: Rgba8) {
        for px in self.rgba.as_chunks_mut::<4>().0 {
            *px = [color.r, color.g, color.b, color.a];
        }
    }

    /// Paints a solid circular brush stamp centered at normalized `uv` with pixel radius `radius_px`.
    /// Returns `(byte_offset, previous_rgba)` entries for lightweight undo.
    pub fn paint_disc(
        &mut self,
        uv: [f32; 2],
        radius_px: f32,
        color: Rgba8,
    ) -> Vec<(usize, [u8; 4])> {
        let w = self.width as i32;
        let h = self.height as i32;
        if w <= 0 || h <= 0 || self.rgba.len() < (w as usize * h as usize * 4) {
            return Vec::new();
        }
        let cx = (uv[0].clamp(0.0, 1.0) * (w - 1) as f32).round() as i32;
        let cy = (uv[1].clamp(0.0, 1.0) * (h - 1) as f32).round() as i32;
        let r = radius_px.clamp(1.0, 96.0);
        let r_ceil = r.ceil() as i32;
        let r2 = r * r;

        let mut undo_pixels = Vec::new();
        let new_px = [color.r, color.g, color.b, color.a];

        let min_y = (cy - r_ceil).max(0);
        let max_y = (cy + r_ceil).min(h - 1);
        let min_x = (cx - r_ceil).max(0);
        let max_x = (cx + r_ceil).min(w - 1);

        for py in min_y..=max_y {
            let dy = (py - cy) as f32;
            for px in min_x..=max_x {
                let dx = (px - cx) as f32;
                if dx * dx + dy * dy <= r2 {
                    let offset = ((py as usize) * (w as usize) + (px as usize)) * 4;
                    let prev = [
                        self.rgba[offset],
                        self.rgba[offset + 1],
                        self.rgba[offset + 2],
                        self.rgba[offset + 3],
                    ];
                    if prev != new_px {
                        undo_pixels.push((offset, prev));
                        self.rgba[offset..offset + 4].copy_from_slice(&new_px);
                    }
                }
            }
        }
        undo_pixels
    }

    pub fn restore_pixels(&mut self, edits: &[(usize, [u8; 4])]) {
        for &(offset, prev) in edits.iter().rev() {
            if offset + 4 <= self.rgba.len() {
                self.rgba[offset..offset + 4].copy_from_slice(&prev);
            }
        }
    }
}

/// Deterministic 3x2 box-projection UV atlas mapping any 3D point + outward normal
/// onto 6 non-overlapping cells in `[0, 1] x [0, 1]`.
pub fn box_atlas_uv(pos: Vec3, normal: Vec3, min: Vec3, max: Vec3) -> [f32; 2] {
    let ext = (max - min).max(Vec3::splat(1e-4));
    let nx = ((pos.x - min.x) / ext.x).clamp(0.0, 1.0);
    let ny = ((pos.y - min.y) / ext.y).clamp(0.0, 1.0);
    let nz = ((pos.z - min.z) / ext.z).clamp(0.0, 1.0);

    let ax = normal.x.abs();
    let ay = normal.y.abs();
    let az = normal.z.abs();

    let (col, row, local_u, local_v) = if ax >= ay && ax >= az {
        if normal.x >= 0.0 {
            (0.0_f32, 0.0_f32, nz, 1.0 - ny) // +X Right
        } else {
            (1.0_f32, 0.0_f32, 1.0 - nz, 1.0 - ny) // -X Left
        }
    } else if ay >= ax && ay >= az {
        if normal.y >= 0.0 {
            (2.0_f32, 0.0_f32, nx, 1.0 - nz) // +Y Top
        } else {
            (0.0_f32, 1.0_f32, nx, nz) // -Y Bottom
        }
    } else if normal.z >= 0.0 {
        (1.0_f32, 1.0_f32, nx, 1.0 - ny) // +Z Front
    } else {
        (2.0_f32, 1.0_f32, 1.0 - nx, 1.0 - ny) // -Z Back
    };

    let pad = 0.03;
    let lu = pad + local_u * (1.0 - 2.0 * pad);
    let lv = pad + local_v * (1.0 - 2.0 * pad);
    [(col + lu) / 3.0, (row + lv) / 2.0]
}
