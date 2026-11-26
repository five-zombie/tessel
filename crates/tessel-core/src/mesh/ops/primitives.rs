use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::palette::PaletteIndex;
use crate::voxel::{greedy_mesh_volume, VoxelVolume};

use super::super::face::Face;
use super::super::lowpoly::LowPolyMesh;
use super::super::nano::{PrimitiveAnchor, PrimitiveKind};

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
        modifiers: Vec::new(),
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
                modifiers: Vec::new(),
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
                modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
        smooth_shading: false,
        material: Default::default(),
    })
}

fn cross_2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub(super) fn point_in_ccw_triangle_2d(
    p: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    eps: f32,
) -> bool {
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
        modifiers: Vec::new(),
        smooth_shading: false,
        material: Default::default(),
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
        smooth_shading,
        material: Default::default(),
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
}
