use glam::{Vec2, Vec3};

use crate::palette::Rgba8;

use super::lowpoly::LowPolyMesh;
use super::texture::MeshTexture;

/// Parameters for 3D seam-aware, visibility-checked surface painting in mesh-local space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintSpot3DParams {
    pub center: Vec3,
    pub radius: f32,
    pub color: Rgba8,
    /// Optional brush surface normal; skips triangles facing away from the stroke.
    pub brush_normal: Option<Vec3>,
    /// Optional camera position in mesh-local space; culls back-facing and ray-occluded texels.
    pub camera_pos: Option<Vec3>,
}

#[derive(Debug, Clone, Copy)]
struct PaintTri {
    p0: Vec3,
    p1: Vec3,
    p2: Vec3,
    aabb_min: Vec3,
    aabb_max: Vec3,
    geo_normal: Vec3,
}

impl PaintTri {
    fn new(p0: Vec3, p1: Vec3, p2: Vec3) -> Self {
        let aabb_min = p0.min(p1).min(p2);
        let aabb_max = p0.max(p1).max(p2);
        let geo_normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        Self {
            p0,
            p1,
            p2,
            aabb_min,
            aabb_max,
            geo_normal,
        }
    }
}

/// Paints a 3D spherical brush stamp directly across UV seams onto `tex`, optionally culling
/// back-facing or camera-occluded texels via raycast against the mesh geometry.
/// Returns `(byte_offset, previous_rgba)` entries for lightweight undo.
pub fn paint_mesh_spot_3d(
    mesh: &LowPolyMesh,
    tex: &mut MeshTexture,
    params: &PaintSpot3DParams,
) -> Vec<(usize, [u8; 4])> {
    let w = tex.width as i32;
    let h = tex.height as i32;
    let Some(expected_bytes) = (tex.width as usize)
        .checked_mul(tex.height as usize)
        .and_then(|px| px.checked_mul(4))
    else {
        return Vec::new();
    };
    if w <= 0 || h <= 0 || tex.rgba.len() < expected_bytes {
        return Vec::new();
    }
    if !params.center.is_finite() || !params.radius.is_finite() || params.radius <= 0.0 {
        return Vec::new();
    }
    let camera_pos = params.camera_pos.filter(|c| c.is_finite());
    let radius = params.radius.clamp(1e-4, 100.0);
    let radius_sq = radius * radius;

    let tri_mesh = mesh.to_triangulated_flat();
    let tri_count = tri_mesh.indices.len() / 3;
    if tri_count == 0 {
        return Vec::new();
    }

    let mut tris = Vec::with_capacity(tri_count);
    for t in 0..tri_count {
        let i0 = tri_mesh.indices[t * 3] as usize;
        let i1 = tri_mesh.indices[t * 3 + 1] as usize;
        let i2 = tri_mesh.indices[t * 3 + 2] as usize;
        let p0 = Vec3::from_array(tri_mesh.vertices[i0].position);
        let p1 = Vec3::from_array(tri_mesh.vertices[i1].position);
        let p2 = Vec3::from_array(tri_mesh.vertices[i2].position);
        tris.push(PaintTri::new(p0, p1, p2));
    }

    let (min_b, max_b) = mesh.bounds();
    let diag = (max_b - min_b).length().max(1e-3);
    let bias = (diag * 1e-3).clamp(1e-5, 1e-2);

    let brush_min = params.center - Vec3::splat(radius);
    let brush_max = params.center + Vec3::splat(radius);
    let brush_normal = params
        .brush_normal
        .map(Vec3::normalize_or_zero)
        .filter(|n| n.length_squared() > 1e-6);

    let new_px = [
        params.color.r,
        params.color.g,
        params.color.b,
        params.color.a,
    ];
    let mut undo_pixels = Vec::new();

    let wf = w as f32;
    let hf = h as f32;

    for (t, tri) in tris.iter().enumerate() {
        if !aabb_overlaps(tri.aabb_min, tri.aabb_max, brush_min, brush_max) {
            continue;
        }
        if let Some(bn) = brush_normal {
            if tri.geo_normal.dot(bn) < -0.15 {
                continue;
            }
        }

        let i0 = tri_mesh.indices[t * 3] as usize;
        let i1 = tri_mesh.indices[t * 3 + 1] as usize;
        let i2 = tri_mesh.indices[t * 3 + 2] as usize;

        let uv0 = Vec2::from_array(tri_mesh.vertices[i0].uv);
        let uv1 = Vec2::from_array(tri_mesh.vertices[i1].uv);
        let uv2 = Vec2::from_array(tri_mesh.vertices[i2].uv);
        if !uv0.is_finite() || !uv1.is_finite() || !uv2.is_finite() {
            continue;
        }

        let area = edge_fn(uv0, uv1, uv2);
        if !area.is_finite() || area.abs() < 1e-10 {
            continue;
        }
        let inv_area = 1.0 / area;

        let min_u = uv0.x.min(uv1.x).min(uv2.x).clamp(0.0, 1.0);
        let max_u = uv0.x.max(uv1.x).max(uv2.x).clamp(0.0, 1.0);
        let min_v = uv0.y.min(uv1.y).min(uv2.y).clamp(0.0, 1.0);
        let max_v = uv0.y.max(uv1.y).max(uv2.y).clamp(0.0, 1.0);

        let min_px = ((min_u * wf).floor() as i32).clamp(0, w - 1);
        let max_px = ((max_u * wf).ceil() as i32).clamp(0, w - 1);
        let min_py = ((min_v * hf).floor() as i32).clamp(0, h - 1);
        let max_py = ((max_v * hf).ceil() as i32).clamp(0, h - 1);

        for py in min_py..=max_py {
            let tv = (py as f32 + 0.5) / hf;
            for px in min_px..=max_px {
                let tu = (px as f32 + 0.5) / wf;
                let p_uv = Vec2::new(tu, tv);
                let w0 = edge_fn(uv1, uv2, p_uv) * inv_area;
                let w1 = edge_fn(uv2, uv0, p_uv) * inv_area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-3 || w1 < -1e-3 || w2 < -1e-3 {
                    continue;
                }

                let pos = tri.p0 * w0 + tri.p1 * w1 + tri.p2 * w2;
                if (pos - params.center).length_squared() > radius_sq {
                    continue;
                }

                if let Some(cam) = camera_pos {
                    let to_cam = cam - pos;
                    let dist = to_cam.length();
                    if dist > 1e-5 {
                        let dir = to_cam / dist;
                        if tri.geo_normal.dot(dir) <= 1e-4 {
                            continue;
                        }
                        let max_t = (dist - bias).max(0.0);
                        if max_t > 1e-5
                            && ray_occluded(pos + tri.geo_normal * bias, dir, &tris, t, max_t)
                        {
                            continue;
                        }
                    }
                }

                let offset = ((py as usize) * (w as usize) + (px as usize)) * 4;
                let prev = [
                    tex.rgba[offset],
                    tex.rgba[offset + 1],
                    tex.rgba[offset + 2],
                    tex.rgba[offset + 3],
                ];
                if prev != new_px {
                    undo_pixels.push((offset, prev));
                    tex.rgba[offset..offset + 4].copy_from_slice(&new_px);
                }
            }
        }
    }

    undo_pixels
}

