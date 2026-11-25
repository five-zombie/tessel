use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::lowpoly::LowPolyMesh;
use super::texture::MeshTexture;

/// Alpha blending / cutout mode mapping 1:1 to glTF 2.0 `material.alphaMode`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask {
        cutoff: f32,
    },
    Blend,
}

/// Automated NPR / silhouette mesh hygiene & texel-density report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopologyReport {
    pub vertex_count: usize,
    pub face_count: usize,
    pub triangle_count: usize,
    pub sliver_triangles: usize,
    pub degenerate_normals: usize,
    pub non_manifold_edges: usize,
    pub min_texel_density: f32,
    pub max_texel_density: f32,
    pub texel_density_ratio: f32,
    pub is_clean: bool,
}

/// Computes averaged smooth outline normals per source vertex by merging face normals across
/// spatially coincident vertices (ignoring UV/hard-edge splits) so inverted-hull outlines never
/// crack at sharp corners.
pub fn compute_smooth_outline_normals(mesh: &LowPolyMesh) -> Vec<Vec3> {
    let quantize = |p: Vec3| -> (i64, i64, i64) {
        (
            (p.x * 10_000.0).round() as i64,
            (p.y * 10_000.0).round() as i64,
            (p.z * 10_000.0).round() as i64,
        )
    };

    let mut accum: BTreeMap<(i64, i64, i64), Vec3> = BTreeMap::new();
    for face in &mesh.faces {
        if face.vertices.len() < 3 {
            continue;
        }
        let Some(&i0) = face.vertices.first() else {
            continue;
        };
        let Some(&p0) = mesh.vertices.get(i0) else {
            continue;
        };
        for fan in 1..(face.vertices.len() - 1) {
            let (Some(&i1), Some(&i2)) = (face.vertices.get(fan), face.vertices.get(fan + 1))
            else {
                continue;
            };
            let (Some(&p1), Some(&p2)) = (mesh.vertices.get(i1), mesh.vertices.get(i2)) else {
                continue;
            };
            let cross = (p1 - p0).cross(p2 - p0);
            if cross.length_squared() >= 1e-10 {
                for p in [p0, p1, p2] {
                    *accum.entry(quantize(p)).or_insert(Vec3::ZERO) += cross;
                }
            }
        }
    }

    mesh.vertices
        .iter()
        .enumerate()
        .map(|(idx, &p)| {
            if let Some(&proxy) = mesh.proxy_normals.get(&idx) {
                let n = proxy.normalize_or_zero();
                if n.length_squared() > 1e-6 {
                    return n;
                }
            }
            let n = accum
                .get(&quantize(p))
                .copied()
                .unwrap_or(Vec3::Y)
                .normalize_or_zero();
            if n.length_squared() > 1e-6 {
                n
            } else {
                Vec3::Y
            }
        })
        .collect()
}

/// Validates mesh silhouette topology (sliver triangles, degenerate normals, non-manifold edges)
/// and measures UV texel-density uniformity.
pub fn validate_mesh_topology(mesh: &LowPolyMesh) -> TopologyReport {
    let mut edge_counts: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    let mut sliver_triangles = 0usize;
    let mut degenerate_normals = 0usize;
    let mut triangle_count = 0usize;
    let mut min_td = f32::INFINITY;
    let mut max_td = 0.0_f32;

    let (min_b, max_b) = mesh.bounds();

    for face in &mesh.faces {
        let len = face.vertices.len();
        if len < 3 {
            continue;
        }
        for e in 0..len {
            let a = face.vertices[e];
            let b = face.vertices[(e + 1) % len];
            let key = if a < b { (a, b) } else { (b, a) };
            *edge_counts.entry(key).or_insert(0) += 1;
        }

        let Some(&i0) = face.vertices.first() else {
            continue;
        };
        let Some(&p0) = mesh.vertices.get(i0) else {
            continue;
        };
        let has_uvs = face.uvs.len() == len;

        for fan in 1..(len - 1) {
            let (Some(&i1), Some(&i2)) = (face.vertices.get(fan), face.vertices.get(fan + 1))
            else {
                continue;
            };
            let (Some(&p1), Some(&p2)) = (mesh.vertices.get(i1), mesh.vertices.get(i2)) else {
                continue;
            };
            triangle_count += 1;
            let cross = (p1 - p0).cross(p2 - p0);
            let area2 = cross.length();
            if !area2.is_finite() || area2 < 1e-6 {
                degenerate_normals += 1;
                continue;
            }
            let perim = (p1 - p0).length() + (p2 - p1).length() + (p0 - p2).length();
            let quality = if perim > 1e-6 {
                (0.5 * area2) / (perim * perim)
            } else {
                0.0
            };
            if quality < 1e-3 {
                sliver_triangles += 1;
            }

            let fn_dir = cross / area2;
            let uv0 = if has_uvs {
                face.uvs[0]
            } else {
                super::texture::box_atlas_uv(p0, fn_dir, min_b, max_b)
            };
            let uv1 = if has_uvs {
                face.uvs[fan]
            } else {
                super::texture::box_atlas_uv(p1, fn_dir, min_b, max_b)
            };
            let uv2 = if has_uvs {
                face.uvs[fan + 1]
            } else {
                super::texture::box_atlas_uv(p2, fn_dir, min_b, max_b)
            };
            let uv_area = 0.5
                * ((uv1[0] - uv0[0]) * (uv2[1] - uv0[1]) - (uv2[0] - uv0[0]) * (uv1[1] - uv0[1]))
                    .abs();
            let world_area = 0.5 * area2;
            if world_area > 1e-8 && uv_area > 1e-10 {
                let td = (uv_area / world_area).sqrt();
                min_td = min_td.min(td);
                max_td = max_td.max(td);
            }
        }
    }

    let non_manifold_edges = edge_counts.values().filter(|&&c| c > 2).count();
    let (min_td, max_td, ratio) = if min_td.is_finite() && max_td > 0.0 {
        (min_td, max_td, max_td / min_td.max(1e-6))
    } else {
        (0.0, 0.0, 1.0)
    };

    TopologyReport {
        vertex_count: mesh.vertices.len(),
        face_count: mesh.faces.len(),
        triangle_count,
        sliver_triangles,
        degenerate_normals,
        non_manifold_edges,
        min_texel_density: min_td,
        max_texel_density: max_td,
        texel_density_ratio: ratio,
        is_clean: sliver_triangles == 0 && degenerate_normals == 0 && non_manifold_edges == 0,
    }
}

