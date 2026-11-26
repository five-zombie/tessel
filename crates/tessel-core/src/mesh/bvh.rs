use glam::Vec3;

use crate::mesh::ops::MeshRayHit;
use crate::mesh::{FaceId, LowPolyMesh};

const MAX_LEAF_TRIS: usize = 8;

#[derive(Debug, Clone, Copy)]
struct BvhTri {
    v0: Vec3,
    v1: Vec3,
    v2: Vec3,
    centroid: Vec3,
    normal: Vec3,
    face_id: FaceId,
}

#[derive(Debug, Clone, Copy)]
struct BvhNode {
    aabb_min: Vec3,
    aabb_max: Vec3,
    /// If `count > 0`, index of first triangle in `tris`; otherwise index of left child (`right = left + 1`).
    left_or_first: u32,
    /// Number of triangles in leaf, or `0` for internal node.
    count: u32,
}

/// Flat-array Bounding Volume Hierarchy for fast `O(log N)` raycast queries on `LowPolyMesh`.
#[derive(Debug, Clone, Default)]
pub struct MeshBvh {
    nodes: Vec<BvhNode>,
    tris: Vec<BvhTri>,
}

impl MeshBvh {
    /// Builds a BVH over all triangles fanned from `mesh.faces`.
    pub fn from_mesh(mesh: &LowPolyMesh) -> Self {
        let mut tris = Vec::new();
        for (fi, face) in mesh.faces.iter().enumerate() {
            if face.vertices.len() < 3 {
                continue;
            }
            let Some(&i0) = face.vertices.first() else {
                continue;
            };
            let Some(&v0) = mesh.vertices.get(i0) else {
                continue;
            };
            if !v0.is_finite() {
                continue;
            }
            for k in 1..(face.vertices.len() - 1) {
                let (Some(&i1), Some(&i2)) = (face.vertices.get(k), face.vertices.get(k + 1))
                else {
                    continue;
                };
                let (Some(&v1), Some(&v2)) = (mesh.vertices.get(i1), mesh.vertices.get(i2)) else {
                    continue;
                };
                if !v1.is_finite() || !v2.is_finite() {
                    continue;
                }
                let cross = (v1 - v0).cross(v2 - v0);
                let normal = if cross.length_squared() > 1e-12 {
                    cross.normalize()
                } else {
                    continue;
                };
                tris.push(BvhTri {
                    v0,
                    v1,
                    v2,
                    centroid: (v0 + v1 + v2) / 3.0,
                    normal,
                    face_id: FaceId(fi),
                });
            }
        }
        if tris.is_empty() {
            return Self::default();
        }
        let mut nodes = Vec::with_capacity(tris.len().saturating_mul(2).min(131_072));
        let root_count = tris.len() as u32;
        nodes.push(BvhNode {
            aabb_min: Vec3::splat(f32::INFINITY),
            aabb_max: Vec3::splat(f32::NEG_INFINITY),
            left_or_first: 0,
            count: root_count,
        });
        let mut bvh = Self { nodes, tris };
        bvh.update_bounds(0);
        bvh.subdivide(0, 0);
        bvh
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    fn update_bounds(&mut self, node_idx: usize) {
        let Some(node) = self.nodes.get(node_idx).copied() else {
            return;
        };
        let start = node.left_or_first as usize;
        let end = start
            .saturating_add(node.count as usize)
            .min(self.tris.len());
        let mut bmin = Vec3::splat(f32::INFINITY);
        let mut bmax = Vec3::splat(f32::NEG_INFINITY);
        for tri in &self.tris[start..end] {
            bmin = bmin.min(tri.v0).min(tri.v1).min(tri.v2);
            bmax = bmax.max(tri.v0).max(tri.v1).max(tri.v2);
        }
        if let Some(n) = self.nodes.get_mut(node_idx) {
            n.aabb_min = bmin;
            n.aabb_max = bmax;
        }
    }

    fn subdivide(&mut self, node_idx: usize, depth: usize) {
        if depth >= 32 {
            return;
        }
        let Some(node) = self.nodes.get(node_idx).copied() else {
            return;
        };
        if (node.count as usize) <= MAX_LEAF_TRIS {
            return;
        }
        let start = node.left_or_first as usize;
        let count = node.count as usize;
        let end = start.saturating_add(count).min(self.tris.len());
        if end <= start + MAX_LEAF_TRIS {
            return;
        }

        let extent = node.aabb_max - node.aabb_min;
        let axis = if extent.y > extent.x && extent.y >= extent.z {
            1
        } else if extent.z > extent.x && extent.z > extent.y {
            2
        } else {
            0
        };
        let split_pos = (node.aabb_min[axis] + node.aabb_max[axis]) * 0.5;

        let mut i = start;
        let mut j = end;
        while i < j {
            if self.tris[i].centroid[axis] < split_pos {
                i += 1;
            } else {
                j -= 1;
                self.tris.swap(i, j);
            }
        }
        let mut left_count = i - start;
        if left_count == 0 || left_count == count {
            // Degenerate midpoint split: fall back to median partition.
            left_count = count / 2;
        }
        let right_count = count - left_count;
        if left_count == 0 || right_count == 0 {
            return;
        }

        let left_idx = self.nodes.len();
        let right_idx = left_idx + 1;
        self.nodes.push(BvhNode {
            aabb_min: Vec3::splat(f32::INFINITY),
            aabb_max: Vec3::splat(f32::NEG_INFINITY),
            left_or_first: start as u32,
            count: left_count as u32,
        });
        self.nodes.push(BvhNode {
            aabb_min: Vec3::splat(f32::INFINITY),
            aabb_max: Vec3::splat(f32::NEG_INFINITY),
            left_or_first: (start + left_count) as u32,
            count: right_count as u32,
        });
        if let Some(n) = self.nodes.get_mut(node_idx) {
            n.left_or_first = left_idx as u32;
            n.count = 0;
        }
        self.update_bounds(left_idx);
        self.update_bounds(right_idx);
        self.subdivide(left_idx, depth + 1);
        self.subdivide(right_idx, depth + 1);
    }

    /// Casts a ray `(origin, dir)` and returns the closest hit with `t > 1e-5`.
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<MeshRayHit> {
        if self.nodes.is_empty() || !origin.is_finite() || !dir.is_finite() {
            return None;
        }
        let dir_len_sq = dir.length_squared();
        if dir_len_sq <= 1e-12 {
            return None;
        }
        let inv_dir = Vec3::new(
            if dir.x.abs() > 1e-12 {
                1.0 / dir.x
            } else {
                f32::INFINITY
            },
            if dir.y.abs() > 1e-12 {
                1.0 / dir.y
            } else {
                f32::INFINITY
            },
            if dir.z.abs() > 1e-12 {
                1.0 / dir.z
            } else {
                f32::INFINITY
            },
        );

        let mut stack = [0u32; 64];
        let mut sp: usize = 0;
        stack[0] = 0;
        sp += 1;

        let mut best_t = f32::INFINITY;
        let mut best_hit: Option<MeshRayHit> = None;

        while sp > 0 {
            sp -= 1;
            let node_idx = stack[sp] as usize;
            let Some(node) = self.nodes.get(node_idx) else {
                continue;
            };
            if !ray_intersects_aabb(origin, inv_dir, node.aabb_min, node.aabb_max, best_t) {
                continue;
            }
            if node.count > 0 {
                let start = node.left_or_first as usize;
                let end = start
                    .saturating_add(node.count as usize)
                    .min(self.tris.len());
                for tri in &self.tris[start..end] {
                    if let Some(t) = intersect_triangle(origin, dir, tri.v0, tri.v1, tri.v2) {
                        if t > 1e-5 && t < best_t {
                            best_t = t;
                            best_hit = Some(MeshRayHit {
                                face_id: tri.face_id,
                                point: origin + dir * t,
                                normal: tri.normal,
                                uv: [0.0, 0.0],
                            });
                        }
                    }
                }
            } else {
                let left = node.left_or_first;
                let right = left.saturating_add(1);
                if sp + 2 <= stack.len() {
                    stack[sp] = right;
                    stack[sp + 1] = left;
                    sp += 2;
                }
            }
        }
        best_hit
    }
}

#[inline]
fn ray_intersects_aabb(origin: Vec3, inv_dir: Vec3, bmin: Vec3, bmax: Vec3, max_t: f32) -> bool {
    let mut tmin = 0.0_f32;
    let mut tmax = max_t;
    for axis in 0..3 {
        let inv = inv_dir[axis];
        if inv.is_infinite() {
            if origin[axis] < bmin[axis] || origin[axis] > bmax[axis] {
                return false;
            }
            continue;
        }
        let mut t0 = (bmin[axis] - origin[axis]) * inv;
        let mut t1 = (bmax[axis] - origin[axis]) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        tmin = tmin.max(t0);
        tmax = tmax.min(t1);
        if tmax < tmin {
            return false;
        }
    }
    tmax >= 0.0
}

#[inline]
fn intersect_triangle(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv_det = 1.0 / det;
    let tvec = origin - v0;
    let u = tvec.dot(p) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = tvec.cross(e1);
    let v = dir.dot(q) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv_det;
    if t.is_finite() && t > 1e-5 {
        Some(t)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ops::{create_primitive, raycast_mesh_hit};
    use crate::mesh::PrimitiveKind;
    use crate::palette::PaletteIndex;

    #[test]
    fn bvh_raycast_matches_linear_scan_on_subdivided_mesh() {
        let mat = PaletteIndex::new(1).unwrap();
        let mesh = create_primitive(PrimitiveKind::Sphere, 4.0, mat);
        let bvh = MeshBvh::from_mesh(&mesh);
        assert!(bvh.triangle_count() >= 64);

        for ix in -3..=3 {
            for iz in -3..=3 {
                let origin = Vec3::new(ix as f32 * 0.6 + 0.07, 10.0, iz as f32 * 0.6 + 0.05);
                let dir = -Vec3::Y;
                let linear = raycast_mesh_hit(&mesh, origin, dir);
                let accel = bvh.raycast(origin, dir);
                match (linear, accel) {
                    (Some(a), Some(b)) => {
                        assert!((a.point - b.point).length() < 1e-4);
                        assert!((a.normal - b.normal).length() < 1e-4);
                    }
                    (None, None) => {}
                    (l, r) => panic!("mismatch at ({ix},{iz}): linear={l:?}, bvh={r:?}"),
                }
            }
        }
    }
}
