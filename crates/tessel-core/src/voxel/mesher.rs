use crate::palette::PaletteIndex;

use super::nano::VoxelCoord;
use super::volume::VoxelVolume;

/// Pure CPU vertex output from Greedy Meshing (renderer maps directly to `StudioVertex3D`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub palette_index: PaletteIndex,
    pub ao: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoxelMeshOutput {
    pub vertices: Vec<VoxelVertex>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaskCell {
    color: PaletteIndex,
    back_face: bool,
    ao: [u8; 4],
}

/// Runs deterministic Greedy Meshing across the entire `VoxelVolume` with per-vertex AO
/// and CCW front-face winding (Right-Handed Y-Up).
/// `unit_scale` maps 1 voxel to world meters (e.g. `0.1` or `1.0`).
///
/// # Limitations
/// Volumes whose bounding-box span exceeds `513` voxels along any axis return an empty
/// `VoxelMeshOutput` as a memory-safety cap on the 2D greedy-meshing slice mask.
pub fn greedy_mesh_volume(volume: &VoxelVolume, unit_scale: f32) -> VoxelMeshOutput {
    let mut output = VoxelMeshOutput::default();
    let Some((min_b, max_b)) = volume.bounds() else {
        return output;
    };

    let dx = i64::from(max_b.x) - i64::from(min_b.x) + 1;
    let dy = i64::from(max_b.y) - i64::from(min_b.y) + 1;
    let dz = i64::from(max_b.z) - i64::from(min_b.z) + 1;
    if dx <= 0 || dy <= 0 || dz <= 0 || dx > 513 || dy > 513 || dz > 513 {
        return output;
    }
    let dims = [dx as usize, dy as usize, dz as usize];
    let origin = [min_b.x, min_b.y, min_b.z];

    let sample =
        |p: [i32; 3]| -> Option<PaletteIndex> { volume.get(VoxelCoord::new(p[0], p[1], p[2])) };

    for d in 0..3usize {
        let u = (d + 1) % 3;
        let v = (d + 2) % 3;
        let u_len = dims[u];
        let v_len = dims[v];
        let mut mask: Vec<Option<MaskCell>> = vec![None; u_len * v_len];

        let mut q = [0i32; 3];
        q[d] = 1;

        for slice in -1..(dims[d] as i32) {
            mask.fill(None);
            let mut n = 0usize;

            for j in 0..v_len {
                for i in 0..u_len {
                    let mut a_pos = origin;
                    a_pos[d] += slice;
                    a_pos[u] += i as i32;
                    a_pos[v] += j as i32;

                    let b_pos = [a_pos[0] + q[0], a_pos[1] + q[1], a_pos[2] + q[2]];
                    let a = sample(a_pos);
                    let b = sample(b_pos);

                    mask[n] = match (a, b) {
                        (Some(color), None) => {
                            let ao = compute_face_ao(&sample, b_pos, u, v);
                            Some(MaskCell {
                                color,
                                back_face: false,
                                ao,
                            })
                        }
                        (None, Some(color)) => {
                            let ao = compute_face_ao(&sample, a_pos, u, v);
                            Some(MaskCell {
                                color,
                                back_face: true,
                                ao,
                            })
                        }
                        _ => None,
                    };
                    n += 1;
                }
            }

            // Greedy merge rectangles in `mask`
            let mut j = 0usize;
            while j < v_len {
                let mut i = 0usize;
                while i < u_len {
                    let Some(cell) = mask[j * u_len + i] else {
                        i += 1;
                        continue;
                    };

                    let mut w = 1usize;
                    while i + w < u_len && mask[j * u_len + i + w] == Some(cell) {
                        w += 1;
                    }

                    let mut h = 1usize;
                    'height: while j + h < v_len {
                        for k in 0..w {
                            if mask[(j + h) * u_len + i + k] != Some(cell) {
                                break 'height;
                            }
                        }
                        h += 1;
                    }

                    // Emit merged quad
                    let mut x = origin;
                    x[d] += slice + 1;
                    x[u] += i as i32;
                    x[v] += j as i32;

                    let mut du = [0i32; 3];
                    du[u] = w as i32;
                    let mut dv = [0i32; 3];
                    dv[v] = h as i32;

                    let p0 = [
                        x[0] as f32 * unit_scale,
                        x[1] as f32 * unit_scale,
                        x[2] as f32 * unit_scale,
                    ];
                    let p1 = [
                        (x[0] + du[0]) as f32 * unit_scale,
                        (x[1] + du[1]) as f32 * unit_scale,
                        (x[2] + du[2]) as f32 * unit_scale,
                    ];
                    let p2 = [
                        (x[0] + du[0] + dv[0]) as f32 * unit_scale,
                        (x[1] + du[1] + dv[1]) as f32 * unit_scale,
                        (x[2] + du[2] + dv[2]) as f32 * unit_scale,
                    ];
                    let p3 = [
                        (x[0] + dv[0]) as f32 * unit_scale,
                        (x[1] + dv[1]) as f32 * unit_scale,
                        (x[2] + dv[2]) as f32 * unit_scale,
                    ];

                    let mut normal = [0.0f32; 3];
                    normal[d] = if cell.back_face { -1.0 } else { 1.0 };

                    let base = output.vertices.len() as u32;
                    let ao_f = |v: u8| 0.55 + (f32::from(v) / 3.0) * 0.45;
                    output.vertices.push(VoxelVertex {
                        position: p0,
                        normal,
                        palette_index: cell.color,
                        ao: ao_f(cell.ao[0]),
                    });
                    output.vertices.push(VoxelVertex {
                        position: p1,
                        normal,
                        palette_index: cell.color,
                        ao: ao_f(cell.ao[1]),
                    });
                    output.vertices.push(VoxelVertex {
                        position: p2,
                        normal,
                        palette_index: cell.color,
                        ao: ao_f(cell.ao[2]),
                    });
                    output.vertices.push(VoxelVertex {
                        position: p3,
                        normal,
                        palette_index: cell.color,
                        ao: ao_f(cell.ao[3]),
                    });

                    // Winding: (u × v) = +d in right-handed cyclic order (0,1,2) -> (1,2,0) -> (2,0,1)
                    // So (p0, p1, p2) & (p0, p2, p3) is CCW when looking from +d.
                    if cell.back_face {
                        output.indices.extend_from_slice(&[
                            base,
                            base + 3,
                            base + 2,
                            base,
                            base + 2,
                            base + 1,
                        ]);
                    } else {
                        output.indices.extend_from_slice(&[
                            base,
                            base + 1,
                            base + 2,
                            base,
                            base + 2,
                            base + 3,
                        ]);
                    }

                    for dy in 0..h {
                        for dx in 0..w {
                            mask[(j + dy) * u_len + i + dx] = None;
                        }
                    }
                    i += w;
                }
                j += 1;
            }
        }
    }

    output
}

