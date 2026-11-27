use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::{Vec2, Vec3};

use super::lowpoly::LowPolyMesh;
use super::texture::box_atlas_uv;

/// Summary returned by [`normalize_texel_density`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexelDensityReport {
    pub island_count: usize,
    pub average_texel_density: f32,
    pub uniform_scale: f32,
}

/// Canonicalizes an undirected mesh edge `(a, b)` into `(min(a, b), max(a, b))`.
pub fn canonical_edge(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn face_normal(mesh: &LowPolyMesh, fi: usize) -> Vec3 {
    let face = &mesh.faces[fi];
    if face.vertices.len() < 3 {
        return Vec3::Y;
    }
    let (Some(&v0), Some(&v1), Some(&v2)) = (
        mesh.vertices.get(face.vertices[0]),
        mesh.vertices.get(face.vertices[1]),
        mesh.vertices.get(face.vertices[2]),
    ) else {
        return Vec3::Y;
    };
    let n = (v1 - v0).cross(v2 - v0);
    if n.length_squared() > 1e-12 {
        n.normalize()
    } else {
        Vec3::Y
    }
}

/// Collects face-connected UV islands separated by `mesh.uv_seams` (and optionally UV discontinuities).
fn collect_uv_islands(mesh: &LowPolyMesh, respect_uv_splits: bool) -> Vec<Vec<usize>> {
    let seam_set: BTreeSet<(usize, usize)> = mesh
        .uv_seams
        .iter()
        .map(|&(a, b)| canonical_edge(a, b))
        .collect();

    type EdgeFaceMap = BTreeMap<(usize, usize), Vec<(usize, usize, usize)>>;
    let mut edge_to_faces: EdgeFaceMap = BTreeMap::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = face.vertices.len();
        if n < 3 {
            continue;
        }
        for k in 0..n {
            let va = face.vertices[k];
            let vb = face.vertices[(k + 1) % n];
            let edge = canonical_edge(va, vb);
            if seam_set.contains(&edge) {
                continue;
            }
            edge_to_faces
                .entry(edge)
                .or_default()
                .push((fi, k, (k + 1) % n));
        }
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); mesh.faces.len()];
    for incident in edge_to_faces.values() {
        if incident.len() < 2 {
            continue;
        }
        for i in 0..incident.len() {
            for j in (i + 1)..incident.len() {
                let (f1, k1a, k1b) = incident[i];
                let (f2, k2a, k2b) = incident[j];
                if f1 == f2 {
                    continue;
                }
                let connected = if !respect_uv_splits {
                    true
                } else {
                    let u1 = &mesh.faces[f1].uvs;
                    let u2 = &mesh.faces[f2].uvs;
                    if k1a < u1.len() && k1b < u1.len() && k2a < u2.len() && k2b < u2.len() {
                        let a1 = Vec2::from_array(u1[k1a]);
                        let b1 = Vec2::from_array(u1[k1b]);
                        let a2 = Vec2::from_array(u2[k2a]);
                        let b2 = Vec2::from_array(u2[k2b]);
                        (a1.distance(a2) < 1e-2 && b1.distance(b2) < 1e-2)
                            || (a1.distance(b2) < 1e-2 && b1.distance(a2) < 1e-2)
                    } else {
                        true
                    }
                };
                if connected {
                    adj[f1].push(f2);
                    adj[f2].push(f1);
                }
            }
        }
    }

    let mut visited = vec![false; mesh.faces.len()];
    let mut islands = Vec::new();
    for start in 0..mesh.faces.len() {
        if visited[start] || mesh.faces[start].vertices.len() < 3 {
            continue;
        }
        let mut queue = VecDeque::new();
        let mut island = Vec::new();
        visited[start] = true;
        queue.push_back(start);
        while let Some(cur) = queue.pop_front() {
            island.push(cur);
            for &nxt in &adj[cur] {
                if !visited[nxt] {
                    visited[nxt] = true;
                    queue.push_back(nxt);
                }
            }
        }
        island.sort_unstable();
        islands.push(island);
    }
    islands
}

fn ensure_face_uvs(mesh: &mut LowPolyMesh) {
    let (bmin, bmax) = mesh.bounds();
    for fi in 0..mesh.faces.len() {
        if mesh.faces[fi].uvs.len() == mesh.faces[fi].vertices.len() {
            continue;
        }
        let normal = face_normal(mesh, fi);
        let verts = mesh.faces[fi].vertices.clone();
        let mut uvs = Vec::with_capacity(verts.len());
        for vi in verts {
            let p = mesh.vertices.get(vi).copied().unwrap_or(Vec3::ZERO);
            uvs.push(box_atlas_uv(p, normal, bmin, bmax));
        }
        mesh.faces[fi].uvs = uvs;
    }
}

fn face_area_3d(mesh: &LowPolyMesh, fi: usize) -> f32 {
    let face = &mesh.faces[fi];
    if face.vertices.len() < 3 {
        return 0.0;
    }
    let Some(&v0) = mesh.vertices.get(face.vertices[0]) else {
        return 0.0;
    };
    let mut area = 0.0f32;
    for k in 1..(face.vertices.len() - 1) {
        let (Some(&v1), Some(&v2)) = (
            mesh.vertices.get(face.vertices[k]),
            mesh.vertices.get(face.vertices[k + 1]),
        ) else {
            continue;
        };
        area += 0.5 * (v1 - v0).cross(v2 - v0).length();
    }
    area
}

