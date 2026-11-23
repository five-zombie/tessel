use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::palette::PaletteIndex;

use super::face::Face;
use super::texture::{box_atlas_uv, MeshTexture};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub palette_index: PaletteIndex,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriangulatedMesh {
    pub vertices: Vec<TriVertex>,
    pub indices: Vec<u32>,
}

/// Unit Anchor: Editable indexed low-poly mesh supporting triangles & quads with flat shading
/// and optional auto-UV RGBA texture painting.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LowPolyMesh {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
    #[serde(default)]
    pub texture: Option<MeshTexture>,
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

    /// Triangulates all faces into flat-shaded triangles with CCW outward normals and auto-UVs.
    pub fn to_triangulated_flat(&self) -> TriangulatedMesh {
        let mut out = TriangulatedMesh::default();
        let (min_b, max_b) = self.bounds();
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
                let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                if normal.length_squared() < 1e-8 {
                    continue;
                }
                let n_arr = normal.to_array();
                let base = out.vertices.len() as u32;
                out.vertices.push(TriVertex {
                    position: p0.to_array(),
                    normal: n_arr,
                    uv: box_atlas_uv(p0, normal, min_b, max_b),
                    palette_index: face.material,
                });
                out.vertices.push(TriVertex {
                    position: p1.to_array(),
                    normal: n_arr,
                    uv: box_atlas_uv(p1, normal, min_b, max_b),
                    palette_index: face.material,
                });
                out.vertices.push(TriVertex {
                    position: p2.to_array(),
                    normal: n_arr,
                    uv: box_atlas_uv(p2, normal, min_b, max_b),
                    palette_index: face.material,
                });
                out.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
        out
    }
}