fn edge_fn(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)
}

fn aabb_overlaps(a_min: Vec3, a_max: Vec3, b_min: Vec3, b_max: Vec3) -> bool {
    a_min.x <= b_max.x
        && a_max.x >= b_min.x
        && a_min.y <= b_max.y
        && a_max.y >= b_min.y
        && a_min.z <= b_max.z
        && a_max.z >= b_min.z
}

fn ray_occluded(
    origin: Vec3,
    dir: Vec3,
    tris: &[PaintTri],
    self_tri: usize,
    max_dist: f32,
) -> bool {
    let inv_dir = Vec3::new(
        if dir.x.abs() > 1e-8 {
            1.0 / dir.x
        } else {
            f32::INFINITY
        },
        if dir.y.abs() > 1e-8 {
            1.0 / dir.y
        } else {
            f32::INFINITY
        },
        if dir.z.abs() > 1e-8 {
            1.0 / dir.z
        } else {
            f32::INFINITY
        },
    );
    for (idx, tri) in tris.iter().enumerate() {
        if idx == self_tri {
            continue;
        }
        if !ray_aabb_hit(origin, inv_dir, tri.aabb_min, tri.aabb_max, max_dist) {
            continue;
        }
        if let Some(t) = ray_triangle_t(origin, dir, tri.p0, tri.p1, tri.p2) {
            if t > 1e-5 && t < max_dist {
                return true;
            }
        }
    }
    false
}

