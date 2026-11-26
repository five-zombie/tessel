use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::character::VertexSkinWeights;
use crate::error::CoreError;
use crate::palette::PaletteIndex;
use crate::voxel::{greedy_mesh_volume, VoxelVolume};

use super::face::Face;
use super::lowpoly::{LowPolyMesh, MorphTarget};
use super::nano::{FaceId, PrimitiveAnchor, PrimitiveKind};

/// Minimum primitive size allowed by [`create_primitive`] (5 mm).
pub const MIN_PRIMITIVE_SIZE: f32 = 0.005;

/// Parameters for procedural low-poly heightmap / fBm noise terrain generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainParams {
    pub width: f32,
    pub depth: f32,
    pub subdiv_x: u32,
    pub subdiv_z: u32,
    pub height_scale: f32,
    pub seed: u64,
    pub octaves: u32,
    pub frequency: f32,
    pub island_falloff: f32,
    pub heights: Option<Vec<f32>>,
    pub palette_bands: Vec<PaletteIndex>,
    pub default_material: PaletteIndex,
    pub smooth_shading: bool,
}

/// Generates a faceted (or smooth) low-poly terrain mesh centered on X/Z from an explicit
/// heightmap (`heights`) or deterministic seeded 2D fBm noise, with planar UVs and optional
/// elevation-banded palette coloring.
pub fn create_terrain_mesh(params: &TerrainParams) -> Result<LowPolyMesh, CoreError> {
    if !params.width.is_finite()
        || params.width < MIN_PRIMITIVE_SIZE
        || !params.depth.is_finite()
        || params.depth < MIN_PRIMITIVE_SIZE
        || !params.height_scale.is_finite()
        || !params.frequency.is_finite()
        || params.frequency <= 0.0
        || !params.island_falloff.is_finite()
        || params.island_falloff < 0.0
    {
        return Err(CoreError::OutOfBounds);
    }

    let sx = params.subdiv_x.clamp(1, 128) as usize;
    let sz = params.subdiv_z.clamp(1, 128) as usize;
    let cols = sx + 1;
    let rows = sz + 1;
    let total_verts = cols * rows;

    if let Some(ref h) = params.heights {
        if h.len() != total_verts || h.iter().any(|v| !v.is_finite()) {
            return Err(CoreError::OutOfBounds);
        }
    }

    let octaves = params.octaves.clamp(1, 8);
    let half_w = params.width * 0.5;
    let half_d = params.depth * 0.5;

    let mut vertices = Vec::with_capacity(total_verts);
    let mut uvs = Vec::with_capacity(total_verts);
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for iz in 0..rows {
        let v = iz as f32 / sz as f32;
        let z = -half_d + v * params.depth;
        for ix in 0..cols {
            let u = ix as f32 / sx as f32;
            let x = -half_w + u * params.width;
            let raw_h = if let Some(ref h) = params.heights {
                h[iz * cols + ix]
            } else {
                let nx = x * params.frequency;
                let nz = z * params.frequency;
                let mut n = terrain_fbm_2d(nx, nz, params.seed, octaves);
                if params.island_falloff > 0.0 {
                    let rx = u * 2.0 - 1.0;
                    let rz = v * 2.0 - 1.0;
                    let r_sq = rx * rx + rz * rz;
                    let mask = (1.0 - r_sq * params.island_falloff).clamp(0.0, 1.0);
                    n *= mask;
                }
                n
            };
            let y = raw_h * params.height_scale;
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            vertices.push(Vec3::new(x, y, z));
            uvs.push([u, v]);
        }
    }

    let y_span = (max_y - min_y).max(1e-5);
    let pick_material = |a: usize, b: usize, c: usize| -> PaletteIndex {
        if params.palette_bands.is_empty() {
            return params.default_material;
        }
        let avg_y = (vertices[a].y + vertices[b].y + vertices[c].y) / 3.0;
        let t = ((avg_y - min_y) / y_span).clamp(0.0, 1.0);
        let idx = ((t * (params.palette_bands.len() - 1) as f32).round() as usize)
            .min(params.palette_bands.len() - 1);
        params.palette_bands[idx]
    };

    let mut faces = Vec::with_capacity(sx * sz * 2);
    for iz in 0..sz {
        for ix in 0..sx {
            let i00 = iz * cols + ix;
            let i10 = i00 + 1;
            let i01 = (iz + 1) * cols + ix;
            let i11 = i01 + 1;

            let tris = if (ix + iz) % 2 == 0 {
                [[i01, i11, i10], [i01, i10, i00]]
            } else {
                [[i01, i11, i00], [i11, i10, i00]]
            };
            for [a, b, c] in tris {
                let mat = pick_material(a, b, c);
                faces.push(Face {
                    vertices: vec![a, b, c],
                    uvs: vec![uvs[a], uvs[b], uvs[c]],
                    material: mat,
                });
            }
        }
    }

    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: params.smooth_shading,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

fn terrain_hash_2d(ix: i32, iz: i32, seed: u64) -> f32 {
    let mut h = seed ^ ((ix as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    h ^= (iz as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    ((h >> 40) as f32) / ((1u32 << 24) as f32)
}

fn terrain_value_noise_2d(x: f32, z: f32, seed: u64) -> f32 {
    let ix = x.floor() as i32;
    let iz = z.floor() as i32;
    let fx = x - ix as f32;
    let fz = z - iz as f32;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let a = terrain_hash_2d(ix, iz, seed);
    let b = terrain_hash_2d(ix.wrapping_add(1), iz, seed);
    let c = terrain_hash_2d(ix, iz.wrapping_add(1), seed);
    let d = terrain_hash_2d(ix.wrapping_add(1), iz.wrapping_add(1), seed);

    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uz
}

fn terrain_fbm_2d(x: f32, z: f32, seed: u64, octaves: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 0.5f32;
    let mut freq = 1.0f32;
    let mut norm = 0.0f32;
    for o in 0..octaves {
        let s = seed.wrapping_add((o as u64).wrapping_mul(0x517C_C1B7_2722_0A95));
        sum += terrain_value_noise_2d(x * freq, z * freq, s) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}

/// Shifts `mesh` vertices vertically so its local `Y = 0` pivot matches `anchor` (`Bottom`, `Center`, or `Top`).
pub fn apply_primitive_anchor(mesh: &mut LowPolyMesh, anchor: PrimitiveAnchor) {
    if anchor == PrimitiveAnchor::Bottom || mesh.vertices.is_empty() {
        return;
    }
    let (lo, hi) = mesh.bounds();
    let dy = match anchor {
        PrimitiveAnchor::Bottom => 0.0,
        PrimitiveAnchor::Center => -0.5 * (lo.y + hi.y),
        PrimitiveAnchor::Top => -hi.y,
    };
    if dy.abs() > 1e-7 {
        for v in &mut mesh.vertices {
            v.y += dy;
        }
    }
}

/// Generates a starter low-poly primitive mesh centered on X/Z with base at Y = 0.
pub fn create_primitive(kind: PrimitiveKind, size: f32, material: PaletteIndex) -> LowPolyMesh {
    let s = if size.is_finite() {
        size.max(MIN_PRIMITIVE_SIZE)
    } else {
        1.0
    };
    let h = s * 0.5;
    match kind {
        PrimitiveKind::Cube => {
            let vertices = vec![
                Vec3::new(-h, 0.0, h),
                Vec3::new(h, 0.0, h),
                Vec3::new(h, s, h),
                Vec3::new(-h, s, h),
                Vec3::new(-h, 0.0, -h),
                Vec3::new(h, 0.0, -h),
                Vec3::new(h, s, -h),
                Vec3::new(-h, s, -h),
            ];
            let faces = vec![
                Face::new(vec![0, 1, 2, 3], material), // +Z front
                Face::new(vec![5, 4, 7, 6], material), // -Z back
                Face::new(vec![1, 5, 6, 2], material), // +X right
                Face::new(vec![4, 0, 3, 7], material), // -X left
                Face::new(vec![3, 2, 6, 7], material), // +Y top
                Face::new(vec![4, 5, 1, 0], material), // -Y bottom
            ];
            LowPolyMesh {
                vertices,
                faces,
                texture: None,
                normal_map: None,
                ao_map: None,
                paint_layers: Vec::new(),
                active_paint_layer: None,
                vertex_colors: BTreeMap::new(),
                crease_edges: Vec::new(),
                proxy_normals: BTreeMap::new(),
                smooth_shading: false,
                material: Default::default(),
                skin_weights: BTreeMap::new(),
                morphs: Default::default(),
            }
        }
        PrimitiveKind::Plane => {
            let vertices = vec![
                Vec3::new(-h, 0.0, h),
                Vec3::new(h, 0.0, h),
                Vec3::new(h, 0.0, -h),
                Vec3::new(-h, 0.0, -h),
            ];
            let faces = vec![Face::new(vec![0, 1, 2, 3], material)];
            LowPolyMesh {
                vertices,
                faces,
                texture: None,
                normal_map: None,
                ao_map: None,
                paint_layers: Vec::new(),
                active_paint_layer: None,
                vertex_colors: BTreeMap::new(),
                crease_edges: Vec::new(),
                proxy_normals: BTreeMap::new(),
                smooth_shading: false,
                material: Default::default(),
                skin_weights: BTreeMap::new(),
                morphs: Default::default(),
            }
        }
        PrimitiveKind::Cylinder => create_cylinder(8, h, s, material),
        PrimitiveKind::Cone => create_cone(8, h, s, material),
        PrimitiveKind::Sphere => create_uv_sphere(8, 12, h, material),
        PrimitiveKind::Torus => create_torus(12, 8, s * 0.36, s * 0.14, material),
        PrimitiveKind::Capsule => create_capsule(4, 12, s * 0.25, s, material),
    }
}

fn create_cylinder(
    segments: usize,
    radius: f32,
    height: f32,
    material: PaletteIndex,
) -> LowPolyMesh {
    let mut vertices = Vec::with_capacity(segments * 2);
    for i in 0..segments {
        let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let x = angle.cos() * radius;
        let z = angle.sin() * radius;
        vertices.push(Vec3::new(x, 0.0, z));
        vertices.push(Vec3::new(x, height, z));
    }

    let mut faces = Vec::with_capacity(segments + 2);
    let mut top = Vec::with_capacity(segments);
    let mut bottom = Vec::with_capacity(segments);
    for i in 0..segments {
        let next = (i + 1) % segments;
        let b0 = i * 2;
        let t0 = i * 2 + 1;
        let b1 = next * 2;
        let t1 = next * 2 + 1;
        faces.push(Face::new(vec![b0, b1, t1, t0], material));
        top.push(t0);
        bottom.push(b0);
    }
    top.reverse();
    faces.push(Face::new(top, material));
    faces.push(Face::new(bottom, material));
    LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    }
}

fn create_cone(segments: usize, radius: f32, height: f32, material: PaletteIndex) -> LowPolyMesh {
    let mut vertices = Vec::with_capacity(segments + 1);
    for i in 0..segments {
        let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
        vertices.push(Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius));
    }
    let apex = vertices.len();
    vertices.push(Vec3::new(0.0, height, 0.0));

    let mut faces = Vec::with_capacity(segments + 1);
    let mut bottom = Vec::with_capacity(segments);
    for i in 0..segments {
        let next = (i + 1) % segments;
        faces.push(Face::new(vec![i, next, apex], material));
        bottom.push(i);
    }
    faces.push(Face::new(bottom, material));
    LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    }
}

fn create_uv_sphere(
    rings: usize,
    sectors: usize,
    radius: f32,
    material: PaletteIndex,
) -> LowPolyMesh {
    let mut vertices = Vec::with_capacity(2 + (rings - 1) * sectors);
    vertices.push(Vec3::new(0.0, 0.0, 0.0)); // 0: bottom pole
    for r in 1..rings {
        let phi = std::f32::consts::PI * (r as f32 / rings as f32);
        let y = radius - radius * phi.cos();
        let ring_r = radius * phi.sin();
        for s in 0..sectors {
            let theta = std::f32::consts::TAU * (s as f32 / sectors as f32);
            vertices.push(Vec3::new(ring_r * theta.cos(), y, -ring_r * theta.sin()));
        }
    }
    let top_pole = vertices.len();
    vertices.push(Vec3::new(0.0, radius * 2.0, 0.0));

    let mut faces = Vec::with_capacity(rings * sectors);
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        faces.push(Face::new(vec![0, 1 + next, 1 + s], material));
    }
    for r in 0..(rings - 2) {
        let row0 = 1 + r * sectors;
        let row1 = row0 + sectors;
        for s in 0..sectors {
            let next = (s + 1) % sectors;
            faces.push(Face::new(
                vec![row0 + s, row0 + next, row1 + next, row1 + s],
                material,
            ));
        }
    }
    let last_row = 1 + (rings - 2) * sectors;
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        faces.push(Face::new(
            vec![last_row + s, last_row + next, top_pole],
            material,
        ));
    }

    LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: true,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    }
}