fn face_area_uv(mesh: &LowPolyMesh, fi: usize) -> f32 {
    let uvs = &mesh.faces[fi].uvs;
    if uvs.len() < 3 {
        return 0.0;
    }
    let u0 = Vec2::from_array(uvs[0]);
    let mut area = 0.0f32;
    for k in 1..(uvs.len() - 1) {
        let e1 = Vec2::from_array(uvs[k]) - u0;
        let e2 = Vec2::from_array(uvs[k + 1]) - u0;
        area += 0.5 * (e1.x * e2.y - e1.y * e2.x).abs();
    }
    area
}

/// Packs UV islands cleanly into `[0, 1] x [0, 1]` using a deterministic shelf packer
/// while preserving relative island scales (uniformly scaling down only if needed to fit).
fn pack_islands_into_unit_square(
    mesh: &mut LowPolyMesh,
    islands: &[Vec<usize>],
    padding: f32,
) -> f32 {
    if islands.is_empty() {
        return 1.0;
    }
    let pad = padding.clamp(0.0, 0.05);

    struct IslandBox {
        faces: Vec<usize>,
        min: Vec2,
        size: Vec2,
    }

    let mut boxes: Vec<IslandBox> = Vec::with_capacity(islands.len());
    let mut total_area = 0.0f32;
    for island in islands {
        let mut mn = Vec2::splat(f32::INFINITY);
        let mut mx = Vec2::splat(f32::NEG_INFINITY);
        for &fi in island {
            for &u in &mesh.faces[fi].uvs {
                let v = Vec2::from_array(u);
                if v.is_finite() {
                    mn = mn.min(v);
                    mx = mx.max(v);
                }
            }
        }
        if !mn.is_finite() || !mx.is_finite() {
            continue;
        }
        let size = (mx - mn).max(Vec2::splat(1e-4));
        total_area += (size.x + pad) * (size.y + pad);
        boxes.push(IslandBox {
            faces: island.clone(),
            min: mn,
            size,
        });
    }
    if boxes.is_empty() {
        return 1.0;
    }

    boxes.sort_by(|a, b| {
        b.size
            .y
            .total_cmp(&a.size.y)
            .then_with(|| b.size.x.total_cmp(&a.size.x))
    });

    let target_width = total_area.sqrt().max(1.0);
    let mut cursor_x = pad;
    let mut cursor_y = pad;
    let mut shelf_h = 0.0f32;
    let mut placements: Vec<(usize, Vec2)> = Vec::with_capacity(boxes.len());
    let mut max_x = 0.0f32;
    let mut max_y = 0.0f32;

    for (idx, b) in boxes.iter().enumerate() {
        if cursor_x > pad && cursor_x + b.size.x + pad > target_width {
            cursor_x = pad;
            cursor_y += shelf_h + pad;
            shelf_h = 0.0;
        }
        placements.push((idx, Vec2::new(cursor_x, cursor_y)));
        max_x = max_x.max(cursor_x + b.size.x + pad);
        max_y = max_y.max(cursor_y + b.size.y + pad);
        cursor_x += b.size.x + pad;
        shelf_h = shelf_h.max(b.size.y);
    }

    let extent = max_x.max(max_y).max(1e-4);
    let uniform_scale = if extent > 1.0 { 1.0 / extent } else { 1.0 };

    for (idx, offset) in placements {
        let b = &boxes[idx];
        for &fi in &b.faces {
            for u in &mut mesh.faces[fi].uvs {
                let local = Vec2::from_array(*u) - b.min;
                let packed = (local + offset) * uniform_scale;
                *u = [packed.x.clamp(0.0, 1.0), packed.y.clamp(0.0, 1.0)];
            }
        }
    }
    uniform_scale
}

/// Unwraps UV islands cut along `mesh.uv_seams` using per-island conformal tangent-plane
/// projection and deterministic shelf packing into `[0, 1] x [0, 1]` (`M1`).
pub fn unwrap_with_uv_seams(mesh: &mut LowPolyMesh, padding_uv: f32) -> usize {
    if mesh.faces.is_empty() {
        return 0;
    }
    let islands = collect_uv_islands(mesh, false);
    for island in &islands {
        let mut avg_n = Vec3::ZERO;
        for &fi in island {
            avg_n += face_normal(mesh, fi) * face_area_3d(mesh, fi).max(1e-5);
        }
        let n = if avg_n.length_squared() > 1e-8 {
            avg_n.normalize()
        } else {
            Vec3::Y
        };
        let up = if n.y.abs() > 0.9 { Vec3::Z } else { Vec3::Y };
        let tangent = up.cross(n).normalize_or_zero();
        let bitangent = n.cross(tangent).normalize_or_zero();

        for &fi in island {
            let verts = mesh.faces[fi].vertices.clone();
            let mut uvs = Vec::with_capacity(verts.len());
            for vi in verts {
                let p = mesh.vertices.get(vi).copied().unwrap_or(Vec3::ZERO);
                uvs.push([p.dot(tangent), p.dot(bitangent)]);
            }
            mesh.faces[fi].uvs = uvs;
        }
    }
    pack_islands_into_unit_square(mesh, &islands, padding_uv);
    islands.len()
}