fn ray_aabb_hit(origin: Vec3, inv_dir: Vec3, min: Vec3, max: Vec3, max_t: f32) -> bool {
    let t1 = (min.x - origin.x) * inv_dir.x;
    let t2 = (max.x - origin.x) * inv_dir.x;
    let mut tmin = t1.min(t2);
    let mut tmax = t1.max(t2);

    let ty1 = (min.y - origin.y) * inv_dir.y;
    let ty2 = (max.y - origin.y) * inv_dir.y;
    tmin = tmin.max(ty1.min(ty2));
    tmax = tmax.min(ty1.max(ty2));

    let tz1 = (min.z - origin.z) * inv_dir.z;
    let tz2 = (max.z - origin.z) * inv_dir.z;
    tmin = tmin.max(tz1.min(tz2));
    tmax = tmax.min(tz1.max(tz2));

    tmax >= tmin.max(0.0) && tmin <= max_t
}

fn ray_triangle_t(orig: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
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
    use crate::mesh::face::Face;
    use crate::mesh::nano::PrimitiveKind;
    use crate::mesh::ops::create_primitive;
    use crate::mesh::texture::box_atlas_uv;
    use crate::palette::PaletteIndex;

    fn sample_tex_color(tex: &MeshTexture, uv: [f32; 2]) -> [u8; 4] {
        let px = ((uv[0].clamp(0.0, 1.0) * (tex.width - 1) as f32).round() as usize)
            .min(tex.width as usize - 1);
        let py = ((uv[1].clamp(0.0, 1.0) * (tex.height - 1) as f32).round() as usize)
            .min(tex.height as usize - 1);
        let off = (py * tex.width as usize + px) * 4;
        [
            tex.rgba[off],
            tex.rgba[off + 1],
            tex.rgba[off + 2],
            tex.rgba[off + 3],
        ]
    }

    #[test]
    fn paint_across_uv_seam_covers_both_adjacent_cube_faces() {
        let mat = PaletteIndex::new(3).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let mut tex = MeshTexture::solid(128, 128, Rgba8::rgb(200, 200, 200));
        let red = Rgba8::rgb(220, 40, 40);

        // Stamp centered directly on the seam edge between +X (x=0.5) and +Y (y=1.0).
        let edits = paint_mesh_spot_3d(
            &cube,
            &mut tex,
            &PaintSpot3DParams {
                center: Vec3::new(0.5, 1.0, 0.0),
                radius: 0.25,
                color: red,
                brush_normal: None,
                camera_pos: Some(Vec3::new(3.0, 3.0, 0.0)),
            },
        );
        assert!(!edits.is_empty());

        let (min_b, max_b) = cube.bounds();
        // Point on +X face near seam, and point on +Y face near seam (two distinct box-atlas UV cells).
        let uv_pos_x = box_atlas_uv(Vec3::new(0.5, 0.92, 0.0), Vec3::X, min_b, max_b);
        let uv_pos_y = box_atlas_uv(Vec3::new(0.42, 1.0, 0.0), Vec3::Y, min_b, max_b);
        assert_eq!(sample_tex_color(&tex, uv_pos_x), [220, 40, 40, 255]);
        assert_eq!(sample_tex_color(&tex, uv_pos_y), [220, 40, 40, 255]);

        // Back face (-X) must remain untouched.
        let uv_neg_x = box_atlas_uv(Vec3::new(-0.5, 0.92, 0.0), -Vec3::X, min_b, max_b);
        assert_eq!(sample_tex_color(&tex, uv_neg_x), [200, 200, 200, 255]);
    }

    #[test]
    fn camera_visibility_blocks_occluded_back_plate() {
        let mat = PaletteIndex::new(3).unwrap();
        // Two parallel front-facing (+Z) quads stacked at z = +0.5 (front) and z = -0.5 (back).
        // Explicit non-overlapping UVs: front on left half [0..0.4], back on right half [0.6..1.0].
        let mut mesh = LowPolyMesh::new();
        mesh.vertices = vec![
            // Front quad (z = 0.5, normal +Z)
            Vec3::new(-1.0, -1.0, 0.5),
            Vec3::new(1.0, -1.0, 0.5),
            Vec3::new(1.0, 1.0, 0.5),
            Vec3::new(-1.0, 1.0, 0.5),
            // Back quad (z = -0.5, normal +Z, fully occluded from camera at +Z)
            Vec3::new(-1.0, -1.0, -0.5),
            Vec3::new(1.0, -1.0, -0.5),
            Vec3::new(1.0, 1.0, -0.5),
            Vec3::new(-1.0, 1.0, -0.5),
        ];
        let mut front = Face::new(vec![0, 1, 2, 3], mat);
        front.uvs = vec![[0.05, 0.05], [0.45, 0.05], [0.45, 0.95], [0.05, 0.95]];
        let mut back = Face::new(vec![4, 5, 6, 7], mat);
        back.uvs = vec![[0.55, 0.05], [0.95, 0.05], [0.95, 0.95], [0.55, 0.95]];
        mesh.faces = vec![front, back];

        let mut tex = MeshTexture::solid(64, 64, Rgba8::rgb(180, 180, 180));
        let blue = Rgba8::rgb(40, 90, 240);

        // Large sphere radius (2.0) encompasses both plates, but camera at +Z only sees front plate.
        paint_mesh_spot_3d(
            &mesh,
            &mut tex,
            &PaintSpot3DParams {
                center: Vec3::new(0.0, 0.0, 0.5),
                radius: 2.0,
                color: blue,
                brush_normal: Some(Vec3::Z),
                camera_pos: Some(Vec3::new(0.0, 0.0, 5.0)),
            },
        );

        assert_eq!(sample_tex_color(&tex, [0.25, 0.5]), [40, 90, 240, 255]);
        assert_eq!(sample_tex_color(&tex, [0.75, 0.5]), [180, 180, 180, 255]);
    }

    #[test]
    fn brush_normal_culls_opposite_facing_side_of_thin_box() {
        let mat = PaletteIndex::new(3).unwrap();
        let mut thin = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        for v in &mut thin.vertices {
            v.z *= 0.05;
        }
        let mut tex = MeshTexture::solid(128, 128, Rgba8::rgb(200, 200, 200));
        let green = Rgba8::rgb(40, 200, 80);

        paint_mesh_spot_3d(
            &thin,
            &mut tex,
            &PaintSpot3DParams {
                center: Vec3::new(0.0, 0.5, 0.025),
                radius: 0.3,
                color: green,
                brush_normal: Some(Vec3::Z),
                camera_pos: None,
            },
        );

        let (min_b, max_b) = thin.bounds();
        let uv_front = box_atlas_uv(Vec3::new(0.0, 0.5, 0.025), Vec3::Z, min_b, max_b);
        let uv_back = box_atlas_uv(Vec3::new(0.0, 0.5, -0.025), -Vec3::Z, min_b, max_b);
        assert_eq!(sample_tex_color(&tex, uv_front), [40, 200, 80, 255]);
        assert_eq!(sample_tex_color(&tex, uv_back), [200, 200, 200, 255]);
    }
}
