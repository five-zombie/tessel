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
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4));
        if w <= 0
            || h <= 0
            || expected.is_none_or(|e| self.rgba.len() < e)
            || !uv[0].is_finite()
            || !uv[1].is_finite()
            || !radius_px.is_finite()
        {
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

    /// Paints a solid line segment from `uv_start` to `uv_end` with pixel width `width_px`.
    /// Returns `(byte_offset, previous_rgba)` entries for lightweight undo.
    pub fn paint_line(
        &mut self,
        uv_start: [f32; 2],
        uv_end: [f32; 2],
        width_px: f32,
        color: Rgba8,
    ) -> Vec<(usize, [u8; 4])> {
        let w = self.width as i32;
        let h = self.height as i32;
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4));
        if w <= 0
            || h <= 0
            || expected.is_none_or(|e| self.rgba.len() < e)
            || !uv_start[0].is_finite()
            || !uv_start[1].is_finite()
            || !uv_end[0].is_finite()
            || !uv_end[1].is_finite()
            || !width_px.is_finite()
        {
            return Vec::new();
        }
        let x0 = uv_start[0].clamp(0.0, 1.0) * (w - 1) as f32;
        let y0 = uv_start[1].clamp(0.0, 1.0) * (h - 1) as f32;
        let x1 = uv_end[0].clamp(0.0, 1.0) * (w - 1) as f32;
        let y1 = uv_end[1].clamp(0.0, 1.0) * (h - 1) as f32;
        let r = (width_px.clamp(1.0, 96.0) * 0.5).max(0.5);
        let r_ceil = r.ceil() as i32;
        let r2 = r * r;

        let min_x = (x0.min(x1).floor() as i32 - r_ceil).max(0);
        let max_x = (x0.max(x1).ceil() as i32 + r_ceil).min(w - 1);
        let min_y = (y0.min(y1).floor() as i32 - r_ceil).max(0);
        let max_y = (y0.max(y1).ceil() as i32 + r_ceil).min(h - 1);

        let vx = x1 - x0;
        let vy = y1 - y0;
        let len2 = vx * vx + vy * vy;
        let new_px = [color.r, color.g, color.b, color.a];
        let mut undo_pixels = Vec::new();

        for py in min_y..=max_y {
            let fy = py as f32;
            for px in min_x..=max_x {
                let fx = px as f32;
                let t = if len2 > 1e-6 {
                    (((fx - x0) * vx + (fy - y0) * vy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dx = fx - (x0 + vx * t);
                let dy = fy - (y0 + vy * t);
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

    /// Paints a solid axis-aligned UV rectangle `[uv_min, uv_max]`.
    /// Returns `(byte_offset, previous_rgba)` entries for lightweight undo.
    pub fn paint_rect(
        &mut self,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        color: Rgba8,
    ) -> Vec<(usize, [u8; 4])> {
        let w = self.width as i32;
        let h = self.height as i32;
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4));
        if w <= 0
            || h <= 0
            || expected.is_none_or(|e| self.rgba.len() < e)
            || !uv_min[0].is_finite()
            || !uv_min[1].is_finite()
            || !uv_max[0].is_finite()
            || !uv_max[1].is_finite()
        {
            return Vec::new();
        }
        let u0 = uv_min[0].min(uv_max[0]).clamp(0.0, 1.0);
        let u1 = uv_min[0].max(uv_max[0]).clamp(0.0, 1.0);
        let v0 = uv_min[1].min(uv_max[1]).clamp(0.0, 1.0);
        let v1 = uv_min[1].max(uv_max[1]).clamp(0.0, 1.0);

        let min_x = ((u0 * (w - 1) as f32).floor() as i32).max(0);
        let max_x = ((u1 * (w - 1) as f32).ceil() as i32).min(w - 1);
        let min_y = ((v0 * (h - 1) as f32).floor() as i32).max(0);
        let max_y = ((v1 * (h - 1) as f32).ceil() as i32).min(h - 1);

        let new_px = [color.r, color.g, color.b, color.a];
        let mut undo_pixels = Vec::new();
        for py in min_y..=max_y {
            for px in min_x..=max_x {
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
        undo_pixels
    }
}

/// Automatic UV projection mode for `LowPolyMesh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UvProjection {
    Box,
    PlanarX,
    PlanarY,
    PlanarZ,
    CylindricalY,
    Spherical,
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