fn create_torus(
    major_seg: usize,
    minor_seg: usize,
    major_r: f32,
    minor_r: f32,
    material: PaletteIndex,
) -> LowPolyMesh {
    let mut vertices = Vec::with_capacity(major_seg * minor_seg);
    for i in 0..major_seg {
        let theta = std::f32::consts::TAU * (i as f32 / major_seg as f32);
        let dir = Vec3::new(theta.cos(), 0.0, -theta.sin());
        for j in 0..minor_seg {
            let phi = std::f32::consts::TAU * (j as f32 / minor_seg as f32);
            let radial = major_r + minor_r * phi.cos();
            let y = minor_r + minor_r * phi.sin();
            vertices.push(dir * radial + Vec3::Y * y);
        }
    }
    let mut faces = Vec::with_capacity(major_seg * minor_seg);
    for i in 0..major_seg {
        let i_next = (i + 1) % major_seg;
        for j in 0..minor_seg {
            let j_next = (j + 1) % minor_seg;
            let v00 = i * minor_seg + j;
            let v10 = i_next * minor_seg + j;
            let v11 = i_next * minor_seg + j_next;
            let v01 = i * minor_seg + j_next;
            faces.push(Face::new(vec![v00, v10, v11, v01], material));
        }
    }
    LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: true,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    }
}

fn create_capsule(
    hemi_rings: usize,
    sectors: usize,
    radius: f32,
    total_height: f32,
    material: PaletteIndex,
) -> LowPolyMesh {
    let r = radius.min(total_height * 0.45);
    let cyl_h = (total_height - 2.0 * r).max(0.001);
    let mut vertices = Vec::new();
    vertices.push(Vec3::new(0.0, 0.0, 0.0)); // 0: bottom pole

    // Bottom hemisphere (1..=hemi_rings)
    for ring in 1..=hemi_rings {
        let phi = std::f32::consts::FRAC_PI_2 * (ring as f32 / hemi_rings as f32);
        let y = r - r * phi.cos();
        let ring_r = r * phi.sin();
        for s in 0..sectors {
            let theta = std::f32::consts::TAU * (s as f32 / sectors as f32);
            vertices.push(Vec3::new(ring_r * theta.cos(), y, -ring_r * theta.sin()));
        }
    }
    // Top hemisphere (0..hemi_rings) shifted by cyl_h
    for ring in 0..hemi_rings {
        let phi = std::f32::consts::FRAC_PI_2
            + std::f32::consts::FRAC_PI_2 * (ring as f32 / hemi_rings as f32);
        let y = r + cyl_h - r * phi.cos();
        let ring_r = r * phi.sin();
        for s in 0..sectors {
            let theta = std::f32::consts::TAU * (s as f32 / sectors as f32);
            vertices.push(Vec3::new(ring_r * theta.cos(), y, -ring_r * theta.sin()));
        }
    }
    let top_pole = vertices.len();
    vertices.push(Vec3::new(0.0, 2.0 * r + cyl_h, 0.0));

    let total_rings = hemi_rings * 2;
    let mut faces = Vec::new();
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        faces.push(Face::new(vec![0, 1 + next, 1 + s], material));
    }
    for ring in 0..(total_rings - 1) {
        let row0 = 1 + ring * sectors;
        let row1 = row0 + sectors;
        for s in 0..sectors {
            let next = (s + 1) % sectors;
            faces.push(Face::new(
                vec![row0 + s, row0 + next, row1 + next, row1 + s],
                material,
            ));
        }
    }
    let last_row = 1 + (total_rings - 1) * sectors;
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        faces.push(Face::new(
            vec![last_row + s, last_row + next, top_pole],
            material,
        ));
    }

    LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: true,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    }
}

/// Generates a curved strap, belt, pipe, or cable mesh along 3D control `points`.
/// `flatness` in `0.1..=1.0` controls elliptical thickness (`1.0` = round tube, `0.25` = flat strap).
pub fn create_curve_tube(
    points: &[Vec3],
    radius: f32,
    flatness: f32,
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if points.len() < 2
        || points.len() > 128
        || !radius.is_finite()
        || radius <= 0.001
        || !flatness.is_finite()
        || points.iter().any(|p| !p.is_finite())
    {
        return Err(CoreError::OutOfBounds);
    }

    // Apply 1 pass of Chaikin smoothing when >= 3 control points so straps curve naturally
    let path: Vec<Vec3> = if points.len() >= 3 {
        let mut smooth = Vec::with_capacity(points.len() * 2);
        smooth.push(points[0]);
        for seg in points.windows(2) {
            smooth.push(seg[0].lerp(seg[1], 0.25));
            smooth.push(seg[0].lerp(seg[1], 0.75));
        }
        smooth.push(*points.last().unwrap_or(&points[0]));
        smooth
    } else {
        points.to_vec()
    };

    let sectors = 8usize;
    let rx = radius.max(0.005);
    let ry = rx * flatness.clamp(0.1, 1.0);
    let mut vertices = Vec::with_capacity(path.len() * sectors + 2);
    let mut prev_right = Vec3::X;

    for i in 0..path.len() {
        let tangent = if i == 0 {
            (path[1] - path[0]).normalize_or_zero()
        } else if i + 1 == path.len() {
            (path[i] - path[i - 1]).normalize_or_zero()
        } else {
            (path[i + 1] - path[i - 1]).normalize_or_zero()
        };
        let t = if tangent.length_squared() > 1e-6 {
            tangent
        } else {
            Vec3::Y
        };
        let mut right = (prev_right - t * prev_right.dot(t)).normalize_or_zero();
        if right.length_squared() < 1e-4 {
            let fallback = if t.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
            right = fallback.cross(t).normalize_or_zero();
        }
        let up = t.cross(right).normalize_or_zero();
        prev_right = right;

        for s in 0..sectors {
            let ang = std::f32::consts::TAU * (s as f32 / sectors as f32);
            let offset = right * (ang.cos() * rx) + up * (ang.sin() * ry);
            vertices.push(path[i] + offset);
        }
    }

    let mut faces = Vec::with_capacity((path.len() - 1) * sectors + 2);
    for ring in 0..(path.len() - 1) {
        let r0 = ring * sectors;
        let r1 = r0 + sectors;
        for s in 0..sectors {
            let next = (s + 1) % sectors;
            faces.push(Face::new(
                vec![r0 + s, r0 + next, r1 + next, r1 + s],
                material,
            ));
        }
    }
    // Start and end caps
    let start_cap: Vec<usize> = (0..sectors).rev().collect();
    faces.push(Face::new(start_cap, material));
    let last_base = (path.len() - 1) * sectors;
    let end_cap: Vec<usize> = (0..sectors).map(|s| last_base + s).collect();
    faces.push(Face::new(end_cap, material));

    Ok(LowPolyMesh {
        vertices,
        faces,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: true,
        material: Default::default(),
    })
}

/// Generates a surface-of-revolution (lathe) mesh by revolving a 2D `profile` (`[radius, y]`,
/// ordered bottom-to-top) around the `+Y` axis. Supports full 360° revolutions and partial arcs
/// (e.g. `180.0` centered on `+Z` front for visors/shields) with automatic pole-collapse and
/// guaranteed outward CCW winding.
pub fn create_lathe_mesh(
    profile: &[[f32; 2]],
    segments: u32,
    angle_deg: f32,
    cap_ends: bool,
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if profile.len() < 2
        || profile.len() > 128
        || !angle_deg.is_finite()
        || angle_deg <= 1.0
        || angle_deg > 360.0
        || profile
            .iter()
            .any(|&[r, y]| !r.is_finite() || !y.is_finite() || r < 0.0)
    {
        return Err(CoreError::OutOfBounds);
    }

    let segs = segments.clamp(3, 64) as usize;
    let full_circle = angle_deg >= 359.99;
    let sweep = if full_circle {
        std::f32::consts::TAU
    } else {
        angle_deg.to_radians()
    };
    let start_angle = if full_circle { 0.0 } else { -sweep * 0.5 };
    let cols = if full_circle { segs } else { segs + 1 };

    let mut vertices = Vec::with_capacity(profile.len() * cols);
    // Each profile point maps either to a single pole vertex or a ring of `cols` vertices.
    enum Ring {
        Pole(usize),
        Loop(usize),
    }
    let mut rings = Vec::with_capacity(profile.len());

    for &[r, y] in profile {
        if r <= 1e-4 {
            let idx = vertices.len();
            vertices.push(Vec3::new(0.0, y, 0.0));
            rings.push(Ring::Pole(idx));
        } else {
            let base = vertices.len();
            for s in 0..cols {
                let frac = s as f32 / segs as f32;
                let theta = start_angle + frac * sweep;
                vertices.push(Vec3::new(r * theta.sin(), y, r * theta.cos()));
            }
            rings.push(Ring::Loop(base));
        }
    }

    let mut faces = Vec::with_capacity(profile.len() * segs + 2);
    for pair in rings.windows(2) {
        match (&pair[0], &pair[1]) {
            (Ring::Pole(_), Ring::Pole(_)) => {}
            (Ring::Pole(p0), Ring::Loop(r1)) => {
                for s in 0..segs {
                    let next = if full_circle { (s + 1) % segs } else { s + 1 };
                    faces.push(Face::new(vec![*p0, r1 + next, r1 + s], material));
                }
            }
            (Ring::Loop(r0), Ring::Pole(p1)) => {
                for s in 0..segs {
                    let next = if full_circle { (s + 1) % segs } else { s + 1 };
                    faces.push(Face::new(vec![r0 + s, r0 + next, *p1], material));
                }
            }
            (Ring::Loop(r0), Ring::Loop(r1)) => {
                for s in 0..segs {
                    let next = if full_circle { (s + 1) % segs } else { s + 1 };
                    faces.push(Face::new(
                        vec![r0 + s, r0 + next, r1 + next, r1 + s],
                        material,
                    ));
                }
            }
        }
    }

    if cap_ends && full_circle {
        if let Some(Ring::Loop(r0)) = rings.first() {
            let bottom: Vec<usize> = (0..segs).rev().map(|s| r0 + s).collect();
            faces.push(Face::new(bottom, material));
        }
        if let Some(Ring::Loop(rl)) = rings.last() {
            let top: Vec<usize> = (0..segs).map(|s| rl + s).collect();
            faces.push(Face::new(top, material));
        }
    }

    if faces.is_empty() {
        return Err(CoreError::OutOfBounds);
    }

    Ok(LowPolyMesh {
        vertices,
        faces,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
    })
}

