use std::fmt::Write;

use tessel_core::SceneState;

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
