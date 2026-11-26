use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

use glam::Vec3;
use tessel_core::{
    Face, LowPolyMesh, MeshTexture, MorphTarget, PaletteIndex, SceneState, VertexSkinWeights,
};

use crate::error::IoError;
use crate::gltf::CollectedMesh;
use crate::ExportPreset;

/// Exports visible scene 3D geometry to Wavefront `.obj` text.
pub fn export_obj_string(
    scene: &SceneState,
    preset: ExportPreset,
) -> Result<(String, usize), IoError> {
    let mesh = CollectedMesh::from_scene(scene, preset)?;
    let mut out = String::from("# Exported by Tessel\no TesselAsset\n");

    for p in &mesh.positions {
        let _ = writeln!(out, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for n in &mesh.normals {
        let _ = writeln!(out, "vn {:.6} {:.6} {:.6}", n[0], n[1], n[2]);
    }
    for uv in &mesh.uvs {
        let _ = writeln!(out, "vt {:.6} {:.6}", uv[0], uv[1]);
    }
    for tri in mesh.indices.as_chunks::<3>().0 {
        let a = tri[0] + 1;
        let b = tri[1] + 1;
        let c = tri[2] + 1;
        let _ = writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
    }

    Ok((out, mesh.indices.len() / 3))
}

/// Exports visible scene 3D geometry to binary `.stl` (suitable for 3D printing).
pub fn export_stl_bytes(
    scene: &SceneState,
    preset: ExportPreset,
) -> Result<(Vec<u8>, usize), IoError> {
    let mesh = CollectedMesh::from_scene(scene, preset)?;
    let tri_count = mesh.indices.len() / 3;
    let mut out = Vec::with_capacity(84 + tri_count * 50);

    let mut header = [0u8; 80];
    let label = b"Exported by Tessel";
    header[..label.len()].copy_from_slice(label);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(tri_count as u32).to_le_bytes());

    for tri in mesh.indices.as_chunks::<3>().0 {
        let p0 = mesh.positions[tri[0] as usize];
        let p1 = mesh.positions[tri[1] as usize];
        let p2 = mesh.positions[tri[2] as usize];
        let n = mesh.normals[tri[0] as usize];

        for v in [n, p0, p1, p2] {
            for f in v {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }

    Ok((out, tri_count))
}

const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMPORT_VERTICES: usize = 500_000;
const MAX_IMPORT_FACES: usize = 500_000;
const MAX_IMPORT_ACCESSOR_ITEMS: usize = 2_000_000;
const MAX_IMPORT_MORPH_TARGETS: usize = 128;
const MAX_OBJ_FACE_VERTICES: usize = 256;

/// Imports a Wavefront `.obj` text document into a `LowPolyMesh`, preserving `vt` UVs when present.
pub fn import_obj_str(obj_text: &str, material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    if obj_text.len() > MAX_IMPORT_BYTES {
        return Err(IoError::PayloadTooLarge(
            "OBJ text exceeds 64 MiB safety limit".into(),
        ));
    }
    let mut vertices = Vec::new();
    let mut tex_coords: Vec<[f32; 2]> = Vec::new();
    let mut faces = Vec::new();

    for raw_line in obj_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(tag) = parts.next() else {
            continue;
        };
        match tag {
            "v" => {
                let x: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex x".into()))?;
                let y: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex y".into()))?;
                let z: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| IoError::InvalidFormat("invalid OBJ vertex z".into()))?;
                let v = Vec3::new(x, y, z);
                if !v.is_finite() {
                    return Err(IoError::InvalidFormat(
                        "non-finite OBJ vertex coordinate".into(),
                    ));
                }
                if vertices.len() >= MAX_IMPORT_VERTICES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "OBJ vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
                vertices.push(v);
            }
            "vt" => {
                if tex_coords.len() >= MAX_IMPORT_ACCESSOR_ITEMS {
                    return Err(IoError::PayloadTooLarge(format!(
                        "OBJ texcoord count exceeds limit {MAX_IMPORT_ACCESSOR_ITEMS}"
                    )));
                }
                if let (Some(u), Some(v)) = (
                    parts.next().and_then(|s| s.parse::<f32>().ok()),
                    parts.next().and_then(|s| s.parse::<f32>().ok()),
                ) {
                    if u.is_finite() && v.is_finite() {
                        tex_coords.push([u, 1.0 - v]);
                    }
                }
            }
            "f" => {
                let mut indices = Vec::new();
                let mut face_uvs = Vec::new();
                for token in parts {
                    let mut segs = token.split('/');
                    let v_str = segs.next().unwrap_or("");
                    let raw_idx: isize = v_str
                        .parse()
                        .map_err(|_| IoError::InvalidFormat("invalid OBJ face index".into()))?;
                    let resolved = if raw_idx > 0 {
                        (raw_idx - 1) as usize
                    } else if raw_idx < 0 {
                        let back = raw_idx.unsigned_abs();
                        vertices
                            .len()
                            .checked_sub(back)
                            .ok_or_else(|| IoError::InvalidFormat("OBJ index underflow".into()))?
                    } else {
                        return Err(IoError::InvalidFormat("OBJ index cannot be 0".into()));
                    };
                    if resolved >= vertices.len() {
                        return Err(IoError::InvalidFormat(
                            "OBJ face index out of bounds".into(),
                        ));
                    }
                    if indices.len() >= MAX_OBJ_FACE_VERTICES {
                        return Err(IoError::PayloadTooLarge(format!(
                            "OBJ face vertex count exceeds limit {MAX_OBJ_FACE_VERTICES}"
                        )));
                    }
                    indices.push(resolved);
                    if let Some(vt_str) = segs.next().filter(|s| !s.is_empty()) {
                        if let Ok(vt_raw) = vt_str.parse::<isize>() {
                            let vt_idx = if vt_raw > 0 {
                                Some((vt_raw - 1) as usize)
                            } else if vt_raw < 0 {
                                tex_coords.len().checked_sub(vt_raw.unsigned_abs())
                            } else {
                                None
                            };
                            if let Some(uv) = vt_idx.and_then(|i| tex_coords.get(i).copied()) {
                                face_uvs.push(uv);
                            }
                        }
                    }
                }
                if indices.len() >= 3 {
                    if faces.len() >= MAX_IMPORT_FACES {
                        return Err(IoError::PayloadTooLarge(format!(
                            "OBJ face count exceeds limit {MAX_IMPORT_FACES}"
                        )));
                    }
                    if face_uvs.len() == indices.len() {
                        faces.push(Face::with_uvs(indices, material, face_uvs));
                    } else {
                        faces.push(Face::new(indices, material));
                    }
                }
            }
            _ => {}
        }
    }

    if vertices.is_empty() || faces.is_empty() {
        return Err(IoError::InvalidFormat(
            "OBJ file contains no valid mesh faces".into(),
        ));
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights: Default::default(),
        morphs: Default::default(),
    })
}

