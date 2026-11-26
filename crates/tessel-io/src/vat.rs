use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use tessel_core::{bake_vertex_animation_texture, SceneState, VatBakeParams, VatMetadata};

use crate::error::IoError;
use crate::resolve_safe_export_path;

/// Report returned after baking and exporting a Vertex Animation Texture (VAT) bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VatExportReport {
    pub position_png_path: String,
    pub normal_png_path: String,
    pub metadata_json_path: String,
    pub metadata: VatMetadata,
}

fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, IoError> {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or(IoError::InvalidDimensions(width, height))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Bakes and writes `<stem>_vat_pos.png`, `<stem>_vat_nrm.png`, and `<stem>_vat.json`
/// safely inside `allowed_dir`.
pub fn export_vat_bundle(
    scene: &SceneState,
    params: &VatBakeParams,
    allowed_dir: &Path,
    stem: &str,
) -> Result<VatExportReport, IoError> {
    let clean_stem = stem
        .trim()
        .trim_end_matches(".png")
        .trim_end_matches(".json");
    if clean_stem.is_empty() {
        return Err(IoError::ExportPathDenied("empty VAT stem".into()));
    }

    let pos_path = resolve_safe_export_path(allowed_dir, &format!("{clean_stem}_vat_pos.png"))?;
    let nrm_path = resolve_safe_export_path(allowed_dir, &format!("{clean_stem}_vat_nrm.png"))?;
    let meta_path = resolve_safe_export_path(allowed_dir, &format!("{clean_stem}_vat.json"))?;

    if let Some(parent) = pos_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let baked = bake_vertex_animation_texture(scene, params)
        .map_err(|e| IoError::InvalidFormat(e.to_string()))?;
    let pos_png = encode_rgba_png(
        baked.metadata.width,
        baked.metadata.height,
        &baked.position_rgba,
    )?;
    let nrm_png = encode_rgba_png(
        baked.metadata.width,
        baked.metadata.height,
        &baked.normal_rgba,
    )?;
    let meta_json = serde_json::to_string_pretty(&baked.metadata)?;

    fs::write(&pos_path, pos_png)?;
    fs::write(&nrm_path, nrm_png)?;
    fs::write(&meta_path, meta_json)?;

    Ok(VatExportReport {
        position_png_path: pos_path.display().to_string(),
        normal_png_path: nrm_path.display().to_string(),
        metadata_json_path: meta_path.display().to_string(),
        metadata: baked.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, PaletteIndex, PrimitiveKind};

    #[test]
    fn vat_bundle_exports_pngs_and_json_metadata() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "Banner".to_string(),
            kind: PrimitiveKind::Cube,
            size: 1.0,
            material: PaletteIndex::new(1).unwrap(),
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.active_lowpoly_node().unwrap();

        let dir = std::env::temp_dir().join("tessel_vat_export_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let report = export_vat_bundle(
            &scene,
            &VatBakeParams {
                node_id,
                clip_index: 0,
                fps: 8.0,
                max_frames: 16,
            },
            &dir,
            "banner",
        )
        .unwrap();

        assert!(Path::new(&report.position_png_path).exists());
        assert!(Path::new(&report.normal_png_path).exists());
        assert!(Path::new(&report.metadata_json_path).exists());
        assert_eq!(report.metadata.vertex_count, 8);
        let _ = fs::remove_dir_all(&dir);
    }
}
