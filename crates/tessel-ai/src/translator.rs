use std::path::Path;

use serde_json::{json, Value};
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    AppMode, Command, HistoryStack, NodeId, PaletteIndex, PrimitiveKind, SceneState, VoxelCoord,
};
use tessel_io::{export_scene_to_dir, ExportFormat, ExportPreset};

use crate::error::AiError;

const MAX_COORD_ABS: i32 = 256;
const MAX_BATCH_VOXELS: usize = 50_000;

fn check_coord(c: VoxelCoord) -> Result<(), AiError> {
    if c.x.abs() > MAX_COORD_ABS || c.y.abs() > MAX_COORD_ABS || c.z.abs() > MAX_COORD_ABS {
        return Err(AiError::OutOfBounds);
    }
    Ok(())
}

fn parse_ivec3(val: &Value, field: &str) -> Result<VoxelCoord, AiError> {
    let arr = val
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::InvalidArguments(format!("missing array '{field}'")))?;
    if arr.len() != 3 {
        return Err(AiError::InvalidArguments(format!(
            "'{field}' must have 3 integers"
        )));
    }
    let x = arr[0]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[0]' must be int")))?
        as i32;
    let y = arr[1]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[1]' must be int")))?
        as i32;
    let z = arr[2]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[2]' must be int")))?
        as i32;
    let c = VoxelCoord::new(x, y, z);
    check_coord(c)?;
    Ok(c)
}

fn parse_palette_index(val: &Value, default: u8) -> Result<PaletteIndex, AiError> {
    let raw = val
        .get("palette_index")
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(default));
    let byte =
        u8::try_from(raw).map_err(|_| AiError::InvalidArguments("palette_index > 255".into()))?;
    Ok(PaletteIndex::new(byte)?)
}

fn resolve_voxel_node(
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) -> Result<NodeId, AiError> {
    if let Some(raw_id) = args.get("node_id").and_then(Value::as_u64) {
        return Ok(NodeId(raw_id));
    }
    if let Some(existing) = scene.active_voxel_node() {
        return Ok(existing);
    }
    history.execute(
        scene,
        Command::CreateVoxelVolume {
            name: "AI Voxel".into(),
        },
    )?;
    scene
        .active_voxel_node()
        .ok_or_else(|| AiError::InvalidArguments("failed to create voxel volume".into()))
}

/// Validates and executes a named tool call against `SceneState` & `HistoryStack`.
pub fn execute_tool_call(
    name: &str,
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &Path,
) -> Result<Value, AiError> {
    match name {
        "scene_set_mode" => {
            let mode_str = args
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'mode'".into()))?;
            let mode = match mode_str {
                "pixel" => AppMode::Pixel,
                "voxel" => AppMode::Voxel,
                "lowpoly" => AppMode::LowPoly,
                "character" => AppMode::Character,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unsupported mode '{other}'"
                    )));
                }
            };
            history.execute(scene, Command::SetMode(mode))?;
            Ok(json!({ "ok": true, "active_mode": mode.label() }))
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
            let color = parse_palette_index(args, 4)?;
            let hollow = args.get("hollow").and_then(Value::as_bool).unwrap_or(false);
            let node_id = resolve_voxel_node(args, scene, history)?;
            history.execute(
                scene,
                Command::FillVoxelBox {
                    node_id,
                    min,
                    max,
                    color: Some(color),
                    hollow,
                },
            )?;
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
            let radii = [
                radii_arr[0].as_f64().unwrap_or(1.0) as f32,
                radii_arr[1].as_f64().unwrap_or(1.0) as f32,
                radii_arr[2].as_f64().unwrap_or(1.0) as f32,
            ];
            let edits =
                voxel_ops::ellipsoid_fill_edits(center, radii, Some(parse_palette_index(args, 4)?));
            if edits.len() > MAX_BATCH_VOXELS {
                return Err(AiError::BatchLimitExceeded);
            }
            for e in &edits {
                check_coord(e.coord)?;
            }
            let node_id = resolve_voxel_node(args, scene, history)?;
            let modified = edits.len();
            history.execute(scene, Command::SetVoxels { node_id, edits })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "voxels_modified": modified }))
        }
        "voxel_convert_to_lowpoly" => {
            let voxel_node_id = resolve_voxel_node(args, scene, history)?;
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
        "mesh_create_primitive" => {
            let kind_str = args.get("kind").and_then(Value::as_str).unwrap_or("cube");
            let kind = match kind_str {
                "cube" => PrimitiveKind::Cube,
                "plane" => PrimitiveKind::Plane,
                "cylinder" => PrimitiveKind::Cylinder,
                "cone" => PrimitiveKind::Cone,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown primitive '{other}'"
                    )));
                }
            };
            let size = args.get("size").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(kind.label())
                .to_string();
            history.execute(
                scene,
                Command::CreatePrimitiveMesh {
                    name,
                    kind,
                    size,
                    material,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": scene.selected_node.map(|n| n.0) }))
        }
        "character_apply_biped_rig" => {
            history.execute(scene, Command::ApplyBipedRig)?;
            Ok(json!({ "ok": true, "bones": 7 }))
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
            )?;
            let preset: ExportPreset = match args.get("preset") {
                Some(v) => serde_json::from_value(v.clone())?,
                None => ExportPreset::Godot4,
            };
            let report = export_scene_to_dir(scene, allowed_export_dir, filename, format, preset)?;
            Ok(serde_json::to_value(report)?)
        }
        "history_undo" => {
            history.undo(scene)?;
            Ok(json!({ "ok": true, "undo_remaining": history.undo_len() }))
        }
        "history_redo" => {
            history.redo(scene)?;
            Ok(json!({ "ok": true, "redo_remaining": history.redo_len() }))
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}

/// Builds a compact JSON summary of `SceneState` (~300 tokens) for MCP resources & Direct AI context.
pub fn scene_summary_json(scene: &SceneState) -> Value {
    let nodes: Vec<Value> = scene
        .nodes
        .values()
        .map(|n| {
            let detail = match n.kind {
                tessel_core::NodeKind::VoxelVolume => {
                    let count = scene
                        .voxel_volumes
                        .get(&n.id)
                        .map_or(0, |v| v.voxel_count());
                    json!({ "voxels": count })
                }
                tessel_core::NodeKind::LowPolyMesh => {
                    let tris = scene
                        .lowpoly_meshes
                        .get(&n.id)
                        .map_or(0, |m| m.triangle_count());
                    json!({ "triangles": tris })
                }
                tessel_core::NodeKind::PixelCanvas => {
                    let frames = scene
                        .pixel_canvases
                        .get(&n.id)
                        .map_or(0, |c| c.frames.len());
                    json!({ "frames": frames })
                }
                _ => json!({}),
            };
            json!({
                "id": n.id.0,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "visible": n.visible,
                "stats": detail
            })
        })
        .collect();

    json!({
        "active_mode": scene.active_mode.label(),
        "revision": scene.revision,
        "selected_node": scene.selected_node.map(|n| n.0),
        "has_character_rig": scene.character_rig.is_some(),
        "nodes": nodes
    })
}

pub fn palette_summary_json(scene: &SceneState) -> Value {
    let colors: Vec<Value> = scene
        .palette
        .iter()
        .map(|(idx, e)| {
            json!({
                "index": idx.get(),
                "name": e.name,
                "hex": format!("#{:02X}{:02X}{:02X}", e.color.r, e.color.g, e.color.b)
            })
        })
        .collect();
    json!({ "colors": colors })
}
