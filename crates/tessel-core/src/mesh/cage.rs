use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

use super::lowpoly::{LowPolyMesh, MorphTarget};

/// Kinematic low-resolution 3D lattice proxy cage (Free-Form Deformation) for deterministic
/// mesh deformation and baking without a runtime physics solver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyCage {
    /// Grid resolution along `(X, Y, Z)`, each clamped to `2..=6`.
    pub divisions: [usize; 3],
    pub min: Vec3,
    pub max: Vec3,
    pub rest_points: Vec<Vec3>,
    pub deformed_points: Vec<Vec3>,
}

impl ProxyCage {
    /// Constructs a rest-pose proxy cage enclosing `mesh` with optional world/local `padding`.
    pub fn from_mesh_bounds(mesh: &LowPolyMesh, divisions: [usize; 3], padding: f32) -> Self {
        let div = [
            divisions[0].clamp(2, 6),
            divisions[1].clamp(2, 6),
            divisions[2].clamp(2, 6),
        ];
        let pad = if padding.is_finite() {
            padding.max(0.0)
        } else {
            0.0
        };
        let (mut min, mut max) = mesh.bounds();
        min -= Vec3::splat(pad);
        max += Vec3::splat(pad);
        // Guard against degenerate zero-thickness axes
        let eps = 1e-4;
        if (max.x - min.x).abs() < eps {
            max.x = min.x + eps;
        }
        if (max.y - min.y).abs() < eps {
            max.y = min.y + eps;
        }
        if (max.z - min.z).abs() < eps {
            max.z = min.z + eps;
        }

        let mut rest_points = Vec::with_capacity(div[0] * div[1] * div[2]);
        for iz in 0..div[2] {
            let tz = iz as f32 / (div[2] - 1) as f32;
            for iy in 0..div[1] {
                let ty = iy as f32 / (div[1] - 1) as f32;
                for ix in 0..div[0] {
                    let tx = ix as f32 / (div[0] - 1) as f32;
                    rest_points.push(Vec3::new(
                        min.x + (max.x - min.x) * tx,
                        min.y + (max.y - min.y) * ty,
                        min.z + (max.z - min.z) * tz,
                    ));
                }
            }
        }
        let deformed_points = rest_points.clone();
        Self {
            divisions: div,
            min,
            max,
            rest_points,
            deformed_points,
        }
    }

    pub fn control_point_index(&self, ix: usize, iy: usize, iz: usize) -> Option<usize> {
        if ix >= self.divisions[0] || iy >= self.divisions[1] || iz >= self.divisions[2] {
            return None;
        }
        Some((iz * self.divisions[1] + iy) * self.divisions[0] + ix)
    }

    /// Applies sparse deltas `(control_point_index, delta)` to `deformed_points`.
    pub fn apply_control_deltas(&mut self, deltas: &[(usize, Vec3)]) -> Result<(), CoreError> {
        for &(idx, delta) in deltas {
            if !delta.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let pt = self
                .deformed_points
                .get_mut(idx)
                .ok_or(CoreError::OutOfBounds)?;
            *pt += delta;
        }
        Ok(())
    }

    /// Evaluates smooth tri-Bernstein FFD lattice interpolation for a point `p` in mesh local space.
    pub fn deform_point(&self, p: Vec3) -> Vec3 {
        let size = (self.max - self.min).max(Vec3::splat(1e-5));
        let u = ((p.x - self.min.x) / size.x).clamp(0.0, 1.0);
        let v = ((p.y - self.min.y) / size.y).clamp(0.0, 1.0);
        let w = ((p.z - self.min.z) / size.z).clamp(0.0, 1.0);

        let nx = self.divisions[0] - 1;
        let ny = self.divisions[1] - 1;
        let nz = self.divisions[2] - 1;

        let bx: Vec<f32> = (0..=nx).map(|i| bernstein(nx, i, u)).collect();
        let by: Vec<f32> = (0..=ny).map(|j| bernstein(ny, j, v)).collect();
        let bz: Vec<f32> = (0..=nz).map(|k| bernstein(nz, k, w)).collect();

        let mut out = Vec3::ZERO;
        for (iz, &wz) in bz.iter().enumerate() {
            for (iy, &wy) in by.iter().enumerate() {
                let wyz = wy * wz;
                for (ix, &wx) in bx.iter().enumerate() {
                    if let Some(idx) = self.control_point_index(ix, iy, iz) {
                        if let Some(&cp) = self.deformed_points.get(idx) {
                            out += cp * (wx * wyz);
                        }
                    }
                }
            }
        }
        out
    }