/// Automatic UV gutter dilation: bleeds opaque pixels (`alpha > 0`) outward into transparent
/// gutter pixels for `passes` iterations so mipmapping never bleeds black UV borders.
pub fn dilate_rgba_texture(tex: &MeshTexture, passes: u32) -> MeshTexture {
    if passes == 0 || tex.width == 0 || tex.height == 0 {
        return tex.clone();
    }
    let w = tex.width as i32;
    let h = tex.height as i32;
    let Some(expected) = (tex.width as usize)
        .checked_mul(tex.height as usize)
        .and_then(|px| px.checked_mul(4))
    else {
        return tex.clone();
    };
    if tex.rgba.len() < expected {
        return tex.clone();
    }

    let mut cur = tex.rgba[..expected].to_vec();
    let mut next = cur.clone();
    const OFFSETS: [(i32, i32); 8] = [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ];

    for _ in 0..passes.clamp(0, 16) {
        let mut changed = false;
        for y in 0..h {
            for x in 0..w {
                let off = ((y as usize) * (w as usize) + (x as usize)) * 4;
                if cur[off + 3] > 0 {
                    continue;
                }
                let mut sr = 0u32;
                let mut sg = 0u32;
                let mut sb = 0u32;
                let mut cnt = 0u32;
                for (dx, dy) in OFFSETS {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx >= 0 && nx < w && ny >= 0 && ny < h {
                        let noff = ((ny as usize) * (w as usize) + (nx as usize)) * 4;
                        if cur[noff + 3] > 0 {
                            sr += u32::from(cur[noff]);
                            sg += u32::from(cur[noff + 1]);
                            sb += u32::from(cur[noff + 2]);
                            cnt += 1;
                        }
                    }
                }
                if let (Some(r), Some(g), Some(b)) = (
                    sr.checked_div(cnt),
                    sg.checked_div(cnt),
                    sb.checked_div(cnt),
                ) {
                    next[off] = r as u8;
                    next[off + 1] = g as u8;
                    next[off + 2] = b as u8;
                    next[off + 3] = 255;
                    changed = true;
                }
            }
        }
        cur.copy_from_slice(&next);
        if !changed {
            break;
        }
    }

    MeshTexture {
        width: tex.width,
        height: tex.height,
        rgba: cur,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::nano::PrimitiveKind;
    use crate::mesh::ops::create_primitive;
    use crate::palette::PaletteIndex;

    #[test]
    fn smooth_outline_normals_and_topology_report_on_cube() {
        let mat = PaletteIndex::new(3).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let normals = compute_smooth_outline_normals(&cube);
        assert_eq!(normals.len(), cube.vertices.len());
        // Corner vertex (0.5, 1.0, 0.5) should point diagonally outward ~(1,1,1)/sqrt(3).
        let corner_n = normals[2];
        assert!(corner_n.x > 0.4 && corner_n.y > 0.4 && corner_n.z > 0.4);

        let report = validate_mesh_topology(&cube);
        assert!(report.is_clean);
        assert_eq!(report.triangle_count, 12);
        assert_eq!(report.non_manifold_edges, 0);
        assert!(report.texel_density_ratio >= 1.0);
    }

    #[test]
    fn dilate_rgba_bleeds_opaque_pixel_into_transparent_neighbors() {
        let mut tex = MeshTexture::solid(16, 16, crate::palette::Rgba8::rgba(0, 0, 0, 0));
        let center = (8 * 16 + 8) * 4;
        tex.rgba[center..center + 4].copy_from_slice(&[200, 100, 50, 255]);
        let dilated = dilate_rgba_texture(&tex, 2);
        let neighbor = (8 * 16 + 10) * 4;
        assert_eq!(&dilated.rgba[neighbor..neighbor + 4], &[200, 100, 50, 255]);
    }
}
