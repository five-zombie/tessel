use std::collections::BTreeMap;

use glam::Vec3;

use crate::error::CoreError;
use crate::palette::PaletteIndex;

use super::super::face::Face;
use super::super::lowpoly::LowPolyMesh;
use super::super::nano::FaceId;

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
        modifiers: Vec::new(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
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
        modifiers: Vec::new(),
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
        modifiers: Vec::new(),
        smooth_shading: mesh.smooth_shading,
        material: mesh.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
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
        modifiers: Vec::new(),
        smooth_shading: first.smooth_shading,
        material: first.material,
        skin_weights: BTreeMap::new(),
        morphs: Default::default(),
    })
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
                    let uv = super::super::texture::box_atlas_uv(point, normal, min_b, max_b);
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
