use glam::{IVec3, Vec3};

use crate::palette::PaletteIndex;
use crate::pixel::{PixelCanvas, PixelCoord};

use super::nano::{Symmetry3D, VoxelCoord, VoxelEdit, VoxelHit};
use super::volume::VoxelVolume;

/// Expands a voxel coordinate across active X/Y/Z mirror planes around the origin.
pub fn expand_symmetry_3d(coord: VoxelCoord, sym: Symmetry3D) -> Vec<VoxelCoord> {
    let mut out = vec![coord];
    if sym.x {
        let current = out.clone();
        for c in current {
            let m = VoxelCoord::new(-c.x - 1, c.y, c.z);
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    if sym.y {
        let current = out.clone();
        for c in current {
            let m = VoxelCoord::new(c.x, -c.y - 1, c.z);
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    if sym.z {
        let current = out.clone();
        for c in current {
            let m = VoxelCoord::new(c.x, c.y, -c.z - 1);
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    out
}

/// Generates box fill edits (solid or hollow) for inclusive `(min..=max)`.
pub fn box_fill_edits(
    min: VoxelCoord,
    max: VoxelCoord,
    color: Option<PaletteIndex>,
    hollow: bool,
) -> Vec<VoxelEdit> {
    let min_x = min.x.min(max.x);
    let max_x = min.x.max(max.x);
    let min_y = min.y.min(max.y);
    let max_y = min.y.max(max.y);
    let min_z = min.z.min(max.z);
    let max_z = min.z.max(max.z);

    const MAX_FILL_EDITS: usize = 50_000;
    let mut edits = Vec::new();
    for y in min_y..=max_y {
        for z in min_z..=max_z {
            for x in min_x..=max_x {
                if hollow
                    && x > min_x
                    && x < max_x
                    && y > min_y
                    && y < max_y
                    && z > min_z
                    && z < max_z
                {
                    continue;
                }
                if edits.len() >= MAX_FILL_EDITS {
                    return edits;
                }
                edits.push(VoxelEdit {
                    coord: VoxelCoord::new(x, y, z),
                    color,
                });
            }
        }
    }
    edits
}

/// Generates ellipsoid fill edits centered at `center` with semi-axes `radii`.
pub fn ellipsoid_fill_edits(
    center: VoxelCoord,
    radii: [f32; 3],
    color: Option<PaletteIndex>,
) -> Vec<VoxelEdit> {
    if !radii.iter().all(|r| r.is_finite()) {
        return Vec::new();
    }
    const MAX_FILL_EDITS: usize = 50_000;
    let rx = radii[0].clamp(0.5, 128.0);
    let ry = radii[1].clamp(0.5, 128.0);
    let rz = radii[2].clamp(0.5, 128.0);
    let ix = rx.ceil() as i32;
    let iy = ry.ceil() as i32;
    let iz = rz.ceil() as i32;

    let mut edits = Vec::new();
    for dy in -iy..=iy {
        for dz in -iz..=iz {
            for dx in -ix..=ix {
                let nx = dx as f32 / rx;
                let ny = dy as f32 / ry;
                let nz = dz as f32 / rz;
                if nx * nx + ny * ny + nz * nz <= 1.0 {
                    if edits.len() >= MAX_FILL_EDITS {
                        return edits;
                    }
                    edits.push(VoxelEdit {
                        coord: VoxelCoord::new(
                            center.x.saturating_add(dx),
                            center.y.saturating_add(dy),
                            center.z.saturating_add(dz),
                        ),
                        color,
                    });
                }
            }
        }
    }
    edits
}

/// Converts the active frame of a `PixelCanvas` into a `VoxelVolume` extruded along `Z`.
/// Maps pixel top row `y = 0` to voxel `y = height - 1` (Right-Handed Y-Up).
pub fn pixel_to_voxel_volume(canvas: &PixelCanvas, depth: u32) -> VoxelVolume {
    let mut vol = VoxelVolume::new();
    let d = depth.clamp(1, 32) as i32;
    let half_w = (canvas.width as i32) / 2;
    let half_d = d / 2;

    for py in 0..canvas.height {
        for px in 0..canvas.width {
            if let Some(color) = canvas.sample_active_pixel(PixelCoord::new(px, py)) {
                let vx = px as i32 - half_w;
                let vy = (canvas.height - 1 - py) as i32;
                for z in 0..d {
                    vol.set(VoxelCoord::new(vx, vy, z - half_d), Some(color));
                }
            }
        }
    }
    vol
}

/// 3D DDA ray traversal (Amanatides & Woo) in voxel space (`world_pos / unit_scale`).
pub fn raycast_voxels(
    volume: &VoxelVolume,
    ray_origin_world: Vec3,
    ray_dir_world: Vec3,
    unit_scale: f32,
    max_steps: u32,
) -> Option<VoxelHit> {
    if !ray_origin_world.is_finite()
        || !ray_dir_world.is_finite()
        || !unit_scale.is_finite()
        || unit_scale <= 0.0
    {
        return None;
    }
    let dir = ray_dir_world.normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return None;
    }

    let origin = ray_origin_world / unit_scale;
    if !origin.is_finite() || origin.x.abs() > 1e8 || origin.y.abs() > 1e8 || origin.z.abs() > 1e8 {
        return None;
    }
    let mut voxel = origin.floor().as_ivec3();

    let step = IVec3::new(
        if dir.x >= 0.0 { 1 } else { -1 },
        if dir.y >= 0.0 { 1 } else { -1 },
        if dir.z >= 0.0 { 1 } else { -1 },
    );

    let next_boundary = Vec3::new(
        if dir.x >= 0.0 {
            voxel.x as f32 + 1.0
        } else {
            voxel.x as f32
        },
        if dir.y >= 0.0 {
            voxel.y as f32 + 1.0
        } else {
            voxel.y as f32
        },
        if dir.z >= 0.0 {
            voxel.z as f32 + 1.0
        } else {
            voxel.z as f32
        },
    );

    let mut t_max = Vec3::new(
        if dir.x.abs() > 1e-6 {
            (next_boundary.x - origin.x) / dir.x
        } else {
            f32::INFINITY
        },
        if dir.y.abs() > 1e-6 {
            (next_boundary.y - origin.y) / dir.y
        } else {
            f32::INFINITY
        },
        if dir.z.abs() > 1e-6 {
            (next_boundary.z - origin.z) / dir.z
        } else {
            f32::INFINITY
        },
    );

    let t_delta = Vec3::new(
        if dir.x.abs() > 1e-6 {
            (1.0 / dir.x).abs()
        } else {
            f32::INFINITY
        },
        if dir.y.abs() > 1e-6 {
            (1.0 / dir.y).abs()
        } else {
            f32::INFINITY
        },
        if dir.z.abs() > 1e-6 {
            (1.0 / dir.z).abs()
        } else {
            f32::INFINITY
        },
    );

    let mut last_normal = IVec3::ZERO;
    for _ in 0..max_steps {
        let coord = VoxelCoord::from_ivec3(voxel);
        if volume.get(coord).is_some() {
            return Some(VoxelHit {
                voxel: coord,
                normal: last_normal,
            });
        }

        if t_max.x < t_max.y {
            if t_max.x < t_max.z {
                voxel.x = voxel.x.checked_add(step.x)?;
                t_max.x += t_delta.x;
                last_normal = IVec3::new(-step.x, 0, 0);
            } else {
                voxel.z = voxel.z.checked_add(step.z)?;
                t_max.z += t_delta.z;
                last_normal = IVec3::new(0, 0, -step.z);
            }
        } else if t_max.y < t_max.z {
            voxel.y = voxel.y.checked_add(step.y)?;
            t_max.y += t_delta.y;
            last_normal = IVec3::new(0, -step.y, 0);
        } else {
            voxel.z = voxel.z.checked_add(step.z)?;
            t_max.z += t_delta.z;
            last_normal = IVec3::new(0, 0, -step.z);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dda_raycast_hits_front_face_and_normal() {
        let mut vol = VoxelVolume::new();
        let c = PaletteIndex::new(4).unwrap();
        vol.set(VoxelCoord::new(0, 0, 0), Some(c));

        let hit = raycast_voxels(
            &vol,
            Vec3::new(0.05, 0.05, 2.0),
            Vec3::new(0.0, 0.0, -1.0),
            0.1,
            64,
        )
        .unwrap();
        assert_eq!(hit.voxel, VoxelCoord::new(0, 0, 0));
        assert_eq!(hit.normal, IVec3::new(0, 0, 1));
    }

    #[test]
    fn pixel_to_voxel_preserves_upright_orientation() {
        let mut canvas = PixelCanvas::new(4, 4).unwrap();
        let c = PaletteIndex::new(4).unwrap();
        // Top-left pixel (0, 0) should become top voxel y = 3
        canvas.frames[0].layers[0].set(PixelCoord::new(0, 0), Some(c));
        let vol = pixel_to_voxel_volume(&canvas, 2);
        assert_eq!(vol.voxel_count(), 2);
        assert_eq!(vol.get(VoxelCoord::new(-2, 3, -1)), Some(c));
        assert_eq!(vol.get(VoxelCoord::new(-2, 3, 0)), Some(c));
    }
}
