#![allow(unused_imports, unused_variables)]

use std::path::Path;

use serde_json::{json, Value};
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    apply_primitive_anchor, bake_mesh_maps, generate_smart_material_texture, AppMode, BakeMapKind,
    BakeParams, BoneId, Command, FrameIndex, HistoryStack, Keyframe, LayerId, NodeId, PaletteEntry,
    PaletteIndex, PixelCoord, PixelEdit, PrimitiveAnchor, PrimitiveKind, Rgba8, SceneState,
    SmartMaterialParams, VoxelCoord,
};
use tessel_io::{
    export_mesh_texture_png_bytes, export_scene_to_dir, export_sprite_sheet_bytes,
    import_mesh_from_path, import_mesh_texture_from_path, load_tessel_project,
    resolve_safe_export_path, save_tessel_project_atomic, ExportFormat, ExportPreset,
};

use super::helpers::*;
use super::{execute_tool_call, palette_summary_json, scene_summary_json};
use crate::error::AiError;

pub(super) fn execute(
    name: &str,
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &Path,
) -> Result<Value, AiError> {
    match name {
        "project_save" => {
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(tessel_io::IoError::from)?;
            }
            save_tessel_project_atomic(&target, scene)?;
            let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            Ok(json!({
                "ok": true,
                "output_path": target.display().to_string(),
                "file_size_bytes": size
            }))
        }
        "project_open" => {
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            let mut loaded = load_tessel_project(&target)?;
            loaded.mark_modified();
            let node_count = loaded.nodes.len();
            let has_rig = loaded.character_rig.is_some();
            *scene = loaded;
            *history = HistoryStack::new();
            Ok(json!({
                "ok": true,
                "path": target.display().to_string(),
                "node_count": node_count,
                "has_character_rig": has_rig
            }))
        }
        "project_export" => {
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let format: ExportFormat = serde_json::from_value(
                args.get("format")
                    .cloned()
                    .unwrap_or_else(|| Value::String("glb".into())),
            )
            .map_err(|e| AiError::InvalidArguments(format!("invalid 'format': {e}")))?;
            let preset: ExportPreset = match args.get("preset") {
                None | Some(Value::Null) => ExportPreset::Godot4,
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| AiError::InvalidArguments(format!("invalid 'preset': {e}")))?,
            };
            let report = export_scene_to_dir(scene, allowed_export_dir, filename, format, preset)?;
            Ok(serde_json::to_value(report)?)
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
