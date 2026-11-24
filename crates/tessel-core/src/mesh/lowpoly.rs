use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::character::VertexSkinWeights;
use crate::palette::PaletteIndex;

use super::face::Face;
use super::texture::{box_atlas_uv, MeshTexture};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
    pub palette_index: PaletteIndex,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriangulatedMesh {
    pub vertices: Vec<TriVertex>,
    pub indices: Vec<u32>,
    /// Source `LowPolyMesh::vertices` index per output vertex (for manual
    /// skin-weight / morph lookups that triangulation otherwise erases).
    pub source_indices: Vec<usize>,
}

/// Sub-Unit: PBR surface material properties for a `LowPolyMesh` (rendered live & exported to `.glb`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeshMaterial {
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    #[serde(default)]
    pub double_sided: bool,
}

impl Default for MeshMaterial {
    fn default() -> Self {
        Self {
            metallic: 0.0,
            roughness: 0.85,
            emissive: [0.0, 0.0, 0.0],
            double_sided: false,
        }
    }
}

/// Named blend-shape target: sparse per-vertex position deltas applied on
/// top of the base mesh (glTF morph target). Deltas accumulate via
/// `mesh_morph_delta` and survive index-stable edits; full rebuilds
/// (bevel/subdivide/decimate) clear all morphs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MorphTarget {
    pub name: String,
    pub deltas: BTreeMap<usize, Vec3>,
}

/// Unit Anchor: Editable indexed low-poly mesh supporting triangles & quads with flat/smooth shading,
/// PBR material properties, and optional auto-UV RGBA texture painting.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LowPolyMesh {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
    #[serde(default)]
    pub texture: Option<MeshTexture>,
    #[serde(default)]
    pub normal_map: Option<MeshTexture>,
    #[serde(default)]
    pub smooth_shading: bool,
    #[serde(default)]
    pub material: MeshMaterial,
    /// Sparse manual skin-weight overrides by vertex index (weight paint).
    /// Export prefers these over the auto solver. Index-stable edits
    /// (move/set/translate/extrude/inset) keep them; full rebuilds
    /// (bevel/subdivide/decimate) clear them since indices stop meaning
    /// the same corners.
    #[serde(default)]
    pub skin_weights: BTreeMap<usize, VertexSkinWeights>,
    /// Named morph targets (blend shapes) as sparse per-vertex deltas.
    #[serde(default)]
    pub morphs: Vec<MorphTarget>,
}