    /// Deforms every vertex in `mesh`, optionally applying a deterministic kinematic `floor_y` clamp.
    pub fn apply_to_mesh(&self, mesh: &LowPolyMesh, floor_y: Option<f32>) -> Vec<Vec3> {
        mesh.vertices
            .iter()
            .map(|&v| {
                let mut d = self.deform_point(v);
                if let Some(fy) = floor_y {
                    if fy.is_finite() && d.y < fy {
                        d.y = fy;
                    }
                }
                d
            })
            .collect()
    }

    /// Bakes the proxy cage deformation into a named `MorphTarget` (with both position & normal deltas).
    pub fn bake_to_morph_target(
        &self,
        mesh: &LowPolyMesh,
        target_name: impl Into<String>,
        floor_y: Option<f32>,
    ) -> MorphTarget {
        let deformed_verts = self.apply_to_mesh(mesh, floor_y);
        let mut deltas = BTreeMap::new();
        for (i, (&orig, &def)) in mesh.vertices.iter().zip(deformed_verts.iter()).enumerate() {
            let d = def - orig;
            if d.length_squared() > 1e-10 {
                deltas.insert(i, d);
            }
        }

        let base_normals = compute_vertex_normals(&mesh.vertices, &mesh.faces);
        let def_normals = compute_vertex_normals(&deformed_verts, &mesh.faces);
        let mut normal_deltas = BTreeMap::new();
        for (i, (&bn, &dn)) in base_normals.iter().zip(def_normals.iter()).enumerate() {
            let nd = dn - bn;
            if nd.length_squared() > 1e-8 {
                normal_deltas.insert(i, nd);
            }
        }

        MorphTarget {
            name: target_name.into(),
            deltas,
            normal_deltas,
            default_weight: 0.0,
        }
    }
}

fn bernstein(n: usize, i: usize, t: f32) -> f32 {
    binomial(n, i) as f32 * t.powi(i as i32) * (1.0 - t).powi((n - i) as i32)
}

fn binomial(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut res = 1usize;
    for i in 0..k {
        res = res * (n - i) / (i + 1);
    }
    res
}

fn compute_vertex_normals(vertices: &[Vec3], faces: &[super::face::Face]) -> Vec<Vec3> {
    let mut acc = vec![Vec3::ZERO; vertices.len()];
    for face in faces {
        if face.vertices.len() < 3 {
            continue;
        }
        let Some(&i0) = face.vertices.first() else {
            continue;
        };
        let Some(&p0) = vertices.get(i0) else {
            continue;
        };
        for fan in 1..(face.vertices.len() - 1) {
            let (Some(&i1), Some(&i2)) = (face.vertices.get(fan), face.vertices.get(fan + 1))
            else {
                continue;
            };
            let (Some(&p1), Some(&p2)) = (vertices.get(i1), vertices.get(i2)) else {
                continue;
            };
            let cross = (p1 - p0).cross(p2 - p0);
            if cross.length_squared() > 1e-10 {
                acc[i0] += cross;
                acc[i1] += cross;
                acc[i2] += cross;
            }
        }
    }
    for n in &mut acc {
        *n = n.normalize_or_zero();
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{ops, PrimitiveKind};
    use crate::palette::PaletteIndex;

    #[test]
    fn proxy_cage_identity_preserves_mesh_and_floor_clamp_works() {
        let mesh = ops::create_primitive(PrimitiveKind::Cube, 1.0, PaletteIndex::new(1).unwrap());
        let mut cage = ProxyCage::from_mesh_bounds(&mesh, [2, 2, 2], 0.0);
        let unchanged = cage.apply_to_mesh(&mesh, None);
        for (a, b) in mesh.vertices.iter().zip(unchanged.iter()) {
            assert!((*a - *b).length() < 1e-5);
        }

        // Push all control points down by -2.0 and clamp to floor y = 0.0
        let deltas: Vec<(usize, Vec3)> = (0..cage.deformed_points.len())
            .map(|i| (i, Vec3::new(0.0, -2.0, 0.0)))
            .collect();
        cage.apply_control_deltas(&deltas).unwrap();
        let clamped = cage.apply_to_mesh(&mesh, Some(0.0));
        assert!(clamped.iter().all(|v| v.y >= -1e-5));

        let morph = cage.bake_to_morph_target(&mesh, "SquashFloor", Some(0.0));
        assert_eq!(morph.name, "SquashFloor");
        assert!(!morph.deltas.is_empty());
    }
}
