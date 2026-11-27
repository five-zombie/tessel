use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::lowpoly::LowPolyMesh;

/// Organic sculpting brush mode (`M2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SculptBrushKind {
    #[default]
    Smooth,
    Inflate,
    Grab,
    Crease,
}

impl SculptBrushKind {
    pub const ALL: [Self; 4] = [Self::Smooth, Self::Inflate, Self::Grab, Self::Crease];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Inflate => "Inflate",
            Self::Grab => "Grab",
            Self::Crease => "Crease",
        }
    }
}

/// Parameters for a single deterministic organic sculpting brush stroke (`M2`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SculptStrokeParams {
    pub brush: SculptBrushKind,
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub drag_delta: [f32; 3],
    pub radius: f32,
    pub strength: f32,
    pub mirror_x: bool,
}

fn build_vertex_adjacency(mesh: &LowPolyMesh) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); mesh.vertices.len()];
    for face in &mesh.faces {
        let n = face.vertices.len();
        if n < 2 {
            continue;
        }
        for k in 0..n {
            let a = face.vertices[k];
            let b = face.vertices[(k + 1) % n];
            if a < adj.len() && b < adj.len() && a != b {
                if !adj[a].contains(&b) {
                    adj[a].push(b);
                }
                if !adj[b].contains(&a) {
                    adj[b].push(a);
                }
            }
        }
    }
    adj
}

fn compute_smooth_vertex_normals(mesh: &LowPolyMesh) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; mesh.vertices.len()];
    for face in &mesh.faces {
        if face.vertices.len() < 3 {
            continue;
        }
        let (Some(&v0), Some(&v1), Some(&v2)) = (
            mesh.vertices.get(face.vertices[0]),
            mesh.vertices.get(face.vertices[1]),
            mesh.vertices.get(face.vertices[2]),
        ) else {
            continue;
        };
        let n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
        if n.is_finite() {
            for &vi in &face.vertices {
                if let Some(acc) = normals.get_mut(vi) {
                    *acc += n;
                }
            }
        }
    }
    for n in &mut normals {
        let len = n.length();
        *n = if len > 1e-6 { *n / len } else { Vec3::Y };
    }
    normals
}