/// Equalizes UV island scales so every island shares uniform texel density (`px / world unit`)
/// and repacks them cleanly into `[0, 1] x [0, 1]` (`M1`).
pub fn normalize_texel_density(
    mesh: &mut LowPolyMesh,
    texture_size: u32,
    target_px_per_unit: Option<f32>,
    padding_uv: f32,
) -> TexelDensityReport {
    if mesh.faces.is_empty() {
        return TexelDensityReport {
            island_count: 0,
            average_texel_density: 0.0,
            uniform_scale: 1.0,
        };
    }
    ensure_face_uvs(mesh);
    let tex_px = texture_size.clamp(16, 4096) as f32;
    let islands = collect_uv_islands(mesh, true);

    let mut island_stats: Vec<(f32, f32, Vec2)> = Vec::with_capacity(islands.len());
    let mut total_3d = 0.0f32;
    let mut weighted_density_sum = 0.0f32;

    for island in &islands {
        let mut a3d = 0.0f32;
        let mut auv = 0.0f32;
        let mut centroid = Vec2::ZERO;
        let mut uv_count = 0usize;
        for &fi in island {
            a3d += face_area_3d(mesh, fi);
            auv += face_area_uv(mesh, fi);
            for &u in &mesh.faces[fi].uvs {
                centroid += Vec2::from_array(u);
                uv_count += 1;
            }
        }
        if uv_count > 0 {
            centroid /= uv_count as f32;
        }
        let density = if a3d > 1e-8 && auv > 1e-10 {
            (auv.sqrt() * tex_px) / a3d.sqrt()
        } else {
            0.0
        };
        if density > 0.0 {
            total_3d += a3d;
            weighted_density_sum += density * a3d;
        }
        island_stats.push((a3d, density, centroid));
    }

    let mean_density = if total_3d > 1e-8 {
        weighted_density_sum / total_3d
    } else {
        tex_px
    };
    let desired_density = target_px_per_unit
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(mean_density);

    for (island, &(_a3d, density, centroid)) in islands.iter().zip(&island_stats) {
        if density <= 1e-6 {
            continue;
        }
        let factor = (desired_density / density).clamp(0.05, 20.0);
        for &fi in island {
            for u in &mut mesh.faces[fi].uvs {
                let v = Vec2::from_array(*u);
                let scaled = centroid + (v - centroid) * factor;
                *u = [scaled.x, scaled.y];
            }
        }
    }

    let uniform_scale = pack_islands_into_unit_square(mesh, &islands, padding_uv);
    TexelDensityReport {
        island_count: islands.len(),
        average_texel_density: desired_density * uniform_scale,
        uniform_scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ops::create_primitive;
    use crate::mesh::PrimitiveKind;
    use crate::palette::PaletteIndex;

    #[test]
    fn normalize_texel_density_equalizes_mismatched_uv_islands() {
        let mut mesh = create_primitive(PrimitiveKind::Cube, 1.0, PaletteIndex::DEFAULT_ACTIVE);
        mesh.faces[0].uvs = vec![[0.0, 0.0], [0.9, 0.0], [0.9, 0.9], [0.0, 0.9]];
        mesh.faces[1].uvs = vec![[0.0, 0.0], [0.05, 0.0], [0.05, 0.05], [0.0, 0.05]];
        for face in &mesh.faces {
            let n = face.vertices.len();
            for k in 0..n {
                mesh.uv_seams
                    .push(canonical_edge(face.vertices[k], face.vertices[(k + 1) % n]));
            }
        }
        let report = normalize_texel_density(&mut mesh, 256, None, 0.01);
        assert_eq!(report.island_count, 6);
        let a0 = face_area_uv(&mesh, 0);
        let a1 = face_area_uv(&mesh, 1);
        assert!(
            (a0 - a1).abs() < 1e-4,
            "equal-area cube faces must have matching UV areas after normalization: a0={a0}, a1={a1}"
        );
    }

    #[test]
    fn unwrap_with_uv_seams_splits_mesh_into_requested_islands() {
        let mut mesh = create_primitive(PrimitiveKind::Cube, 1.0, PaletteIndex::DEFAULT_ACTIVE);
        assert_eq!(unwrap_with_uv_seams(&mut mesh, 0.01), 1);

        let f0 = mesh.faces[0].vertices.clone();
        for k in 0..f0.len() {
            mesh.uv_seams
                .push(canonical_edge(f0[k], f0[(k + 1) % f0.len()]));
        }
        let islands = unwrap_with_uv_seams(&mut mesh, 0.01);
        assert_eq!(islands, 2);
        for face in &mesh.faces {
            assert_eq!(face.uvs.len(), face.vertices.len());
            for &[u, v] in &face.uvs {
                assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
            }
        }
    }
}