/// Imports a binary glTF 2.0 (`.glb`) buffer into a `LowPolyMesh`, preserving `TEXCOORD_0` UVs.
pub fn import_glb_bytes(bytes: &[u8], material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(IoError::PayloadTooLarge(
            "GLB buffer exceeds 64 MiB safety limit".into(),
        ));
    }
    let gltf = gltf::Gltf::from_slice(bytes)
        .map_err(|e| IoError::InvalidFormat(format!("invalid GLB: {e}")))?;
    let blob = gltf.blob.as_deref();

    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    let mut skin_weights = BTreeMap::new();
    let mut morphs: Vec<MorphTarget> = Vec::new();

    for mesh in gltf.meshes() {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            if let Some(pos_acc) = prim.get(&gltf::Semantic::Positions) {
                if pos_acc.count() > MAX_IMPORT_ACCESSOR_ITEMS
                    || vertices.len().saturating_add(pos_acc.count()) > MAX_IMPORT_VERTICES
                {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
            }
            if let Some(idx_acc) = prim.indices() {
                if idx_acc.count() > MAX_IMPORT_ACCESSOR_ITEMS
                    || faces.len().saturating_add(idx_acc.count() / 3) > MAX_IMPORT_FACES
                {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB index/face count exceeds safety limit ({MAX_IMPORT_FACES} faces)"
                    )));
                }
            }
            for semantic in [
                gltf::Semantic::Joints(0),
                gltf::Semantic::Weights(0),
                gltf::Semantic::TexCoords(0),
            ] {
                if let Some(acc) = prim.get(&semantic) {
                    if acc.count() > MAX_IMPORT_ACCESSOR_ITEMS {
                        return Err(IoError::PayloadTooLarge(format!(
                            "GLB accessor count {} exceeds limit {MAX_IMPORT_ACCESSOR_ITEMS}",
                            acc.count()
                        )));
                    }
                }
            }
            if morphs.len().saturating_add(prim.morph_targets().len()) > MAX_IMPORT_MORPH_TARGETS {
                return Err(IoError::PayloadTooLarge(format!(
                    "GLB morph target count exceeds limit {MAX_IMPORT_MORPH_TARGETS}"
                )));
            }
            let reader = prim.reader(|_| blob);
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let base = vertices.len();
            for p in positions {
                if vertices.len() >= MAX_IMPORT_VERTICES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB vertex count exceeds limit {MAX_IMPORT_VERTICES}"
                    )));
                }
                let v = Vec3::from_array(p);
                if !v.is_finite() {
                    return Err(IoError::InvalidFormat(
                        "non-finite GLB vertex position".into(),
                    ));
                }
                vertices.push(v);
            }
            let added = vertices.len() - base;
            if added < 3 {
                continue;
            }
            // Round-trip skinning: JOINTS_0/WEIGHTS_0 become manual overrides.
            // All-zero rows are skipped so the auto solver still covers them.
            if let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) {
                let joints: Vec<[u16; 4]> = joints.into_u16().collect();
                let weights: Vec<[f32; 4]> = weights.into_f32().collect();
                for (i, (j, w)) in joints.iter().zip(weights.iter()).enumerate() {
                    if i >= added || !w.iter().all(|x| x.is_finite()) {
                        continue;
                    }
                    if w.iter().any(|x| *x > 0.0) {
                        skin_weights.insert(
                            base + i,
                            VertexSkinWeights {
                                joints: *j,
                                weights: *w,
                            },
                        );
                    }
                }
            }
            // Round-trip blend shapes: target POSITION + NORMAL deltas and mesh default weights.
            let mesh_weights = mesh.weights().unwrap_or(&[]);
            for (t_idx, (positions, normals, _)) in reader.read_morph_targets().enumerate() {
                let Some(positions) = positions else {
                    continue;
                };
                let name = format!("Morph{}", morphs.len());
                let mut map = BTreeMap::new();
                for (i, d) in positions.enumerate() {
                    if i >= added {
                        break;
                    }
                    let v = Vec3::from_array(d);
                    if v.is_finite() && v.length_squared() > 1e-12 {
                        map.insert(base + i, v);
                    }
                }
                let mut normal_map = BTreeMap::new();
                if let Some(normals) = normals {
                    for (i, nd) in normals.enumerate() {
                        if i >= added {
                            break;
                        }
                        let v = Vec3::from_array(nd);
                        if v.is_finite() && v.length_squared() > 1e-12 {
                            normal_map.insert(base + i, v);
                        }
                    }
                }
                let default_weight = mesh_weights
                    .get(t_idx)
                    .copied()
                    .filter(|w| w.is_finite())
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                if !map.is_empty() || !normal_map.is_empty() {
                    morphs.push(MorphTarget {
                        name,
                        deltas: map,
                        normal_deltas: normal_map,
                        default_weight,
                    });
                }
            }
            let prim_uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|tc| {
                    tc.into_f32()
                        .map(|uv| {
                            if uv[0].is_finite() && uv[1].is_finite() {
                                uv
                            } else {
                                [0.0, 0.0]
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let indices: Vec<usize> = match reader.read_indices() {
                Some(iter) => iter.into_u32().map(|i| base + i as usize).collect(),
                None => (base..base + added).collect(),
            };
            for tri in indices.as_chunks::<3>().0 {
                if faces.len() >= MAX_IMPORT_FACES {
                    return Err(IoError::PayloadTooLarge(format!(
                        "GLB face count exceeds limit {MAX_IMPORT_FACES}"
                    )));
                }
                if tri[0] < vertices.len() && tri[1] < vertices.len() && tri[2] < vertices.len() {
                    let (l0, l1, l2) = (tri[0] - base, tri[1] - base, tri[2] - base);
                    if l0 < prim_uvs.len() && l1 < prim_uvs.len() && l2 < prim_uvs.len() {
                        faces.push(Face::with_uvs(
                            vec![tri[0], tri[1], tri[2]],
                            material,
                            vec![prim_uvs[l0], prim_uvs[l1], prim_uvs[l2]],
                        ));
                    } else {
                        faces.push(Face::new(vec![tri[0], tri[1], tri[2]], material));
                    }
                }
            }
        }
    }

    if vertices.is_empty() || faces.is_empty() {
        return Err(IoError::InvalidFormat(
            "GLB file contains no triangle geometry".into(),
        ));
    }
    Ok(LowPolyMesh {
        vertices,
        faces,
        texture: None,
        normal_map: None,
        ao_map: None,
        paint_layers: Vec::new(),
        active_paint_layer: None,
        vertex_colors: BTreeMap::new(),
        crease_edges: Vec::new(),
        proxy_normals: BTreeMap::new(),
        smooth_shading: false,
        material: Default::default(),
        skin_weights,
        morphs,
    })
}

/// Imports a `.obj` or `.glb` 3D mesh from disk, returning `(asset_name, mesh)`.
pub fn import_mesh_from_path(
    path: &Path,
    material: PaletteIndex,
) -> Result<(String, LowPolyMesh), IoError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported Mesh")
        .to_string();

    let meta = fs::metadata(path)?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(IoError::InvalidFormat(
            "mesh file exceeds 64 MiB safety limit".into(),
        ));
    }
    let mesh = match ext.as_str() {
        "obj" => {
            let text = fs::read_to_string(path)?;
            import_obj_str(&text, material)?
        }
        "glb" => {
            let bytes = fs::read(path)?;
            import_glb_bytes(&bytes, material)?
        }
        other => {
            return Err(IoError::InvalidFormat(format!(
                "unsupported mesh format '.{other}' (expected .obj or .glb)"
            )));
        }
    };
    Ok((name, mesh))
}