/// Applies one deterministic sculpting brush stroke to `mesh` with smooth cubic falloff
/// and optional X-mirror symmetry. Returns `(vertex_index, previous_position)` pairs for undo.
pub fn apply_sculpt_stroke(
    mesh: &mut LowPolyMesh,
    params: SculptStrokeParams,
) -> Vec<(usize, Vec3)> {
    if mesh.vertices.is_empty()
        || !params.radius.is_finite()
        || !params.strength.is_finite()
        || params.radius <= 1e-5
        || params.strength.abs() <= 1e-6
        || !params.center.iter().all(|c| c.is_finite())
        || !params.normal.iter().all(|c| c.is_finite())
        || !params.drag_delta.iter().all(|c| c.is_finite())
    {
        return Vec::new();
    }

    let radius = params.radius.clamp(1e-4, 50.0);
    let strength = params.strength.clamp(-1.0, 1.0);
    let radius_sq = radius * radius;
    let base_center = Vec3::from_array(params.center);
    let base_normal = Vec3::from_array(params.normal).normalize_or_zero();
    let base_delta = Vec3::from_array(params.drag_delta);

    let adj = if params.brush == SculptBrushKind::Smooth {
        Some(build_vertex_adjacency(mesh))
    } else {
        None
    };
    let smooth_normals = if matches!(
        params.brush,
        SculptBrushKind::Inflate | SculptBrushKind::Crease
    ) {
        Some(compute_smooth_vertex_normals(mesh))
    } else {
        None
    };

    let original_verts = mesh.vertices.clone();
    let mut previous_map: BTreeMap<usize, Vec3> = BTreeMap::new();

    let strokes: [(Vec3, Vec3, Vec3); 2] = [
        (base_center, base_normal, base_delta),
        (
            Vec3::new(-base_center.x, base_center.y, base_center.z),
            Vec3::new(-base_normal.x, base_normal.y, base_normal.z),
            Vec3::new(-base_delta.x, base_delta.y, base_delta.z),
        ),
    ];
    let stroke_count = if params.mirror_x && base_center.x.abs() > 1e-5 {
        2
    } else {
        1
    };

    for (center, stroke_normal, drag_delta) in strokes.into_iter().take(stroke_count) {
        for (vi, &orig_pos) in original_verts.iter().enumerate() {
            let diff = orig_pos - center;
            let dist_sq = diff.length_squared();
            if dist_sq >= radius_sq {
                continue;
            }
            let t = (dist_sq / radius_sq).clamp(0.0, 1.0);
            let falloff = (1.0 - t) * (1.0 - t);
            let w = falloff * strength;

            let new_pos = match params.brush {
                SculptBrushKind::Smooth => {
                    let Some(neighbors) = adj.as_ref().and_then(|a| a.get(vi)) else {
                        continue;
                    };
                    if neighbors.is_empty() {
                        continue;
                    }
                    let mut avg = Vec3::ZERO;
                    for &ni in neighbors {
                        avg += original_verts[ni];
                    }
                    avg /= neighbors.len() as f32;
                    mesh.vertices[vi].lerp(avg, w.abs().clamp(0.0, 1.0))
                }
                SculptBrushKind::Inflate => {
                    let vn = smooth_normals
                        .as_ref()
                        .and_then(|ns| ns.get(vi).copied())
                        .unwrap_or(stroke_normal);
                    let dir = if vn.length_squared() > 1e-6 {
                        vn
                    } else {
                        Vec3::Y
                    };
                    mesh.vertices[vi] + dir * (w * radius * 0.25)
                }
                SculptBrushKind::Grab => mesh.vertices[vi] + drag_delta * w,
                SculptBrushKind::Crease => {
                    let vn = smooth_normals
                        .as_ref()
                        .and_then(|ns| ns.get(vi).copied())
                        .unwrap_or(stroke_normal);
                    let dir = if vn.length_squared() > 1e-6 {
                        vn
                    } else {
                        Vec3::Y
                    };
                    let pinch = (center - orig_pos) * (w * 0.2);
                    let indent = -dir * (w * radius * 0.18);
                    mesh.vertices[vi] + pinch + indent
                }
            };

            if new_pos.is_finite() && new_pos.distance_squared(mesh.vertices[vi]) > 1e-12 {
                previous_map.entry(vi).or_insert(orig_pos);
                mesh.vertices[vi] = new_pos;
            }
        }
    }

    previous_map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ops::create_primitive;
    use crate::mesh::PrimitiveKind;
    use crate::palette::PaletteIndex;

    #[test]
    fn sculpt_brushes_deform_and_mirror_x_symmetrically() {
        let mut mesh = create_primitive(PrimitiveKind::Cube, 1.0, PaletteIndex::DEFAULT_ACTIVE);
        let undo = apply_sculpt_stroke(
            &mut mesh,
            SculptStrokeParams {
                brush: SculptBrushKind::Inflate,
                center: [0.5, 0.5, 0.5],
                normal: [1.0, 0.0, 0.0],
                drag_delta: [0.0, 0.0, 0.0],
                radius: 0.75,
                strength: 0.5,
                mirror_x: true,
            },
        );
        assert!(
            undo.len() >= 2,
            "mirror_x inflate must affect both +X and -X vertices"
        );

        let undo_grab = apply_sculpt_stroke(
            &mut mesh,
            SculptStrokeParams {
                brush: SculptBrushKind::Grab,
                center: [0.5, 0.5, 0.5],
                normal: [0.0, 1.0, 0.0],
                drag_delta: [0.1, 0.2, 0.0],
                radius: 0.8,
                strength: 0.8,
                mirror_x: false,
            },
        );
        assert!(!undo_grab.is_empty());

        let undo_smooth = apply_sculpt_stroke(
            &mut mesh,
            SculptStrokeParams {
                brush: SculptBrushKind::Smooth,
                center: [0.0, 0.5, 0.0],
                normal: [0.0, 1.0, 0.0],
                drag_delta: [0.0; 3],
                radius: 2.0,
                strength: 0.5,
                mirror_x: false,
            },
        );
        assert!(!undo_smooth.is_empty());

        let undo_crease = apply_sculpt_stroke(
            &mut mesh,
            SculptStrokeParams {
                brush: SculptBrushKind::Crease,
                center: [0.0, 1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                drag_delta: [0.0; 3],
                radius: 1.5,
                strength: 0.5,
                mirror_x: false,
            },
        );
        assert!(!undo_crease.is_empty());
    }
}
