use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::mesh::{LowPolyMesh, MeshTexture};
use crate::palette::Rgba8;
use crate::scene::{NodeId, SceneState};

/// Non-visual collision hull metadata attached to a scene node (`v5.8`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum CollisionHull {
    Box {
        half_extents: [f32; 3],
        offset: [f32; 3],
    },
    Capsule {
        radius: f32,
        height: f32,
        offset: [f32; 3],
    },
    ConvexHull {
        points: Vec<[f32; 3]>,
    },
}

/// Non-destructive mesh modifier stack entry (`v5.9`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeshModifier {
    Mirror {
        axis: u8,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    Bevel {
        width: f32,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    Solidify {
        thickness: f32,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    Decimate {
        ratio: f32,
        #[serde(default = "default_true")]
        enabled: bool,
    },
}

fn default_true() -> bool {
    true
}

/// Severity level for `preflight_check_scene` (`v5.9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightSeverity {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightIssue {
    pub severity: PreflightSeverity,
    pub code: String,
    pub node_id: Option<u64>,
    pub message: String,
}

/// Unified pre-export QA report (`v5.9`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub status: PreflightSeverity,
    pub total_triangles: usize,
    pub max_triangles: usize,
    pub issues: Vec<PreflightIssue>,
}

/// One-command mesh cleanup: welds near-duplicate vertices and drops degenerate faces (`v5.8`).
pub fn cleanup_mesh(mesh: &mut LowPolyMesh, weld_epsilon: f32) -> (usize, usize) {
    let before_verts = mesh.vertices.len();
    let eps = if weld_epsilon.is_finite() && weld_epsilon > 0.0 {
        weld_epsilon
    } else {
        1e-4
    };
    if let Ok((welded, _)) = crate::mesh::ops::weld_vertices(mesh, eps) {
        *mesh = welded;
    }
    let before_faces = mesh.faces.len();
    mesh.faces.retain(|f| {
        if f.vertices.len() < 3 {
            return false;
        }
        let (Some(&i0), Some(&i1), Some(&i2)) =
            (f.vertices.first(), f.vertices.get(1), f.vertices.get(2))
        else {
            return false;
        };
        let (Some(&p0), Some(&p1), Some(&p2)) = (
            mesh.vertices.get(i0),
            mesh.vertices.get(i1),
            mesh.vertices.get(i2),
        ) else {
            return false;
        };
        (p1 - p0).cross(p2 - p0).length_squared() > 1e-12
    });
    (
        before_verts.saturating_sub(mesh.vertices.len()),
        before_faces.saturating_sub(mesh.faces.len()),
    )
}

