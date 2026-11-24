use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tessel_core::SceneState;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::IoError;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub format: String,
    pub schema_version: u32,
    pub app_version: String,
}

/// Serializes `SceneState` into `.tessel` ZIP archive bytes.
pub fn encode_tessel_project(scene: &SceneState) -> Result<Vec<u8>, IoError> {
    encode_tessel_project_with_thumbnail(scene, None)
}

/// Serializes `SceneState` and optional `thumbnail.png` into `.tessel` ZIP archive bytes.
pub fn encode_tessel_project_with_thumbnail(
    scene: &SceneState,
    thumbnail_png: Option<&[u8]>,
) -> Result<Vec<u8>, IoError> {
    let manifest = ProjectManifest {
        format: "tessel-project".into(),
        schema_version: CURRENT_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("manifest.json", opts)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;

        zip.start_file("scene.json", opts)?;
        zip.write_all(&serde_json::to_vec(scene)?)?;

        if let Some(png) = thumbnail_png {
            zip.start_file("thumbnail.png", opts)?;
            zip.write_all(png)?;
        }

        zip.finish()?;
    }
    Ok(cursor.into_inner())
}

/// Decodes `.tessel` archive bytes with forward schema version checking.
pub fn decode_tessel_project(bytes: &[u8]) -> Result<SceneState, IoError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let mut manifest_buf = String::new();
    archive
        .by_name("manifest.json")?
        .read_to_string(&mut manifest_buf)?;
    let manifest: ProjectManifest = serde_json::from_str(&manifest_buf)?;
    if manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(IoError::UnsupportedSchemaVersion(manifest.schema_version));
    }

    let mut scene_buf = String::new();
    archive
        .by_name("scene.json")?
        .read_to_string(&mut scene_buf)?;
    let scene: SceneState = serde_json::from_str(&scene_buf)?;
    Ok(scene)
}

/// Atomically writes a `.tessel` project file (`<path>.tmp` -> `sync_all` -> `rename`).
pub fn save_tessel_project_atomic(path: &Path, scene: &SceneState) -> Result<(), IoError> {
    save_tessel_project_with_thumbnail(path, scene, None)
}

/// Atomically writes a `.tessel` project file with an optional `thumbnail.png` entry.
pub fn save_tessel_project_with_thumbnail(
    path: &Path,
    scene: &SceneState,
    thumbnail_png: Option<&[u8]>,
) -> Result<(), IoError> {
    let bytes = encode_tessel_project_with_thumbnail(scene, thumbnail_png)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp_path = path.with_extension("tessel.tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        let dir_path = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Ok(dir) = fs::File::open(dir_path) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Loads a `.tessel` project file from disk.
pub fn load_tessel_project(path: &Path) -> Result<SceneState, IoError> {
    let bytes = fs::read(path)?;
    decode_tessel_project(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, PaletteIndex, VoxelCoord};

    #[test]
    fn tessel_project_roundtrip_preserves_scene_and_thumbnail() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume {
            name: "Statue".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let id = scene.active_voxel_node().unwrap();
        Command::FillVoxelBox {
            node_id: id,
            min: VoxelCoord::new(0, 0, 0),
            max: VoxelCoord::new(1, 1, 1),
            color: Some(PaletteIndex::new(4).unwrap()),
            hollow: false,
        }
        .apply(&mut scene)
        .unwrap();

        let fake_thumb = [0x89u8, 0x50, 0x4E, 0x47];
        let encoded = encode_tessel_project_with_thumbnail(&scene, Some(&fake_thumb)).unwrap();
        let archive = ZipArchive::new(Cursor::new(&encoded)).unwrap();
        assert!(archive.file_names().any(|n| n == "thumbnail.png"));

        let decoded = decode_tessel_project(&encoded).unwrap();
        assert_eq!(decoded.voxel_volumes[&id].voxel_count(), 8);
        assert_eq!(decoded.nodes[&id].name, "Statue");
    }
}
