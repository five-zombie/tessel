use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use crate::character::VertexSkinWeights;
use crate::error::CoreError;
use crate::palette::PaletteIndex;

use super::super::lowpoly::{LowPolyMesh, MorphTarget};
use super::super::nano::FaceId;
use super::super::texture;

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

/// Paints manual skin weights onto explicit vertices (weight paint). `joints`
/// holds up to 4 bone indices and `weights` their influences; weights are
/// normalized to sum 1. At least one weight must be positive. Export prefers
/// these over the auto solver. Survives index-stable edits; cleared by
/// bevel/subdivide/decimate.
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

/// Projects explicit per-corner UVs onto every face of `mesh` using `mode`.
pub fn project_uvs(
    mesh: &LowPolyMesh,
    mode: texture::UvProjection,
) -> Result<LowPolyMesh, CoreError> {
    use texture::{box_atlas_uv, UvProjection};
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
