use std::collections::BTreeMap;

use glam::Vec3;

use crate::error::CoreError;
use crate::palette::PaletteIndex;
use crate::voxel::{greedy_mesh_volume, VoxelVolume};

use super::face::Face;
use super::lowpoly::LowPolyMesh;
use super::nano::{FaceId, PrimitiveKind};

/// Generates a starter low-poly primitive mesh centered on X/Z with base at Y = 0.
pub fn create_primitive(kind: PrimitiveKind, size: f32, material: PaletteIndex) -> LowPolyMesh {
    let s = size.max(0.1);
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
                smooth_shading: false,
                material: Default::default(),
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
                smooth_shading: false,
                material: Default::default(),
            }
        }
        PrimitiveKind::Cylinder => create_cylinder(8, h, s, material),
        PrimitiveKind::Cone => create_cone(8, h, s, material),
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
        smooth_shading: false,
        material: Default::default(),
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
        smooth_shading: false,
        material: Default::default(),
    }
}

/// Extrudes `face_id` along its face normal by `distance`.
pub fn extrude_face(
    mesh: &LowPolyMesh,
    face_id: FaceId,
    distance: f32,
) -> Result<LowPolyMesh, CoreError> {
    let face = mesh.faces.get(face_id.0).ok_or(CoreError::OutOfBounds)?;
    if face.vertices.len() < 3 {
        return Err(CoreError::OutOfBounds);
    }
    let p0 = mesh.vertices[face.vertices[0]];
    let p1 = mesh.vertices[face.vertices[1]];
    let p2 = mesh.vertices[face.vertices[2]];
    let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();

    let mut next = mesh.clone();
    let old_verts = face.vertices.clone();
    let material = face.material;
    let mut new_verts = Vec::with_capacity(old_verts.len());

    for &vi in &old_verts {
        let idx = next.vertices.len();
        next.vertices.push(next.vertices[vi] + normal * distance);
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

/// Applies one iteration of Catmull-Clark smooth subdivision to `mesh`.
/// Turns sharp corners (e.g. a Cube) into a rounded box and progressively into a smooth sphere.
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

    Ok(LowPolyMesh {
        vertices: new_vertices,
        faces: new_faces,
        texture: mesh.texture.clone(),
        normal_map: mesh.normal_map.clone(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
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
        texture: None,
        normal_map: None,
        smooth_shading: false,
        material: Default::default(),
    }
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
}