/// Extrudes a 2D polygon `points` (`[[x, y], ...]`, 3..=64 vertices) along the Z axis centered at
/// `[-depth/2, +depth/2]`. Automatically normalizes clockwise input to CCW via signed shoelace
/// area so front cap (`+Z`), back cap (`-Z`), and all perimeter side walls have outward normals.
pub fn create_extruded_polygon(
    points: &[[f32; 2]],
    depth: f32,
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if points.len() < 3
        || points.len() > 64
        || !depth.is_finite()
        || depth < MIN_PRIMITIVE_SIZE
        || points
            .iter()
            .any(|&[x, y]| !x.is_finite() || !y.is_finite())
    {
        return Err(CoreError::OutOfBounds);
    }

    let mut poly: Vec<[f32; 2]> = points.to_vec();
    let mut area2 = 0.0f32;
    for i in 0..poly.len() {
        let [x0, y0] = poly[i];
        let [x1, y1] = poly[(i + 1) % poly.len()];
        area2 += x0 * y1 - x1 * y0;
    }
    if area2.abs() < 1e-7 {
        return Err(CoreError::OutOfBounds);
    }
    if area2 < 0.0 {
        poly.reverse();
    }
    if polygon_is_self_intersecting_2d(&poly) {
        return Err(CoreError::OutOfBounds);
    }

    let n = poly.len();
    let half_z = depth * 0.5;
    let mut vertices = Vec::with_capacity(n * 2);
    for &[x, y] in &poly {
        vertices.push(Vec3::new(x, y, half_z));
    }
    for &[x, y] in &poly {
        vertices.push(Vec3::new(x, y, -half_z));
    }

    let cap_tris = ear_clip_ccw(&poly, area2.abs());
    let mut faces = Vec::with_capacity(n + cap_tris.len() * 2);
    // Front (+Z) cap (ear-clipped CCW triangles so concave polygons never produce diagonal webs)
    for &[a, b, c] in &cap_tris {
        faces.push(Face::new(vec![a, b, c], material));
    }
    // Back (-Z) cap (reversed winding for outward -Z normals)
    for &[a, b, c] in &cap_tris {
        faces.push(Face::new(vec![n + a, n + c, n + b], material));
    }
    // Perimeter side walls (outward CCW)
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(Face::new(vec![i, n + i, n + j, j], material));
    }

    Ok(LowPolyMesh {
        vertices,
        faces,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
    })
}

fn cross_2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_ccw_triangle_2d(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2], eps: f32) -> bool {
    cross_2d(a, b, p) >= -eps && cross_2d(b, c, p) >= -eps && cross_2d(c, a, p) >= -eps
}

fn segments_properly_intersect_2d(
    a1: [f32; 2],
    a2: [f32; 2],
    b1: [f32; 2],
    b2: [f32; 2],
    eps: f32,
) -> bool {
    let d1 = cross_2d(a1, a2, b1);
    let d2 = cross_2d(a1, a2, b2);
    let d3 = cross_2d(b1, b2, a1);
    let d4 = cross_2d(b1, b2, a2);
    ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
}

fn polygon_is_self_intersecting_2d(poly: &[[f32; 2]]) -> bool {
    let n = poly.len();
    if n <= 3 {
        return false;
    }
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &[x, y] in poly {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let diag_sq = (max_x - min_x).powi(2) + (max_y - min_y).powi(2);
    let eps = (diag_sq * 1e-9).max(1e-12);
    for i in 0..n {
        let i_next = (i + 1) % n;
        for j in (i + 2)..n {
            let j_next = (j + 1) % n;
            if j_next == i {
                continue;
            }
            if segments_properly_intersect_2d(poly[i], poly[i_next], poly[j], poly[j_next], eps) {
                return true;
            }
        }
    }
    false
}

/// Triangulates a simple 2D polygon in CCW winding order using ear clipping with scale-relative epsilon.
fn ear_clip_ccw(poly: &[[f32; 2]], area2: f32) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &[x, y] in poly {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let diag_sq = (max_x - min_x).powi(2) + (max_y - min_y).powi(2);
    let eps = (area2 * 1e-6).clamp(1e-12, 1e-6);
    let min_tri_area = (area2 * 1e-9).clamp(1e-14, 1e-10);
    let near_eps_sq = (diag_sq * 1e-12).max(1e-18);

    let mut remaining: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n - 2);

    while remaining.len() > 3 {
        let m = remaining.len();
        let mut ear_pos: Option<usize> = None;

        for strict in [true, false] {
            for k in 0..m {
                let prev = remaining[(k + m - 1) % m];
                let curr = remaining[k];
                let next = remaining[(k + 1) % m];
                let a = poly[prev];
                let b = poly[curr];
                let c = poly[next];
                let area = cross_2d(a, b, c);
                if (strict && area <= eps) || (!strict && area < -eps * 0.1) {
                    continue;
                }
                let contains_other = remaining.iter().copied().any(|idx| {
                    if idx == prev || idx == curr || idx == next {
                        return false;
                    }
                    let p = poly[idx];
                    let near_corner = [a, b, c].iter().any(|&v| {
                        let dx = p[0] - v[0];
                        let dy = p[1] - v[1];
                        dx * dx + dy * dy <= near_eps_sq
                    });
                    !near_corner && point_in_ccw_triangle_2d(p, a, b, c, eps)
                });
                if !contains_other {
                    ear_pos = Some(k);
                    break;
                }
            }
            if ear_pos.is_some() {
                break;
            }
        }

        let k = ear_pos.unwrap_or(1);
        let prev = remaining[(k + m - 1) % m];
        let curr = remaining[k];
        let next = remaining[(k + 1) % m];
        if cross_2d(poly[prev], poly[curr], poly[next]) > min_tri_area {
            tris.push([prev, curr, next]);
        }
        remaining.remove(k);
    }

    if remaining.len() == 3 {
        let [a, b, c] = [remaining[0], remaining[1], remaining[2]];
        if cross_2d(poly[a], poly[b], poly[c]) > min_tri_area || tris.is_empty() {
            tris.push([a, b, c]);
        }
    }

    tris
}

/// Connects 2..=32 closed cross-section rings of equal vertex count (`3..=64`) into a lofted
/// Low-Poly mesh with automatic outward CCW winding and optional end caps.
pub fn create_loft_mesh(
    sections: &[Vec<Vec3>],
    cap_ends: bool,
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if sections.len() < 2 || sections.len() > 32 {
        return Err(CoreError::OutOfBounds);
    }
    let ring_len = sections[0].len();
    if !(3..=64).contains(&ring_len)
        || sections
            .iter()
            .any(|s| s.len() != ring_len || s.iter().any(|v| !v.is_finite()))
    {
        return Err(CoreError::OutOfBounds);
    }

    // Detect whether the first band's quad normals point outward from ring 0's centroid.
    let c0: Vec3 = sections[0].iter().copied().sum::<Vec3>() / (ring_len as f32);
    let mut outward_score = 0.0f32;
    for i in 0..ring_len {
        let j = (i + 1) % ring_len;
        let v00 = sections[0][i];
        let v01 = sections[0][j];
        let v10 = sections[1][i];
        let n = (v01 - v00).cross(v10 - v00);
        let radial = (v00 + v01) * 0.5 - c0;
        outward_score += n.dot(radial);
    }
    let flip = outward_score < 0.0;

    let mut vertices = Vec::with_capacity(sections.len() * ring_len);
    for sec in sections {
        if flip {
            vertices.extend(sec.iter().rev().copied());
        } else {
            vertices.extend(sec.iter().copied());
        }
    }

    let mut faces = Vec::with_capacity((sections.len() - 1) * ring_len + 2);
    for r in 0..(sections.len() - 1) {
        let b0 = r * ring_len;
        let b1 = (r + 1) * ring_len;
        for i in 0..ring_len {
            let j = (i + 1) % ring_len;
            faces.push(Face::new(vec![b0 + i, b0 + j, b1 + j, b1 + i], material));
        }
    }

    if cap_ends {
        faces.push(Face::new((0..ring_len).rev().collect(), material));
        let last_base = (sections.len() - 1) * ring_len;
        faces.push(Face::new(
            (0..ring_len).map(|i| last_base + i).collect(),
            material,
        ));
    }

    Ok(LowPolyMesh {
        vertices,
        faces,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
    })
}

