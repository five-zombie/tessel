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
        "voxel_create_volume" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Voxel Volume")
                .to_string();
            history.execute(scene, Command::CreateVoxelVolume { name: name.clone() })?;
            let node_id = scene.selected_node.map_or(0, |n| n.0);
            Ok(json!({ "ok": true, "node_id": node_id, "name": name }))
        }
        "voxel_inspect" => {
            let node_id = find_voxel_node(args, scene)?;
            let vol = scene
                .voxel_volumes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("voxel volume not found".into()))?;
            let bounds = vol.bounds();
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "voxel_count": vol.voxel_count(),
                "chunk_count": vol.chunks().len(),
                "bounds_min": bounds.map(|(mn, _)| [mn.x, mn.y, mn.z]),
                "bounds_max": bounds.map(|(_, mx)| [mx.x, mx.y, mx.z]),
            }))
        }
        "voxel_import" => {
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let path = resolve_safe_export_path(allowed_export_dir, filename)?;
            let meta = std::fs::metadata(&path).map_err(tessel_io::IoError::from)?;
            if meta.len() > 64 * 1024 * 1024 {
                return Err(AiError::InvalidArguments(
                    "VOX file exceeds 64 MiB safety limit".into(),
                ));
            }
            let bytes = std::fs::read(&path).map_err(tessel_io::IoError::from)?;
            let (vol, _) = tessel_io::import_vox_bytes(&bytes)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Imported Voxel")
                        .to_string()
                });
            let edits: Vec<tessel_core::VoxelEdit> = vol
                .iter_voxels()
                .into_iter()
                .take(MAX_BATCH_VOXELS)
                .map(|(coord, color)| tessel_core::VoxelEdit {
                    coord,
                    color: Some(color),
                })
                .collect();
            let count = edits.len();
            history.execute(scene, Command::CreateVoxelVolume { name: name.clone() })?;
            let node_id = scene
                .selected_node
                .ok_or_else(|| AiError::InvalidArguments("failed to create voxel volume".into()))?;
            if !edits.is_empty() {
                history.execute(scene, Command::SetVoxels { node_id, edits })?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "name": name,
                "voxels_imported": count
            }))
        }
        "voxel_set_voxels" => {
            let raw = args
                .get("voxels")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'voxels' array".into()))?;
            if raw.is_empty() || raw.len() > MAX_BATCH_VOXELS {
                return Err(AiError::BatchLimitExceeded);
            }
            let sym = parse_symmetry_3d(args)?;
            let mut edits = Vec::with_capacity(raw.len());
            for (i, item) in raw.iter().enumerate() {
                let x_raw = item
                    .get("x")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].x missing")))?;
                let y_raw = item
                    .get("y")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].y missing")))?;
                let z_raw = item
                    .get("z")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].z missing")))?;
                let x = i32::try_from(x_raw).map_err(|_| AiError::OutOfBounds)?;
                let y = i32::try_from(y_raw).map_err(|_| AiError::OutOfBounds)?;
                let z = i32::try_from(z_raw).map_err(|_| AiError::OutOfBounds)?;
                let coord = VoxelCoord::new(x, y, z);
                check_coord(coord)?;
                let color = parse_opt_palette_index(item, None)?;
                for mc in voxel_ops::expand_symmetry_3d(coord, sym) {
                    check_coord(mc)?;
                    if edits.len() >= MAX_BATCH_VOXELS {
                        return Err(AiError::BatchLimitExceeded);
                    }
                    edits.push(tessel_core::VoxelEdit { coord: mc, color });
                }
            }
            let node_id = resolve_voxel_node(args, scene, history)?;
            let modified = edits.len();
            history.execute(scene, Command::SetVoxels { node_id, edits })?;
            let total = scene
                .voxel_volumes
                .get(&node_id)
                .map_or(0, |v| v.voxel_count());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "voxels_modified": modified,
                "total_voxels": total
            }))
        }
        "voxel_fill_box" => {
            let min = parse_ivec3(args, "min")?;
            let max = parse_ivec3(args, "max")?;
            let dx = (max.x - min.x).unsigned_abs() as usize + 1;
            let dy = (max.y - min.y).unsigned_abs() as usize + 1;
            let dz = (max.z - min.z).unsigned_abs() as usize + 1;
            let count = dx.saturating_mul(dy).saturating_mul(dz);
            if count > MAX_BATCH_VOXELS {
                return Err(AiError::BatchLimitExceeded);
            }
            let color = parse_opt_palette_index(args, Some(4))?;
            let hollow = args.get("hollow").and_then(Value::as_bool).unwrap_or(false);
            let sym = parse_symmetry_3d(args)?;
            let node_id = resolve_voxel_node(args, scene, history)?;
            if sym.x || sym.y || sym.z {
                let base_edits = voxel_ops::box_fill_edits(min, max, color, hollow);
                let mut edits = Vec::with_capacity(base_edits.len());
                for e in base_edits {
                    for mc in voxel_ops::expand_symmetry_3d(e.coord, sym) {
                        check_coord(mc)?;
                        if edits.len() >= MAX_BATCH_VOXELS {
                            return Err(AiError::BatchLimitExceeded);
                        }
                        edits.push(tessel_core::VoxelEdit { coord: mc, color });
                    }
                }
                history.execute(scene, Command::SetVoxels { node_id, edits })?;
            } else {
                history.execute(
                    scene,
                    Command::FillVoxelBox {
                        node_id,
                        min,
                        max,
                        color,
                        hollow,
                    },
                )?;
            }
            let total = scene
                .voxel_volumes
                .get(&node_id)
                .map_or(0, |v| v.voxel_count());
            Ok(json!({ "ok": true, "node_id": node_id.0, "total_voxels": total }))
        }
        "voxel_fill_ellipsoid" => {
            let center = parse_ivec3(args, "center")?;
            let radii_arr = args
                .get("radii")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'radii'".into()))?;
            if radii_arr.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'radii' must have 3 numbers".into(),
                ));
            }
            let mut radii = [1.0f32; 3];
            for (i, item) in radii_arr.iter().enumerate() {
                let v = item.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'radii[{i}]' must be a number"))
                })? as f32;
                if !v.is_finite() || v <= 0.0 {
                    return Err(AiError::InvalidArguments(format!(
                        "'radii[{i}]' must be finite and > 0"
                    )));
                }
                radii[i] = v;
            }
            // Reject oversized ellipsoids BEFORE allocating the full voxel edit buffer.
            let bx = (radii[0].ceil() as u64).saturating_mul(2).saturating_add(1);
            let by = (radii[1].ceil() as u64).saturating_mul(2).saturating_add(1);
            let bz = (radii[2].ceil() as u64).saturating_mul(2).saturating_add(1);
            if bx.saturating_mul(by).saturating_mul(bz) > (MAX_BATCH_VOXELS as u64) * 2 {
                return Err(AiError::BatchLimitExceeded);
            }
            let color = parse_opt_palette_index(args, Some(4))?;
            let sym = parse_symmetry_3d(args)?;
            let base_edits = voxel_ops::ellipsoid_fill_edits(center, radii, color);
            let edits = if sym.x || sym.y || sym.z {
                let mut expanded = Vec::with_capacity(base_edits.len());
                for e in base_edits {
                    for mc in voxel_ops::expand_symmetry_3d(e.coord, sym) {
                        check_coord(mc)?;
                        if expanded.len() >= MAX_BATCH_VOXELS {
                            return Err(AiError::BatchLimitExceeded);
                        }
                        expanded.push(tessel_core::VoxelEdit { coord: mc, color });
                    }
                }
                expanded
            } else {
                if base_edits.len() > MAX_BATCH_VOXELS {
                    return Err(AiError::BatchLimitExceeded);
                }
                for e in &base_edits {
                    check_coord(e.coord)?;
                }
                base_edits
            };
            let node_id = resolve_voxel_node(args, scene, history)?;
            let modified = edits.len();
            history.execute(scene, Command::SetVoxels { node_id, edits })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "voxels_modified": modified }))
        }
        "voxel_convert_to_lowpoly" => {
            let voxel_node_id = find_voxel_node(args, scene)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Converted LowPoly")
                .to_string();
            history.execute(
                scene,
                Command::ConvertVoxelToLowPoly {
                    voxel_node_id,
                    name,
                },
            )?;
            Ok(json!({ "ok": true, "new_node_id": scene.selected_node.map(|n| n.0) }))
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