/// Packs Occlusion (R), Roughness (G), and Metallic (B) channels into a single glTF ORM texture (`v5.8`).
pub fn pack_orm_texture(
    ao: Option<&MeshTexture>,
    roughness: Option<&MeshTexture>,
    metallic: Option<&MeshTexture>,
    default_roughness: f32,
    default_metallic: f32,
) -> Option<MeshTexture> {
    let (w, h) = ao.or(roughness).or(metallic).map(|t| (t.width, t.height))?;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    for tex in [ao, roughness, metallic].into_iter().flatten() {
        if tex.width != w || tex.height != h {
            return None;
        }
    }
    let pixel_count = (w as usize).checked_mul(h as usize)?;
    let byte_count = pixel_count.checked_mul(4)?;
    let mut rgba = vec![255u8; byte_count];
    let def_r = if default_roughness.is_finite() {
        (default_roughness.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        204
    };
    let def_m = if default_metallic.is_finite() {
        (default_metallic.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        0
    };
    for i in 0..pixel_count {
        let base = i * 4;
        rgba[base] = ao
            .filter(|t| t.rgba.len() >= byte_count)
            .map_or(255, |t| t.rgba[base]);
        rgba[base + 1] = roughness
            .filter(|t| t.rgba.len() >= byte_count)
            .map_or(def_r, |t| t.rgba[base + 1]);
        rgba[base + 2] = metallic
            .filter(|t| t.rgba.len() >= byte_count)
            .map_or(def_m, |t| t.rgba[base + 2]);
        rgba[base + 3] = 255;
    }
    Some(MeshTexture {
        width: w,
        height: h,
        rgba,
    })
}

/// Evaluates a non-destructive `MeshModifier` stack over `base` without mutating `base` (`v5.9`).
pub fn evaluate_modifier_stack(base: &LowPolyMesh, modifiers: &[MeshModifier]) -> LowPolyMesh {
    let mut out = base.clone();
    for m in modifiers {
        match *m {
            MeshModifier::Mirror { axis, enabled } if enabled => {
                if let Ok(m) = crate::mesh::ops::mirror_mesh(&out, axis.min(2), true) {
                    out = m;
                }
            }
            MeshModifier::Bevel { width, enabled }
                if enabled && width.is_finite() && width > 0.0 =>
            {
                if let Ok(m) = crate::mesh::ops::bevel_mesh(&out, width.clamp(0.05, 0.45)) {
                    out = m;
                }
            }
            MeshModifier::Solidify { thickness, enabled }
                if enabled && thickness.is_finite() && thickness.abs() > 1e-5 =>
            {
                let orig_v = out.vertices.len();
                let mut normals = vec![Vec3::ZERO; orig_v];
                for f in &out.faces {
                    if f.vertices.len() < 3 {
                        continue;
                    }
                    let (Some(&i0), Some(&i1), Some(&i2)) =
                        (f.vertices.first(), f.vertices.get(1), f.vertices.get(2))
                    else {
                        continue;
                    };
                    if let (Some(&p0), Some(&p1), Some(&p2)) = (
                        out.vertices.get(i0),
                        out.vertices.get(i1),
                        out.vertices.get(i2),
                    ) {
                        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                        for &vi in &f.vertices {
                            if let Some(acc) = normals.get_mut(vi) {
                                *acc += n;
                            }
                        }
                    }
                }
                for (i, n) in normals.into_iter().enumerate() {
                    let dir = if n.length_squared() > 1e-6 {
                        n.normalize()
                    } else {
                        Vec3::Y
                    };
                    out.vertices.push(out.vertices[i] - dir * thickness);
                }
                let orig_faces = out.faces.clone();
                for mut f in orig_faces {
                    if f.vertices.iter().any(|&vi| vi >= orig_v) {
                        continue;
                    }
                    f.vertices = f.vertices.into_iter().rev().map(|vi| vi + orig_v).collect();
                    if f.uvs.len() == f.vertices.len() {
                        f.uvs.reverse();
                    }
                    out.faces.push(f);
                }
            }
            MeshModifier::Decimate { ratio, enabled }
                if enabled && ratio.is_finite() && ratio > 0.0 && ratio < 1.0 =>
            {
                if let Ok(m) = crate::mesh::ops::decimate_mesh(&out, ratio.clamp(0.05, 0.99)) {
                    out = m;
                }
            }
            _ => {}
        }
    }
    out
}

/// Runs unified pre-export QA checks across all meshes, textures, and naming conventions (`v5.9`).
pub fn preflight_check_scene(scene: &SceneState, max_triangles: usize) -> PreflightReport {
    let mut issues = Vec::new();
    let mut total_triangles = 0usize;
    let mut seen_names: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();

    for (&nid, node) in &scene.nodes {
        seen_names.entry(node.name.clone()).or_default().push(nid);
    }
    for (name, ids) in &seen_names {
        if ids.len() > 1 {
            issues.push(PreflightIssue {
                severity: PreflightSeverity::Warn,
                code: "DUPLICATE_NODE_NAME".into(),
                node_id: ids.first().map(|n| n.0),
                message: format!("duplicate node name '{name}' used {} times", ids.len()),
            });
        }
    }

    for (&nid, mesh) in &scene.lowpoly_meshes {
        let inst_mult = scene
            .mesh_instances
            .get(&nid)
            .map_or(1usize, |v| v.len().max(1));
        let tris: usize = mesh
            .faces
            .iter()
            .map(|f| f.vertices.len().saturating_sub(2))
            .sum::<usize>()
            .saturating_mul(inst_mult);
        total_triangles = total_triangles.saturating_add(tris);

        if mesh.faces.is_empty() {
            issues.push(PreflightIssue {
                severity: PreflightSeverity::Fail,
                code: "EMPTY_MESH".into(),
                node_id: Some(nid.0),
                message: "mesh has zero faces".into(),
            });
            continue;
        }
        let topo = crate::mesh::npr::validate_mesh_topology(mesh);
        if topo.degenerate_normals > 0 {
            issues.push(PreflightIssue {
                severity: PreflightSeverity::Warn,
                code: "DEGENERATE_FACES".into(),
                node_id: Some(nid.0),
                message: format!("{} degenerate face(s) detected", topo.degenerate_normals),
            });
        }
        if topo.non_manifold_edges > 0 {
            issues.push(PreflightIssue {
                severity: PreflightSeverity::Warn,
                code: "NON_MANIFOLD_EDGES".into(),
                node_id: Some(nid.0),
                message: format!("{} non-manifold edge(s) detected", topo.non_manifold_edges),
            });
        }
        if mesh.texture.is_some() && mesh.faces.iter().any(|f| f.uvs.len() != f.vertices.len()) {
            issues.push(PreflightIssue {
                severity: PreflightSeverity::Warn,
                code: "MISSING_UVS".into(),
                node_id: Some(nid.0),
                message: "textured mesh has faces with missing or mismatched UVs".into(),
            });
        }
        for (label, opt_tex) in [
            ("albedo", mesh.texture.as_ref()),
            ("normal_map", mesh.normal_map.as_ref()),
            ("ao_map", mesh.ao_map.as_ref()),
        ] {
            if let Some(tex) = opt_tex {
                if tex.width > 2048 || tex.height > 2048 {
                    issues.push(PreflightIssue {
                        severity: PreflightSeverity::Warn,
                        code: "OVERSIZED_TEXTURE".into(),
                        node_id: Some(nid.0),
                        message: format!(
                            "{label} texture {}x{} exceeds 2048px budget",
                            tex.width, tex.height
                        ),
                    });
                }
            }
        }
    }

    if total_triangles > max_triangles {
        issues.push(PreflightIssue {
            severity: PreflightSeverity::Warn,
            code: "POLYCOUNT_BUDGET".into(),
            node_id: None,
            message: format!(
                "scene triangle count ({total_triangles}) exceeds budget ({max_triangles})"
            ),
        });
    }

    let status = if issues.iter().any(|i| i.severity == PreflightSeverity::Fail) {
        PreflightSeverity::Fail
    } else if issues.iter().any(|i| i.severity == PreflightSeverity::Warn) {
        PreflightSeverity::Warn
    } else {
        PreflightSeverity::Pass
    };

    PreflightReport {
        status,
        total_triangles,
        max_triangles,
        issues,
    }
}

/// Result of extracting silhouette bounds and dominant palette colors from an RGBA image (`E7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSilhouettePalette {
    pub width: u32,
    pub height: u32,
    pub opaque_pixels: usize,
    pub bbox_min: [u32; 2],
    pub bbox_max: [u32; 2],
    pub aspect_ratio: f32,
    pub palette_hex: Vec<String>,
}

/// Extracts opaque silhouette bounding box and top `max_colors` quantized palette colors from RGBA8 bytes (`E7`).
pub fn extract_rgba_silhouette_and_palette(
    rgba: &[u8],
    width: u32,
    height: u32,
    max_colors: usize,
) -> Option<ImageSilhouettePalette> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() < expected {
        return None;
    }
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut opaque = 0usize;
    let mut hist: BTreeMap<[u8; 3], usize> = BTreeMap::new();

    for y in 0..height {
        for x in 0..width {
            let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
            let a = rgba[idx + 3];
            if a >= 16 {
                opaque += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                // Quantize to 5-bit per channel for clean dominant color grouping.
                let q = [rgba[idx] & 0xF8, rgba[idx + 1] & 0xF8, rgba[idx + 2] & 0xF8];
                *hist.entry(q).or_insert(0) += 1;
            }
        }
    }
    let mut sorted: Vec<([u8; 3], usize)> = hist.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    let cap = max_colors.clamp(1, 32);
    let palette_hex = sorted
        .into_iter()
        .take(cap)
        .map(|(rgb, _)| {
            let c = Rgba8::rgb(rgb[0], rgb[1], rgb[2]);
            format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
        })
        .collect();
    let (bbox_min, bbox_max, aspect_ratio) = if opaque > 0 {
        let bw = (max_x - min_x + 1) as f32;
        let bh = (max_y - min_y + 1) as f32;
        ([min_x, min_y], [max_x, max_y], bw / bh.max(1.0))
    } else {
        ([0, 0], [0, 0], 1.0)
    };
    Some(ImageSilhouettePalette {
        width,
        height,
        opaque_pixels: opaque,
        bbox_min,
        bbox_max,
        aspect_ratio,
        palette_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ops::create_primitive;
    use crate::mesh::PrimitiveKind;
    use crate::palette::PaletteIndex;

    #[test]
    fn orm_packing_modifier_stack_preflight_and_silhouette_work() {
        let mat = PaletteIndex::new(1).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let solid = evaluate_modifier_stack(
            &cube,
            &[MeshModifier::Solidify {
                thickness: 0.05,
                enabled: true,
            }],
        );
        assert_eq!(solid.faces.len(), cube.faces.len() * 2);

        let ao = MeshTexture {
            width: 2,
            height: 2,
            rgba: vec![200; 16],
        };
        let orm = pack_orm_texture(Some(&ao), None, None, 0.8, 0.2).unwrap();
        assert_eq!(orm.rgba[0], 200);
        assert_eq!(orm.rgba[1], 204);
        assert_eq!(orm.rgba[2], 51);

        let mut rgba = vec![0u8; 4 * 4 * 4];
        // Opaque red pixel at (1, 2)
        let idx = (2 * 4 + 1) * 4;
        rgba[idx] = 240;
        rgba[idx + 1] = 16;
        rgba[idx + 2] = 16;
        rgba[idx + 3] = 255;
        let sil = extract_rgba_silhouette_and_palette(&rgba, 4, 4, 4).unwrap();
        assert_eq!(sil.opaque_pixels, 1);
        assert_eq!(sil.bbox_min, [1, 2]);
        assert_eq!(sil.bbox_max, [1, 2]);
        assert_eq!(sil.palette_hex.len(), 1);
    }
}
