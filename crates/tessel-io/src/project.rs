use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tessel_core::{AppMode, SceneState};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::error::IoError;

pub const PROJECT_FORMAT_ID: &str = "tessel-project";
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

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
const MAX_SCENE_JSON_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SCENE_DECODED_PIXELS: usize = 256 * 1024 * 1024;
const MAX_PIXEL_CANVASES: usize = 64;
const MAX_PIXEL_CANVAS_DIM: u32 = tessel_core::MAX_PIXEL_CANVAS_DIM;
const MAX_PIXEL_CANVAS_FRAMES: usize = 128;
const MAX_PIXEL_LAYERS_PER_FRAME: usize = 32;

fn validate_loaded_scene(scene: &SceneState) -> Result<(), IoError> {
    if scene.pixel_canvases.len() > MAX_PIXEL_CANVASES {
        return Err(IoError::PayloadTooLarge(format!(
            "pixel canvas count {} exceeds limit {MAX_PIXEL_CANVASES}",
            scene.pixel_canvases.len()
        )));
    }
    for canvas in scene.pixel_canvases.values() {
        if canvas.width == 0
            || canvas.height == 0
            || canvas.width > MAX_PIXEL_CANVAS_DIM
            || canvas.height > MAX_PIXEL_CANVAS_DIM
        {
            return Err(IoError::InvalidDimensions(canvas.width, canvas.height));
        }
        if canvas.frames.is_empty() || canvas.active_frame >= canvas.frames.len() {
            return Err(IoError::InvalidFormat(
                "pixel canvas must have at least one frame and a valid active_frame".into(),
            ));
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
            if frame.layers.is_empty() || frame.active_layer >= frame.layers.len() {
                return Err(IoError::InvalidFormat(
                    "pixel frame must have at least one layer and a valid active_layer".into(),
                ));
            }
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
            "scene.json exceeds 256 MiB decompressed limit".into(),
        ));
    }
    let scene: SceneState =
        tessel_core::pixel::with_pixel_decode_budget(MAX_SCENE_DECODED_PIXELS, || {
            serde_json::from_str(&scene_buf)
        })?;
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

        // 4K-wide PixelCanvas (<= 4096) must round-trip cleanly.
        let mut valid_4k = SceneState::new();
        let ok_node_id = valid_4k.allocate_node_id();
        let ok_canvas = tessel_core::PixelCanvas::new(MAX_PIXEL_CANVAS_DIM, 4).unwrap();
        valid_4k.pixel_canvases.insert(ok_node_id, ok_canvas);
        let encoded_4k = encode_tessel_project(&valid_4k).unwrap();
        let decoded_4k = decode_tessel_project(&encoded_4k).unwrap();
        assert_eq!(
            decoded_4k.pixel_canvases[&ok_node_id].width,
            MAX_PIXEL_CANVAS_DIM
        );

        // Oversized PixelCanvas (> 4096) must be rejected post-deserialization.
        let mut oversized = SceneState::new();
        let node_id = oversized.allocate_node_id();
        let mut bad_canvas = tessel_core::PixelCanvas::new(16, 16).unwrap();
        bad_canvas.width = MAX_PIXEL_CANVAS_DIM + 1;
        oversized.pixel_canvases.insert(node_id, bad_canvas);
        let encoded_oversized = encode_tessel_project(&oversized).unwrap();
        assert!(matches!(
            decode_tessel_project(&encoded_oversized),
            Err(IoError::InvalidDimensions(4097, 16))
        ));
    }

    #[test]
    fn five_layer_4k_pixel_project_roundtrips_compactly_under_scene_json_limit() {
        let mut scene = SceneState::new();
        Command::CreatePixelCanvas {
            name: "Artwork4K".into(),
            width: MAX_PIXEL_CANVAS_DIM,
            height: MAX_PIXEL_CANVAS_DIM,
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_pixel_node().unwrap();
        for idx in 2..=5 {
            Command::AddPixelLayer {
                node_id,
                frame: tessel_core::FrameIndex(0),
                name: format!("Layer {idx}"),
            }
            .apply(&mut scene)
            .unwrap();
        }
        let color = PaletteIndex::new(5).unwrap();
        let layer_id = scene.pixel_canvases[&node_id].frames[0].layers[4].id;
        Command::DrawPixels {
            node_id,
            frame: tessel_core::FrameIndex(0),
            layer_id,
            edits: vec![
                tessel_core::PixelEdit {
                    coord: tessel_core::PixelCoord::new(0, 0),
                    color: Some(color),
                },
                tessel_core::PixelEdit {
                    coord: tessel_core::PixelCoord::new(4095, 4095),
                    color: Some(color),
                },
            ],
        }
        .apply(&mut scene)
        .unwrap();

        let encoded = encode_tessel_project(&scene).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(&encoded)).unwrap();
        let scene_json_len = archive.by_name("scene.json").unwrap().size();
        assert!(
            scene_json_len < 1024 * 1024,
            "5-layer 4K RLE scene.json must stay well below 1 MiB, got {scene_json_len} bytes"
        );

        let decoded = decode_tessel_project(&encoded).unwrap();
        let canvas = &decoded.pixel_canvases[&node_id];
        assert_eq!(canvas.width, MAX_PIXEL_CANVAS_DIM);
        assert_eq!(canvas.height, MAX_PIXEL_CANVAS_DIM);
        assert_eq!(canvas.frames[0].layers.len(), 5);
        assert_eq!(
            canvas.frames[0].layers[4].get(tessel_core::PixelCoord::new(4095, 4095)),
            Some(color)
        );
    }

    #[test]
    fn pixel_layer_serde_supports_legacy_flat_array_and_rejects_rle_bombs() {
        use tessel_core::{LayerId, PixelCoord, PixelLayer};

        let mut layer = PixelLayer::new(LayerId(1), "Sky", 4096, 4096);
        let c1 = PaletteIndex::new(3).unwrap();
        let c2 = PaletteIndex::new(7).unwrap();
        layer.set(PixelCoord::new(0, 0), Some(c1));
        layer.set(PixelCoord::new(1, 0), Some(c1));
        layer.set(PixelCoord::new(4095, 4095), Some(c2));

        let json = serde_json::to_string(&layer).unwrap();
        assert!(
            json.len() < 256,
            "4K RLE layer JSON should be compact, got {} bytes",
            json.len()
        );
        let decoded: PixelLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, layer);

        // Legacy flat array compatibility.
        let legacy = r#"{
            "id": 1,
            "name": "Legacy",
            "visible": true,
            "opacity": 1.0,
            "width": 2,
            "height": 2,
            "pixels": [null, 2, 2, null]
        }"#;
        let legacy_decoded: PixelLayer = serde_json::from_str(legacy).unwrap();
        assert_eq!(legacy_decoded.get(PixelCoord::new(0, 0)), None);
        assert_eq!(
            legacy_decoded.get(PixelCoord::new(1, 0)),
            Some(PaletteIndex::new(2).unwrap())
        );
        assert_eq!(
            legacy_decoded.get(PixelCoord::new(0, 1)),
            Some(PaletteIndex::new(2).unwrap())
        );
        assert_eq!(legacy_decoded.get(PixelCoord::new(1, 1)), None);

        // Zero-count run, oversized RLE bomb, and unknown field must fail closed.
        let zero_run = r#"{
            "id": 1, "name": "Bad", "visible": true, "opacity": 1.0,
            "width": 2, "height": 2, "pixels": {"rle": [[0, 1]]}
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(zero_run).is_err());

        let bomb = r#"{
            "id": 1, "name": "Bomb", "visible": true, "opacity": 1.0,
            "width": 2, "height": 2, "pixels": {"rle": [[16777216, null], [1, 1]]}
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(bomb).is_err());

        let unknown_key = r#"{
            "id": 1, "name": "BadKey", "visible": true, "opacity": 1.0,
            "width": 2, "height": 2, "pixels": {"unknown": []}
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(unknown_key).is_err());

        // PaletteIndex(0) must be rejected in both RLE and legacy flat arrays.
        let zero_palette_rle = r#"{
            "id": 1, "name": "ZeroRle", "visible": true, "opacity": 1.0,
            "width": 2, "height": 2, "pixels": {"rle": [[4, 0]]}
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(zero_palette_rle).is_err());

        let zero_palette_flat = r#"{
            "id": 1, "name": "ZeroFlat", "visible": true, "opacity": 1.0,
            "width": 2, "height": 2, "pixels": [0, null, null, null]
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(zero_palette_flat).is_err());

        // Mismatched pixel length vs declared width*height must be rejected.
        let truncated = r#"{
            "id": 1, "name": "Trunc", "visible": true, "opacity": 1.0,
            "width": 4, "height": 4, "pixels": {"rle": [[1, 1]]}
        }"#;
        assert!(serde_json::from_str::<PixelLayer>(truncated).is_err());

        // Multi-layer RLE OOM amplification bomb must fail closed under decode budget.
        let bomb_layers = (1..=20)
            .map(|i| {
                format!(
                    r#"{{"id":{i},"name":"L{i}","visible":true,"opacity":1.0,"width":4096,"height":4096,"pixels":{{"rle":[[16777216,null]]}}}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let bomb_frame = format!(r#"[{bomb_layers}]"#);
        let budgeted =
            tessel_core::pixel::with_pixel_decode_budget(MAX_SCENE_DECODED_PIXELS, || {
                serde_json::from_str::<Vec<PixelLayer>>(&bomb_frame)
            });
        assert!(
            budgeted.is_err(),
            "20x 4K RLE layers (335M pixels) must exceed 256M scene decode budget"
        );
    }
}
