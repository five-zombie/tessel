use std::fmt::Write;
use std::fs;
use std::path::Path;

use glam::Vec3;
use tessel_core::{Face, LowPolyMesh, PaletteIndex, SceneState};

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

/// Imports a Wavefront `.obj` text document into a `LowPolyMesh`.
pub fn import_obj_str(obj_text: &str, material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    let mut vertices = Vec::new();
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
                vertices.push(Vec3::new(x, y, z));
            }
            "f" => {
                let mut indices = Vec::new();
                for token in parts {
                    let v_str = token.split('/').next().unwrap_or("");
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
                    indices.push(resolved);
                }
                if indices.len() >= 3 {
                    faces.push(Face::new(indices, material));
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
        smooth_shading: false,
    })
}

/// Imports a binary glTF 2.0 (`.glb`) buffer into a `LowPolyMesh`.
pub fn import_glb_bytes(bytes: &[u8], material: PaletteIndex) -> Result<LowPolyMesh, IoError> {
    let gltf = gltf::Gltf::from_slice(bytes)
        .map_err(|e| IoError::InvalidFormat(format!("invalid GLB: {e}")))?;
    let blob = gltf.blob.as_deref();

    let mut vertices = Vec::new();
    let mut faces = Vec::new();

    for mesh in gltf.meshes() {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|_| blob);
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let base = vertices.len();
            for p in positions {
                vertices.push(Vec3::from_array(p));
            }
            let added = vertices.len() - base;
            if added < 3 {
                continue;
            }
            let indices: Vec<usize> = match reader.read_indices() {
                Some(iter) => iter.into_u32().map(|i| base + i as usize).collect(),
                None => (base..base + added).collect(),
            };
            for tri in indices.as_chunks::<3>().0 {
                if tri[0] < vertices.len() && tri[1] < vertices.len() && tri[2] < vertices.len() {
                    faces.push(Face::new(vec![tri[0], tri[1], tri[2]], material));
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
        smooth_shading: false,
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
}