fn compute_face_ao<F>(sample: &F, air_pos: [i32; 3], u: usize, v: usize) -> [u8; 4]
where
    F: Fn([i32; 3]) -> Option<PaletteIndex>,
{
    let is_solid = |du: i32, dv: i32| -> bool {
        let mut p = air_pos;
        p[u] += du;
        p[v] += dv;
        sample(p).is_some()
    };

    let vertex_ao = |s1: bool, s2: bool, corner: bool| -> u8 {
        if s1 && s2 {
            0
        } else {
            3 - u8::from(s1) - u8::from(s2) - u8::from(corner)
        }
    };

    let c0 = vertex_ao(is_solid(-1, 0), is_solid(0, -1), is_solid(-1, -1));
    let c1 = vertex_ao(is_solid(1, 0), is_solid(0, -1), is_solid(1, -1));
    let c2 = vertex_ao(is_solid(1, 0), is_solid(0, 1), is_solid(1, 1));
    let c3 = vertex_ao(is_solid(-1, 0), is_solid(0, 1), is_solid(-1, 1));
    [c0, c1, c2, c3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_16x16x16_cube_merges_into_exactly_6_quads() {
        let mut vol = VoxelVolume::new();
        let amber = PaletteIndex::new(4).unwrap();
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    vol.set(VoxelCoord::new(x, y, z), Some(amber));
                }
            }
        }
        assert_eq!(vol.voxel_count(), 4096);
        let mesh = greedy_mesh_volume(&vol, 0.1);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36); // 6 quads = 12 triangles = 36 indices
    }

    #[test]
    fn greedy_mesh_32x32x32_solid_volume_stays_compact() {
        let mut vol = VoxelVolume::new();
        let cobalt = PaletteIndex::new(7).unwrap();
        for y in 0..32 {
            for z in 0..32 {
                for x in 0..32 {
                    vol.set(VoxelCoord::new(x, y, z), Some(cobalt));
                }
            }
        }
        assert_eq!(vol.voxel_count(), 32_768);
        let mesh = greedy_mesh_volume(&vol, 0.1);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
    }
}
