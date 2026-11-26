use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tessel_core::{AppMode, SceneState};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::error::IoError;

pub const PROJECT_FORMAT_ID: &str = "tessel-project";
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

fn default_unit_scale() -> f32 {
    0.1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub format: String,
    pub schema_version: u32,
    pub app_version: String,
    #[serde(default)]
    pub active_mode: AppMode,
    #[serde(default = "default_unit_scale")]
    pub unit_scale: f32,
}

/// Serializes `SceneState` into `.tessel` ZIP archive bytes.
pub fn encode_tessel_project(scene: &SceneState) -> Result<Vec<u8>, IoError> {
    encode_tessel_project_with_thumbnail(scene, None)
}

/// Serializes `SceneState` and optional `thumbnail.png` into deterministic `.tessel` ZIP archive bytes.
pub fn encode_tessel_project_with_thumbnail(
    scene: &SceneState,
    thumbnail_png: Option<&[u8]>,
) -> Result<Vec<u8>, IoError> {
    let manifest = ProjectManifest {
        format: PROJECT_FORMAT_ID.into(),
        schema_version: CURRENT_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        active_mode: scene.active_mode,
        unit_scale: scene.unit_scale,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default());

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

const MAX_MANIFEST_JSON_BYTES: u64 = 64 * 1024;
const MAX_SCENE_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PIXEL_CANVAS_DIM: u32 = 512;
const MAX_PIXEL_CANVAS_FRAMES: usize = 128;
const MAX_PIXEL_LAYERS_PER_FRAME: usize = 32;

fn validate_loaded_scene(scene: &SceneState) -> Result<(), IoError> {
    for canvas in scene.pixel_canvases.values() {
        if canvas.width == 0
            || canvas.height == 0
            || canvas.width > MAX_PIXEL_CANVAS_DIM
            || canvas.height > MAX_PIXEL_CANVAS_DIM
        {
            return Err(IoError::InvalidDimensions(canvas.width, canvas.height));
        }
        if canvas.frames.len() > MAX_PIXEL_CANVAS_FRAMES {
            return Err(IoError::PayloadTooLarge(format!(
                "pixel canvas frame count {} exceeds limit {MAX_PIXEL_CANVAS_FRAMES}",
                canvas.frames.len()
            )));
        }
        let expected_pixels = (canvas.width as usize)
            .checked_mul(canvas.height as usize)
            .ok_or(IoError::InvalidDimensions(canvas.width, canvas.height))?;
        for frame in &canvas.frames {
            if frame.layers.len() > MAX_PIXEL_LAYERS_PER_FRAME {
                return Err(IoError::PayloadTooLarge(format!(
                    "pixel frame layer count {} exceeds limit {MAX_PIXEL_LAYERS_PER_FRAME}",
                    frame.layers.len()
                )));
            }
            for layer in &frame.layers {
                if layer.width() == 0
                    || layer.height() == 0
                    || layer.width() > MAX_PIXEL_CANVAS_DIM
                    || layer.height() > MAX_PIXEL_CANVAS_DIM
                    || layer.width() != canvas.width
                    || layer.height() != canvas.height
                    || layer.pixels().len() != expected_pixels
                {
                    return Err(IoError::InvalidDimensions(layer.width(), layer.height()));
                }
            }
        }
    }
    Ok(())
}

/// Decodes `.tessel` archive bytes with forward schema version checking and decompression bounds.
pub fn decode_tessel_project(bytes: &[u8]) -> Result<SceneState, IoError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let mut manifest_buf = String::new();
    archive
        .by_name("manifest.json")?
        .take(MAX_MANIFEST_JSON_BYTES)
        .read_to_string(&mut manifest_buf)?;
    let manifest: ProjectManifest = serde_json::from_str(&manifest_buf)?;
    if manifest.format != PROJECT_FORMAT_ID {
        return Err(IoError::InvalidFormat(format!(
            "unexpected project format '{}' (expected '{PROJECT_FORMAT_ID}')",
            manifest.format
        )));
    }
    if manifest.schema_version == 0 || manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(IoError::UnsupportedSchemaVersion(manifest.schema_version));
    }

    let mut scene_buf = String::new();
    let read_bytes = archive
        .by_name("scene.json")?
        .take(MAX_SCENE_JSON_BYTES + 1)
        .read_to_string(&mut scene_buf)?;
    if read_bytes as u64 > MAX_SCENE_JSON_BYTES {
        return Err(IoError::PayloadTooLarge(
            "scene.json exceeds 64 MiB decompressed limit".into(),
        ));
    }
    let scene: SceneState = serde_json::from_str(&scene_buf)?;
    validate_loaded_scene(&scene)?;
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