impl LowPolyMesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for &v in &self.vertices {
            min = min.min(v);
            max = max.max(v);
        }
        if !min.is_finite() || !max.is_finite() {
            (Vec3::ZERO, Vec3::ONE)
        } else {
            (min, max)
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.faces
            .iter()
            .map(|f| f.vertices.len().saturating_sub(2))
            .sum()
    }

    /// Triangulates all faces into indexed triangles with flat or averaged smooth vertex normals and auto-UVs.
    pub fn to_triangulated_flat(&self) -> TriangulatedMesh {
        let mut out = TriangulatedMesh::default();
        let (min_b, max_b) = self.bounds();

        let smooth_normals: Option<Vec<Vec3>> = if self.smooth_shading {
            let mut acc = vec![Vec3::ZERO; self.vertices.len()];
            for face in &self.faces {
                if face.vertices.len() < 3 {
                    continue;
                }
                let Some(&i0) = face.vertices.first() else {
                    continue;
                };
                let Some(&p0) = self.vertices.get(i0) else {
                    continue;
                };
                for fan in 1..(face.vertices.len() - 1) {
                    let (Some(&i1), Some(&i2)) =
                        (face.vertices.get(fan), face.vertices.get(fan + 1))
                    else {
                        continue;
                    };
                    let (Some(&p1), Some(&p2)) = (self.vertices.get(i1), self.vertices.get(i2))
                    else {
                        continue;
                    };
                    let cross = (p1 - p0).cross(p2 - p0);
                    if cross.length_squared() >= 1e-10 {
                        acc[i0] += cross;
                        acc[i1] += cross;
                        acc[i2] += cross;
                    }
                }
            }
            for n in &mut acc {
                *n = n.normalize_or_zero();
            }
            Some(acc)
        } else {
            None
        };

        for face in &self.faces {
            if face.vertices.len() < 3 {
                continue;
            }
            let Some(&i0) = face.vertices.first() else {
                continue;
            };
            let Some(&p0) = self.vertices.get(i0) else {
                continue;
            };

            for fan in 1..(face.vertices.len() - 1) {
                let (Some(&i1), Some(&i2)) = (face.vertices.get(fan), face.vertices.get(fan + 1))
                else {
                    continue;
                };
                let (Some(&p1), Some(&p2)) = (self.vertices.get(i1), self.vertices.get(i2)) else {
                    continue;
                };
                let face_normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                if face_normal.length_squared() < 1e-8 {
                    continue;
                }
                let n0 = smooth_normals
                    .as_ref()
                    .and_then(|sn| sn.get(i0).copied())
                    .filter(|n| n.length_squared() > 1e-6)
                    .unwrap_or(face_normal)
                    .to_array();
                let n1 = smooth_normals
                    .as_ref()
                    .and_then(|sn| sn.get(i1).copied())
                    .filter(|n| n.length_squared() > 1e-6)
                    .unwrap_or(face_normal)
                    .to_array();
                let n2 = smooth_normals
                    .as_ref()
                    .and_then(|sn| sn.get(i2).copied())
                    .filter(|n| n.length_squared() > 1e-6)
                    .unwrap_or(face_normal)
                    .to_array();
                let has_face_uvs = face.uvs.len() == face.vertices.len();
                let uv0 = if has_face_uvs {
                    face.uvs[0]
                } else {
                    box_atlas_uv(p0, face_normal, min_b, max_b)
                };
                let uv1 = if has_face_uvs {
                    face.uvs[fan]
                } else {
                    box_atlas_uv(p1, face_normal, min_b, max_b)
                };
                let uv2 = if has_face_uvs {
                    face.uvs[fan + 1]
                } else {
                    box_atlas_uv(p2, face_normal, min_b, max_b)
                };
                let tangent = compute_triangle_tangent(p0, p1, p2, uv0, uv1, uv2, face_normal);

                let base = out.vertices.len() as u32;
                out.vertices.push(TriVertex {
                    position: p0.to_array(),
                    normal: n0,
                    uv: uv0,
                    tangent,
                    palette_index: face.material,
                });
                out.vertices.push(TriVertex {
                    position: p1.to_array(),
                    normal: n1,
                    uv: uv1,
                    tangent,
                    palette_index: face.material,
                });
                out.vertices.push(TriVertex {
                    position: p2.to_array(),
                    normal: n2,
                    uv: uv2,
                    tangent,
                    palette_index: face.material,
                });
                out.source_indices.extend_from_slice(&[i0, i1, i2]);
                out.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
        out
    }
}

fn compute_triangle_tangent(
    p0: Vec3,
    p1: Vec3,
    p2: Vec3,
    uv0: [f32; 2],
    uv1: [f32; 2],
    uv2: [f32; 2],
    normal: Vec3,
) -> [f32; 4] {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let du1 = uv1[0] - uv0[0];
    let dv1 = uv1[1] - uv0[1];
    let du2 = uv2[0] - uv0[0];
    let dv2 = uv2[1] - uv0[1];
    let det = du1 * dv2 - du2 * dv1;
    let (t, b) = if det.abs() > 1e-8 {
        let inv = 1.0 / det;
        (
            ((e1 * dv2 - e2 * dv1) * inv).normalize_or_zero(),
            ((e2 * du1 - e1 * du2) * inv).normalize_or_zero(),
        )
    } else {
        let fallback_axis = if normal.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let t = fallback_axis.cross(normal).normalize_or_zero();
        (t, normal.cross(t).normalize_or_zero())
    };
    let ortho_t = (t - normal * normal.dot(t)).normalize_or_zero();
    let ortho_t = if ortho_t.length_squared() < 1e-6 {
        Vec3::X
    } else {
        ortho_t
    };
    let w = if normal.cross(ortho_t).dot(b) < 0.0 {
        -1.0
    } else {
        1.0
    };
    [ortho_t.x, ortho_t.y, ortho_t.z, w]
}