/// Simplifies `mesh` by clustering vertices on a 3D grid scaled to `target_ratio` (`0.1..=0.9`).
/// Clears manual skin weights (full rebuild reindexes every vertex).
pub fn decimate_mesh(mesh: &LowPolyMesh, target_ratio: f32) -> Result<LowPolyMesh, CoreError> {
    if !target_ratio.is_finite() || mesh.faces.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    if mesh.vertices.len() <= 8 {
        return Ok(mesh.clone());
    }
    let ratio = target_ratio.clamp(0.1, 0.9);
    let (bmin, bmax) = mesh.bounds();
    let extent = (bmax - bmin).max(Vec3::splat(1e-4));
    let target_verts = ((mesh.vertices.len() as f32) * ratio).max(4.0);
    let grid_res = (target_verts.cbrt().round() as i32).clamp(2, 48);

    let mut cell_accum: BTreeMap<[i32; 3], (Vec3, usize, usize)> = BTreeMap::new();
    let mut vert_to_cell: Vec<[i32; 3]> = Vec::with_capacity(mesh.vertices.len());
    let mut new_vertices: Vec<Vec3> = Vec::new();

    for &p in &mesh.vertices {
        let norm = (p - bmin) / extent;
        let cell = [
            ((norm.x * grid_res as f32).floor() as i32).clamp(0, grid_res),
            ((norm.y * grid_res as f32).floor() as i32).clamp(0, grid_res),
            ((norm.z * grid_res as f32).floor() as i32).clamp(0, grid_res),
        ];
        vert_to_cell.push(cell);
        cell_accum
            .entry(cell)
            .and_modify(|(sum, count, _)| {
                *sum += p;
                *count += 1;
            })
            .or_insert_with(|| {
                let idx = new_vertices.len();
                new_vertices.push(p);
                (p, 1, idx)
            });
    }

    for (sum, count, idx) in cell_accum.values() {
        new_vertices[*idx] = *sum / (*count as f32);
    }

    let mut new_faces = Vec::new();
    for face in &mesh.faces {
        let mut loop_verts: Vec<usize> = Vec::new();
        for &vi in &face.vertices {
            let cell = vert_to_cell.get(vi).ok_or(CoreError::OutOfBounds)?;
            let idx = cell_accum.get(cell).ok_or(CoreError::OutOfBounds)?.2;
            if !loop_verts.contains(&idx) {
                loop_verts.push(idx);
            }
        }
        if loop_verts.len() >= 3 {
            new_faces.push(Face::new(loop_verts, face.material));
        }
    }

    if new_faces.is_empty() {
        return Ok(mesh.clone());
    }

    // Full rebuild: vertex indices stop meaning the same corners, so manual
    // skin weights and morphs cannot be carried over and are cleared.
    Ok(LowPolyMesh {
        vertices: new_vertices,
        faces: new_faces,
        texture: mesh.texture.clone(),
        normal_map: mesh.normal_map.clone(),
        ao_map: mesh.ao_map.clone(),
        paint_layers: mesh.paint_layers.clone(),
        active_paint_layer: mesh.active_paint_layer,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

/// Translates all unique vertices belonging to `face_id` by `delta`.
pub fn translate_face(
    mesh: &LowPolyMesh,
    face_id: FaceId,
    delta: Vec3,
) -> Result<LowPolyMesh, CoreError> {
    if !delta.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    if face.vertices.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let mut next = mesh.clone();
    let unique_verts: BTreeSet<usize> = face.vertices.iter().copied().collect();
    for vi in unique_verts {
        let v = next.vertices.get_mut(vi).ok_or(CoreError::OutOfBounds)?;
        *v += delta;
    }
    Ok(next)
}

/// Moves an explicit list of vertices by `delta` (per-vertex editing: unlike
/// `translate_face`, the caller picks individual corners, including vertices
/// shared across faces, without touching their neighbours).
pub fn move_mesh_vertices(
    mesh: &LowPolyMesh,
    vertex_ids: &[usize],
    delta: Vec3,
) -> Result<LowPolyMesh, CoreError> {
    if !delta.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    if vertex_ids.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let unique_verts: BTreeSet<usize> = vertex_ids.iter().copied().collect();
    if unique_verts.iter().any(|vi| *vi >= mesh.vertices.len()) {
        return Err(CoreError::OutOfBounds);
    }
    let mut next = mesh.clone();
    for vi in unique_verts {
        next.vertices[vi] += delta;
    }
    Ok(next)
}

/// Sets one vertex to an absolute `position` (precise 1:1 placement).
pub fn set_mesh_vertex(
    mesh: &LowPolyMesh,
    vertex_id: usize,
    position: Vec3,
) -> Result<LowPolyMesh, CoreError> {
    if !position.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    if vertex_id >= mesh.vertices.len() {
        return Err(CoreError::OutOfBounds);
    }
    let mut next = mesh.clone();
    next.vertices[vertex_id] = position;
    Ok(next)
}

/// Sets manual per-corner UVs on `face_id` (manual UV unwrap: `uvs` must carry
/// exactly one coordinate per face corner; values are clamped to `0..=1`).
/// Export preserves them via `TEXCOORD_0`, overriding the auto box-atlas UV.
pub fn set_face_uvs(
    mesh: &LowPolyMesh,
    face_id: FaceId,
    uvs: &[[f32; 2]],
) -> Result<LowPolyMesh, CoreError> {
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    if face.vertices.len() < 3 || uvs.len() != face.vertices.len() {
        return Err(CoreError::OutOfBounds);
    }
    if uvs
        .iter()
        .any(|uv| !uv[0].is_finite() || !uv[1].is_finite())
    {
        return Err(CoreError::OutOfBounds);
    }
    let mut next = mesh.clone();
    next.faces[face_id.0].uvs = uvs
        .iter()
        .map(|uv| [uv[0].clamp(0.0, 1.0), uv[1].clamp(0.0, 1.0)])
        .collect();
    Ok(next)
}

/// Paints manual skin weights onto explicit vertices (weight paint). `joints`
/// holds up to 4 bone indices and `weights` their influences; weights are
/// normalized to sum 1. At least one weight must be positive. Export prefers
/// these over the auto solver. Survives index-stable edits; cleared by
/// bevel/subdivide/decimate.
/// Paints smooth radial skin-weight influence onto vertices of `mesh` within `radius`
/// of local-space `hit_point` for `joint_index` with `strength` in `(0.0..=1.0]`.
/// Blends smoothly with existing manual or `fallback_weights` (from `AutoSkinSolver`),
/// keeping up to 4 normalized influences per vertex. Returns `Ok(None)` if no vertex falls
/// within `radius`.
pub fn paint_vertex_weights_brush(
    mesh: &LowPolyMesh,
    hit_point: Vec3,
    radius: f32,
    joint_index: u16,
    strength: f32,
    fallback_weights: Option<&dyn Fn(usize, Vec3) -> VertexSkinWeights>,
) -> Result<Option<LowPolyMesh>, CoreError> {
    if !hit_point.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || !strength.is_finite()
        || !(0.0..=1.0).contains(&strength)
    {
        return Err(CoreError::OutOfBounds);
    }
    if strength <= 1e-6 {
        return Ok(None);
    }

    let mut next = mesh.clone();
    let mut touched = false;

    for (vi, &vpos) in mesh.vertices.iter().enumerate() {
        if !vpos.is_finite() {
            continue;
        }
        let dist = (vpos - hit_point).length();
        if !dist.is_finite() || dist > radius {
            continue;
        }
        let falloff = (1.0 - dist / radius).clamp(0.0, 1.0);
        let alpha = (strength * falloff).clamp(0.0, 1.0);
        if !alpha.is_finite() || alpha <= 1e-6 {
            continue;
        }

        let base = next
            .skin_weights
            .get(&vi)
            .copied()
            .or_else(|| fallback_weights.map(|f| f(vi, vpos)))
            .unwrap_or(VertexSkinWeights {
                joints: [joint_index, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            });

        let mut merged: BTreeMap<u16, f32> = BTreeMap::new();
        for (&j, &w) in base.joints.iter().zip(base.weights.iter()) {
            if !w.is_finite() || w <= 0.0 {
                continue;
            }
            let scaled = w * (1.0 - alpha);
            if scaled.is_finite() && scaled > 1e-6 {
                *merged.entry(j).or_insert(0.0) += scaled;
            }
        }
        *merged.entry(joint_index).or_insert(0.0) += alpha;

        let mut pairs: Vec<(u16, f32)> = merged.into_iter().collect();
        pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
        pairs.truncate(4);
        let sum: f32 = pairs.iter().map(|(_, w)| *w).sum();
        if !sum.is_finite() || sum <= 1e-6 {
            continue;
        }

        let mut joints = [0u16; 4];
        let mut weights = [0.0f32; 4];
        for (slot, (j, w)) in pairs.into_iter().enumerate() {
            joints[slot] = j;
            weights[slot] = w / sum;
        }
        next.skin_weights
            .insert(vi, VertexSkinWeights { joints, weights });
        touched = true;
    }

    if touched {
        Ok(Some(next))
    } else {
        Ok(None)
    }
}

pub fn set_vertex_weights(
    mesh: &LowPolyMesh,
    vertex_ids: &[usize],
    joints: [u16; 4],
    weights: [f32; 4],
) -> Result<LowPolyMesh, CoreError> {
    if vertex_ids.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    if !weights.iter().all(|w| w.is_finite() && *w >= 0.0) {
        return Err(CoreError::OutOfBounds);
    }
    let sum: f32 = weights.iter().sum();
    if !sum.is_finite() || sum <= 1e-6 {
        return Err(CoreError::OutOfBounds);
    }
    let unique_verts: BTreeSet<usize> = vertex_ids.iter().copied().collect();
    if unique_verts.iter().any(|vi| *vi >= mesh.vertices.len()) {
        return Err(CoreError::OutOfBounds);
    }
    let normalized = VertexSkinWeights {
        joints,
        weights: [
            weights[0] / sum,
            weights[1] / sum,
            weights[2] / sum,
            weights[3] / sum,
        ],
    };
    let mut next = mesh.clone();
    for vi in unique_verts {
        next.skin_weights.insert(vi, normalized);
    }
    Ok(next)
}

/// Clears manual skin-weight overrides: all of them when `vertex_ids` is
/// `None`, otherwise only the listed vertices.
pub fn clear_vertex_weights(
    mesh: &LowPolyMesh,
    vertex_ids: Option<&[usize]>,
) -> Result<LowPolyMesh, CoreError> {
    let mut next = mesh.clone();
    match vertex_ids {
        None => next.skin_weights.clear(),
        Some(ids) => {
            for vi in ids {
                if *vi >= mesh.vertices.len() {
                    return Err(CoreError::OutOfBounds);
                }
                next.skin_weights.remove(vi);
            }
        }
    }
    Ok(next)
}

/// Adds (accumulates) a position delta onto a named morph target
/// (blend shape), creating the target if missing. Deltas compose with the
/// base vertex positions at export. Survives index-stable edits; cleared by
/// bevel/subdivide/decimate.
pub fn add_morph_delta(
    mesh: &LowPolyMesh,
    name: &str,
    vertex_ids: &[usize],
    delta: Vec3,
) -> Result<LowPolyMesh, CoreError> {
    if name.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    if !delta.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    if vertex_ids.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let unique_verts: BTreeSet<usize> = vertex_ids.iter().copied().collect();
    if unique_verts.iter().any(|vi| *vi >= mesh.vertices.len()) {
        return Err(CoreError::OutOfBounds);
    }
    let mut next = mesh.clone();
    let target = match next.morphs.iter_mut().find(|m| m.name == name) {
        Some(t) => t,
        None => {
            let idx = next.morphs.len();
            next.morphs.push(MorphTarget {
                name: name.to_string(),
                deltas: BTreeMap::new(),
                normal_deltas: BTreeMap::new(),
                default_weight: 0.0,
            });
            &mut next.morphs[idx]
        }
    };
    for vi in unique_verts {
        let entry = target.deltas.entry(vi).or_insert(Vec3::ZERO);
        *entry += delta;
        if entry.length_squared() < 1e-12 {
            target.deltas.remove(&vi);
        }
    }
    next.morphs.retain(|m| !m.deltas.is_empty());
    Ok(next)
}

/// Clears morph targets: all of them when `name` is `None`, otherwise only
/// the named target (missing names are a no-op success).
pub fn clear_morphs(mesh: &LowPolyMesh, name: Option<&str>) -> Result<LowPolyMesh, CoreError> {
    let mut next = mesh.clone();
    match name {
        None => next.morphs.clear(),
        Some(n) => next.morphs.retain(|m| m.name != n),
    }
    Ok(next)
}

/// Extrudes `face_id` along its face normal by `distance`.
pub fn extrude_face(
    mesh: &LowPolyMesh,
    face_id: FaceId,
    distance: f32,
) -> Result<LowPolyMesh, CoreError> {
    if !distance.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    if face.vertices.len() < 3 {
        return Err(CoreError::OutOfBounds);
    }
    for &vi in &face.vertices {
        if vi >= mesh.vertices.len() {
            return Err(CoreError::OutOfBounds);
        }
    }
    let p0 = *mesh
        .vertices
        .get(face.vertices[0])
        .ok_or(CoreError::OutOfBounds)?;
    let p1 = *mesh
        .vertices
        .get(face.vertices[1])
        .ok_or(CoreError::OutOfBounds)?;
    let p2 = *mesh
        .vertices
        .get(face.vertices[2])
        .ok_or(CoreError::OutOfBounds)?;
    let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();

    let mut next = mesh.clone();
    let old_verts = face.vertices.clone();
    let material = face.material;
    let mut new_verts = Vec::with_capacity(old_verts.len());

    for &vi in &old_verts {
        let base = *next.vertices.get(vi).ok_or(CoreError::OutOfBounds)?;
        let idx = next.vertices.len();
        next.vertices.push(base + normal * distance);
        new_verts.push(idx);
    }

    // Replace extruded cap face
    next.faces[face_id.0] = Face::new(new_verts.clone(), material);

    // Add side quads
    let n = old_verts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        next.faces.push(Face::new(
            vec![old_verts[i], old_verts[j], new_verts[j], new_verts[i]],
            material,
        ));
    }

    Ok(next)
}

/// Insets `face_id` toward its centroid by `factor` (clamped to `0.05..=0.9`) and adds side quads.
pub fn inset_face(
    mesh: &LowPolyMesh,
    face_id: FaceId,
    factor: f32,
) -> Result<LowPolyMesh, CoreError> {
    if !factor.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    if face.vertices.len() < 3 {
        return Err(CoreError::OutOfBounds);
    }

    let t = factor.clamp(0.05, 0.9);
    let mut centroid = Vec3::ZERO;
    for &vi in &face.vertices {
        centroid += *mesh.vertices.get(vi).ok_or(CoreError::OutOfBounds)?;
    }
    centroid /= face.vertices.len() as f32;

    let mut next = mesh.clone();
    let old_verts = face.vertices.clone();
    let material = face.material;
    let mut new_verts = Vec::with_capacity(old_verts.len());

    for &vi in &old_verts {
        let p = next.vertices[vi];
        let idx = next.vertices.len();
        next.vertices.push(p.lerp(centroid, t));
        new_verts.push(idx);
    }

    next.faces[face_id.0] = Face::new(new_verts.clone(), material);
    let n = old_verts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        next.faces.push(Face::new(
            vec![old_verts[i], old_verts[j], new_verts[j], new_verts[i]],
            material,
        ));
    }

    Ok(next)
}

/// Chamfers/bevels all shared edges and corners of `mesh` by `amount` (`0.05..=0.45`)
/// while keeping main face centers flat (Doo-Sabin edge/corner bevel).
/// Clears manual skin weights (full rebuild reindexes every vertex).
pub fn bevel_mesh(mesh: &LowPolyMesh, amount: f32) -> Result<LowPolyMesh, CoreError> {
    if !amount.is_finite() || mesh.faces.is_empty() || mesh.faces.len() > 8_000 {
        return Err(CoreError::OutOfBounds);
    }
    let t = amount.clamp(0.05, 0.45);
    let mut new_vertices = Vec::new();
    let mut face_corner_verts: Vec<Vec<usize>> = Vec::with_capacity(mesh.faces.len());
    let mut face_normals: Vec<Vec3> = Vec::with_capacity(mesh.faces.len());
    let mut face_materials: Vec<PaletteIndex> = Vec::with_capacity(mesh.faces.len());

    // Map directed edge (a, b) -> (face_idx, inset_a, inset_b)
    let mut half_edges: BTreeMap<(usize, usize), (usize, usize, usize)> = BTreeMap::new();
    // Map original vertex vi -> Vec<(face_idx, inset_vi)>
    let mut vert_corners: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();

    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = face.vertices.len();
        if n < 3 {
            return Err(CoreError::OutOfBounds);
        }
        let mut centroid = Vec3::ZERO;
        for &vi in &face.vertices {
            centroid += *mesh.vertices.get(vi).ok_or(CoreError::OutOfBounds)?;
        }
        centroid /= n as f32;

        let p0 = mesh.vertices[face.vertices[0]];
        let p1 = mesh.vertices[face.vertices[1]];
        let p2 = mesh.vertices[face.vertices[2]];
        let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        face_normals.push(normal);
        face_materials.push(face.material);

        let mut corners = Vec::with_capacity(n);
        for &vi in &face.vertices {
            let idx = new_vertices.len();
            new_vertices.push(mesh.vertices[vi].lerp(centroid, t));
            corners.push(idx);
            vert_corners.entry(vi).or_default().push((fi, idx));
        }
        for i in 0..n {
            let a = face.vertices[i];
            let b = face.vertices[(i + 1) % n];
            half_edges.insert((a, b), (fi, corners[i], corners[(i + 1) % n]));
        }
        face_corner_verts.push(corners);
    }

    let mut new_faces = Vec::new();
    // 1. Shrunk main faces
    for (fi, corners) in face_corner_verts.iter().enumerate() {
        new_faces.push(Face::new(corners.clone(), face_materials[fi]));
    }

    // 2. Bevel quads along shared edges
    for (&(a, b), &(f1, u1_a, u1_b)) in &half_edges {
        if a < b {
            if let Some(&(_f2, u2_b, u2_a)) = half_edges.get(&(b, a)) {
                new_faces.push(Face::new(vec![u1_b, u1_a, u2_a, u2_b], face_materials[f1]));
            }
        }
    }

    // 3. Corner polygons at vertices shared by >= 3 faces
    for (vi, items) in vert_corners {
        if items.len() < 3 {
            continue;
        }
        let mut avg_n = Vec3::ZERO;
        let mut center = Vec3::ZERO;
        for &(fi, u_idx) in &items {
            avg_n += face_normals[fi];
            center += new_vertices[u_idx];
        }
        center /= items.len() as f32;
        let n_dir = if avg_n.length_squared() > 1e-6 {
            avg_n.normalize()
        } else {
            (mesh.vertices[vi] - center).normalize_or_zero()
        };
        let up = if n_dir.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let t_axis = up.cross(n_dir).normalize_or_zero();
        let b_axis = n_dir.cross(t_axis).normalize_or_zero();

        let mut sorted = items.clone();
        sorted.sort_by(|&(_, idx_a), &(_, idx_b)| {
            let da = new_vertices[idx_a] - center;
            let db = new_vertices[idx_b] - center;
            let ang_a = da.dot(b_axis).atan2(da.dot(t_axis));
            let ang_b = db.dot(b_axis).atan2(db.dot(t_axis));
            ang_a.total_cmp(&ang_b)
        });
        let mat = face_materials[sorted[0].0];
        let loop_verts: Vec<usize> = sorted.into_iter().map(|(_, idx)| idx).collect();
        new_faces.push(Face::new(loop_verts, mat));
    }

    // Full rebuild: vertex indices stop meaning the same corners, so manual
    // skin weights and morphs cannot be carried over and are cleared.
    Ok(LowPolyMesh {
        vertices: new_vertices,
        faces: new_faces,
        texture: mesh.texture.clone(),
        normal_map: mesh.normal_map.clone(),
        ao_map: mesh.ao_map.clone(),
        paint_layers: mesh.paint_layers.clone(),
        active_paint_layer: mesh.active_paint_layer,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

/// Applies one iteration of Catmull-Clark smooth subdivision to `mesh`.
/// Turns sharp corners (e.g. a Cube) into a rounded box and progressively into a smooth sphere.
/// Clears manual skin weights (full rebuild reindexes every vertex).
pub fn smooth_subdivide_mesh(mesh: &LowPolyMesh) -> Result<LowPolyMesh, CoreError> {
    if mesh.faces.is_empty() || mesh.faces.len() > 8_000 {
        return Err(CoreError::OutOfBounds);
    }

    let num_verts = mesh.vertices.len();
    let num_faces = mesh.faces.len();

    // 1. Face points
    let mut face_points = Vec::with_capacity(num_faces);
    for face in &mesh.faces {
        if face.vertices.len() < 3 {
            return Err(CoreError::OutOfBounds);
        }
        let mut sum = Vec3::ZERO;
        for &vi in &face.vertices {
            sum += *mesh.vertices.get(vi).ok_or(CoreError::OutOfBounds)?;
        }
        face_points.push(sum / face.vertices.len() as f32);
    }

    // 2. Canonical edge -> incident faces & vertex adjacency
    let mut edge_faces: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    let mut vert_faces: Vec<Vec<usize>> = vec![Vec::new(); num_verts];
    let mut vert_edges: Vec<Vec<(usize, usize)>> = vec![Vec::new(); num_verts];

    let canon = |a: usize, b: usize| -> (usize, usize) {
        if a < b {
            (a, b)
        } else {
            (b, a)
        }
    };

    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = face.vertices.len();
        for i in 0..n {
            let v0 = face.vertices[i];
            let v1 = face.vertices[(i + 1) % n];
            vert_faces[v0].push(fi);
            let e = canon(v0, v1);
            edge_faces.entry(e).or_default().push(fi);
            if !vert_edges[v0].contains(&e) {
                vert_edges[v0].push(e);
            }
            if !vert_edges[v1].contains(&e) {
                vert_edges[v1].push(e);
            }
        }
    }

    // 3. Allocate new vertex buffer: [0..num_verts] = updated originals, then face points, then edge points
    let mut new_vertices = Vec::with_capacity(num_verts + num_faces + edge_faces.len());
    for vi in 0..num_verts {
        let old_p = mesh.vertices[vi];
        let boundary_edges: Vec<(usize, usize)> = vert_edges[vi]
            .iter()
            .copied()
            .filter(|e| edge_faces.get(e).is_some_and(|fs| fs.len() < 2))
            .collect();

        if !boundary_edges.is_empty() {
            let mut sum = old_p * 2.0;
            for (a, b) in &boundary_edges {
                sum += (mesh.vertices[*a] + mesh.vertices[*b]) * 0.5;
            }
            new_vertices.push(sum / (2.0 + boundary_edges.len() as f32));
        } else if vert_faces[vi].is_empty() {
            new_vertices.push(old_p);
        } else {
            let n = vert_faces[vi].len() as f32;
            let mut avg_f = Vec3::ZERO;
            for &fi in &vert_faces[vi] {
                avg_f += face_points[fi];
            }
            avg_f /= n;

            let mut avg_r = Vec3::ZERO;
            for &(a, b) in &vert_edges[vi] {
                avg_r += (mesh.vertices[a] + mesh.vertices[b]) * 0.5;
            }
            avg_r /= vert_edges[vi].len().max(1) as f32;

            let updated = (avg_f + 2.0 * avg_r + (n - 3.0).max(0.0) * old_p) / n;
            new_vertices.push(updated);
        }
    }

    let face_pt_offset = new_vertices.len();
    new_vertices.extend(face_points.iter().copied());

    let mut edge_pt_index: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (&(a, b), fs) in &edge_faces {
        let mid = (mesh.vertices[a] + mesh.vertices[b]) * 0.5;
        let ep = if fs.len() == 2 {
            (mesh.vertices[a] + mesh.vertices[b] + face_points[fs[0]] + face_points[fs[1]]) * 0.25
        } else {
            mid
        };
        let idx = new_vertices.len();
        new_vertices.push(ep);
        edge_pt_index.insert((a, b), idx);
    }

    // 4. Emit 1 quad per corner of every original face
    let mut new_faces = Vec::with_capacity(num_faces * 4);
    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = face.vertices.len();
        let f_idx = face_pt_offset + fi;
        for i in 0..n {
            let v_curr = face.vertices[i];
            let v_next = face.vertices[(i + 1) % n];
            let v_prev = face.vertices[(i + n - 1) % n];
            let e_next = edge_pt_index[&canon(v_curr, v_next)];
            let e_prev = edge_pt_index[&canon(v_prev, v_curr)];
            new_faces.push(Face::new(
                vec![v_curr, e_next, f_idx, e_prev],
                face.material,
            ));
        }
    }

    // Full rebuild: vertex indices stop meaning the same corners, so manual
    // skin weights and morphs cannot be carried over and are cleared.
    Ok(LowPolyMesh {
        vertices: new_vertices,
        faces: new_faces,
        texture: mesh.texture.clone(),
        normal_map: mesh.normal_map.clone(),
        ao_map: mesh.ao_map.clone(),
        paint_layers: mesh.paint_layers.clone(),
        active_paint_layer: mesh.active_paint_layer,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

/// Converts a `VoxelVolume` into an optimized welded `LowPolyMesh` via Greedy Meshing.
pub fn voxel_to_lowpoly(volume: &VoxelVolume, unit_scale: f32) -> LowPolyMesh {
    let greedy = greedy_mesh_volume(volume, unit_scale);
    let mut weld_map: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut vertices = Vec::new();
    let mut faces = Vec::new();

    let quantize = |p: [f32; 3]| -> [i64; 3] {
        [
            (p[0] * 10_000.0).round() as i64,
            (p[1] * 10_000.0).round() as i64,
            (p[2] * 10_000.0).round() as i64,
        ]
    };

    for tri in greedy.indices.as_chunks::<3>().0 {
        let mut face_verts = Vec::with_capacity(3);
        let mat = greedy.vertices[tri[0] as usize].palette_index;
        for &idx in tri {
            let pos = greedy.vertices[idx as usize].position;
            let key = quantize(pos);
            let vi = *weld_map.entry(key).or_insert_with(|| {
                let id = vertices.len();
                vertices.push(Vec3::from_array(pos));
                id
            });
            face_verts.push(vi);
        }
        faces.push(Face::new(face_verts, mat));
    }

    LowPolyMesh {
        vertices,
        faces,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
    }
}

/// Validates and builds a custom `LowPolyMesh` directly from explicit vertices and faces.
pub fn create_raw_mesh(
    vertices: Vec<Vec3>,
    faces: Vec<Face>,
    smooth_shading: bool,
) -> Result<LowPolyMesh, CoreError> {
    if vertices.is_empty() || faces.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    if vertices.iter().any(|v| !v.is_finite()) {
        return Err(CoreError::OutOfBounds);
    }
    let vlen = vertices.len();
    for face in &faces {
        if face.vertices.len() < 3 {
            return Err(CoreError::OutOfBounds);
        }
        if face.vertices.iter().any(|&vi| vi >= vlen) {
            return Err(CoreError::OutOfBounds);
        }
        if !face.uvs.is_empty()
            && (face.uvs.len() != face.vertices.len()
                || face
                    .uvs
                    .iter()
                    .any(|uv| !uv[0].is_finite() || !uv[1].is_finite()))
        {
            return Err(CoreError::OutOfBounds);
        }
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

/// Updates multiple vertex positions in one atomic operation.
pub fn set_vertices_batch(
    mesh: &LowPolyMesh,
    updates: &[(usize, Vec3)],
) -> Result<LowPolyMesh, CoreError> {
    if updates.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let vlen = mesh.vertices.len();
    for &(vid, pos) in updates {
        if vid >= vlen || !pos.is_finite() {
            return Err(CoreError::OutOfBounds);
        }
    }
    let mut out = mesh.clone();
    for &(vid, pos) in updates {
        out.vertices[vid] = pos;
    }
    Ok(out)
}

/// Assigns `material` to one or more faces on `mesh`.
pub fn set_face_materials(
    mesh: &LowPolyMesh,
    face_ids: &[FaceId],
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if face_ids.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let flen = mesh.faces.len();
    for &fid in face_ids {
        if fid.0 >= flen {
            return Err(CoreError::OutOfBounds);
        }
    }
    let mut out = mesh.clone();
    for &fid in face_ids {
        out.faces[fid.0].material = material;
    }
    Ok(out)
}

/// Mirrors `mesh` across `axis` (`0=X`, `1=Y`, `2=Z`), flipping mirrored face winding so normals
/// remain outward. When `merge_original` is true, appends the mirrored copy to the original mesh.
pub fn mirror_mesh(
    mesh: &LowPolyMesh,
    axis: u8,
    merge_original: bool,
) -> Result<LowPolyMesh, CoreError> {
    if axis > 2 || mesh.vertices.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let flip = |v: Vec3| -> Vec3 {
        match axis {
            0 => Vec3::new(-v.x, v.y, v.z),
            1 => Vec3::new(v.x, -v.y, v.z),
            _ => Vec3::new(v.x, v.y, -v.z),
        }
    };
    let mut out = mesh.clone();
    out.skin_weights.clear();
    out.morphs.clear();
    if !merge_original {
        for v in &mut out.vertices {
            *v = flip(*v);
        }
        for f in &mut out.faces {
            f.vertices.reverse();
            f.uvs.reverse();
        }
        return Ok(out);
    }

    let base_v = out.vertices.len();
    let orig_verts = out.vertices.clone();
    let orig_faces = out.faces.clone();
    out.vertices.extend(orig_verts.into_iter().map(flip));
    for mut f in orig_faces {
        for vi in &mut f.vertices {
            *vi += base_v;
        }
        f.vertices.reverse();
        f.uvs.reverse();
        out.faces.push(f);
    }
    Ok(out)
}

/// Linearly tapers perpendicular axes of `mesh` along `axis` (`0=X`, `1=Y`, `2=Z`)
/// from `start_scale` (at min) to `end_scale` (at max).
pub fn taper_mesh(
    mesh: &LowPolyMesh,
    axis: u8,
    start_scale: f32,
    end_scale: f32,
) -> Result<LowPolyMesh, CoreError> {
    if axis > 2
        || mesh.vertices.is_empty()
        || !start_scale.is_finite()
        || !end_scale.is_finite()
        || start_scale <= 0.0
        || end_scale <= 0.0
    {
        return Err(CoreError::OutOfBounds);
    }
    let (min_b, max_b) = mesh.bounds();
    let center = (min_b + max_b) * 0.5;
    let a = usize::from(axis);
    let span = (max_b[a] - min_b[a]).max(1e-5);
    let mut out = mesh.clone();
    for v in &mut out.vertices {
        let t = ((v[a] - min_b[a]) / span).clamp(0.0, 1.0);
        let s = start_scale + (end_scale - start_scale) * t;
        for k in 0..3 {
            if k != a {
                v[k] = center[k] + (v[k] - center[k]) * s;
            }
        }
    }
    Ok(out)
}

/// Smoothly bends `mesh` along `bend_axis` (`0=X`, `1=Y`, `2=Z`) curving toward `curve_axis`
/// by total angle `angle_rad` around the mesh center.
pub fn bend_mesh(
    mesh: &LowPolyMesh,
    bend_axis: u8,
    curve_axis: u8,
    angle_rad: f32,
) -> Result<LowPolyMesh, CoreError> {
    if bend_axis > 2
        || curve_axis > 2
        || bend_axis == curve_axis
        || mesh.vertices.is_empty()
        || !angle_rad.is_finite()
    {
        return Err(CoreError::OutOfBounds);
    }
    if angle_rad.abs() < 1e-5 {
        return Ok(mesh.clone());
    }
    let (min_b, max_b) = mesh.bounds();
    let center = (min_b + max_b) * 0.5;
    let ba = usize::from(bend_axis);
    let ca = usize::from(curve_axis);
    let span = (max_b[ba] - min_b[ba]).max(1e-5);
    let radius = span / angle_rad;
    let mut out = mesh.clone();
    for v in &mut out.vertices {
        let local_b = v[ba] - center[ba];
        let local_c = v[ca] - center[ca];
        let theta = (local_b / span) * angle_rad;
        let r = radius - local_c;
        v[ba] = center[ba] + r * theta.sin();
        v[ca] = center[ca] + radius - r * theta.cos();
    }
    Ok(out)
}

/// Shifts all vertices by `-origin_local` so the mesh's local origin moves to `origin_local`.
pub fn set_mesh_origin(mesh: &LowPolyMesh, origin_local: Vec3) -> Result<LowPolyMesh, CoreError> {
    if !origin_local.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let mut out = mesh.clone();
    for v in &mut out.vertices {
        *v -= origin_local;
    }
    Ok(out)
}

/// Joins multiple `(mesh, world_matrix)` parts into one unified `LowPolyMesh` in `target_inv` space,
/// preserving per-face materials and manual UVs.
pub fn join_meshes(
    parts: &[(&LowPolyMesh, glam::Mat4)],
    target_inv: glam::Mat4,
) -> Result<LowPolyMesh, CoreError> {
    let Some((first, _)) = parts.first() else {
        return Err(CoreError::OutOfBounds);
    };
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for &(mesh, world_mat) in parts {
        let rel = target_inv * world_mat;
        let det_neg = rel.determinant() < 0.0;
        let base_v = vertices.len();
        for &v in &mesh.vertices {
            vertices.push(rel.transform_point3(v));
        }
        for f in &mesh.faces {
            let mut next = f.clone();
            for vi in &mut next.vertices {
                *vi += base_v;
            }
            if det_neg {
                next.vertices.reverse();
                next.uvs.reverse();
            }
            faces.push(next);
        }
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: first.texture.clone(),
        normal_map: first.normal_map.clone(),
        ao_map: first.ao_map.clone(),
        paint_layers: first.paint_layers.clone(),
        active_paint_layer: first.active_paint_layer,
        vertex_colors: first.vertex_colors.clone(),
        crease_edges: first.crease_edges.clone(),
        proxy_normals: first.proxy_normals.clone(),
        smooth_shading: first.smooth_shading,
        material: first.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}

/// Projects explicit per-corner UVs onto every face of `mesh` using `mode`.
pub fn project_uvs(
    mesh: &LowPolyMesh,
    mode: super::texture::UvProjection,
) -> Result<LowPolyMesh, CoreError> {
    use super::texture::{box_atlas_uv, UvProjection};
    if mesh.vertices.is_empty() || mesh.faces.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let (min_b, max_b) = mesh.bounds();
    let ext = (max_b - min_b).max(Vec3::splat(1e-4));
    let center = (min_b + max_b) * 0.5;
    let mut out = mesh.clone();

    for face in &mut out.faces {
        if face.vertices.len() < 3 {
            continue;
        }
        let p0 = mesh
            .vertices
            .get(face.vertices[0])
            .copied()
            .unwrap_or_default();
        let p1 = mesh
            .vertices
            .get(face.vertices[1])
            .copied()
            .unwrap_or_default();
        let p2 = mesh
            .vertices
            .get(face.vertices[2])
            .copied()
            .unwrap_or_default();
        let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();

        let mut uvs = Vec::with_capacity(face.vertices.len());
        for &vi in &face.vertices {
            let p = mesh.vertices.get(vi).copied().unwrap_or_default();
            let uv = match mode {
                UvProjection::Box => box_atlas_uv(p, normal, min_b, max_b),
                UvProjection::PlanarX => [
                    ((p.z - min_b.z) / ext.z).clamp(0.0, 1.0),
                    (1.0 - (p.y - min_b.y) / ext.y).clamp(0.0, 1.0),
                ],
                UvProjection::PlanarY => [
                    ((p.x - min_b.x) / ext.x).clamp(0.0, 1.0),
                    ((p.z - min_b.z) / ext.z).clamp(0.0, 1.0),
                ],
                UvProjection::PlanarZ => [
                    ((p.x - min_b.x) / ext.x).clamp(0.0, 1.0),
                    (1.0 - (p.y - min_b.y) / ext.y).clamp(0.0, 1.0),
                ],
                UvProjection::CylindricalY => {
                    let rel = p - center;
                    let u = (rel.x.atan2(rel.z) / std::f32::consts::TAU + 0.5).clamp(0.0, 1.0);
                    let v = (1.0 - (p.y - min_b.y) / ext.y).clamp(0.0, 1.0);
                    [u, v]
                }
                UvProjection::Spherical => {
                    let dir = (p - center).normalize_or_zero();
                    let u = (dir.x.atan2(dir.z) / std::f32::consts::TAU + 0.5).clamp(0.0, 1.0);
                    let v = (dir.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI).clamp(0.0, 1.0);
                    [u, v]
                }
            };
            uvs.push(uv);
        }
        face.uvs = uvs;
    }
    Ok(out)
}

/// Locally subdivides a single face into `N` quads around its centroid and edge midpoints
/// without subdividing the rest of the mesh (ideal for local joint/face topology density).
pub fn subdivide_face(mesh: &LowPolyMesh, face_id: FaceId) -> Result<LowPolyMesh, CoreError> {
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    let n = face.vertices.len();
    if n < 3 {
        return Err(CoreError::OutOfBounds);
    }
    let mut out = mesh.clone();
    out.skin_weights.clear();
    out.morphs.clear();

    let mut centroid = Vec3::ZERO;
    for &vi in &face.vertices {
        centroid += *out.vertices.get(vi).ok_or(CoreError::OutOfBounds)?;
    }
    centroid /= n as f32;
    let c_idx = out.vertices.len();
    out.vertices.push(centroid);

    let mut mid_indices = Vec::with_capacity(n);
    for i in 0..n {
        let a = out.vertices[face.vertices[i]];
        let b = out.vertices[face.vertices[(i + 1) % n]];
        let m_idx = out.vertices.len();
        out.vertices.push((a + b) * 0.5);
        mid_indices.push(m_idx);
    }

    let has_uvs = face.uvs.len() == n;
    let c_uv = if has_uvs {
        let sum = face
            .uvs
            .iter()
            .fold([0.0f32, 0.0f32], |acc, uv| [acc[0] + uv[0], acc[1] + uv[1]]);
        [sum[0] / n as f32, sum[1] / n as f32]
    } else {
        [0.0, 0.0]
    };

    let mat = face.material;
    let orig_verts = face.vertices.clone();
    let orig_uvs = face.uvs.clone();

    let mut new_faces = Vec::with_capacity(n);
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let mut q = Face::new(
            vec![orig_verts[i], mid_indices[i], c_idx, mid_indices[prev]],
            mat,
        );
        if has_uvs {
            let uv_i = orig_uvs[i];
            let uv_next = orig_uvs[(i + 1) % n];
            let uv_prev = orig_uvs[prev];
            q.uvs = vec![
                uv_i,
                [(uv_i[0] + uv_next[0]) * 0.5, (uv_i[1] + uv_next[1]) * 0.5],
                c_uv,
                [(uv_prev[0] + uv_i[0]) * 0.5, (uv_prev[1] + uv_i[1]) * 0.5],
            ];
        }
        new_faces.push(q);
    }

    out.faces.splice(face_id.0..=face_id.0, new_faces);
    Ok(out)
}

/// Bakes a node's local `Transform` (`translation`, `rotation`, `scale`) directly into the mesh
/// vertices so the node's transform can be reset to identity (standard "Apply Transform").
pub fn apply_transform(
    mesh: &LowPolyMesh,
    transform: &crate::scene::Transform,
) -> Result<LowPolyMesh, CoreError> {
    if !transform.translation.is_finite()
        || !transform.rotation.is_finite()
        || !transform.scale.is_finite()
    {
        return Err(CoreError::OutOfBounds);
    }
    let mat = glam::Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    );
    let det_neg = mat.determinant() < 0.0;
    let mut out = mesh.clone();
    for v in &mut out.vertices {
        *v = mat.transform_point3(*v);
    }
    if det_neg {
        for f in &mut out.faces {
            f.vertices.reverse();
            f.uvs.reverse();
        }
    }
    Ok(out)
}

/// Welds coincident vertices within `tolerance` (`1e-5..=0.5`), remapping all face indices
/// (ideal for removing center-seam creases after `mesh_mirror` or `mesh_join`).
/// Returns `(welded_mesh, removed_vertex_count)`.
pub fn weld_vertices(
    mesh: &LowPolyMesh,
    tolerance: f32,
) -> Result<(LowPolyMesh, usize), CoreError> {
    if !tolerance.is_finite() || tolerance <= 0.0 || mesh.vertices.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let tol = tolerance.clamp(1e-5, 0.5);
    let inv = 1.0 / tol;
    let mut cell_map: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut new_verts: Vec<Vec3> = Vec::with_capacity(mesh.vertices.len());
    let mut remap: Vec<usize> = Vec::with_capacity(mesh.vertices.len());

    for &v in &mesh.vertices {
        let key = [
            (v.x * inv).round() as i64,
            (v.y * inv).round() as i64,
            (v.z * inv).round() as i64,
        ];
        let idx = *cell_map.entry(key).or_insert_with(|| {
            let id = new_verts.len();
            new_verts.push(v);
            id
        });
        remap.push(idx);
    }

    let removed = mesh.vertices.len().saturating_sub(new_verts.len());
    let mut new_faces = Vec::with_capacity(mesh.faces.len());
    for f in &mesh.faces {
        let mut verts = Vec::with_capacity(f.vertices.len());
        let mut uvs = Vec::new();
        let has_uvs = f.uvs.len() == f.vertices.len();
        for (i, &vi) in f.vertices.iter().enumerate() {
            let mapped = *remap.get(vi).ok_or(CoreError::OutOfBounds)?;
            if verts.last() != Some(&mapped) {
                verts.push(mapped);
                if has_uvs {
                    uvs.push(f.uvs[i]);
                }
            }
        }
        if verts.len() > 1 && verts.first() == verts.last() {
            verts.pop();
            if has_uvs {
                uvs.pop();
            }
        }
        if verts.len() >= 3 {
            let mut next = Face::new(verts, f.material);
            if has_uvs && uvs.len() == next.vertices.len() {
                next.uvs = uvs;
            }
            new_faces.push(next);
        }
    }

    let mut out = mesh.clone();
    out.vertices = new_verts;
    out.faces = new_faces;
    if removed > 0 {
        out.skin_weights.clear();
        out.morphs.clear();
        out.vertex_colors.clear();
        out.crease_edges.clear();
        out.proxy_normals.clear();
    }
    Ok((out, removed))
}

/// Reverses face winding (flipping outward normals) on all faces or on explicit `face_ids`.
pub fn flip_normals(
    mesh: &LowPolyMesh,
    face_ids: Option<&[FaceId]>,
) -> Result<LowPolyMesh, CoreError> {
    if mesh.faces.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    let mut out = mesh.clone();
    if let Some(ids) = face_ids {
        if ids.is_empty() {
            return Err(CoreError::OutOfBounds);
        }
        for &fid in ids {
            if fid.0 >= out.faces.len() {
                return Err(CoreError::OutOfBounds);
            }
        }
        for &fid in ids {
            out.faces[fid.0].vertices.reverse();
            out.faces[fid.0].uvs.reverse();
        }
    } else {
        for f in &mut out.faces {
            f.vertices.reverse();
            f.uvs.reverse();
        }
    }
    Ok(out)
}

/// Detailed raycast hit on a `LowPolyMesh`, including the auto-UV coordinate for 3D texture painting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshRayHit {
    pub face_id: FaceId,
    pub point: Vec3,
    pub normal: Vec3,
    pub uv: [f32; 2],
}

/// Möller–Trumbore raycast returning closest hit face, 3D point, normal, and box-atlas UV.
pub fn raycast_mesh_hit(mesh: &LowPolyMesh, ray_origin: Vec3, ray_dir: Vec3) -> Option<MeshRayHit> {
    let dir = ray_dir.normalize_or_zero();
    if dir.length_squared() < 1e-6 {
        return None;
    }

    let (min_b, max_b) = mesh.bounds();
    let mut closest_t = f32::INFINITY;
    let mut best: Option<MeshRayHit> = None;

    for (face_idx, face) in mesh.faces.iter().enumerate() {
        if face.vertices.len() < 3 {
            continue;
        }
        let Some(&p0) = mesh.vertices.get(face.vertices[0]) else {
            continue;
        };
        for fan in 1..(face.vertices.len() - 1) {
            let (Some(&p1), Some(&p2)) = (
                mesh.vertices.get(face.vertices[fan]),
                mesh.vertices.get(face.vertices[fan + 1]),
            ) else {
                continue;
            };
            if let Some(t) = ray_triangle_intersect(ray_origin, dir, p0, p1, p2) {
                if t < closest_t {
                    closest_t = t;
                    let point = ray_origin + dir * t;
                    let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                    let uv = super::texture::box_atlas_uv(point, normal, min_b, max_b);
                    best = Some(MeshRayHit {
                        face_id: FaceId(face_idx),
                        point,
                        normal,
                        uv,
                    });
                }
            }
        }
    }

    best
}

/// Möller–Trumbore raycast picking returning the closest hit `FaceId` on `mesh`.
pub fn raycast_mesh_face(mesh: &LowPolyMesh, ray_origin: Vec3, ray_dir: Vec3) -> Option<FaceId> {
    raycast_mesh_hit(mesh, ray_origin, ray_dir).map(|h| h.face_id)
}

fn ray_triangle_intersect(orig: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let eps = 1e-6;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < eps {
        return None;
    }
    let f = 1.0 / a;
    let s = orig - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    (t > eps).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::VoxelCoord;

    #[test]
    fn cube_extrude_adds_four_side_faces() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        assert_eq!(cube.faces.len(), 6);
        let extruded = extrude_face(&cube, FaceId(4), 0.5).unwrap();
        assert_eq!(extruded.faces.len(), 10);
        assert_eq!(extruded.to_triangulated_flat().indices.len(), 60);
    }

    #[test]
    fn voxel_to_lowpoly_welds_shared_vertices() {
        let mut vol = VoxelVolume::new();
        let c = PaletteIndex::new(4).unwrap();
        vol.set(VoxelCoord::new(0, 0, 0), Some(c));
        let mesh = voxel_to_lowpoly(&vol, 0.1);
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn raycast_picks_front_face_of_cube() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let hit = raycast_mesh_face(&cube, Vec3::new(0.0, 0.5, 3.0), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(hit, Some(FaceId(0))); // +Z front face
    }

    #[test]
    fn lathe_dome_has_outward_normals_and_pole_collapse() {
        let mat = PaletteIndex::new(3).unwrap();
        let profile = [[0.5, 0.0], [0.5, 0.3], [0.35, 0.55], [0.0, 0.65]];
        let dome = create_lathe_mesh(&profile, 12, 360.0, true, mat).unwrap();
        // 3 rings of 12 + 1 collapsed top pole = 37 vertices
        assert_eq!(dome.vertices.len(), 37);
        // 2 quad bands (24) + 1 top cap band of tris (12) + 1 bottom cap (1) = 37 faces
        assert_eq!(dome.faces.len(), 37);
        let tri = dome.to_triangulated_flat();
        assert!(tri.vertices[0].normal[2] > 0.5);
    }

    #[test]
    fn lathe_partial_arc_visor_centers_on_front_z() {
        let mat = PaletteIndex::new(5).unwrap();
        let profile = [[0.4, 0.0], [0.4, 0.25]];
        let visor = create_lathe_mesh(&profile, 8, 180.0, true, mat).unwrap();
        assert_eq!(visor.faces.len(), 8);
        let (lo, hi) = visor.bounds();
        assert!(hi.z > 0.35 && lo.z >= -1e-4, "180° visor faces +Z front");
    }

    #[test]
    fn extruded_polygon_normalizes_cw_input_and_caps_both_sides() {
        let mat = PaletteIndex::new(2).unwrap();
        // Deliberately clockwise square
        let cw = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let mesh = create_extruded_polygon(&cw, 0.2, mat).unwrap();
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.faces.len(), 8); // 2 front tris + 2 back tris + 4 side quads
        let tri = mesh.to_triangulated_flat();
        // Front cap normal must point +Z
        assert!(tri.vertices[0].normal[2] > 0.99);
        // Reject degenerate collinear polygon
        assert!(create_extruded_polygon(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0.2, mat).is_err());
    }

    #[test]
    fn extruded_concave_polygon_ear_clips_without_bridging_concavity() {
        let mat = PaletteIndex::new(3).unwrap();
        // Concave C-shape polygon (CCW) with a deep notch at x in [0.4, 1.0], y in [0.3, 0.7]
        let c_shape = [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.3],
            [0.4, 0.3],
            [0.4, 0.7],
            [1.0, 0.7],
            [1.0, 1.0],
            [0.0, 1.0],
        ];
        let mesh = create_extruded_polygon(&c_shape, 0.2, mat).unwrap();
        let n = c_shape.len();
        assert_eq!(mesh.vertices.len(), n * 2);
        // Expected area of C-shape = 1.0 - (0.6 * 0.4) = 0.76
        let front_cap_area: f32 = mesh
            .faces
            .iter()
            .take(n - 2)
            .map(|f| {
                let a = mesh.vertices[f.vertices[0]];
                let b = mesh.vertices[f.vertices[1]];
                let c = mesh.vertices[f.vertices[2]];
                let area_z = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
                assert!(area_z > 0.0, "every front cap triangle must be CCW (+Z)");
                0.5 * area_z
            })
            .sum();
        assert!(
            (front_cap_area - 0.76).abs() < 1e-4,
            "front cap area {front_cap_area} must equal concave C-shape area 0.76"
        );
        // Verify no front cap triangle covers the concave notch center (0.75, 0.5)
        let notch = [0.75, 0.5];
        for f in mesh.faces.iter().take(n - 2) {
            let a = [
                mesh.vertices[f.vertices[0]].x,
                mesh.vertices[f.vertices[0]].y,
            ];
            let b = [
                mesh.vertices[f.vertices[1]].x,
                mesh.vertices[f.vertices[1]].y,
            ];
            let c = [
                mesh.vertices[f.vertices[2]].x,
                mesh.vertices[f.vertices[2]].y,
            ];
            assert!(
                !point_in_ccw_triangle_2d(notch, a, b, c, 1e-6),
                "ear-clipped cap triangle must not bridge concave notch"
            );
        }

        // Reject self-intersecting bowtie polygon with nonzero net area
        let self_intersecting = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.8]];
        assert!(create_extruded_polygon(&self_intersecting, 0.2, mat).is_err());

        // Sub-millimeter concave polygon (scale 0.0005) still triangulates with exact area
        let tiny_scale = 0.0005f32;
        let tiny_c: Vec<[f32; 2]> = c_shape
            .iter()
            .map(|&[x, y]| [x * tiny_scale, y * tiny_scale])
            .collect();
        let tiny_mesh = create_extruded_polygon(&tiny_c, 0.01, mat).unwrap();
        let tiny_area: f32 = tiny_mesh
            .faces
            .iter()
            .take(n - 2)
            .map(|f| {
                let a = tiny_mesh.vertices[f.vertices[0]];
                let b = tiny_mesh.vertices[f.vertices[1]];
                let c = tiny_mesh.vertices[f.vertices[2]];
                0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x))
            })
            .sum();
        let expected_tiny = 0.76 * tiny_scale * tiny_scale;
        assert!(
            (tiny_area - expected_tiny).abs() < expected_tiny * 1e-3,
            "tiny concave area {tiny_area} must match {expected_tiny}"
        );
    }

    #[test]
    fn loft_mesh_connects_tapered_rings_with_outward_normals() {
        let mat = PaletteIndex::new(4).unwrap();
        let ring = |y: f32, r: f32| {
            vec![
                Vec3::new(0.0, y, r),
                Vec3::new(r, y, 0.0),
                Vec3::new(0.0, y, -r),
                Vec3::new(-r, y, 0.0),
            ]
        };
        let mesh = create_loft_mesh(
            &[ring(0.0, 0.5), ring(0.5, 0.35), ring(1.0, 0.2)],
            true,
            mat,
        )
        .unwrap();
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(mesh.faces.len(), 10); // 8 side quads + 2 caps
        let tri = mesh.to_triangulated_flat();
        assert!(tri.vertices[0].normal[2] > 0.4 && tri.vertices[0].normal[0] > 0.4);
    }

    #[test]
    fn smooth_subdivide_turns_cube_into_rounded_mesh_and_sphere() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let round1 = smooth_subdivide_mesh(&cube).unwrap();
        assert_eq!(round1.faces.len(), 24);
        assert_eq!(round1.vertices.len(), 26);

        let round2 = smooth_subdivide_mesh(&round1).unwrap();
        assert_eq!(round2.faces.len(), 96);
        assert_eq!(round2.to_triangulated_flat().indices.len(), 96 * 6);
    }

    #[test]
    fn smooth_shading_averages_vertex_normals_across_shared_corners() {
        let mat = PaletteIndex::new(4).unwrap();
        let mut cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let flat_n = cube.to_triangulated_flat().vertices[0].normal;
        cube.smooth_shading = true;
        let smooth_n = cube.to_triangulated_flat().vertices[0].normal;
        // Flat normal is axis-aligned [0, 0, 1]; smooth corner normal averages all 3 adjacent faces.
        assert!((flat_n[0]).abs() < 1e-4 && (flat_n[1]).abs() < 1e-4);
        assert!(smooth_n[0].abs() > 0.3 && smooth_n[1].abs() > 0.3 && smooth_n[2].abs() > 0.3);
    }

    #[test]
    fn translate_face_moves_selected_face_vertices() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let before_z = cube.vertices[cube.faces[0].vertices[0]].z;
        let moved = translate_face(&cube, FaceId(0), Vec3::new(0.0, 0.0, 0.5)).unwrap();
        let after_z = moved.vertices[moved.faces[0].vertices[0]].z;
        assert!((after_z - before_z - 0.5).abs() < 1e-5);
    }

    #[test]
    fn primitives_sphere_torus_capsule_have_valid_geometry() {
        let mat = PaletteIndex::new(4).unwrap();
        for kind in [
            PrimitiveKind::Sphere,
            PrimitiveKind::Torus,
            PrimitiveKind::Capsule,
        ] {
            let mesh = create_primitive(kind, 1.0, mat);
            assert!(!mesh.vertices.is_empty());
            assert!(!mesh.faces.is_empty());
            assert!(mesh.smooth_shading);
            assert!(mesh.triangle_count() > 20);
        }
    }

    #[test]
    fn bevel_mesh_chamfers_cube_edges_and_corners() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let beveled = bevel_mesh(&cube, 0.2).unwrap();
        // 6 main faces + 12 edge chamfer quads + 8 corner triangles = 26 faces
        assert_eq!(beveled.faces.len(), 26);
        assert_eq!(beveled.vertices.len(), 24);
    }

    #[test]
    fn curve_tube_and_decimate_produce_valid_lowpoly_meshes() {
        let mat = PaletteIndex::new(4).unwrap();
        let strap = create_curve_tube(
            &[
                Vec3::new(0.0, 0.2, 0.0),
                Vec3::new(0.2, 0.6, 0.1),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            0.08,
            0.3,
            mat,
        )
        .unwrap();
        assert!(strap.faces.len() > 20);
        let decimated = decimate_mesh(&strap, 0.5).unwrap();
        assert!(!decimated.faces.is_empty());
        assert!(decimated.vertices.len() < strap.vertices.len());
    }

    #[test]
    fn move_vertices_shifts_only_listed_corners() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let untouched = cube.vertices[1];
        let moved = move_mesh_vertices(&cube, &[0], Vec3::new(0.25, 0.0, 0.0)).unwrap();
        assert!((moved.vertices[0].x - (cube.vertices[0].x + 0.25)).abs() < 1e-6);
        assert_eq!(moved.vertices[1], untouched);
    }

    #[test]
    fn move_vertices_rejects_bad_input() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        assert!(move_mesh_vertices(&cube, &[], Vec3::ONE).is_err());
        assert!(move_mesh_vertices(&cube, &[999], Vec3::ONE).is_err());
        assert!(move_mesh_vertices(&cube, &[0], Vec3::new(f32::NAN, 0.0, 0.0)).is_err());
    }

    #[test]
    fn set_mesh_vertex_places_corner_absolutely() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let target = Vec3::new(0.1, 0.2, 0.3);
        let moved = set_mesh_vertex(&cube, 2, target).unwrap();
        assert_eq!(moved.vertices[2], target);
        assert!(set_mesh_vertex(&cube, 999, target).is_err());
        assert!(set_mesh_vertex(&cube, 0, Vec3::new(f32::INFINITY, 0.0, 0.0)).is_err());
    }

    #[test]
    fn set_face_uvs_stores_manual_unwrap() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let unwrapped = set_face_uvs(&cube, FaceId(0), &uvs).unwrap();
        assert_eq!(unwrapped.faces[0].uvs, uvs);
        // Out-of-range values clamp into UV space instead of failing.
        let clamped = set_face_uvs(
            &cube,
            FaceId(0),
            &[[0.0, 0.0], [2.0, 0.0], [1.0, 1.0], [0.0, -1.0]],
        )
        .unwrap();
        assert_eq!(clamped.faces[0].uvs[1], [1.0, 0.0]);
        assert_eq!(clamped.faces[0].uvs[3], [0.0, 0.0]);
    }

    #[test]
    fn set_face_uvs_rejects_bad_input() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        assert!(set_face_uvs(&cube, FaceId(99), &[[0.0, 0.0]; 4]).is_err());
        assert!(set_face_uvs(&cube, FaceId(0), &[[0.0, 0.0]; 3]).is_err());
        assert!(set_face_uvs(&cube, FaceId(0), &[[f32::NAN, 0.0]; 4]).is_err());
    }

    #[test]
    fn set_vertex_weights_normalizes_and_rejects_bad_input() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let painted =
            set_vertex_weights(&cube, &[0, 1], [2, 3, 0, 0], [3.0, 1.0, 0.0, 0.0]).unwrap();
        let w = painted.skin_weights[&0];
        assert_eq!(w.joints, [2, 3, 0, 0]);
        assert!((w.weights[0] - 0.75).abs() < 1e-6);
        assert!((w.weights[1] - 0.25).abs() < 1e-6);
        assert!((w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(painted.skin_weights[&1], w);
        assert!(!painted.skin_weights.contains_key(&2));

        assert!(set_vertex_weights(&cube, &[], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).is_err());
        assert!(set_vertex_weights(&cube, &[99], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).is_err());
        assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [0.0, 0.0, 0.0, 0.0]).is_err());
        assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [-1.0, 1.0, 0.0, 0.0]).is_err());
        assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [f32::NAN, 0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn clear_vertex_weights_clears_all_or_listed() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let painted =
            set_vertex_weights(&cube, &[0, 1, 2], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
        let partial = clear_vertex_weights(&painted, Some(&[0])).unwrap();
        assert!(!partial.skin_weights.contains_key(&0));
        assert!(partial.skin_weights.contains_key(&1));
        let cleared = clear_vertex_weights(&painted, None).unwrap();
        assert!(cleared.skin_weights.is_empty());
        assert!(clear_vertex_weights(&painted, Some(&[99])).is_err());
    }

    #[test]
    fn morph_deltas_accumulate_and_clear() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let m1 = add_morph_delta(&cube, "Smile", &[0, 1], Vec3::new(0.0, 0.1, 0.0)).unwrap();
        assert_eq!(m1.morphs.len(), 1);
        assert_eq!(m1.morphs[0].name, "Smile");
        let m2 = add_morph_delta(&m1, "Smile", &[0], Vec3::new(0.0, 0.1, 0.0)).unwrap();
        assert_eq!(m2.morphs[0].deltas[&0], Vec3::new(0.0, 0.2, 0.0));
        assert_eq!(m2.morphs[0].deltas[&1], Vec3::new(0.0, 0.1, 0.0));
        // Cancelling deltas prune the vertex; empty targets are dropped.
        let m3 = add_morph_delta(&m2, "Smile", &[0, 1], Vec3::new(0.0, -0.1, 0.0)).unwrap();
        assert_eq!(m3.morphs.len(), 1);
        assert!(!m3.morphs[0].deltas.contains_key(&1));
        let cleared = clear_morphs(&m3, Some("Smile")).unwrap();
        assert!(cleared.morphs.is_empty());
        // Missing names are a no-op; None clears all.
        let kept = clear_morphs(&m3, Some("Frown")).unwrap();
        assert_eq!(kept.morphs.len(), 1);
        let all = clear_morphs(&m3, None).unwrap();
        assert!(all.morphs.is_empty());

        assert!(add_morph_delta(&cube, "", &[0], Vec3::ONE).is_err());
        assert!(add_morph_delta(&cube, "X", &[], Vec3::ONE).is_err());
        assert!(add_morph_delta(&cube, "X", &[99], Vec3::ONE).is_err());
        assert!(add_morph_delta(&cube, "X", &[0], Vec3::new(f32::NAN, 0.0, 0.0)).is_err());
    }

    #[test]
    fn morphs_survive_move_but_not_rebuild() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let morphed = add_morph_delta(&cube, "Smile", &[0], Vec3::new(0.0, 0.1, 0.0)).unwrap();
        let moved = move_mesh_vertices(&morphed, &[0], Vec3::new(0.1, 0.0, 0.0)).unwrap();
        assert_eq!(moved.morphs.len(), 1);
        let beveled = bevel_mesh(&morphed, 0.2).unwrap();
        assert!(beveled.morphs.is_empty());
    }

    #[test]
    fn skin_weights_survive_move_but_not_rebuild() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let painted = set_vertex_weights(&cube, &[0], [5, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
        let moved = move_mesh_vertices(&painted, &[0], Vec3::new(0.1, 0.0, 0.0)).unwrap();
        assert_eq!(moved.skin_weights[&0].joints, [5, 0, 0, 0]);
        let beveled = bevel_mesh(&painted, 0.2).unwrap();
        assert!(beveled.skin_weights.is_empty());
    }

    #[test]
    fn weight_paint_brush_blends_smoothly_and_normalizes() {
        let mat = PaletteIndex::new(4).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        // Seed vertex 0 with joint 1 weight 1.0
        let seeded = set_vertex_weights(&cube, &[0], [1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
        let v0 = seeded.vertices[0];

        // Paint joint 3 directly on vertex 0 with strength 0.5 -> expect 50/50 blend between joint 1 and joint 3
        let painted = paint_vertex_weights_brush(&seeded, v0, 0.25, 3, 0.5, None)
            .unwrap()
            .expect("vertex 0 must be within radius");
        let w = painted.skin_weights[&0];
        let sum: f32 = w.weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "weights must normalize to 1.0");
        assert!(w.joints.contains(&1) && w.joints.contains(&3));

        // Hit point far outside mesh returns Ok(None)
        let missed =
            paint_vertex_weights_brush(&seeded, Vec3::new(10.0, 10.0, 10.0), 0.25, 3, 0.5, None)
                .unwrap();
        assert!(missed.is_none());

        // Invalid radius/strength errors cleanly
        assert!(paint_vertex_weights_brush(&seeded, v0, 0.0, 3, 0.5, None).is_err());
        assert!(paint_vertex_weights_brush(&seeded, v0, 0.25, 3, 1.5, None).is_err());

        // Duplicate base joints are merged and NaN vertices are skipped safely
        let mut dup_mesh =
            set_vertex_weights(&cube, &[0], [1, 1, 2, 0], [0.4, 0.4, 0.2, 0.0]).unwrap();
        dup_mesh.vertices[1] = Vec3::new(f32::NAN, 0.0, 0.0);
        let dup_painted = paint_vertex_weights_brush(&dup_mesh, v0, 0.25, 3, 0.5, None)
            .unwrap()
            .unwrap();
        let dw = dup_painted.skin_weights[&0];
        assert_eq!(dw.joints[0], 3);
        assert_eq!(dw.joints[1], 1);
        assert_eq!(dw.joints[2], 2);
        assert!((dw.weights[0] - 0.5).abs() < 1e-4);
        assert!((dw.weights[1] - 0.4).abs() < 1e-4);
        assert!((dw.weights[2] - 0.1).abs() < 1e-4);
        assert!(!dup_painted.skin_weights.contains_key(&1));
    }
}
