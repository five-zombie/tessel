pub mod error;
pub mod gltf;
pub mod mesh_io;
pub mod project;
pub mod sprite;
pub mod vox;

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tessel_core::SceneState;

pub use error::IoError;
pub use gltf::export_glb_bytes;
pub use mesh_io::{
    export_obj_string, export_stl_bytes, import_glb_bytes, import_mesh_from_path,
    import_mesh_texture_from_path, import_obj_str,
};
pub use project::{
    decode_tessel_project, encode_tessel_project, encode_tessel_project_with_thumbnail,
    load_tessel_project, save_tessel_project_atomic, save_tessel_project_with_thumbnail,
    CURRENT_SCHEMA_VERSION,
};
pub use sprite::{
    export_gif_bytes, export_png_bytes, export_sprite_sheet_bytes, SpriteSheetMetadata,
};
pub use vox::{export_vox_bytes, import_vox_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Glb,
    Obj,
    Stl,
    Vox,
    Png,
    Gif,
}

impl ExportFormat {
    pub const ALL: [Self; 6] = [
        Self::Glb,
        Self::Obj,
        Self::Stl,
        Self::Vox,
        Self::Png,
        Self::Gif,
    ];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Obj => "obj",
            Self::Stl => "stl",
            Self::Vox => "vox",
            Self::Png => "png",
            Self::Gif => "gif",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportPreset {
    #[default]
    Godot4,
    Unity,
    Unreal,
    ThreeJs,
    Print3D,
}

impl ExportPreset {
    pub const ALL: [Self; 5] = [
        Self::Godot4,
        Self::Unity,
        Self::Unreal,
        Self::ThreeJs,
        Self::Print3D,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Godot4 => "Godot 4",
            Self::Unity => "Unity",
            Self::Unreal => "Unreal (cm)",
            Self::ThreeJs => "Web / Three.js",
            Self::Print3D => "3D Print (mm)",
        }
    }

    pub const fn scale_factor(self) -> f32 {
        match self {
            Self::Godot4 | Self::Unity | Self::ThreeJs => 1.0,
            Self::Unreal => 100.0,
            Self::Print3D => 1000.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub output_path: String,
    pub file_size_bytes: usize,
    pub triangles: usize,
}

/// Validates that `filename` is a safe relative path without parent traversal (`..`)
/// or symlink escapes outside `allowed_dir`, preventing arbitrary OS file writes from MCP or UI.
pub fn resolve_safe_export_path(allowed_dir: &Path, filename: &str) -> Result<PathBuf, IoError> {
    if filename.trim().is_empty() {
        return Err(IoError::ExportPathDenied("empty filename".into()));
    }
    let candidate = Path::new(filename);
    let joined = if candidate.is_absolute() {
        for comp in candidate.components() {
            if matches!(comp, Component::ParentDir) {
                return Err(IoError::ExportPathDenied(
                    "directory traversal is forbidden".into(),
                ));
            }
        }
        let canon_allowed = std::fs::canonicalize(allowed_dir).ok();
        let rel = candidate.strip_prefix(allowed_dir).ok().or_else(|| {
            canon_allowed
                .as_deref()
                .and_then(|ca| candidate.strip_prefix(ca).ok())
        });
        let Some(rel_inside) = rel else {
            return Err(IoError::ExportPathDenied(
                "absolute paths outside allowed export directory are not allowed".into(),
            ));
        };
        allowed_dir.join(rel_inside)
    } else {
        for comp in candidate.components() {
            if matches!(
                comp,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) {
                return Err(IoError::ExportPathDenied(
                    "directory traversal is forbidden".into(),
                ));
            }
        }
        allowed_dir.join(candidate)
    };
    if let Ok(canon_allowed) = std::fs::canonicalize(allowed_dir) {
        // Walk up to the deepest existing path component (target itself if it exists/is a symlink)
        // and verify its canonical location stays strictly inside `canon_allowed`.
        let mut probe = joined.as_path();
        loop {
            if probe.exists() || probe.symlink_metadata().is_ok() {
                let canon_probe = std::fs::canonicalize(probe).map_err(|_| {
                    IoError::ExportPathDenied("symlink target outside allowed directory".into())
                })?;
                if !canon_probe.starts_with(&canon_allowed) {
                    tracing::warn!(
                        filename,
                        "Blocked export path escaping allowed directory via symlink"
                    );
                    return Err(IoError::ExportPathDenied(
                        "symlink escapes allowed directory".into(),
                    ));
                }
                break;
            }
            let Some(parent) = probe.parent() else {
                break;
            };
            if parent == allowed_dir || parent.as_os_str().is_empty() {
                break;
            }
            probe = parent;
        }
    }
    Ok(joined)
}

/// Exports the current scene in `format` into `allowed_dir/filename` safely.
pub fn export_scene_to_dir(
    scene: &SceneState,
    allowed_dir: &Path,
    filename: &str,
    format: ExportFormat,
    preset: ExportPreset,
) -> Result<ExportReport, IoError> {
    let target = resolve_safe_export_path(allowed_dir, filename)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let (bytes, triangles) = match format {
        ExportFormat::Glb => export_glb_bytes(scene, preset)?,
        ExportFormat::Obj => {
            let (s, tris) = export_obj_string(scene, preset)?;
            (s.into_bytes(), tris)
        }
        ExportFormat::Stl => export_stl_bytes(scene, preset)?,
        ExportFormat::Vox => {
            let node_id = scene.active_voxel_node().ok_or(IoError::EmptyCanvas)?;
            let vol = scene
                .voxel_volumes
                .get(&node_id)
                .ok_or(IoError::EmptyCanvas)?;
            (export_vox_bytes(vol, &scene.palette)?, 0)
        }
        ExportFormat::Png => {
            let node_id = scene.active_pixel_node().ok_or(IoError::EmptyCanvas)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or(IoError::EmptyCanvas)?;
            (
                export_png_bytes(canvas, canvas.active_frame, &scene.palette)?,
                0,
            )
        }
        ExportFormat::Gif => {
            let node_id = scene.active_pixel_node().ok_or(IoError::EmptyCanvas)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or(IoError::EmptyCanvas)?;
            (export_gif_bytes(canvas, &scene.palette)?, 0)
        }
    };

    let size = bytes.len();
    fs::write(&target, bytes)?;
    tracing::info!(
        format = ?format,
        preset = ?preset,
        file_size_bytes = size,
        triangles,
        output_path = %target.display(),
        "Exported scene"
    );
    Ok(ExportReport {
        output_path: target.display().to_string(),
        file_size_bytes: size,
        triangles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_traversal_is_blocked() {
        let base = Path::new("/tmp/tessel_exports");
        assert!(resolve_safe_export_path(base, "../evil.glb").is_err());
        assert!(resolve_safe_export_path(base, "/etc/passwd").is_err());
        assert!(resolve_safe_export_path(base, "hero.glb").is_ok());
        assert!(resolve_safe_export_path(base, "/tmp/tessel_exports/sub/hero.glb").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_blocked() {
        let dir = std::env::temp_dir().join("tessel_symlink_escape_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("escape");
        std::os::unix::fs::symlink("/tmp", &link).unwrap();
        assert!(resolve_safe_export_path(&dir, "escape/outside.glb").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
