use crate::palette::{Palette, PaletteIndex};

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

/// Split greedy-meshing output separating opaque (`transmission == 0.0`) and
/// translucent/glass (`transmission > 0.0`) voxel geometry (`H3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplitVoxelMeshOutput {
    pub opaque: VoxelMeshOutput,
    pub translucent: VoxelMeshOutput,
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

/// Runs split Greedy Meshing across `volume` using `palette` transmission hints (`H3`):
/// - `opaque`: voxels with `transmission == 0.0` (never culled by neighboring translucent voxels).
/// - `translucent`: voxels with `transmission > 0.0` (internal faces between identical
///   `PaletteIndex` translucent voxels are culled; faces between different translucent indices
///   are preserved; faces flush against opaque voxels are culled to avoid Z-fighting).
pub fn greedy_mesh_volume_split(
    volume: &VoxelVolume,
    palette: &Palette,
    unit_scale: f32,
) -> SplitVoxelMeshOutput {
    let mut is_translucent_slot = [false; 256];
    let mut any_translucent_palette = false;
    for raw in 1u8..=255 {
        if let Ok(idx) = PaletteIndex::new(raw) {
            if palette.get(idx).is_some_and(|e| e.transmission > 0.0) {
                is_translucent_slot[usize::from(raw)] = true;
                any_translucent_palette = true;
            }
        }
    }
    if !any_translucent_palette {
        return SplitVoxelMeshOutput {
            opaque: greedy_mesh_volume(volume, unit_scale),
            translucent: VoxelMeshOutput::default(),
        };
    }

    let mut out = SplitVoxelMeshOutput::default();
    let Some((min_b, max_b)) = volume.bounds() else {
        return out;
    };

    let dx = i64::from(max_b.x) - i64::from(min_b.x) + 1;
    let dy = i64::from(max_b.y) - i64::from(min_b.y) + 1;
    let dz = i64::from(max_b.z) - i64::from(min_b.z) + 1;
    if dx <= 0 || dy <= 0 || dz <= 0 || dx > 513 || dy > 513 || dz > 513 {
        return out;
    }
    let dims = [dx as usize, dy as usize, dz as usize];
    let origin = [min_b.x, min_b.y, min_b.z];

    let is_trans = |c: PaletteIndex| -> bool { is_translucent_slot[usize::from(c.get())] };
    let sample =
        |p: [i32; 3]| -> Option<PaletteIndex> { volume.get(VoxelCoord::new(p[0], p[1], p[2])) };
    let sample_opaque =
        |p: [i32; 3]| -> Option<PaletteIndex> { sample(p).filter(|&c| !is_trans(c)) };

    for d in 0..3usize {
        let u = (d + 1) % 3;
        let v = (d + 2) % 3;
        let u_len = dims[u];
        let v_len = dims[v];
        let mut opaque_mask: Vec<Option<MaskCell>> = vec![None; u_len * v_len];
        let mut trans_front_mask: Vec<Option<MaskCell>> = vec![None; u_len * v_len];
        let mut trans_back_mask: Vec<Option<MaskCell>> = vec![None; u_len * v_len];

        let mut q = [0i32; 3];
        q[d] = 1;

        for slice in -1..(dims[d] as i32) {
            opaque_mask.fill(None);
            trans_front_mask.fill(None);
            trans_back_mask.fill(None);
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
                    let a_opaque = a.filter(|&c| !is_trans(c));
                    let b_opaque = b.filter(|&c| !is_trans(c));
                    let a_trans = a.filter(|&c| is_trans(c));
                    let b_trans = b.filter(|&c| is_trans(c));

                    // Opaque faces: translucent neighbors NEVER cull opaque geometry.
                    opaque_mask[n] = match (a_opaque, b_opaque) {
                        (Some(color), None) => Some(MaskCell {
                            color,
                            back_face: false,
                            ao: compute_face_ao(&sample_opaque, b_pos, u, v),
                        }),
                        (None, Some(color)) => Some(MaskCell {
                            color,
                            back_face: true,
                            ao: compute_face_ao(&sample_opaque, a_pos, u, v),
                        }),
                        _ => None,
                    };

                    // Translucent front (+d) face on `a`:
                    // Emit against air or a DIFFERENT translucent palette index;
                    // cull against identical translucent index or opaque neighbor (avoids Z-fighting).
                    if let Some(ca) = a_trans {
                        let visible = match b {
                            None => true,
                            Some(cb) => is_trans(cb) && cb != ca,
                        };
                        if visible {
                            trans_front_mask[n] = Some(MaskCell {
                                color: ca,
                                back_face: false,
                                ao: [3, 3, 3, 3],
                            });
                        }
                    }

                    // Translucent back (-d) face on `b`:
                    if let Some(cb) = b_trans {
                        let visible = match a {
                            None => true,
                            Some(ca) => is_trans(ca) && ca != cb,
                        };
                        if visible {
                            trans_back_mask[n] = Some(MaskCell {
                                color: cb,
                                back_face: true,
                                ao: [3, 3, 3, 3],
                            });
                        }
                    }

                    n += 1;
                }
            }

            emit_merged_mask(
                &mut opaque_mask,
                &mut out.opaque,
                origin,
                d,
                u,
                v,
                u_len,
                v_len,
                slice,
                unit_scale,
            );
            emit_merged_mask(
                &mut trans_front_mask,
                &mut out.translucent,
                origin,
                d,
                u,
                v,
                u_len,
                v_len,
                slice,
                unit_scale,
            );
            emit_merged_mask(
                &mut trans_back_mask,
                &mut out.translucent,
                origin,
                d,
                u,
                v,
                u_len,
                v_len,
                slice,
                unit_scale,
            );
        }
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn emit_merged_mask(
    mask: &mut [Option<MaskCell>],
    output: &mut VoxelMeshOutput,
    origin: [i32; 3],
    d: usize,
    u: usize,
    v: usize,
    u_len: usize,
    v_len: usize,
    slice: i32,
    unit_scale: f32,
) {
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

    #[test]
    fn h3_split_mesher_preserves_opaque_faces_behind_glass_and_culls_same_glass() {
        let mut palette = Palette::studio_default();
        let opaque_idx = PaletteIndex::new(1).unwrap();
        let glass_a = PaletteIndex::new(20).unwrap();
        let glass_b = PaletteIndex::new(21).unwrap();

        let mut ga =
            crate::palette::PaletteEntry::matte("GlassBlue", crate::Rgba8::rgb(120, 200, 255));
        ga.transmission = 0.85;
        ga.ior = 1.5;
        palette.set(glass_a, ga);

        let mut gb =
            crate::palette::PaletteEntry::matte("GlassAmber", crate::Rgba8::rgb(255, 190, 80));
        gb.transmission = 0.75;
        gb.ior = 1.4;
        palette.set(glass_b, gb);

        // 1. Opaque voxel adjacent to glass voxel: opaque MUST retain all 6 faces (36 indices).
        let mut vol1 = VoxelVolume::new();
        vol1.set(VoxelCoord::new(0, 0, 0), Some(opaque_idx));
        vol1.set(VoxelCoord::new(1, 0, 0), Some(glass_a));
        let split1 = greedy_mesh_volume_split(&vol1, &palette, 0.1);
        assert_eq!(
            split1.opaque.indices.len(),
            36,
            "glass neighbor must never cull opaque voxel face"
        );
        // Glass voxel has 5 exposed faces (the face flush against opaque is culled to prevent Z-fighting).
        assert_eq!(split1.translucent.indices.len(), 30);

        // 2. 2x2x2 block of identical glass voxels merges cleanly into 6 outer quads (no internal seams).
        let mut vol2 = VoxelVolume::new();
        for y in 0..2 {
            for z in 0..2 {
                for x in 0..2 {
                    vol2.set(VoxelCoord::new(x, y, z), Some(glass_a));
                }
            }
        }
        let split2 = greedy_mesh_volume_split(&vol2, &palette, 0.1);
        assert!(split2.opaque.indices.is_empty());
        assert_eq!(
            split2.translucent.indices.len(),
            36,
            "identical adjacent translucent voxels must cull internal faces"
        );

        // 3. Adjacent DIFFERENT glass voxels preserve the shared boundary faces on both sides.
        let mut vol3 = VoxelVolume::new();
        vol3.set(VoxelCoord::new(0, 0, 0), Some(glass_a));
        vol3.set(VoxelCoord::new(1, 0, 0), Some(glass_b));
        let split3 = greedy_mesh_volume_split(&vol3, &palette, 0.1);
        assert!(split3.opaque.indices.is_empty());
        assert_eq!(
            split3.translucent.indices.len(),
            72,
            "different adjacent translucent voxels must preserve boundary faces"
        );
    }
}