/// Loads a custom `.png` or `.jpg`/`.jpeg` texture image (e.g. from Blender, Poly Haven, AmbientCG)
/// into a `MeshTexture`, downscaling safely if larger than 1024x1024.
const MAX_IMPORT_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMPORT_IMAGE_DIM: u32 = 4096;
const MAX_IMPORT_IMAGE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

pub fn import_mesh_texture_from_path(path: &Path) -> Result<MeshTexture, IoError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_IMPORT_IMAGE_FILE_BYTES {
        return Err(IoError::ExportPathDenied(
            "image file exceeds 32 MiB import limit".into(),
        ));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMPORT_IMAGE_DIM);
    limits.max_image_height = Some(MAX_IMPORT_IMAGE_DIM);
    limits.max_alloc = Some(MAX_IMPORT_IMAGE_ALLOC_BYTES);
    reader.limits(limits);
    let mut img = reader.decode()?;
    if img.width() > 1024 || img.height() > 1024 {
        img = img.thumbnail(1024, 1024);
    }
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(IoError::InvalidDimensions(width, height));
    }
    Ok(MeshTexture {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gltf::export_glb_bytes;
    use tessel_core::{Command, PaletteIndex, PrimitiveKind};

    #[test]
    fn obj_and_stl_export_valid_triangle_counts() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(4).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();

        let (obj, obj_tris) = export_obj_string(&scene, ExportPreset::Godot4).unwrap();
        assert_eq!(obj_tris, 12);
        assert!(obj.contains("v "));
        assert!(obj.contains("f "));

        let (stl, stl_tris) = export_stl_bytes(&scene, ExportPreset::Print3D).unwrap();
        assert_eq!(stl_tris, 12);
        assert_eq!(stl.len(), 84 + 12 * 50);
    }

    #[test]
    fn obj_and_glb_export_import_roundtrip() {
        let mut scene = SceneState::new();
        let mat = PaletteIndex::new(4).unwrap();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();

        let (obj, _) = export_obj_string(&scene, ExportPreset::Godot4).unwrap();
        let imported_obj = import_obj_str(&obj, mat).unwrap();
        assert_eq!(imported_obj.faces.len(), 12);

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let imported_glb = import_glb_bytes(&glb, mat).unwrap();
        assert_eq!(imported_glb.faces.len(), 12);
    }

    #[test]
    fn glb_roundtrip_preserves_weights_and_morphs() {
        let mut scene = SceneState::new();
        let mat = PaletteIndex::new(4).unwrap();
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: mat,
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();
        Command::SetMeshVertexWeights {
            node_id,
            vertex_ids: vec![0, 1],
            joints: [2, 0, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
        .apply(&mut scene)
        .unwrap();
        Command::AddMorphDelta {
            node_id,
            name: "Smile".into(),
            vertex_ids: vec![0],
            delta: glam::Vec3::new(0.0, 0.5, 0.0),
        }
        .apply(&mut scene)
        .unwrap();
        Command::ApplyBipedRig.apply(&mut scene).unwrap();

        let (glb, _) = export_glb_bytes(&scene, ExportPreset::Godot4).unwrap();
        let imported = import_glb_bytes(&glb, mat).unwrap();
        // Every exported corner round-trips (manual where painted, auto
        // solver values elsewhere); painted corners keep exact values.
        let mesh = scene.lowpoly_meshes.get(&node_id).unwrap();
        let tri = mesh.to_triangulated_flat();
        assert_eq!(imported.skin_weights.len(), tri.vertices.len());
        let painted = imported
            .skin_weights
            .values()
            .filter(|w| w.joints == [2, 0, 0, 0] && w.weights == [1.0, 0.0, 0.0, 0.0])
            .count();
        let corners01: usize = tri
            .source_indices
            .iter()
            .filter(|s| **s == 0 || **s == 1)
            .count();
        assert_eq!(painted, corners01);
        for w in imported.skin_weights.values() {
            assert!((w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        // Morph deltas survive the round trip (renamed Morph0 on import).
        assert_eq!(imported.morphs.len(), 1);
        let corners0: usize = tri.source_indices.iter().filter(|s| **s == 0).count();
        assert_eq!(imported.morphs[0].deltas.len(), corners0);
        for d in imported.morphs[0].deltas.values() {
            assert!((d.y - 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn custom_png_texture_import_loads_rgba_dimensions() {
        let dir = std::env::temp_dir().join("tessel_custom_tex_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("bark.png");
        let img = image::RgbaImage::from_pixel(32, 16, image::Rgba([120, 80, 45, 255]));
        img.save(&path).unwrap();

        let tex = import_mesh_texture_from_path(&path).unwrap();
        assert_eq!(tex.width, 32);
        assert_eq!(tex.height, 16);
        assert_eq!(tex.rgba.len(), 32 * 16 * 4);
        assert_eq!(&tex.rgba[0..4], &[120, 80, 45, 255]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_mesh_texture_supports_tga_tiff_and_exr() {
        let dir = std::env::temp_dir().join("tessel_v47_tex_interop_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let rgba_img = image::RgbaImage::from_pixel(16, 12, image::Rgba([90, 140, 210, 255]));
        for ext in ["tga", "tiff"] {
            let p = dir.join(format!("albedo.{ext}"));
            rgba_img.save(&p).unwrap();
            let tex = import_mesh_texture_from_path(&p).unwrap();
            assert_eq!(tex.width, 16);
            assert_eq!(tex.height, 12);
            assert_eq!(&tex.rgba[0..4], &[90, 140, 210, 255]);
        }

        // HDR 32-bit float EXR round-trip (converts cleanly to RGBA8).
        let exr_path = dir.join("hdr_bake.exr");
        let exr_img =
            image::Rgba32FImage::from_pixel(8, 8, image::Rgba([0.5_f32, 0.25_f32, 0.75_f32, 1.0]));
        exr_img.save(&exr_path).unwrap();
        let exr_tex = import_mesh_texture_from_path(&exr_path).unwrap();
        assert_eq!(exr_tex.width, 8);
        assert_eq!(exr_tex.height, 8);
        assert_eq!(exr_tex.rgba.len(), 8 * 8 * 4);
        assert_eq!(exr_tex.rgba[3], 255);

        let _ = fs::remove_dir_all(&dir);
    }
}