/// Loads a `.tessel` project file from disk (bounded to 64 MiB).
pub fn load_tessel_project(path: &Path) -> Result<SceneState, IoError> {
    let meta = fs::metadata(path)?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(IoError::InvalidFormat(
            "project file exceeds 64 MiB safety limit".into(),
        ));
    }
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
        assert!(archive
            .file_names()
            .any(|n| n.is_ok_and(|s| s == "thumbnail.png")));

        let decoded = decode_tessel_project(&encoded).unwrap();
        assert_eq!(decoded.voxel_volumes[&id].voxel_count(), 8);
        assert_eq!(decoded.nodes[&id].name, "Statue");
    }

    #[test]
    fn tessel_project_zip_is_byte_stable_with_pinned_mtime() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume {
            name: "DeterminismCheck".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let id = scene.active_voxel_node().unwrap();
        Command::FillVoxelBox {
            node_id: id,
            min: VoxelCoord::new(0, 0, 0),
            max: VoxelCoord::new(2, 1, 1),
            color: Some(PaletteIndex::new(2).unwrap()),
            hollow: false,
        }
        .apply(&mut scene)
        .unwrap();

        let thumb = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        let first = encode_tessel_project_with_thumbnail(&scene, Some(&thumb)).unwrap();
        let second = encode_tessel_project_with_thumbnail(&scene, Some(&thumb)).unwrap();
        assert_eq!(
            first, second,
            "identical scenes must produce identical ZIP bytes"
        );

        let mut archive = ZipArchive::new(Cursor::new(&first)).unwrap();
        for name in ["manifest.json", "scene.json", "thumbnail.png"] {
            let entry = archive.by_name(name).unwrap();
            assert_eq!(
                entry.last_modified(),
                Some(DateTime::default()),
                "ZIP entry '{name}' must use pinned epoch mtime"
            );
        }
    }

    #[test]
    fn tessel_project_validates_manifest_and_preserves_legacy_v1_compat() {
        let scene = SceneState::new();
        let scene_json = serde_json::to_vec(&scene).unwrap();
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default());

        // Legacy minimal v1 manifest without active_mode / unit_scale must still decode.
        let mut legacy_buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut legacy_buf);
            zip.start_file("manifest.json", opts).unwrap();
            zip.write_all(
                br#"{"format":"tessel-project","schema_version":1,"app_version":"0.1.0"}"#,
            )
            .unwrap();
            zip.start_file("scene.json", opts).unwrap();
            zip.write_all(&scene_json).unwrap();
            zip.finish().unwrap();
        }
        assert!(decode_tessel_project(&legacy_buf.into_inner()).is_ok());

        // Future schema_version must be rejected with UnsupportedSchemaVersion.
        let mut future_buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut future_buf);
            zip.start_file("manifest.json", opts).unwrap();
            zip.write_all(
                br#"{"format":"tessel-project","schema_version":99,"app_version":"9.0.0"}"#,
            )
            .unwrap();
            zip.start_file("scene.json", opts).unwrap();
            zip.write_all(&scene_json).unwrap();
            zip.finish().unwrap();
        }
        assert!(matches!(
            decode_tessel_project(&future_buf.into_inner()),
            Err(IoError::UnsupportedSchemaVersion(99))
        ));

        // Zero schema_version must also be rejected with UnsupportedSchemaVersion(0).
        let mut zero_buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut zero_buf);
            zip.start_file("manifest.json", opts).unwrap();
            zip.write_all(
                br#"{"format":"tessel-project","schema_version":0,"app_version":"0.1.0"}"#,
            )
            .unwrap();
            zip.start_file("scene.json", opts).unwrap();
            zip.write_all(&scene_json).unwrap();
            zip.finish().unwrap();
        }
        assert!(matches!(
            decode_tessel_project(&zero_buf.into_inner()),
            Err(IoError::UnsupportedSchemaVersion(0))
        ));

        // Unexpected format identifier must be rejected with InvalidFormat.
        let mut bad_fmt_buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut bad_fmt_buf);
            zip.start_file("manifest.json", opts).unwrap();
            zip.write_all(br#"{"format":"other-app","schema_version":1,"app_version":"0.1.0"}"#)
                .unwrap();
            zip.start_file("scene.json", opts).unwrap();
            zip.write_all(&scene_json).unwrap();
            zip.finish().unwrap();
        }
        assert!(matches!(
            decode_tessel_project(&bad_fmt_buf.into_inner()),
            Err(IoError::InvalidFormat(_))
        ));

        // Oversized PixelCanvas (> 512) must be rejected post-deserialization.
        let mut oversized = SceneState::new();
        let node_id = oversized.allocate_node_id();
        let mut bad_canvas = tessel_core::PixelCanvas::new(16, 16).unwrap();
        bad_canvas.width = 1024;
        oversized.pixel_canvases.insert(node_id, bad_canvas);
        let encoded_oversized = encode_tessel_project(&oversized).unwrap();
        assert!(matches!(
            decode_tessel_project(&encoded_oversized),
            Err(IoError::InvalidDimensions(1024, 16))
        ));
    }
}
