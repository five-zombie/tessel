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

use crate::error::AiError;

const MAX_COORD_ABS: i32 = 256;
const MAX_BATCH_VOXELS: usize = 50_000;

/// Returns a canonical (or absolute fallback) display string for the sandboxed export directory.
pub(crate) fn canonical_export_dir_string(allowed_export_dir: &Path) -> String {
    std::fs::canonicalize(allowed_export_dir)
        .or_else(|_| std::path::absolute(allowed_export_dir))
        .unwrap_or_else(|_| allowed_export_dir.to_path_buf())
        .display()
        .to_string()
}

fn parse_opt_bool(args: &Value, field: &str) -> Result<bool, AiError> {
    match args.get(field) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(AiError::InvalidArguments(format!(
            "'{field}' must be a boolean"
        ))),
    }
}

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
    let x_raw = arr[0]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[0]' must be int")))?;
    let y_raw = arr[1]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[1]' must be int")))?;
    let z_raw = arr[2]
        .as_i64()
        .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[2]' must be int")))?;
    let x = i32::try_from(x_raw).map_err(|_| AiError::OutOfBounds)?;
    let y = i32::try_from(y_raw).map_err(|_| AiError::OutOfBounds)?;
    let z = i32::try_from(z_raw).map_err(|_| AiError::OutOfBounds)?;
    let c = VoxelCoord::new(x, y, z);
    check_coord(c)?;
    Ok(c)
}

fn parse_opt_vec3(val: &Value, field: &str) -> Result<Option<glam::Vec3>, AiError> {
    let Some(raw) = val.get(field) else {
        return Ok(None);
    };
    let arr = raw.as_array().ok_or_else(|| {
        AiError::InvalidArguments(format!("'{field}' must be an array of 3 numbers"))
    })?;
    if arr.len() != 3 {
        return Err(AiError::InvalidArguments(format!(
            "'{field}' must have 3 numbers"
        )));
    }
    let mut out = [0.0f32; 3];
    for (i, item) in arr.iter().enumerate() {
        let v = item
            .as_f64()
            .ok_or_else(|| AiError::InvalidArguments(format!("'{field}[{i}]' must be number")))?
            as f32;
        if !v.is_finite() {
            return Err(AiError::InvalidArguments(format!(
                "'{field}[{i}]' must be finite"
            )));
        }
        out[i] = v;
    }
    Ok(Some(glam::Vec3::from_array(out)))
}

fn parse_grounding_args(
    args: &Value,
) -> Result<(Vec<NodeId>, tessel_core::GroundingOptions), AiError> {
    let mut surface_ids = Vec::new();
    if let Some(id) = args.get("surface_node_id").and_then(Value::as_u64) {
        surface_ids.push(NodeId(id));
    }
    if let Some(arr) = args.get("surface_node_ids").and_then(Value::as_array) {
        if arr.len() > 64 {
            return Err(AiError::InvalidArguments(
                "'surface_node_ids' may contain at most 64 entries".into(),
            ));
        }
        for item in arr {
            let id = item.as_u64().ok_or_else(|| {
                AiError::InvalidArguments("'surface_node_ids' entries must be integers".into())
            })?;
            let nid = NodeId(id);
            if !surface_ids.contains(&nid) {
                surface_ids.push(nid);
            }
        }
    }

    let mut contact_chains = Vec::new();
    if let Some(arr) = args.get("contact_chains").and_then(Value::as_array) {
        if arr.len() > 16 {
            return Err(AiError::InvalidArguments(
                "'contact_chains' may contain at most 16 entries".into(),
            ));
        }
        for item in arr {
            let ids = item.as_array().ok_or_else(|| {
                AiError::InvalidArguments(
                    "'contact_chains' entries must be [root_id, mid_id, end_id]".into(),
                )
            })?;
            if ids.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'contact_chains' entries must have 3 bone IDs".into(),
                ));
            }
            let parse_bid = |v: &Value| -> Result<BoneId, AiError> {
                let raw = v
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        AiError::InvalidArguments("contact chain bone_id must be u32".into())
                    })?;
                Ok(BoneId(raw))
            };
            contact_chains.push(tessel_core::ContactChain {
                root: parse_bid(&ids[0])?,
                mid: parse_bid(&ids[1])?,
                end: parse_bid(&ids[2])?,
            });
        }
    }

    let defaults = tessel_core::GroundingOptions::default();
    let step_up_clearance = args
        .get("step_up_clearance")
        .and_then(Value::as_f64)
        .map(|v| (v as f32).clamp(0.05, 20.0))
        .unwrap_or(defaults.step_up_clearance);
    let footprint_half_length = args
        .get("footprint_half_length")
        .and_then(Value::as_f64)
        .map(|v| (v as f32).clamp(0.0, 1.0))
        .unwrap_or(defaults.footprint_half_length);

    Ok((
        surface_ids,
        tessel_core::GroundingOptions {
            solve_foot_ik: args
                .get("solve_foot_ik")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            align_foot_normal: args
                .get("align_foot_normal")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            pelvis_tilt: args
                .get("pelvis_tilt")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            step_up_clearance,
            footprint_half_length,
            contact_chains,
        },
    ))
}

fn parse_rotation_deg(val: &Value) -> Result<glam::Quat, AiError> {
    let arr = val
        .get("rotation_deg")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::InvalidArguments("missing array 'rotation_deg'".into()))?;
    if arr.len() != 3 {
        return Err(AiError::InvalidArguments(
            "'rotation_deg' must have 3 numbers [pitch, yaw, roll]".into(),
        ));
    }
    let rx = arr[0]
        .as_f64()
        .ok_or_else(|| AiError::InvalidArguments("'rotation_deg[0]' must be number".into()))?
        as f32;
    let ry = arr[1]
        .as_f64()
        .ok_or_else(|| AiError::InvalidArguments("'rotation_deg[1]' must be number".into()))?
        as f32;
    let rz = arr[2]
        .as_f64()
        .ok_or_else(|| AiError::InvalidArguments("'rotation_deg[2]' must be number".into()))?
        as f32;
    if !rx.is_finite() || !ry.is_finite() || !rz.is_finite() {
        return Err(AiError::InvalidArguments(
            "'rotation_deg' values must be finite".into(),
        ));
    }
    Ok(glam::Quat::from_euler(
        glam::EulerRot::XYZ,
        rx.to_radians(),
        ry.to_radians(),
        rz.to_radians(),
    ))
}

/// Parses a `bone_id` u64 argument into a [`BoneId`] (rejects overflow).
fn parse_bone_id(args: &Value, key: &str) -> Result<BoneId, AiError> {
    let raw = args
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
    Ok(BoneId(u32::try_from(raw).map_err(|_| {
        AiError::InvalidArguments(format!("'{key}' overflows u32"))
    })?))
}

/// Converts a quaternion to XYZ euler degrees (matches `rotation_deg` input order).
fn quat_to_degrees(q: glam::Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
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

fn parse_opt_palette_index(
    args: &Value,
    default_if_missing: Option<u8>,
) -> Result<Option<PaletteIndex>, AiError> {
    match args.get("palette_index") {
        Some(Value::Null) => Ok(None),
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| {
                AiError::InvalidArguments(
                    "'palette_index' must be an integer 1..=255 or null".into(),
                )
            })?;
            let byte = u8::try_from(raw)
                .map_err(|_| AiError::InvalidArguments("'palette_index' must be 1..=255".into()))?;
            Ok(Some(PaletteIndex::new(byte)?))
        }
        None => match default_if_missing {
            Some(d) => Ok(Some(PaletteIndex::new(d)?)),
            None => Ok(None),
        },
    }
}

fn find_voxel_node(args: &Value, scene: &SceneState) -> Result<NodeId, AiError> {
    if let Some(raw_id) = args.get("node_id").and_then(Value::as_u64) {
        let id = NodeId(raw_id);
        if scene.voxel_volumes.contains_key(&id) {
            return Ok(id);
        }
        return Err(AiError::InvalidArguments(format!(
            "voxel volume node {} not found",
            id.0
        )));
    }
    scene
        .active_voxel_node()
        .ok_or_else(|| AiError::InvalidArguments("no voxel volume node in scene".into()))
}

fn find_pixel_node(args: &Value, scene: &SceneState) -> Result<NodeId, AiError> {
    if let Some(raw_id) = args.get("node_id").and_then(Value::as_u64) {
        let id = NodeId(raw_id);
        if scene.pixel_canvases.contains_key(&id) {
            return Ok(id);
        }
        return Err(AiError::InvalidArguments(format!(
            "pixel canvas node {} not found",
            id.0
        )));
    }
    scene
        .active_pixel_node()
        .ok_or_else(|| AiError::InvalidArguments("no pixel canvas node in scene".into()))
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

/// Resolves `node_id` to a pixel canvas, creating a default 32x32 canvas when
/// neither an explicit id nor an active pixel node exists.
fn resolve_pixel_node(
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) -> Result<NodeId, AiError> {
    if let Some(raw_id) = args.get("node_id").and_then(Value::as_u64) {
        return Ok(NodeId(raw_id));
    }
    if let Some(existing) = scene.active_pixel_node() {
        return Ok(existing);
    }
    history.execute(
        scene,
        Command::CreatePixelCanvas {
            name: "AI Pixel".into(),
            width: 32,
            height: 32,
        },
    )?;
    scene
        .active_pixel_node()
        .ok_or_else(|| AiError::InvalidArguments("failed to create pixel canvas".into()))
}

/// Resolves `frame` to a canvas frame index, defaulting to the active frame.
fn resolve_pixel_frame(
    args: &Value,
    frame_count: usize,
    active_frame: usize,
) -> Result<usize, AiError> {
    let frame = args
        .get("frame")
        .and_then(Value::as_u64)
        .map_or(active_frame, |f| f as usize);
    if frame >= frame_count {
        return Err(AiError::InvalidArguments(format!(
            "pixel frame {frame} out of range (0..{frame_count})"
        )));
    }
    Ok(frame)
}

/// Builds a JSON summary of one canvas frame's layers for MCP responses.
fn pixel_frame_summary(frame: &tessel_core::PixelFrame) -> Vec<Value> {
    frame
        .layers
        .iter()
        .enumerate()
        .map(|(idx, l)| {
            json!({
                "id": l.id.0,
                "name": l.name,
                "visible": l.visible,
                "opacity": l.opacity,
                "active": idx == frame.active_layer,
                "painted_pixels": l.pixels().iter().flatten().count(),
            })
        })
        .collect()
}

/// Resolves `clip_index` or `clip_name` to a clip index. With `auto_create`,
/// a missing clip name creates a new clip (needs `duration_secs`, default 2.0);
/// otherwise unknown names are an error. Falls back to the active clip.
fn resolve_clip_index(
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    auto_create: bool,
) -> Result<usize, AiError> {
    if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
        return Ok(idx as usize);
    }
    if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
        let existing = scene
            .character_rig
            .as_ref()
            .and_then(|r| r.animations.iter().position(|c| c.name == name));
        if let Some(i) = existing {
            return Ok(i);
        }
        if !auto_create {
            return Err(AiError::InvalidArguments(format!(
                "unknown animation clip '{name}'"
            )));
        }
        let duration_secs = args
            .get("duration_secs")
            .and_then(Value::as_f64)
            .unwrap_or(2.0) as f32;
        history.execute(
            scene,
            Command::AddAnimationClip {
                name: name.to_string(),
                duration_secs,
            },
        )?;
        return Ok(scene
            .character_rig
            .as_ref()
            .map_or(0, |r| r.animations.len().saturating_sub(1)));
    }
    Ok(scene
        .character_rig
        .as_ref()
        .map_or(0, |r| r.active_animation))
}

/// Resolves `"$N.field"` / `"$N.a.b"` placeholders in `scene_batch` arguments against prior results.
fn resolve_batch_refs(value: &Value, results: &[Value]) -> Result<Value, AiError> {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let rest = &s[1..];
            let Some((idx_str, path)) = rest.split_once('.') else {
                return Ok(value.clone());
            };
            let Ok(idx) = idx_str.parse::<usize>() else {
                return Ok(value.clone());
            };
            let mut cur = results.get(idx).ok_or_else(|| {
                AiError::InvalidArguments(format!(
                    "batch reference '{s}' out of range (only {} prior results)",
                    results.len()
                ))
            })?;
            for seg in path.split('.') {
                cur = if let Some(obj) = cur.as_object() {
                    obj.get(seg)
                } else if let Some(arr) = cur.as_array() {
                    seg.parse::<usize>().ok().and_then(|i| arr.get(i))
                } else {
                    None
                }
                .ok_or_else(|| {
                    AiError::InvalidArguments(format!(
                        "batch reference '{s}' missing segment '{seg}'"
                    ))
                })?;
            }
            Ok(cur.clone())
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_batch_refs(item, results)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), resolve_batch_refs(v, results)?);
            }
            Ok(Value::Object(out))
        }
        _ => Ok(value.clone()),
    }
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
            let normalized: String = mode_str
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let mode = match normalized.as_str() {
                "pixel" => AppMode::Pixel,
                "voxel" => AppMode::Voxel,
                "lowpoly" => AppMode::LowPoly,
                "character" => AppMode::Character,
                _ => {
                    return Err(AiError::InvalidArguments(format!(
                        "unsupported mode '{mode_str}'"
                    )));
                }
            };
            history.execute(scene, Command::SetMode(mode))?;
            Ok(json!({ "ok": true, "active_mode": mode.label() }))
        }
        "scene_inspect" | "scene_list_nodes" => Ok(json!({
            "ok": true,
            "export_dir": canonical_export_dir_string(allowed_export_dir),
            "scene": scene_summary_json(scene),
            "palette": palette_summary_json(scene)["colors"]
        })),
        "palette_set_color" => {
            let index = parse_palette_index(args, 0)?;
            let mut color = if let Some(arr) = args.get("rgb").and_then(Value::as_array) {
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'rgb' must have 3 integers [r, g, b]".into(),
                    ));
                }
                let parse_ch = |i: usize| -> Result<u8, AiError> {
                    let v = arr[i].as_u64().ok_or_else(|| {
                        AiError::InvalidArguments(format!("'rgb[{i}]' must be an integer 0..=255"))
                    })?;
                    u8::try_from(v).map_err(|_| {
                        AiError::InvalidArguments(format!("'rgb[{i}]' out of range 0..=255"))
                    })
                };
                let (r, g, b) = (parse_ch(0)?, parse_ch(1)?, parse_ch(2)?);
                Rgba8::rgb(r, g, b)
            } else if let Some(hex_raw) = args.get("hex").and_then(Value::as_str) {
                let hex = hex_raw.trim().trim_start_matches('#');
                if hex.len() != 6 && hex.len() != 8 {
                    return Err(AiError::InvalidArguments(
                        "'hex' must be 6 or 8 hex digits '#RRGGBB' or '#RRGGBBAA'".into(),
                    ));
                }
                let r = u8::from_str_radix(&hex[0..2], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex red".into()))?;
                let g = u8::from_str_radix(&hex[2..4], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex green".into()))?;
                let b = u8::from_str_radix(&hex[4..6], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex blue".into()))?;
                let a = if hex.len() == 8 {
                    u8::from_str_radix(&hex[6..8], 16)
                        .map_err(|_| AiError::InvalidArguments("invalid hex alpha".into()))?
                } else {
                    255
                };
                Rgba8::rgba(r, g, b, a)
            } else {
                return Err(AiError::InvalidArguments(
                    "provide either 'rgb': [r,g,b] or 'hex': '#RRGGBB'".into(),
                ));
            };
            if let Some(a_val) = args.get("alpha").and_then(Value::as_u64) {
                color.a = u8::try_from(a_val)
                    .map_err(|_| AiError::InvalidArguments("'alpha' must be 0..=255".into()))?;
            }
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("Color {}", index.get()));
            let mut entry = PaletteEntry::matte(name.clone(), color);
            if let Some(r) = args.get("roughness").and_then(Value::as_f64) {
                let rf = r as f32;
                if rf.is_finite() {
                    entry.roughness = rf.clamp(0.05, 1.0);
                }
            }
            if let Some(m) = args.get("metallic").and_then(Value::as_f64) {
                let mf = m as f32;
                if mf.is_finite() {
                    entry.metallic = mf.clamp(0.0, 1.0);
                }
            }
            if let Some(e) = args.get("emissive").and_then(Value::as_f64) {
                let ef = e as f32;
                if ef.is_finite() {
                    entry.emissive = ef.clamp(0.0, 10.0);
                }
            }
            history.execute(scene, Command::SetPaletteEntry { index, entry })?;
            Ok(json!({
                "ok": true,
                "palette_index": index.get(),
                "name": name,
                "alpha": color.a,
                "hex": format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
            }))
        }
        "scene_select_node" => {
            let target = match args.get("node_id") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let raw = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments("'node_id' must be integer or null".into())
                    })?;
                    let id = NodeId(raw);
                    if !scene.nodes.contains_key(&id) {
                        return Err(AiError::InvalidArguments(format!(
                            "node {} not found",
                            id.0
                        )));
                    }
                    Some(id)
                }
            };
            history.execute(scene, Command::SelectNode(target))?;
            Ok(json!({ "ok": true, "selected_node_id": target.map(|n| n.0) }))
        }
        "scene_delete_node" => {
            let raw_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let node_id = NodeId(raw_id);
            history.execute(scene, Command::DeleteNode(node_id))?;
            Ok(json!({ "ok": true, "deleted_node_id": raw_id }))
        }
        "scene_rename_node" => {
            let raw_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                .trim()
                .to_string();
            if name.is_empty() {
                return Err(AiError::InvalidArguments("'name' cannot be empty".into()));
            }
            history.execute(
                scene,
                Command::RenameNode {
                    node_id: NodeId(raw_id),
                    name: name.clone(),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": raw_id, "name": name }))
        }
        "scene_clear" => {
            let removed = scene.nodes.len();
            history.execute(scene, Command::ClearScene)?;
            Ok(json!({ "ok": true, "removed_nodes": removed }))
        }
        "scene_batch" => {
            const MAX_BATCH_CALLS: usize = 512;
            let calls = args
                .get("calls")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'calls' array".into()))?;
            if calls.is_empty() {
                return Err(AiError::InvalidArguments(
                    "'calls' must contain at least 1 tool call".into(),
                ));
            }
            if calls.len() > MAX_BATCH_CALLS {
                return Err(AiError::BatchLimitExceeded);
            }
            let prev_scene = scene.clone();
            let prev_history = history.clone();
            let mut results = Vec::with_capacity(calls.len());
            for (idx, item) in calls.iter().enumerate() {
                let Some(sub_name) = item.get("name").and_then(Value::as_str) else {
                    *scene = prev_scene;
                    *history = prev_history;
                    return Err(AiError::InvalidArguments(format!(
                        "calls[{idx}] missing 'name'"
                    )));
                };
                if sub_name == "scene_batch" {
                    *scene = prev_scene;
                    *history = prev_history;
                    return Err(AiError::InvalidArguments(
                        "nested 'scene_batch' is not allowed".into(),
                    ));
                }
                let raw_args = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let sub_args = match resolve_batch_refs(&raw_args, &results) {
                    Ok(v) => v,
                    Err(err) => {
                        *scene = prev_scene;
                        *history = prev_history;
                        return Err(AiError::InvalidArguments(format!(
                            "batch aborted at calls[{idx}] ({sub_name}): {err}"
                        )));
                    }
                };
                match execute_tool_call(sub_name, &sub_args, scene, history, allowed_export_dir) {
                    Ok(val) => results.push(val),
                    Err(err) => {
                        *scene = prev_scene;
                        *history = prev_history;
                        return Err(AiError::InvalidArguments(format!(
                            "batch aborted at calls[{idx}] ({sub_name}): {err}"
                        )));
                    }
                }
            }
            Ok(json!({
                "ok": true,
                "count": results.len(),
                "results": results
            }))
        }
        "scene_create_group" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Group")
                .to_string();
            let mut transform = tessel_core::Transform::default();
            if let Some(pos) = parse_opt_vec3(args, "position")? {
                transform.translation = pos;
            }
            if args.get("rotation_deg").is_some() {
                transform.rotation = parse_rotation_deg(args)?;
            }
            if let Some(sc) = parse_opt_vec3(args, "scale")? {
                transform.scale = sc;
            }
            let parent = args
                .get("parent_node_id")
                .and_then(Value::as_u64)
                .map(NodeId);
            history.execute(
                scene,
                Command::CreateGroupNode {
                    name: name.clone(),
                    transform,
                    parent,
                },
            )?;
            let id = scene.selected_node.ok_or_else(|| {
                AiError::InvalidArguments("group creation produced no node".into())
            })?;
            Ok(json!({
                "ok": true,
                "node_id": id.0,
                "name": name,
                "parent_node_id": parent.map(|p| p.0)
            }))
        }
        "scene_set_node_keyframe" => {
            let parse_interp = |obj: &Value| -> Result<tessel_core::KeyframeInterpolation, AiError> {
                match obj
                    .get("interpolation")
                    .and_then(Value::as_str)
                    .unwrap_or("linear")
                {
                    "linear" => Ok(tessel_core::KeyframeInterpolation::Linear),
                    "step" => Ok(tessel_core::KeyframeInterpolation::Step),
                    "ease_in" => Ok(tessel_core::KeyframeInterpolation::EaseIn),
                    "ease_out" => Ok(tessel_core::KeyframeInterpolation::EaseOut),
                    "ease_in_out" => Ok(tessel_core::KeyframeInterpolation::EaseInOut),
                    "cubic_bezier" => {
                        let (p1x, p1y, p2x, p2y) = match obj.get("bezier") {
                            None | Some(Value::Null) => (0.25, 0.1, 0.25, 1.0),
                            Some(val) => {
                                let a = val.as_array().ok_or_else(|| {
                                    AiError::InvalidArguments(
                                        "'bezier' must be an array of 4 numbers [p1x,p1y,p2x,p2y]"
                                            .into(),
                                    )
                                })?;
                                if a.len() != 4 {
                                    return Err(AiError::InvalidArguments(
                                        "'bezier' must have exactly 4 numbers [p1x,p1y,p2x,p2y]"
                                            .into(),
                                    ));
                                }
                                let mut pts = [0.0f32; 4];
                                for (i, item) in a.iter().enumerate() {
                                    let v = item.as_f64().ok_or_else(|| {
                                        AiError::InvalidArguments(
                                            "'bezier' entries must be finite numbers".into(),
                                        )
                                    })? as f32;
                                    if !v.is_finite() {
                                        return Err(AiError::InvalidArguments(
                                            "'bezier' entries must be finite numbers".into(),
                                        ));
                                    }
                                    pts[i] = v;
                                }
                                (
                                    pts[0].clamp(0.0, 1.0),
                                    pts[1].clamp(-1.0, 2.0),
                                    pts[2].clamp(0.0, 1.0),
                                    pts[3].clamp(-1.0, 2.0),
                                )
                            }
                        };
                        Ok(tessel_core::KeyframeInterpolation::CubicBezier {
                            p1x,
                            p1y,
                            p2x,
                            p2y,
                        })
                    }
                    other => Err(AiError::InvalidArguments(format!(
                        "unsupported interpolation '{other}'; use linear|step|ease_in|ease_out|ease_in_out|cubic_bezier"
                    ))),
                }
            };
            let parse_entry = |obj: &Value| -> Result<(NodeId, Keyframe), AiError> {
                let node_id = obj
                    .get("node_id")
                    .and_then(Value::as_u64)
                    .map(NodeId)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
                let time_secs = obj
                    .get("time_secs")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                    as f32;
                let rotation = if obj.get("rotation_deg").is_some() {
                    parse_rotation_deg(obj)?
                } else {
                    glam::Quat::IDENTITY
                };
                let translation = parse_opt_vec3(obj, "translation")?.unwrap_or(glam::Vec3::ZERO);
                let scale = parse_opt_vec3(obj, "scale")?.unwrap_or(glam::Vec3::ONE);
                let interpolation = parse_interp(obj)?;
                Ok((
                    node_id,
                    Keyframe {
                        time_secs,
                        translation,
                        rotation,
                        scale,
                        interpolation,
                    },
                ))
            };

            let bulk_entries: Option<Vec<(NodeId, Keyframe)>> =
                if let Some(arr) = args.get("keyframes") {
                    let list = arr.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'keyframes' must be an array".into())
                    })?;
                    if list.is_empty() || list.len() > 2048 {
                        return Err(AiError::InvalidArguments(
                            "'keyframes' must contain between 1 and 2048 entries".into(),
                        ));
                    }
                    let mut parsed = Vec::with_capacity(list.len());
                    for item in list {
                        parsed.push(parse_entry(item)?);
                    }
                    Some(parsed)
                } else {
                    None
                };

            let single_entry = if bulk_entries.is_none() {
                Some(parse_entry(args)?)
            } else {
                None
            };

            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                if let Some(i) = existing {
                    i
                } else {
                    let duration_secs = args
                        .get("duration_secs")
                        .and_then(Value::as_f64)
                        .unwrap_or(2.0) as f32;
                    history.execute(
                        scene,
                        Command::AddAnimationClip {
                            name: name.to_string(),
                            duration_secs,
                        },
                    )?;
                    scene
                        .character_rig
                        .as_ref()
                        .map_or(0, |r| r.animations.len().saturating_sub(1))
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map_or(0, |r| r.active_animation)
            };

            if let Some(entries) = bulk_entries {
                let count = entries.len();
                let cmds: Vec<Command> = entries
                    .into_iter()
                    .map(|(node_id, keyframe)| Command::SetNodeTransformKeyframe {
                        clip_index,
                        node_id,
                        keyframe,
                    })
                    .collect();
                history.execute(scene, Command::Transaction(cmds))?;
                return Ok(json!({
                    "ok": true,
                    "clip_index": clip_index,
                    "count": count
                }));
            }

            let Some((node_id, keyframe)) = single_entry else {
                return Err(AiError::InvalidArguments("missing keyframe payload".into()));
            };
            let time_secs = keyframe.time_secs;
            let translation = keyframe.translation;
            let scale = keyframe.scale;
            history.execute(
                scene,
                Command::SetNodeTransformKeyframe {
                    clip_index,
                    node_id,
                    keyframe,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "node_id": node_id.0,
                "time_secs": time_secs,
                "translation": translation.to_array(),
                "scale": scale.to_array()
            }))
        }
        "scene_set_node_transform" | "set_node_transform" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or(scene.selected_node)
                .ok_or_else(|| {
                    AiError::InvalidArguments("missing 'node_id' and no node selected".into())
                })?;
            let mut transform = scene
                .nodes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments(format!("node {} not found", node_id.0)))?
                .transform;
            if let Some(pos) = parse_opt_vec3(args, "position")? {
                transform.translation = pos;
            }
            if args.get("rotation_deg").is_some() {
                transform.rotation = parse_rotation_deg(args)?;
            }
            if let Some(sc) = parse_opt_vec3(args, "scale")? {
                transform.scale = sc;
            }
            let dry_run = parse_opt_bool(args, "dry_run")?;
            if dry_run {
                let parent_mat = scene
                    .nodes
                    .get(&node_id)
                    .and_then(|n| n.parent)
                    .map(|pid| scene.node_world_matrix(pid))
                    .unwrap_or(glam::Mat4::IDENTITY);
                let local_mat = glam::Mat4::from_scale_rotation_translation(
                    transform.scale,
                    transform.rotation,
                    transform.translation,
                );
                let world_mat = parent_mat * local_mat;
                let world_pos = world_mat.transform_point3(glam::Vec3::ZERO);
                let world_up = world_mat
                    .transform_vector3(glam::Vec3::Y)
                    .normalize_or_zero();
                let world_fwd = world_mat
                    .transform_vector3(glam::Vec3::Z)
                    .normalize_or_zero();
                let world_bounds = scene.lowpoly_meshes.get(&node_id).and_then(|m| {
                    if m.vertices.is_empty() {
                        return None;
                    }
                    let mut bmin = glam::Vec3::splat(f32::INFINITY);
                    let mut bmax = glam::Vec3::splat(f32::NEG_INFINITY);
                    for &v in &m.vertices {
                        let wv = world_mat.transform_point3(v);
                        bmin = bmin.min(wv);
                        bmax = bmax.max(wv);
                    }
                    (bmin.is_finite() && bmax.is_finite())
                        .then(|| json!({ "min": bmin.to_array(), "max": bmax.to_array() }))
                });
                return Ok(json!({
                    "ok": true,
                    "dry_run": true,
                    "node_id": node_id.0,
                    "position": transform.translation.to_array(),
                    "scale": transform.scale.to_array(),
                    "world_position": world_pos.to_array(),
                    "world_up_dir": world_up.to_array(),
                    "world_forward_dir": world_fwd.to_array(),
                    "world_bounds": world_bounds
                }));
            }
            history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "position": transform.translation.to_array(),
                "scale": transform.scale.to_array()
            }))
        }
        "scene_set_parent" => {
            let raw_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let node_id = NodeId(raw_id);
            let parent = args
                .get("parent_node_id")
                .and_then(Value::as_u64)
                .map(NodeId);
            history.execute(scene, Command::SetNodeParent { node_id, parent })?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "parent_node_id": parent.map(|p| p.0)
            }))
        }
        "scene_set_lighting" => {
            let azimuth = args
                .get("azimuth_deg")
                .and_then(Value::as_f64)
                .map(|d| (d as f32).to_radians())
                .unwrap_or(scene.light_azimuth);
            let elevation = args
                .get("elevation_deg")
                .and_then(Value::as_f64)
                .map(|d| (d as f32).to_radians())
                .unwrap_or(scene.light_elevation);
            history.execute(scene, Command::SetLightDirection { azimuth, elevation })?;
            Ok(json!({
                "ok": true,
                "azimuth_deg": scene.light_azimuth.to_degrees(),
                "elevation_deg": scene.light_elevation.to_degrees()
            }))
        }
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
            let sym = tessel_core::Symmetry3D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
                z: args.get("mirror_z").and_then(Value::as_bool).unwrap_or(false),
            };
            let mut edits = Vec::with_capacity(raw.len());
            for (i, item) in raw.iter().enumerate() {
                let x = item
                    .get("x")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].x missing")))?
                    as i32;
                let y = item
                    .get("y")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].y missing")))?
                    as i32;
                let z = item
                    .get("z")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("voxels[{i}].z missing")))?
                    as i32;
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
            let sym = tessel_core::Symmetry3D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
                z: args.get("mirror_z").and_then(Value::as_bool).unwrap_or(false),
            };
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
            let sym = tessel_core::Symmetry3D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
                z: args.get("mirror_z").and_then(Value::as_bool).unwrap_or(false),
            };
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
        "pixel_create_canvas" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Pixel Sprite")
                .to_string();
            let width = args.get("width").and_then(Value::as_u64).unwrap_or(32);
            let height = args.get("height").and_then(Value::as_u64).unwrap_or(32);
            if width == 0 || height == 0 || width > 512 || height > 512 {
                return Err(AiError::InvalidArguments(
                    "pixel canvas dimensions must be 1..=512".into(),
                ));
            }
            history.execute(
                scene,
                Command::CreatePixelCanvas {
                    name,
                    width: width as u32,
                    height: height as u32,
                },
            )?;
            let node_id = scene.active_pixel_node().ok_or_else(|| {
                AiError::InvalidArguments("failed to create pixel canvas".into())
            })?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("failed to create pixel canvas".into()))?;
            let frame = canvas
                .current_frame()
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas has no frames".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": canvas.width,
                "height": canvas.height,
                "active_frame": canvas.active_frame,
                "layers": pixel_frame_summary(frame),
            }))
        }
        "pixel_add_layer" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let (frame_count, active_frame) = {
                let canvas = scene
                    .pixel_canvases
                    .get(&node_id)
                    .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
                (canvas.frames.len(), canvas.active_frame)
            };
            let frame_idx = resolve_pixel_frame(args, frame_count, active_frame)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Layer")
                .to_string();
            history.execute(
                scene,
                Command::AddPixelLayer {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    name,
                },
            )?;
            let layer_id = scene
                .pixel_canvases
                .get(&node_id)
                .and_then(|c| c.frame(FrameIndex(frame_idx)))
                .and_then(|f| f.layers.last())
                .map(|l| l.id.0)
                .ok_or_else(|| AiError::InvalidArguments("pixel layer not found".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id,
            }))
        }
        "pixel_draw" => {
            const MAX_PIXEL_EDITS: usize = 16_384;
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (u64::from(canvas.width), u64::from(canvas.height));
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(u32::try_from(raw).map_err(|_| {
                    AiError::InvalidArguments("layer_id out of range".into())
                })?),
                None => frame.active_layer_id().ok_or_else(|| {
                    AiError::InvalidArguments("pixel frame has no layers".into())
                })?,
            };
            if frame.layer(layer_id).is_none() {
                return Err(AiError::InvalidArguments(format!(
                    "pixel layer {} not found in frame {frame_idx}",
                    layer_id.0
                )));
            }
            let raw_pixels = args
                .get("pixels")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'pixels' array".into()))?;
            if raw_pixels.is_empty() || raw_pixels.len() > MAX_PIXEL_EDITS {
                return Err(AiError::InvalidArguments(format!(
                    "pixel_draw needs 1..={MAX_PIXEL_EDITS} pixels"
                )));
            }
            let sym = tessel_core::Symmetry2D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
            };
            let mut edits = Vec::with_capacity(raw_pixels.len());
            for (i, item) in raw_pixels.iter().enumerate() {
                let x = item
                    .get("x")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("pixels[{i}].x missing")))?;
                let y = item
                    .get("y")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("pixels[{i}].y missing")))?;
                if x >= width || y >= height {
                    return Err(AiError::InvalidArguments(format!(
                        "pixels[{i}] ({x},{y}) outside {width}x{height} canvas"
                    )));
                }
                let color = parse_opt_palette_index(item, None)?;
                let pts = tessel_core::pixel::ops::expand_symmetry(
                    PixelCoord::new(x as u32, y as u32),
                    width as u32,
                    height as u32,
                    sym,
                );
                for coord in pts {
                    if edits.len() >= MAX_PIXEL_EDITS {
                        return Err(AiError::InvalidArguments(format!(
                            "pixel_draw exceeds {MAX_PIXEL_EDITS} pixel edits after symmetry"
                        )));
                    }
                    edits.push(PixelEdit { coord, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted,
            }))
        }
        "pixel_set_layer" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let raw_lid = args
                .get("layer_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'layer_id'".into()))?;
            let layer_id = LayerId(
                u32::try_from(raw_lid)
                    .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
            );
            let mut cmds = Vec::new();
            if let Some(vis) = args.get("visible").and_then(Value::as_bool) {
                cmds.push(Command::SetPixelLayerVisible {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    visible: vis,
                });
            }
            if let Some(op_val) = args.get("opacity") {
                let op = op_val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'opacity' must be a finite number 0.0..=1.0".into())
                })? as f32;
                if !op.is_finite() {
                    return Err(AiError::InvalidArguments(
                        "'opacity' must be a finite number 0.0..=1.0".into(),
                    ));
                }
                cmds.push(Command::SetPixelLayerOpacity {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    opacity: op.clamp(0.0, 1.0),
                });
            }
            if cmds.is_empty() {
                return Err(AiError::InvalidArguments(
                    "pixel_set_layer requires 'visible' and/or 'opacity'".into(),
                ));
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0
            }))
        }
        "pixel_set_onion_skin" => {
            let node_id = find_pixel_node(args, scene)?;
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| AiError::InvalidArguments("missing boolean 'enabled'".into()))?;
            history.execute(scene, Command::SetPixelOnionSkin { node_id, enabled })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "onion_skin": enabled }))
        }
        "pixel_flood_fill" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let layer = frame
                .layer(layer_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel layer not found".into()))?;
            let x = args
                .get("x")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'x'".into()))?;
            let y = args
                .get("y")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'y'".into()))?;
            if x >= u64::from(canvas.width) || y >= u64::from(canvas.height) {
                return Err(AiError::InvalidArguments(
                    "flood fill coordinate outside canvas".into(),
                ));
            }
            let color = parse_opt_palette_index(args, None)?;
            let edits = tessel_core::pixel::ops::flood_fill_edits(
                layer,
                PixelCoord::new(x as u32, y as u32),
                color,
            );
            let filled = edits.len();
            if !edits.is_empty() {
                history.execute(
                    scene,
                    Command::DrawPixels {
                        node_id,
                        frame: FrameIndex(frame_idx),
                        layer_id,
                        edits,
                    },
                )?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_filled": filled
            }))
        }
        "pixel_draw_line" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (canvas.width, canvas.height);
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let parse_pt = |key: &str| -> Result<PixelCoord, AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() == 2)
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}' must be [x, y]")))?;
                let px = arr[0]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[0]' must be int")))?;
                let py = arr[1]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[1]' must be int")))?;
                if px >= u64::from(width) || py >= u64::from(height) {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' ({px},{py}) outside {width}x{height} canvas"
                    )));
                }
                Ok(PixelCoord::new(px as u32, py as u32))
            };
            let start = parse_pt("start")?;
            let end = parse_pt("end")?;
            let color = parse_opt_palette_index(args, None)?;
            let sym = tessel_core::Symmetry2D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
            };
            let mut edits = Vec::new();
            for pt in tessel_core::pixel::ops::line_points(start, end) {
                for mc in tessel_core::pixel::ops::expand_symmetry(pt, width, height, sym) {
                    edits.push(PixelEdit { coord: mc, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted
            }))
        }
        "pixel_draw_rect" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (canvas.width, canvas.height);
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let parse_pt = |key: &str| -> Result<PixelCoord, AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() == 2)
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}' must be [x, y]")))?;
                let px = arr[0]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[0]' must be int")))?;
                let py = arr[1]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[1]' must be int")))?;
                if px >= u64::from(width) || py >= u64::from(height) {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' ({px},{py}) outside {width}x{height} canvas"
                    )));
                }
                Ok(PixelCoord::new(px as u32, py as u32))
            };
            let p0 = parse_pt("min")?;
            let p1 = parse_pt("max")?;
            let filled = args.get("filled").and_then(Value::as_bool).unwrap_or(false);
            let color = parse_opt_palette_index(args, None)?;
            let sym = tessel_core::Symmetry2D {
                x: args.get("mirror_x").and_then(Value::as_bool).unwrap_or(false),
                y: args.get("mirror_y").and_then(Value::as_bool).unwrap_or(false),
            };
            let pts: Vec<PixelCoord> = if filled {
                let (x0, x1) = (p0.x.min(p1.x), p0.x.max(p1.x));
                let (y0, y1) = (p0.y.min(p1.y), p0.y.max(p1.y));
                let mut v = Vec::new();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        v.push(PixelCoord::new(x, y));
                    }
                }
                v
            } else {
                tessel_core::pixel::ops::rect_outline_points(p0, p1)
            };
            const MAX_RECT_PIXEL_EDITS: usize = 16_384;
            let mut edits = Vec::with_capacity(pts.len());
            for pt in pts {
                for mc in tessel_core::pixel::ops::expand_symmetry(pt, width, height, sym) {
                    if edits.len() >= MAX_RECT_PIXEL_EDITS {
                        return Err(AiError::InvalidArguments(format!(
                            "pixel_draw_rect exceeds {MAX_RECT_PIXEL_EDITS} pixels"
                        )));
                    }
                    edits.push(PixelEdit { coord: mc, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted
            }))
        }
        "pixel_convert_to_voxel" => {
            let pixel_node_id = find_pixel_node(args, scene)?;
            let depth = args
                .get("depth")
                .and_then(Value::as_u64)
                .map(|d| (d.min(32) as u32).max(1))
                .unwrap_or(4);
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Converted Voxel")
                .to_string();
            history.execute(
                scene,
                Command::ConvertPixelToVoxel {
                    pixel_node_id,
                    depth,
                    name,
                },
            )?;
            Ok(json!({ "ok": true, "new_node_id": scene.selected_node.map(|n| n.0) }))
        }
        "pixel_add_frame" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            history.execute(scene, Command::AddPixelFrame { node_id })?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame = canvas
                .current_frame()
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas has no frames".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": canvas.active_frame,
                "layers": pixel_frame_summary(frame),
            }))
        }
        "pixel_set_active_frame" => {
            let node_id = find_pixel_node(args, scene)?;
            let frame_count = scene
                .pixel_canvases
                .get(&node_id)
                .map(|c| c.frames.len())
                .unwrap_or(0);
            let raw_frame = args
                .get("frame")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'frame'".into()))?;
            let frame = usize::try_from(raw_frame)
                .map_err(|_| AiError::InvalidArguments("frame overflow".into()))?;
            if frame >= frame_count {
                return Err(AiError::InvalidArguments(format!(
                    "pixel frame {frame} out of range (0..{frame_count})"
                )));
            }
            history.execute(
                scene,
                Command::SetActivePixelFrame {
                    node_id,
                    frame: FrameIndex(frame),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "frame": frame }))
        }
        "pixel_set_fps" => {
            let node_id = find_pixel_node(args, scene)?;
            let fps = args
                .get("fps")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'fps'".into()))?;
            if fps == 0 || fps > 60 {
                return Err(AiError::InvalidArguments("fps must be 1..=60".into()));
            }
            history.execute(
                scene,
                Command::SetPixelFps {
                    node_id,
                    fps: fps as u16,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "fps": fps }))
        }
        "pixel_inspect" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let only_frame = match args.get("frame").and_then(Value::as_u64) {
                Some(raw) => Some(
                    usize::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("frame overflow".into()))?,
                ),
                None => None,
            };
            let mut frames = Vec::with_capacity(canvas.frames.len());
            for (idx, frame) in canvas.frames.iter().enumerate() {
                if only_frame.is_some_and(|f| f != idx) {
                    continue;
                }
                frames.push(json!({
                    "index": idx,
                    "layers": pixel_frame_summary(frame),
                }));
            }
            if frames.is_empty() {
                return Err(AiError::InvalidArguments("pixel frame not found".into()));
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": canvas.width,
                "height": canvas.height,
                "fps": canvas.fps,
                "onion_skin": canvas.onion_skin,
                "active_frame": canvas.active_frame,
                "frame_count": canvas.frames.len(),
                "frames": frames,
            }))
        }
        "pixel_export_sprite_sheet" => {
            let node_id = find_pixel_node(args, scene)?;
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("sprite_sheet.png");
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (bytes, meta) = export_sprite_sheet_bytes(canvas, &scene.palette)?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(tessel_io::IoError::from)?;
            }
            std::fs::write(&target, &bytes).map_err(tessel_io::IoError::from)?;
            Ok(json!({
                "ok": true,
                "output_path": target.display().to_string(),
                "file_size_bytes": bytes.len(),
                "sheet_width": meta.sheet_width,
                "sheet_height": meta.sheet_height,
                "fps": meta.fps,
                "frames": meta.frames,
            }))
        }
        "mesh_create_primitive" => {
            let kind_str = args.get("kind").and_then(Value::as_str).unwrap_or("cube");
            let kind = match kind_str {
                "cube" => PrimitiveKind::Cube,
                "plane" => PrimitiveKind::Plane,
                "cylinder" => PrimitiveKind::Cylinder,
                "cone" => PrimitiveKind::Cone,
                "sphere" => PrimitiveKind::Sphere,
                "torus" => PrimitiveKind::Torus,
                "capsule" => PrimitiveKind::Capsule,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown primitive '{other}'"
                    )));
                }
            };
            let (size, size_vec_scale) = match args.get("size") {
                None => (1.0f32, None),
                Some(Value::Array(_)) => {
                    let v = parse_opt_vec3(args, "size")?.ok_or_else(|| {
                        AiError::InvalidArguments("'size' array must be [sx, sy, sz]".into())
                    })?;
                    if v.x <= 0.0 || v.y <= 0.0 || v.z <= 0.0 {
                        return Err(AiError::InvalidArguments(
                            "'size' components must be finite and > 0".into(),
                        ));
                    }
                    (1.0f32, Some(v))
                }
                Some(v) => {
                    let s = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(
                            "'size' must be a number or [sx, sy, sz]".into(),
                        )
                    })? as f32;
                    if !s.is_finite() || s <= 0.0 {
                        return Err(AiError::InvalidArguments(
                            "'size' must be finite and > 0".into(),
                        ));
                    }
                    (s, None)
                }
            };
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(kind.label())
                .to_string();
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            let anchor = match args.get("anchor") {
                None => PrimitiveAnchor::Bottom,
                Some(Value::String(s)) => match s.as_str() {
                    "bottom" => PrimitiveAnchor::Bottom,
                    "center" => PrimitiveAnchor::Center,
                    "top" => PrimitiveAnchor::Top,
                    other => {
                        return Err(AiError::InvalidArguments(format!(
                            "unknown anchor '{other}' (expected 'bottom', 'center', or 'top')"
                        )));
                    }
                },
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'anchor' must be a string ('bottom', 'center', or 'top')".into(),
                    ));
                }
            };
            let min_s = tessel_core::mesh::ops::MIN_PRIMITIVE_SIZE;
            let (actual_size_json, clamped) = if let Some(v) = size_vec_scale {
                let actual_v = glam::Vec3::new(v.x.max(min_s), v.y.max(min_s), v.z.max(min_s));
                let mut mesh = tessel_core::mesh::ops::create_primitive(kind, 1.0, material);
                for vert in &mut mesh.vertices {
                    *vert *= actual_v;
                }
                apply_primitive_anchor(&mut mesh, anchor);
                history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
                let was_clamped = (actual_v - v).abs().max_element() > 1e-6;
                (json!(actual_v.to_array()), was_clamped)
            } else if anchor != PrimitiveAnchor::Bottom {
                let actual_s = size.max(min_s);
                let mut mesh = tessel_core::mesh::ops::create_primitive(kind, actual_s, material);
                apply_primitive_anchor(&mut mesh, anchor);
                history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
                (json!(actual_s), (actual_s - size).abs() > 1e-6)
            } else {
                history.execute(
                    scene,
                    Command::CreatePrimitiveMesh {
                        name,
                        kind,
                        size,
                        material,
                    },
                )?;
                let actual_s = size.max(min_s);
                (json!(actual_s), (actual_s - size).abs() > 1e-6)
            };
            let created_id = scene.selected_node;
            if let (Some(node_id), true) =
                (created_id, pos.is_some() || rot.is_some() || sc.is_some())
            {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                if let Some(p) = pos {
                    transform.translation = p;
                }
                if let Some(r) = rot {
                    transform.rotation = r;
                }
                if let Some(s) = sc {
                    transform.scale = s;
                }
                history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            }
            let bounds = created_id
                .and_then(|id| scene.lowpoly_meshes.get(&id))
                .map(|m| {
                    let (lo, hi) = m.bounds();
                    json!({ "min": lo.to_array(), "max": hi.to_array() })
                });
            Ok(json!({
                "ok": true,
                "node_id": created_id.map(|n| n.0),
                "anchor": anchor.as_str(),
                "actual_size": actual_size_json,
                "clamped": clamped,
                "bounds": bounds
            }))
        }
        "mesh_create_terrain" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Terrain")
                .to_string();
            let width = args.get("width").and_then(Value::as_f64).unwrap_or(10.0) as f32;
            let depth = args.get("depth").and_then(Value::as_f64).unwrap_or(10.0) as f32;
            let subdiv_x = args.get("subdiv_x").and_then(Value::as_u64).unwrap_or(16) as u32;
            let subdiv_z = args.get("subdiv_z").and_then(Value::as_u64).unwrap_or(16) as u32;
            let height_scale = args
                .get("height_scale")
                .and_then(Value::as_f64)
                .unwrap_or(2.0) as f32;
            let seed = args.get("seed").and_then(Value::as_u64).unwrap_or(42);
            let octaves = args.get("octaves").and_then(Value::as_u64).unwrap_or(4) as u32;
            let frequency = args
                .get("frequency")
                .and_then(Value::as_f64)
                .unwrap_or(0.35) as f32;
            let island_falloff = args
                .get("island_falloff")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32;
            let smooth_shading = args
                .get("smooth_shading")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let default_material = parse_palette_index(args, scene.active_color.get())?;
            let heights = match args.get("heights") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let arr = v.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'heights' must be an array of numbers".into())
                    })?;
                    let expected_len = (subdiv_x.clamp(1, 128) as usize + 1)
                        * (subdiv_z.clamp(1, 128) as usize + 1);
                    if arr.len() != expected_len {
                        return Err(AiError::InvalidArguments(format!(
                            "'heights' length ({}) must equal (subdiv_x+1)*(subdiv_z+1) ({expected_len})",
                            arr.len()
                        )));
                    }
                    let mut out = Vec::with_capacity(arr.len());
                    for val in arr {
                        let f = val.as_f64().ok_or_else(|| {
                            AiError::InvalidArguments("'heights' entries must be numbers".into())
                        })? as f32;
                        if !f.is_finite() {
                            return Err(AiError::InvalidArguments(
                                "'heights' entries must be finite".into(),
                            ));
                        }
                        out.push(f.clamp(-10_000.0, 10_000.0));
                    }
                    Some(out)
                }
            };
            let palette_bands = match args.get("palette_bands") {
                None | Some(Value::Null) => Vec::new(),
                Some(v) => {
                    let arr = v.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'palette_bands' must be an array".into())
                    })?;
                    if arr.len() > 256 {
                        return Err(AiError::InvalidArguments(
                            "'palette_bands' may contain at most 256 entries".into(),
                        ));
                    }
                    let mut bands = Vec::with_capacity(arr.len());
                    for val in arr {
                        let raw = val.as_u64().ok_or_else(|| {
                            AiError::InvalidArguments(
                                "'palette_bands' entries must be integers 1..=255".into(),
                            )
                        })?;
                        let byte = u8::try_from(raw).map_err(|_| {
                            AiError::InvalidArguments("palette_bands index out of range".into())
                        })?;
                        bands.push(PaletteIndex::new(byte)?);
                    }
                    bands
                }
            };
            let mut transform = tessel_core::Transform::default();
            if let Some(pos) = parse_opt_vec3(args, "position")? {
                transform.translation = pos;
            }
            let params = tessel_core::TerrainParams {
                width,
                depth,
                subdiv_x,
                subdiv_z,
                height_scale,
                seed,
                octaves,
                frequency,
                island_falloff,
                heights,
                palette_bands,
                default_material,
                smooth_shading,
            };
            history.execute(
                scene,
                Command::CreateTerrainMesh {
                    name: name.clone(),
                    params,
                    transform,
                },
            )?;
            let node_id = scene.selected_node.ok_or_else(|| {
                AiError::InvalidArguments("terrain creation produced no node".into())
            })?;
            let (v_count, tri_count, bmin, bmax) = scene
                .lowpoly_meshes
                .get(&node_id)
                .map(|m| {
                    let (mn, mx) = m.bounds();
                    (
                        m.vertices.len(),
                        m.triangle_count(),
                        (mn + transform.translation).to_array(),
                        (mx + transform.translation).to_array(),
                    )
                })
                .unwrap_or((0, 0, [0.0; 3], [0.0; 3]));
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "name": name,
                "vertices": v_count,
                "triangles": tri_count,
                "bounds_min": bmin,
                "bounds_max": bmax
            }))
        }
        "mesh_create_lathe" => {
            let prof_arr = args
                .get("profile")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'profile' array".into()))?;
            let mut profile = Vec::with_capacity(prof_arr.len());
            for (i, item) in prof_arr.iter().enumerate() {
                let pair = item.as_array().ok_or_else(|| {
                    AiError::InvalidArguments(format!("profile[{i}] must be [radius, y]"))
                })?;
                if pair.len() != 2 {
                    return Err(AiError::InvalidArguments(format!(
                        "profile[{i}] must have 2 numbers [radius, y]"
                    )));
                }
                let r = pair[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("profile[{i}][0] must be a number"))
                })? as f32;
                let y = pair[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("profile[{i}][1] must be a number"))
                })? as f32;
                profile.push([r, y]);
            }
            let segments = args
                .get("segments")
                .and_then(Value::as_u64)
                .unwrap_or(16)
                .clamp(3, 64) as u32;
            let angle_deg = args
                .get("angle_deg")
                .and_then(Value::as_f64)
                .unwrap_or(360.0) as f32;
            let cap_ends = args
                .get("cap_ends")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("LatheMesh")
                .to_string();
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            let mesh = tessel_core::mesh::ops::create_lathe_mesh(
                &profile, segments, angle_deg, cap_ends, material,
            )?;
            let vert_count = mesh.vertices.len();
            let face_count = mesh.faces.len();
            let (lo, hi) = mesh.bounds();
            history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
            let created_id = scene.selected_node;
            if let (Some(node_id), true) =
                (created_id, pos.is_some() || rot.is_some() || sc.is_some())
            {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                if let Some(p) = pos {
                    transform.translation = p;
                }
                if let Some(r) = rot {
                    transform.rotation = r;
                }
                if let Some(s) = sc {
                    transform.scale = s;
                }
                history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            }
            Ok(json!({
                "ok": true,
                "node_id": created_id.map(|n| n.0),
                "vertices": vert_count,
                "faces": face_count,
                "bounds": { "min": lo.to_array(), "max": hi.to_array() }
            }))
        }
        "mesh_create_extruded_polygon" => {
            let pts_arr = args
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'points' array".into()))?;
            let mut points = Vec::with_capacity(pts_arr.len());
            for (i, item) in pts_arr.iter().enumerate() {
                let pair = item.as_array().ok_or_else(|| {
                    AiError::InvalidArguments(format!("points[{i}] must be [x, y]"))
                })?;
                if pair.len() != 2 {
                    return Err(AiError::InvalidArguments(format!(
                        "points[{i}] must have 2 numbers [x, y]"
                    )));
                }
                let x = pair[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("points[{i}][0] must be a number"))
                })? as f32;
                let y = pair[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("points[{i}][1] must be a number"))
                })? as f32;
                points.push([x, y]);
            }
            let depth = args
                .get("depth")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'depth'".into()))?
                as f32;
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("ExtrudedPolygon")
                .to_string();
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            let mesh = tessel_core::mesh::ops::create_extruded_polygon(&points, depth, material)?;
            let vert_count = mesh.vertices.len();
            let face_count = mesh.faces.len();
            let (lo, hi) = mesh.bounds();
            history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
            let created_id = scene.selected_node;
            if let (Some(node_id), true) =
                (created_id, pos.is_some() || rot.is_some() || sc.is_some())
            {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                if let Some(p) = pos {
                    transform.translation = p;
                }
                if let Some(r) = rot {
                    transform.rotation = r;
                }
                if let Some(s) = sc {
                    transform.scale = s;
                }
                history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            }
            Ok(json!({
                "ok": true,
                "node_id": created_id.map(|n| n.0),
                "vertices": vert_count,
                "faces": face_count,
                "bounds": { "min": lo.to_array(), "max": hi.to_array() }
            }))
        }
        "mesh_create_loft" => {
            let secs_arr = args
                .get("sections")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'sections' array".into()))?;
            let mut sections = Vec::with_capacity(secs_arr.len());
            for (si, sec_val) in secs_arr.iter().enumerate() {
                let ring_arr = sec_val.as_array().ok_or_else(|| {
                    AiError::InvalidArguments(format!("sections[{si}] must be an array of [x,y,z]"))
                })?;
                let mut ring = Vec::with_capacity(ring_arr.len());
                for (vi, pt) in ring_arr.iter().enumerate() {
                    let coords = pt.as_array().ok_or_else(|| {
                        AiError::InvalidArguments(format!(
                            "sections[{si}][{vi}] must be [x, y, z]"
                        ))
                    })?;
                    if coords.len() != 3 {
                        return Err(AiError::InvalidArguments(format!(
                            "sections[{si}][{vi}] must have 3 numbers"
                        )));
                    }
                    let x = coords[0].as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(format!("sections[{si}][{vi}][0] must be number"))
                    })? as f32;
                    let y = coords[1].as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(format!("sections[{si}][{vi}][1] must be number"))
                    })? as f32;
                    let z = coords[2].as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(format!("sections[{si}][{vi}][2] must be number"))
                    })? as f32;
                    ring.push(glam::Vec3::new(x, y, z));
                }
                sections.push(ring);
            }
            let cap_ends = args
                .get("cap_ends")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("LoftMesh")
                .to_string();
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            let mesh = tessel_core::mesh::ops::create_loft_mesh(&sections, cap_ends, material)?;
            let vert_count = mesh.vertices.len();
            let face_count = mesh.faces.len();
            let (lo, hi) = mesh.bounds();
            history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
            let created_id = scene.selected_node;
            if let (Some(node_id), true) =
                (created_id, pos.is_some() || rot.is_some() || sc.is_some())
            {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                if let Some(p) = pos {
                    transform.translation = p;
                }
                if let Some(r) = rot {
                    transform.rotation = r;
                }
                if let Some(s) = sc {
                    transform.scale = s;
                }
                history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            }
            Ok(json!({
                "ok": true,
                "node_id": created_id.map(|n| n.0),
                "vertices": vert_count,
                "faces": face_count,
                "bounds": { "min": lo.to_array(), "max": hi.to_array() }
            }))
        }
        "mesh_create_raw" => {
            let verts_arr = args
                .get("vertices")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'vertices' array".into()))?;
            let faces_arr = args
                .get("faces")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'faces' array".into()))?;
            let default_mat = parse_palette_index(args, 4)?;
            let smooth = args
                .get("smooth")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut vertices = Vec::with_capacity(verts_arr.len());
            for (i, item) in verts_arr.iter().enumerate() {
                let arr = item.as_array().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'vertices[{i}]' must be [x,y,z]"))
                })?;
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(format!(
                        "'vertices[{i}]' must have 3 numbers"
                    )));
                }
                let x = arr[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'vertices[{i}][0]' must be number"))
                })? as f32;
                let y = arr[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'vertices[{i}][1]' must be number"))
                })? as f32;
                let z = arr[2].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'vertices[{i}][2]' must be number"))
                })? as f32;
                vertices.push(glam::Vec3::new(x, y, z));
            }
            let mut faces = Vec::with_capacity(faces_arr.len());
            for (fi, item) in faces_arr.iter().enumerate() {
                let v_ids_arr = item
                    .get("vertices")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        AiError::InvalidArguments(format!("'faces[{fi}].vertices' missing"))
                    })?;
                let mut f_verts = Vec::with_capacity(v_ids_arr.len());
                for v in v_ids_arr {
                    let idx = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments(format!(
                            "'faces[{fi}].vertices' entries must be integers"
                        ))
                    })? as usize;
                    f_verts.push(idx);
                }
                let mat = if item.get("palette_index").is_some() {
                    parse_palette_index(item, default_mat.get())?
                } else {
                    default_mat
                };
                let mut face = tessel_core::mesh::Face::new(f_verts, mat);
                if let Some(uvs_arr) = item.get("uvs").and_then(Value::as_array) {
                    let mut uvs = Vec::with_capacity(uvs_arr.len());
                    for pair in uvs_arr {
                        let p = pair.as_array().ok_or_else(|| {
                            AiError::InvalidArguments(format!(
                                "'faces[{fi}].uvs' entries must be [u,v]"
                            ))
                        })?;
                        if p.len() != 2 {
                            return Err(AiError::InvalidArguments(format!(
                                "'faces[{fi}].uvs' entries must be [u,v]"
                            )));
                        }
                        let u = p[0].as_f64().ok_or_else(|| {
                            AiError::InvalidArguments(format!("'faces[{fi}].uvs' must be numbers"))
                        })? as f32;
                        let v = p[1].as_f64().ok_or_else(|| {
                            AiError::InvalidArguments(format!("'faces[{fi}].uvs' must be numbers"))
                        })? as f32;
                        uvs.push([u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)]);
                    }
                    face.uvs = uvs;
                }
                faces.push(face);
            }
            let mesh = tessel_core::mesh::ops::create_raw_mesh(vertices, faces, smooth)?;
            let v_count = mesh.vertices.len();
            let f_count = mesh.faces.len();
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("CustomMesh")
                .to_string();
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
            let created_id = scene.selected_node;
            if let (Some(node_id), true) =
                (created_id, pos.is_some() || rot.is_some() || sc.is_some())
            {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .map(|n| n.transform)
                    .unwrap_or_default();
                if let Some(p) = pos {
                    transform.translation = p;
                }
                if let Some(r) = rot {
                    transform.rotation = r;
                }
                if let Some(s) = sc {
                    transform.scale = s;
                }
                history.execute(scene, Command::SetNodeTransform { node_id, transform })?;
            }
            Ok(json!({
                "ok": true,
                "node_id": created_id.map(|n| n.0),
                "vertices": v_count,
                "faces": f_count
            }))
        }
        "mesh_create_curve_tube" => {
            let pts_arr = args
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'points' array".into()))?;
            if pts_arr.len() < 2 {
                return Err(AiError::InvalidArguments(
                    "'points' must have at least 2 [x,y,z] points".into(),
                ));
            }
            let mut points = Vec::with_capacity(pts_arr.len());
            for (i, item) in pts_arr.iter().enumerate() {
                let Some(arr) = item.as_array() else {
                    return Err(AiError::InvalidArguments(format!(
                        "'points[{i}]' must be [x,y,z]"
                    )));
                };
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(format!(
                        "'points[{i}]' must have 3 numbers"
                    )));
                }
                let x = arr[0].as_f64().unwrap_or(0.0) as f32;
                let y = arr[1].as_f64().unwrap_or(0.0) as f32;
                let z = arr[2].as_f64().unwrap_or(0.0) as f32;
                points.push(glam::Vec3::new(x, y, z));
            }
            let radius = args.get("radius").and_then(Value::as_f64).unwrap_or(0.06) as f32;
            let flatness = args.get("flatness").and_then(Value::as_f64).unwrap_or(0.3) as f32;
            let material = parse_palette_index(args, 4)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("StrapCurve")
                .to_string();
            history.execute(
                scene,
                Command::CreateCurveTubeMesh {
                    name,
                    points,
                    radius,
                    flatness,
                    material,
                },
            )?;
            let node_id = scene.selected_node;
            let faces = node_id
                .and_then(|id| scene.lowpoly_meshes.get(&id))
                .map_or(0, |m| m.faces.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.map(|n| n.0),
                "faces": faces
            }))
        }
        "mesh_import" => {
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            let material = parse_palette_index(args, 4)?;
            let (default_name, mesh) = import_mesh_from_path(&target, material)?;
            let verts = mesh.vertices.len();
            let faces = mesh.faces.len();
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or(default_name);
            history.execute(scene, Command::ImportLowPolyMesh { name, mesh })?;
            Ok(json!({
                "ok": true,
                "node_id": scene.selected_node.map(|n| n.0),
                "vertices": verts,
                "faces": faces
            }))
        }
        "mesh_fill_texture" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let idx = parse_palette_index(args, 3)?;
            let color = scene
                .palette
                .get(idx)
                .map(|e| e.color)
                .ok_or_else(|| AiError::InvalidArguments("palette index not defined".into()))?;
            history.execute(scene, Command::FillMeshTexture { node_id, color })?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_fill_smart_material" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;

            let resolve_opt_color = |field: &str| -> Result<Option<Rgba8>, AiError> {
                let Some(raw) = args.get(field).and_then(Value::as_u64) else {
                    return Ok(None);
                };
                let idx = u8::try_from(raw)
                    .ok()
                    .and_then(|v| PaletteIndex::new(v).ok())
                    .ok_or_else(|| AiError::InvalidArguments(format!("invalid '{field}'")))?;
                let color = scene
                    .palette
                    .get(idx)
                    .map(|e| e.color)
                    .ok_or_else(|| {
                        AiError::InvalidArguments(format!("palette index for '{field}' not defined"))
                    })?;
                Ok(Some(color))
            };

            let base_color = if let Some(c) = resolve_opt_color("base_palette_index")? {
                c
            } else {
                mesh.faces
                    .first()
                    .and_then(|f| scene.palette.get(f.material))
                    .map(|e| e.color)
                    .unwrap_or(Rgba8::rgb(180, 180, 185))
            };
            let mut params = SmartMaterialParams::from_base(base_color);
            if let Some(c) = resolve_opt_color("edge_palette_index")? {
                params.edge_wear_color = c;
            }
            if let Some(c) = resolve_opt_color("cavity_palette_index")? {
                params.cavity_dirt_color = c;
            }
            if let Some(v) = args.get("edge_strength").and_then(Value::as_f64) {
                params.edge_strength = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = args.get("edge_threshold").and_then(Value::as_f64) {
                params.edge_threshold = (v as f32).clamp(0.5, 0.95);
            }
            if let Some(v) = args.get("cavity_strength").and_then(Value::as_f64) {
                params.cavity_strength = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = args.get("cavity_threshold").and_then(Value::as_f64) {
                params.cavity_threshold = (v as f32).clamp(0.05, 1.0);
            }
            if let Some(v) = args.get("ao_darken").and_then(Value::as_f64) {
                params.ao_darken = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = args.get("resolution").and_then(Value::as_u64) {
                params.resolution = (v as u32).clamp(16, 1024);
            }
            if let Some(v) = args.get("samples").and_then(Value::as_u64) {
                params.samples = (v as u32).clamp(4, 64);
            }
            if let Some(v) = args.get("dilation_px").and_then(Value::as_u64) {
                params.dilation_px = (v as u32).clamp(0, 16);
            }
            let attach_ao = args
                .get("attach_ao")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let out = generate_smart_material_texture(mesh, &params)?;
            let (w, h) = (out.albedo.width, out.albedo.height);
            let mut ao_attached = false;
            if attach_ao {
                if let Some(ao_tex) = out.ao_map {
                    history.execute(
                        scene,
                        Command::Transaction(vec![
                            Command::SetMeshTexture {
                                node_id,
                                texture: Some(out.albedo),
                            },
                            Command::SetMeshAoMap {
                                node_id,
                                ao_map: Some(ao_tex),
                            },
                        ]),
                    )?;
                    ao_attached = true;
                } else {
                    history.execute(
                        scene,
                        Command::SetMeshTexture {
                            node_id,
                            texture: Some(out.albedo),
                        },
                    )?;
                }
            } else {
                history.execute(
                    scene,
                    Command::SetMeshTexture {
                        node_id,
                        texture: Some(out.albedo),
                    },
                )?;
            }

            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": w,
                "height": h,
                "covered_texels": out.covered_texels,
                "ao_attached": ao_attached
            }))
        }
        "mesh_load_texture" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            let tex = import_mesh_texture_from_path(&target)?;
            let (w, h) = (tex.width, tex.height);
            history.execute(
                scene,
                Command::SetMeshTexture {
                    node_id,
                    texture: Some(tex),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "width": w, "height": h }))
        }
        "mesh_load_normal_map" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            let tex = import_mesh_texture_from_path(&target)?;
            let (w, h) = (tex.width, tex.height);
            history.execute(
                scene,
                Command::SetMeshNormalMap {
                    node_id,
                    normal_map: Some(tex),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "width": w, "height": h }))
        }
        "mesh_bake_maps" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let defaults = BakeParams::default();
            let maps = if let Some(arr) = args.get("maps").and_then(Value::as_array) {
                let mut parsed = Vec::with_capacity(arr.len());
                for item in arr {
                    let s = item.as_str().ok_or_else(|| {
                        AiError::InvalidArguments("'maps' entries must be strings".into())
                    })?;
                    let kind = match s.to_ascii_lowercase().as_str() {
                        "ao" => BakeMapKind::Ao,
                        "curvature" => BakeMapKind::Curvature,
                        "thickness" => BakeMapKind::Thickness,
                        other => {
                            return Err(AiError::InvalidArguments(format!(
                                "unknown bake map '{other}', expected ao|curvature|thickness"
                            )))
                        }
                    };
                    if !parsed.contains(&kind) {
                        parsed.push(kind);
                    }
                }
                if parsed.is_empty() {
                    return Err(AiError::InvalidArguments(
                        "'maps' must contain at least one map kind".into(),
                    ));
                }
                parsed
            } else {
                defaults.maps
            };
            let resolution = args
                .get("resolution")
                .and_then(Value::as_u64)
                .map(|v| (v as u32).clamp(16, 1024))
                .unwrap_or(defaults.resolution);
            let samples = args
                .get("samples")
                .and_then(Value::as_u64)
                .map(|v| (v as u32).clamp(4, 64))
                .unwrap_or(defaults.samples);
            let max_distance = args
                .get("max_distance")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(defaults.max_distance);
            let bias = args
                .get("bias")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(defaults.bias);
            let dilation_px = args
                .get("dilation_px")
                .and_then(Value::as_u64)
                .map(|v| (v as u32).clamp(0, 16))
                .unwrap_or(defaults.dilation_px);
            let attach_ao = args
                .get("attach_ao")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let output_prefix = args
                .get("output_prefix")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let params = BakeParams {
                resolution,
                samples,
                max_distance,
                bias,
                dilation_px,
                maps,
            };
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let baked = bake_mesh_maps(mesh, &params)?;

            let mut files = serde_json::Map::new();
            if let Some(prefix) = output_prefix {
                let candidates = [
                    ("ao", baked.ao.as_ref()),
                    ("curvature", baked.curvature.as_ref()),
                    ("thickness", baked.thickness.as_ref()),
                ];
                for (suffix, maybe_tex) in candidates {
                    if let Some(tex) = maybe_tex {
                        let rel = format!("{prefix}_{suffix}.png");
                        let target = resolve_safe_export_path(allowed_export_dir, &rel)?;
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| {
                                AiError::InvalidArguments(format!(
                                    "failed to create export directory: {e}"
                                ))
                            })?;
                        }
                        let png = export_mesh_texture_png_bytes(tex)?;
                        std::fs::write(&target, png).map_err(|e| {
                            AiError::InvalidArguments(format!(
                                "failed to write baked map '{rel}': {e}"
                            ))
                        })?;
                        files.insert(suffix.into(), json!(target.display().to_string()));
                    }
                }
            }

            let mut ao_attached = false;
            if attach_ao {
                if let Some(ao_tex) = baked.ao {
                    history.execute(
                        scene,
                        Command::SetMeshAoMap {
                            node_id,
                            ao_map: Some(ao_tex),
                        },
                    )?;
                    ao_attached = true;
                }
            }

            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "resolution": resolution,
                "samples": samples,
                "covered_texels": baked.covered_texels,
                "ao_attached": ao_attached,
                "files": files
            }))
        }
        "mesh_layer_add" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let opacity = args
                .get("opacity")
                .and_then(Value::as_f64)
                .map(|v| (v as f32).clamp(0.0, 1.0))
                .unwrap_or(1.0);
            let fill_color = if let Some(raw) = args.get("palette_index").and_then(Value::as_u64) {
                let idx = u8::try_from(raw)
                    .ok()
                    .and_then(|v| PaletteIndex::new(v).ok())
                    .ok_or_else(|| AiError::InvalidArguments("invalid 'palette_index'".into()))?;
                let c = scene
                    .palette
                    .get(idx)
                    .map(|e| e.color)
                    .ok_or_else(|| {
                        AiError::InvalidArguments("palette index not defined".into())
                    })?;
                Some(c)
            } else {
                None
            };
            history.execute(
                scene,
                Command::AddMeshPaintLayer {
                    node_id,
                    name,
                    opacity,
                    fill_color,
                },
            )?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let active_id = mesh.active_paint_layer.unwrap_or(0);
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "layer_id": active_id,
                "layer_count": mesh.paint_layers.len()
            }))
        }
        "mesh_layer_set" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let layer_id = args
                .get("layer_id")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| AiError::InvalidArguments("missing valid 'layer_id'".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let visible = args.get("visible").and_then(Value::as_bool);
            let opacity = args
                .get("opacity")
                .and_then(Value::as_f64)
                .map(|v| (v as f32).clamp(0.0, 1.0));
            let set_active = args
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            history.execute(
                scene,
                Command::SetMeshPaintLayer {
                    node_id,
                    layer_id,
                    name,
                    visible,
                    opacity,
                    set_active,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "layer_id": layer_id }))
        }
        "mesh_layer_delete" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let layer_id = args
                .get("layer_id")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| AiError::InvalidArguments("missing valid 'layer_id'".into()))?;
            history.execute(scene, Command::DeleteMeshPaintLayer { node_id, layer_id })?;
            let remaining = scene
                .lowpoly_meshes
                .get(&node_id)
                .map(|m| m.paint_layers.len())
                .unwrap_or(0);
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "deleted_layer_id": layer_id,
                "layer_count": remaining
            }))
        }
        "mesh_layer_list" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let layers: Vec<Value> = mesh
                .paint_layers
                .iter()
                .map(|l| {
                    json!({
                        "id": l.id,
                        "name": l.name,
                        "visible": l.visible,
                        "opacity": l.opacity,
                        "active": mesh.active_paint_layer == Some(l.id),
                        "width": l.texture.width,
                        "height": l.texture.height
                    })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "active_layer_id": mesh.active_paint_layer,
                "layers": layers
            }))
        }
        "mesh_paint_spot" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let idx = parse_palette_index(args, 3)?;
            let color = scene
                .palette
                .get(idx)
                .map(|e| e.color)
                .ok_or_else(|| AiError::InvalidArguments("palette index not defined".into()))?;
            let radius_px = args
                .get("radius_px")
                .and_then(Value::as_f64)
                .unwrap_or(10.0) as f32;
            let mirror_x = args
                .get("mirror_x")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let radius_world_arg = args
                .get("radius_world")
                .and_then(Value::as_f64)
                .map(|v| v as f32);
            let camera_pos = parse_opt_vec3(args, "camera_pos")?;
            let across_seams = args
                .get("across_seams")
                .and_then(Value::as_bool)
                .unwrap_or(radius_world_arg.is_some() || camera_pos.is_some());

            if across_seams && args.get("uv").is_none() {
                let point = parse_opt_vec3(args, "point")?.ok_or_else(|| {
                    AiError::InvalidArguments("missing 'point' for 3D seam-aware paint".into())
                })?;
                let brush_normal = parse_opt_vec3(args, "normal")?;
                let mesh = scene
                    .lowpoly_meshes
                    .get(&node_id)
                    .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
                let (min_b, max_b) = mesh.bounds();
                let diag = (max_b - min_b).length().max(1e-3);
                let tex_size = mesh
                    .texture
                    .as_ref()
                    .map(|t| t.width.max(t.height) as f32)
                    .unwrap_or(tessel_core::DEFAULT_MESH_TEX_SIZE as f32);
                let radius = radius_world_arg
                    .unwrap_or_else(|| ((radius_px / tex_size) * diag * 1.2).max(1e-3))
                    .clamp(1e-4, 100.0);

                // Transform world-space camera_pos into mesh-local space if node has a transform.
                let local_cam = camera_pos.map(|cam| {
                    let world_mat = scene.node_world_matrix(node_id);
                    let inv = world_mat.inverse();
                    if inv.is_finite() {
                        inv.transform_point3(cam)
                    } else {
                        cam
                    }
                });

                if mirror_x {
                    let mirrored_center = glam::Vec3::new(-point.x, point.y, point.z);
                    let mirrored_normal =
                        brush_normal.map(|n| glam::Vec3::new(-n.x, n.y, n.z));
                    let mirrored_cam = local_cam.map(|c| glam::Vec3::new(-c.x, c.y, c.z));
                    history.execute(
                        scene,
                        Command::Transaction(vec![
                            Command::PaintMeshSpot3D {
                                node_id,
                                center: point,
                                radius,
                                color,
                                brush_normal,
                                camera_pos: local_cam,
                            },
                            Command::PaintMeshSpot3D {
                                node_id,
                                center: mirrored_center,
                                radius,
                                color,
                                brush_normal: mirrored_normal,
                                camera_pos: mirrored_cam,
                            },
                        ]),
                    )?;
                } else {
                    history.execute(
                        scene,
                        Command::PaintMeshSpot3D {
                            node_id,
                            center: point,
                            radius,
                            color,
                            brush_normal,
                            camera_pos: local_cam,
                        },
                    )?;
                }
                return Ok(json!({
                    "ok": true,
                    "node_id": node_id.0,
                    "across_seams": true,
                    "center": [point.x, point.y, point.z],
                    "radius_world": radius,
                    "camera_culled": local_cam.is_some(),
                    "mirror_x": mirror_x
                }));
            }

            let mut mirror_uv: Option<[f32; 2]> = None;
            let uv = if let Some(uv_arr) = args.get("uv").and_then(Value::as_array) {
                if uv_arr.len() != 2 {
                    return Err(AiError::InvalidArguments("'uv' must have 2 numbers".into()));
                }
                [
                    uv_arr[0].as_f64().unwrap_or(0.5) as f32,
                    uv_arr[1].as_f64().unwrap_or(0.5) as f32,
                ]
            } else {
                let p_arr = args
                    .get("point")
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'uv' or 'point'".into()))?;
                let n_arr = args
                    .get("normal")
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'normal'".into()))?;
                if p_arr.len() != 3 || n_arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'point' and 'normal' must have 3 numbers".into(),
                    ));
                }
                let point = glam::Vec3::new(
                    p_arr[0].as_f64().unwrap_or(0.0) as f32,
                    p_arr[1].as_f64().unwrap_or(0.0) as f32,
                    p_arr[2].as_f64().unwrap_or(0.0) as f32,
                );
                let normal = glam::Vec3::new(
                    n_arr[0].as_f64().unwrap_or(0.0) as f32,
                    n_arr[1].as_f64().unwrap_or(1.0) as f32,
                    n_arr[2].as_f64().unwrap_or(0.0) as f32,
                );
                let (min_b, max_b) = scene
                    .lowpoly_meshes
                    .get(&node_id)
                    .map(|m| m.bounds())
                    .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
                if mirror_x {
                    mirror_uv = Some(tessel_core::box_atlas_uv(
                        glam::Vec3::new(-point.x, point.y, point.z),
                        glam::Vec3::new(-normal.x, normal.y, normal.z),
                        min_b,
                        max_b,
                    ));
                }
                tessel_core::box_atlas_uv(point, normal, min_b, max_b)
            };
            if let Some(muv) = mirror_uv {
                history.execute(
                    scene,
                    Command::Transaction(vec![
                        Command::PaintMeshTexture {
                            node_id,
                            uv,
                            radius_px,
                            color,
                        },
                        Command::PaintMeshTexture {
                            node_id,
                            uv: muv,
                            radius_px,
                            color,
                        },
                    ]),
                )?;
            } else {
                history.execute(
                    scene,
                    Command::PaintMeshTexture {
                        node_id,
                        uv,
                        radius_px,
                        color,
                    },
                )?;
            }
            Ok(json!({ "ok": true, "node_id": node_id.0, "uv": uv, "mirror_x": mirror_x }))
        }
        "mesh_paint_line" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let idx = parse_palette_index(args, 3)?;
            let color = scene
                .palette
                .get(idx)
                .map(|e| e.color)
                .ok_or_else(|| AiError::InvalidArguments("palette index not defined".into()))?;
            let parse_uv = |key: &str| -> Result<[f32; 2], AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
                if arr.len() != 2 {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' must have 2 numbers"
                    )));
                }
                let u = arr[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'{key}[0]' must be number"))
                })? as f32;
                let v = arr[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'{key}[1]' must be number"))
                })? as f32;
                if !u.is_finite() || !v.is_finite() {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' must be finite"
                    )));
                }
                Ok([u, v])
            };
            let uv_start = parse_uv("uv_start")?;
            let uv_end = parse_uv("uv_end")?;
            let width_px = args
                .get("width_px")
                .and_then(Value::as_f64)
                .unwrap_or(4.0) as f32;
            if !width_px.is_finite() || width_px <= 0.0 {
                return Err(AiError::InvalidArguments(
                    "'width_px' must be finite and > 0".into(),
                ));
            }
            history.execute(
                scene,
                Command::PaintMeshLine {
                    node_id,
                    uv_start,
                    uv_end,
                    width_px,
                    color,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_paint_rect" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let idx = parse_palette_index(args, 3)?;
            let color = scene
                .palette
                .get(idx)
                .map(|e| e.color)
                .ok_or_else(|| AiError::InvalidArguments("palette index not defined".into()))?;
            let parse_uv = |key: &str| -> Result<[f32; 2], AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
                if arr.len() != 2 {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' must have 2 numbers"
                    )));
                }
                let u = arr[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'{key}[0]' must be number"))
                })? as f32;
                let v = arr[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("'{key}[1]' must be number"))
                })? as f32;
                if !u.is_finite() || !v.is_finite() {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' must be finite"
                    )));
                }
                Ok([u, v])
            };
            let uv_min = parse_uv("uv_min")?;
            let uv_max = parse_uv("uv_max")?;
            history.execute(
                scene,
                Command::PaintMeshRect {
                    node_id,
                    uv_min,
                    uv_max,
                    color,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_project_uvs" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mode_str = args
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'mode'".into()))?;
            let mode = match mode_str {
                "box" => tessel_core::mesh::UvProjection::Box,
                "planar_x" => tessel_core::mesh::UvProjection::PlanarX,
                "planar_y" => tessel_core::mesh::UvProjection::PlanarY,
                "planar_z" => tessel_core::mesh::UvProjection::PlanarZ,
                "cylindrical" => tessel_core::mesh::UvProjection::CylindricalY,
                "spherical" => tessel_core::mesh::UvProjection::Spherical,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown UV projection '{other}'"
                    )));
                }
            };
            history.execute(scene, Command::ProjectMeshUvs { node_id, mode })?;
            let face_count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.faces.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "mode": mode_str,
                "faces_projected": face_count
            }))
        }
        "mesh_subdivide_face" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_id = tessel_core::FaceId(
                args.get("face_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'face_id'".into()))?
                    as usize,
            );
            history.execute(scene, Command::SubdivideMeshFace { node_id, face_id })?;
            let (verts, faces) = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or((0, 0), |m| (m.vertices.len(), m.faces.len()));
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertices": verts,
                "faces": faces
            }))
        }
        "mesh_inspect" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let (min_b, max_b) = mesh.bounds();
            let faces: Vec<Value> = mesh
                .faces
                .iter()
                .enumerate()
                .take(64)
                .map(|(fi, f)| {
                    let mut center = glam::Vec3::ZERO;
                    for &vi in &f.vertices {
                        if let Some(&p) = mesh.vertices.get(vi) {
                            center += p;
                        }
                    }
                    if !f.vertices.is_empty() {
                        center /= f.vertices.len() as f32;
                    }
                    let normal = if f.vertices.len() >= 3 {
                        let p0 = mesh
                            .vertices
                            .get(f.vertices[0])
                            .copied()
                            .unwrap_or_default();
                        let p1 = mesh
                            .vertices
                            .get(f.vertices[1])
                            .copied()
                            .unwrap_or_default();
                        let p2 = mesh
                            .vertices
                            .get(f.vertices[2])
                            .copied()
                            .unwrap_or_default();
                        (p1 - p0).cross(p2 - p0).normalize_or_zero()
                    } else {
                        glam::Vec3::Y
                    };
                    json!({
                        "face_id": fi,
                        "center": center.to_array(),
                        "normal": normal.to_array(),
                        "vertices": f.vertices.len(),
                        "palette_index": f.material.get()
                    })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertex_count": mesh.vertices.len(),
                "face_count": mesh.faces.len(),
                "manual_weights": mesh.skin_weights.len(),
                "morphs": mesh.morphs.iter().map(|m| &m.name).collect::<Vec<_>>(),
                "bounds": { "min": min_b.to_array(), "max": max_b.to_array() },
                "faces_preview": faces
            }))
        }
        "mesh_list_vertices" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(64)
                .clamp(1, 512) as usize;
            let vertices: Vec<Value> = mesh
                .vertices
                .iter()
                .enumerate()
                .skip(offset)
                .take(limit)
                .map(|(vi, p)| json!({ "vertex_id": vi, "position": p.to_array() }))
                .collect();
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertex_count": mesh.vertices.len(),
                "offset": offset,
                "vertices": vertices
            }))
        }
        "mesh_extrude_face" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_idx = args
                .get("face_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'face_id'".into()))?
                as usize;
            let distance = args
                .get("distance")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'distance'".into()))?
                as f32;
            if !distance.is_finite() {
                return Err(AiError::InvalidArguments(
                    "'distance' must be finite".into(),
                ));
            }
            history.execute(
                scene,
                Command::ExtrudeMeshFace {
                    node_id,
                    face_id: tessel_core::FaceId(face_idx),
                    distance,
                },
            )?;
            let face_count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.faces.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "face_id": face_idx,
                "total_faces": face_count
            }))
        }
        "mesh_inset_face" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_idx = args
                .get("face_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'face_id'".into()))?
                as usize;
            let factor = args
                .get("factor")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'factor'".into()))?
                as f32;
            if !factor.is_finite() {
                return Err(AiError::InvalidArguments("'factor' must be finite".into()));
            }
            history.execute(
                scene,
                Command::InsetMeshFace {
                    node_id,
                    face_id: tessel_core::FaceId(face_idx),
                    factor,
                },
            )?;
            let face_count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.faces.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "face_id": face_idx,
                "total_faces": face_count
            }))
        }
        "mesh_translate_face" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_idx = args
                .get("face_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'face_id'".into()))?
                as usize;
            let d_arr = args
                .get("delta")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'delta'".into()))?;
            if d_arr.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'delta' must have 3 numbers".into(),
                ));
            }
            let delta = glam::Vec3::new(
                d_arr[0].as_f64().unwrap_or(0.0) as f32,
                d_arr[1].as_f64().unwrap_or(0.0) as f32,
                d_arr[2].as_f64().unwrap_or(0.0) as f32,
            );
            history.execute(
                scene,
                Command::TranslateMeshFace {
                    node_id,
                    face_id: tessel_core::FaceId(face_idx),
                    delta,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "face_id": face_idx }))
        }
        "mesh_move_vertices" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let ids_arr = args
                .get("vertex_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'vertex_ids'".into()))?;
            if ids_arr.is_empty() {
                return Err(AiError::InvalidArguments(
                    "'vertex_ids' must not be empty".into(),
                ));
            }
            let mut vertex_ids = Vec::with_capacity(ids_arr.len());
            for id in ids_arr {
                vertex_ids.push(id.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'vertex_ids' must be integers".into())
                })? as usize);
            }
            let d_arr = args
                .get("delta")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'delta'".into()))?;
            if d_arr.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'delta' must have 3 numbers".into(),
                ));
            }
            let delta = glam::Vec3::new(
                d_arr[0].as_f64().unwrap_or(0.0) as f32,
                d_arr[1].as_f64().unwrap_or(0.0) as f32,
                d_arr[2].as_f64().unwrap_or(0.0) as f32,
            );
            history.execute(
                scene,
                Command::MoveMeshVertices {
                    node_id,
                    vertex_ids: vertex_ids.clone(),
                    delta,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "vertex_ids": vertex_ids }))
        }
        "mesh_set_vertex" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let vertex_idx = args
                .get("vertex_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'vertex_id'".into()))?
                as usize;
            let p_arr = args
                .get("position")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'position'".into()))?;
            if p_arr.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'position' must have 3 numbers".into(),
                ));
            }
            let position = glam::Vec3::new(
                p_arr[0].as_f64().unwrap_or(0.0) as f32,
                p_arr[1].as_f64().unwrap_or(0.0) as f32,
                p_arr[2].as_f64().unwrap_or(0.0) as f32,
            );
            history.execute(
                scene,
                Command::SetMeshVertex {
                    node_id,
                    vertex_id: vertex_idx,
                    position,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "vertex_id": vertex_idx }))
        }
        "mesh_set_vertices" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let upd_arr = args
                .get("updates")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'updates' array".into()))?;
            let mut updates = Vec::with_capacity(upd_arr.len());
            for (i, item) in upd_arr.iter().enumerate() {
                let vid = item
                    .get("vertex_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        AiError::InvalidArguments(format!("'updates[{i}].vertex_id' missing"))
                    })? as usize;
                let pos = parse_opt_vec3(item, "position")?.ok_or_else(|| {
                    AiError::InvalidArguments(format!("'updates[{i}].position' missing"))
                })?;
                updates.push((vid, pos));
            }
            let count = updates.len();
            history.execute(scene, Command::SetMeshVerticesBatch { node_id, updates })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "updated": count }))
        }
        "mesh_set_face_material" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let f_arr = args
                .get("face_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'face_ids' array".into()))?;
            let mut face_ids = Vec::with_capacity(f_arr.len());
            for v in f_arr {
                let idx = v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'face_ids' must be integers".into())
                })? as usize;
                face_ids.push(tessel_core::FaceId(idx));
            }
            let material = parse_palette_index(args, 4)?;
            let count = face_ids.len();
            history.execute(
                scene,
                Command::SetMeshFaceMaterials {
                    node_id,
                    face_ids,
                    material,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "faces_updated": count,
                "palette_index": material.get()
            }))
        }
        "mesh_join" => {
            let target_node_id = NodeId(
                args.get("target_node_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'target_node_id'".into()))?,
            );
            let src_arr = args
                .get("source_node_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AiError::InvalidArguments("missing 'source_node_ids' array".into())
                })?;
            let mut source_node_ids = Vec::with_capacity(src_arr.len());
            for v in src_arr {
                let id = v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'source_node_ids' must be integers".into())
                })?;
                source_node_ids.push(NodeId(id));
            }
            let joined_count = source_node_ids.len();
            history.execute(
                scene,
                Command::JoinLowPolyMeshes {
                    target_node_id,
                    source_node_ids,
                },
            )?;
            let (verts, faces) = scene
                .lowpoly_meshes
                .get(&target_node_id)
                .map_or((0, 0), |m| (m.vertices.len(), m.faces.len()));
            Ok(json!({
                "ok": true,
                "target_node_id": target_node_id.0,
                "joined_sources": joined_count,
                "vertices": verts,
                "faces": faces
            }))
        }
        "mesh_mirror" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let axis = match args.get("axis").and_then(Value::as_str).unwrap_or("x") {
                "x" => 0u8,
                "y" => 1u8,
                "z" => 2u8,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "invalid axis '{other}'; use 'x', 'y', or 'z'"
                    )));
                }
            };
            let merge_original = args
                .get("merge_original")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            history.execute(
                scene,
                Command::MirrorLowPolyMesh {
                    node_id,
                    axis,
                    merge_original,
                },
            )?;
            let (verts, faces) = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or((0, 0), |m| (m.vertices.len(), m.faces.len()));
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertices": verts,
                "faces": faces
            }))
        }
        "mesh_taper" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let axis = match args.get("axis").and_then(Value::as_str).unwrap_or("y") {
                "x" => 0u8,
                "y" => 1u8,
                "z" => 2u8,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "invalid axis '{other}'; use 'x', 'y', or 'z'"
                    )));
                }
            };
            let start_scale = args
                .get("start_scale")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'start_scale'".into()))?
                as f32;
            let end_scale = args
                .get("end_scale")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'end_scale'".into()))?
                as f32;
            history.execute(
                scene,
                Command::TaperLowPolyMesh {
                    node_id,
                    axis,
                    start_scale,
                    end_scale,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_bend" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let parse_ax = |key: &str| -> Result<u8, AiError> {
                match args.get(key).and_then(Value::as_str) {
                    Some("x") => Ok(0),
                    Some("y") => Ok(1),
                    Some("z") => Ok(2),
                    _ => Err(AiError::InvalidArguments(format!(
                        "'{key}' must be 'x', 'y', or 'z'"
                    ))),
                }
            };
            let bend_axis = parse_ax("bend_axis")?;
            let curve_axis = parse_ax("curve_axis")?;
            let angle_deg = args
                .get("angle_deg")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'angle_deg'".into()))?
                as f32;
            history.execute(
                scene,
                Command::BendLowPolyMesh {
                    node_id,
                    bend_axis,
                    curve_axis,
                    angle_rad: angle_deg.to_radians(),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "angle_deg": angle_deg }))
        }
        "mesh_set_origin" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let origin_local = if let Some(explicit) = parse_opt_vec3(args, "origin")? {
                explicit
            } else {
                let (lo, hi) = scene
                    .lowpoly_meshes
                    .get(&node_id)
                    .map(|m| m.bounds())
                    .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
                match args.get("mode").and_then(Value::as_str).unwrap_or("center") {
                    "center" => (lo + hi) * 0.5,
                    "bottom" => glam::Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5),
                    other => {
                        return Err(AiError::InvalidArguments(format!(
                            "unknown mode '{other}'; use 'center', 'bottom', or 'origin'"
                        )));
                    }
                }
            };
            history.execute(
                scene,
                Command::SetMeshOrigin {
                    node_id,
                    origin_local,
                },
            )?;
            let pos = scene
                .nodes
                .get(&node_id)
                .map(|n| n.transform.translation.to_array())
                .unwrap_or([0.0; 3]);
            Ok(json!({ "ok": true, "node_id": node_id.0, "position": pos }))
        }
        "mesh_apply_transform" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            history.execute(scene, Command::ApplyMeshTransform { node_id })?;
            let bounds = scene.lowpoly_meshes.get(&node_id).map(|m| {
                let (lo, hi) = m.bounds();
                json!({ "min": lo.to_array(), "max": hi.to_array() })
            });
            Ok(json!({ "ok": true, "node_id": node_id.0, "bounds": bounds }))
        }
        "mesh_weld_vertices" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let tolerance = args
                .get("tolerance")
                .and_then(Value::as_f64)
                .unwrap_or(0.001) as f32;
            let before = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.vertices.len());
            history.execute(
                scene,
                Command::WeldMeshVertices { node_id, tolerance },
            )?;
            let after = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.vertices.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertices_before": before,
                "vertices_after": after,
                "removed": before.saturating_sub(after)
            }))
        }
        "mesh_flip_normals" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_ids = match args.get("face_ids") {
                None | Some(Value::Null) => None,
                Some(Value::Array(arr)) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        let idx = v.as_u64().ok_or_else(|| {
                            AiError::InvalidArguments("'face_ids' must be integers".into())
                        })? as usize;
                        out.push(tessel_core::FaceId(idx));
                    }
                    Some(out)
                }
                _ => {
                    return Err(AiError::InvalidArguments(
                        "'face_ids' must be an array of integers".into(),
                    ));
                }
            };
            history.execute(scene, Command::FlipMeshNormals { node_id, face_ids })?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_set_face_uvs" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let face_idx = args
                .get("face_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'face_id'".into()))?
                as usize;
            let uvs_arr = args
                .get("uvs")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'uvs'".into()))?;
            if uvs_arr.len() < 3 {
                return Err(AiError::InvalidArguments(
                    "'uvs' must carry one [u, v] pair per face corner".into(),
                ));
            }
            let mut uvs = Vec::with_capacity(uvs_arr.len());
            for pair in uvs_arr {
                let pair = pair.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("'uvs' entries must be [u, v] pairs".into())
                })?;
                if pair.len() != 2 {
                    return Err(AiError::InvalidArguments(
                        "'uvs' entries must be [u, v] pairs".into(),
                    ));
                }
                uvs.push([
                    pair[0].as_f64().unwrap_or(0.0) as f32,
                    pair[1].as_f64().unwrap_or(0.0) as f32,
                ]);
            }
            history.execute(
                scene,
                Command::SetMeshFaceUvs {
                    node_id,
                    face_id: tessel_core::FaceId(face_idx),
                    uvs: uvs.clone(),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "face_id": face_idx }))
        }
        "mesh_set_vertex_weights" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let ids_arr = args
                .get("vertex_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'vertex_ids'".into()))?;
            if ids_arr.is_empty() {
                return Err(AiError::InvalidArguments(
                    "'vertex_ids' must not be empty".into(),
                ));
            }
            let mut vertex_ids = Vec::with_capacity(ids_arr.len());
            for id in ids_arr {
                vertex_ids.push(id.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'vertex_ids' must be integers".into())
                })? as usize);
            }
            let parse_quad = |key: &str, ints: bool| -> Result<[f32; 4], AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
                if arr.len() != 4 {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' must have exactly 4 numbers"
                    )));
                }
                let mut out = [0.0f32; 4];
                for (i, v) in arr.iter().enumerate() {
                    let n = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(format!("'{key}[{i}]' must be number"))
                    })? as f32;
                    if !n.is_finite() || n < 0.0 {
                        return Err(AiError::InvalidArguments(format!(
                            "'{key}[{i}]' must be a finite non-negative number"
                        )));
                    }
                    if ints && n.fract() != 0.0 {
                        return Err(AiError::InvalidArguments(format!(
                            "'{key}[{i}]' must be an integer"
                        )));
                    }
                    out[i] = n;
                }
                Ok(out)
            };
            let joints_f = parse_quad("joints", true)?;
            let weights = parse_quad("weights", false)?;
            let mut joints = [0u16; 4];
            for (i, j) in joints_f.iter().enumerate() {
                joints[i] = u16::try_from(*j as u64).map_err(|_| {
                    AiError::InvalidArguments(format!("'joints[{i}]' overflows u16"))
                })?;
            }
            history.execute(
                scene,
                Command::SetMeshVertexWeights {
                    node_id,
                    vertex_ids: vertex_ids.clone(),
                    joints,
                    weights,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "painted": vertex_ids.len() }))
        }
        "mesh_clear_vertex_weights" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let vertex_ids = match args.get("vertex_ids") {
                None => None,
                Some(arr) => {
                    let arr = arr.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'vertex_ids' must be an array".into())
                    })?;
                    let mut ids = Vec::with_capacity(arr.len());
                    for id in arr {
                        ids.push(id.as_u64().ok_or_else(|| {
                            AiError::InvalidArguments("'vertex_ids' must be integers".into())
                        })? as usize);
                    }
                    Some(ids)
                }
            };
            history.execute(
                scene,
                Command::ClearMeshVertexWeights {
                    node_id,
                    vertex_ids,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_morph_delta" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?;
            if name.is_empty() {
                return Err(AiError::InvalidArguments("'name' must not be empty".into()));
            }
            let vertex_ids: Vec<usize> = if let Some(ids_arr) =
                args.get("vertex_ids").and_then(Value::as_array)
            {
                if ids_arr.is_empty() {
                    return Err(AiError::InvalidArguments(
                        "'vertex_ids' must not be empty".into(),
                    ));
                }
                let mut out = Vec::with_capacity(ids_arr.len());
                for id in ids_arr {
                    out.push(id.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments("'vertex_ids' must be integers".into())
                    })? as usize);
                }
                out
            } else if let (Some(bmin), Some(bmax)) = (
                parse_opt_vec3(args, "bounds_min")?,
                parse_opt_vec3(args, "bounds_max")?,
            ) {
                let lo = bmin.min(bmax);
                let hi = bmin.max(bmax);
                let mesh = scene
                    .lowpoly_meshes
                    .get(&node_id)
                    .ok_or_else(|| AiError::InvalidArguments("mesh node not found".into()))?;
                let matched: Vec<usize> = mesh
                    .vertices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| {
                        v.x >= lo.x
                            && v.x <= hi.x
                            && v.y >= lo.y
                            && v.y <= hi.y
                            && v.z >= lo.z
                            && v.z <= hi.z
                    })
                    .map(|(i, _)| i)
                    .collect();
                if matched.is_empty() {
                    return Err(AiError::InvalidArguments(
                        "no vertices matched bounds_min..=bounds_max".into(),
                    ));
                }
                matched
            } else {
                return Err(AiError::InvalidArguments(
                    "missing 'vertex_ids' or ('bounds_min' + 'bounds_max')".into(),
                ));
            };
            let affected_vertices = vertex_ids.len();
            let d_arr = args
                .get("delta")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'delta'".into()))?;
            if d_arr.len() != 3 {
                return Err(AiError::InvalidArguments(
                    "'delta' must have 3 numbers".into(),
                ));
            }
            let delta = glam::Vec3::new(
                d_arr[0].as_f64().unwrap_or(0.0) as f32,
                d_arr[1].as_f64().unwrap_or(0.0) as f32,
                d_arr[2].as_f64().unwrap_or(0.0) as f32,
            );
            if !delta.is_finite() {
                return Err(AiError::InvalidArguments("'delta' must be finite".into()));
            }
            history.execute(
                scene,
                Command::AddMorphDelta {
                    node_id,
                    name: name.to_string(),
                    vertex_ids,
                    delta,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "morph": name,
                "affected_vertices": affected_vertices
            }))
        }
        "mesh_morph_clear" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let name = args.get("name").and_then(Value::as_str).map(str::to_string);
            history.execute(scene, Command::ClearMorphs { node_id, name })?;
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "mesh_bevel" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let amount = args
                .get("amount")
                .and_then(Value::as_f64)
                .unwrap_or(0.18)
                .clamp(0.05, 0.45) as f32;
            history.execute(scene, Command::BevelMesh { node_id, amount })?;
            let faces = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.faces.len());
            Ok(json!({ "ok": true, "node_id": node_id.0, "faces": faces, "amount": amount }))
        }
        "mesh_smooth_subdivide" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let iterations = args
                .get("iterations")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 3) as usize;
            let cmds = vec![Command::SmoothSubdivideMesh { node_id }; iterations];
            history.execute(scene, Command::Transaction(cmds))?;
            let faces = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.faces.len());
            Ok(json!({ "ok": true, "node_id": node_id.0, "faces": faces }))
        }
        "mesh_decimate" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let ratio = args
                .get("ratio")
                .and_then(Value::as_f64)
                .unwrap_or(0.5)
                .clamp(0.1, 0.9) as f32;
            history.execute(scene, Command::DecimateMesh { node_id, ratio })?;
            let (verts, faces) = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or((0, 0), |m| (m.vertices.len(), m.faces.len()));
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertices": verts,
                "faces": faces
            }))
        }
        "mesh_set_shading" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let smooth = args
                .get("smooth")
                .and_then(Value::as_bool)
                .ok_or_else(|| AiError::InvalidArguments("missing 'smooth'".into()))?;
            history.execute(scene, Command::SetMeshSmoothShading { node_id, smooth })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "smooth": smooth }))
        }
        "mesh_set_material" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mut mat = scene
                .lowpoly_meshes
                .get(&node_id)
                .map(|m| m.material)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            if let Some(m) = args.get("metallic").and_then(Value::as_f64) {
                mat.metallic = m as f32;
            }
            if let Some(r) = args.get("roughness").and_then(Value::as_f64) {
                mat.roughness = r as f32;
            }
            if let Some(arr) = args.get("emissive").and_then(Value::as_array) {
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'emissive' must have 3 numbers [r, g, b]".into(),
                    ));
                }
                let mut em = [0.0f32; 3];
                for (i, item) in arr.iter().enumerate() {
                    let v = item.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'emissive' entries must be finite numbers".into())
                    })? as f32;
                    if !v.is_finite() {
                        return Err(AiError::InvalidArguments(
                            "'emissive' entries must be finite numbers".into(),
                        ));
                    }
                    em[i] = v.max(0.0);
                }
                mat.emissive = em;
            }
            if let Some(ds) = args.get("double_sided").and_then(Value::as_bool) {
                mat.double_sided = ds;
            }
            if let Some(cs) = args.get("cast_shadow").and_then(Value::as_bool) {
                mat.cast_shadow = cs;
            }
            if let Some(rs) = args.get("receive_shadow").and_then(Value::as_bool) {
                mat.receive_shadow = rs;
            }
            let parsed_cutoff = match args.get("alpha_cutoff") {
                None | Some(Value::Null) => None,
                Some(val) => {
                    let c = val.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'alpha_cutoff' must be a finite number".into())
                    })? as f32;
                    if !c.is_finite() {
                        return Err(AiError::InvalidArguments(
                            "'alpha_cutoff' must be a finite number".into(),
                        ));
                    }
                    Some(c.clamp(0.0, 1.0))
                }
            };
            if let Some(am) = args.get("alpha_mode").and_then(Value::as_str) {
                let cutoff = parsed_cutoff.unwrap_or(0.5);
                mat.alpha_mode = match am.to_ascii_lowercase().as_str() {
                    "opaque" => tessel_core::AlphaMode::Opaque,
                    "mask" => tessel_core::AlphaMode::Mask { cutoff },
                    "blend" => tessel_core::AlphaMode::Blend,
                    other => {
                        return Err(AiError::InvalidArguments(format!(
                            "unknown alpha_mode '{other}', expected opaque|mask|blend"
                        )))
                    }
                };
            } else if let Some(cutoff) = parsed_cutoff {
                if let tessel_core::AlphaMode::Mask { cutoff: cur } = &mut mat.alpha_mode {
                    *cur = cutoff;
                }
            }
            if let Some(unlit) = args.get("unlit").and_then(Value::as_bool) {
                mat.unlit = unlit;
            }
            history.execute(
                scene,
                Command::SetMeshMaterial {
                    node_id,
                    material: mat,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "metallic": mat.metallic,
                "roughness": mat.roughness,
                "emissive": mat.emissive,
                "double_sided": mat.double_sided,
                "cast_shadow": mat.cast_shadow,
                "receive_shadow": mat.receive_shadow,
                "alpha_mode": mat.alpha_mode,
                "unlit": mat.unlit
            }))
        }
        "mesh_set_vertex_colors" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let vert_count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map(|m| m.vertices.len())
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let mut updates = Vec::new();
            if let Some(fill_arr) = args.get("fill").and_then(Value::as_array) {
                if fill_arr.len() != 4 {
                    return Err(AiError::InvalidArguments(
                        "'fill' must have 4 numbers [r,g,b,a]".into(),
                    ));
                }
                let mut rgba = [0.0f32; 4];
                for (i, item) in fill_arr.iter().enumerate() {
                    let v = item.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'fill' entries must be finite numbers".into())
                    })? as f32;
                    if !v.is_finite() {
                        return Err(AiError::InvalidArguments(
                            "'fill' entries must be finite numbers".into(),
                        ));
                    }
                    rgba[i] = v.clamp(0.0, 1.0);
                }
                if vert_count > 65_536 {
                    return Err(AiError::InvalidArguments(
                        "mesh exceeds 65536 vertex color update limit".into(),
                    ));
                }
                updates.reserve(vert_count);
                for vid in 0..vert_count {
                    updates.push((vid, rgba));
                }
            }
            if let Some(arr) = args.get("updates").and_then(Value::as_array) {
                if arr.len() > 65_536 {
                    return Err(AiError::InvalidArguments(
                        "'updates' exceeds maximum of 65536 entries".into(),
                    ));
                }
                for item in arr {
                    let raw_vid = item
                        .get("vertex_id")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| AiError::InvalidArguments("missing 'vertex_id'".into()))?;
                    let vid = usize::try_from(raw_vid)
                        .map_err(|_| AiError::InvalidArguments("vertex_id overflow".into()))?;
                    let rgba_arr = item
                        .get("rgba")
                        .and_then(Value::as_array)
                        .ok_or_else(|| AiError::InvalidArguments("missing 'rgba'".into()))?;
                    if rgba_arr.len() != 4 {
                        return Err(AiError::InvalidArguments(
                            "'rgba' must have 4 numbers".into(),
                        ));
                    }
                    let mut rgba = [0.0f32; 4];
                    for (i, c) in rgba_arr.iter().enumerate() {
                        let v = c.as_f64().ok_or_else(|| {
                            AiError::InvalidArguments("'rgba' entries must be finite numbers".into())
                        })? as f32;
                        if !v.is_finite() {
                            return Err(AiError::InvalidArguments(
                                "'rgba' entries must be finite numbers".into(),
                            ));
                        }
                        rgba[i] = v.clamp(0.0, 1.0);
                    }
                    updates.push((vid, rgba));
                }
            }
            let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
            if clear && updates.is_empty() {
                history.execute(scene, Command::ClearMeshVertexColors { node_id })?;
                return Ok(json!({ "ok": true, "node_id": node_id.0, "cleared": true, "updated_vertices": 0 }));
            }
            if updates.is_empty() {
                return Err(AiError::InvalidArguments(
                    "mesh_set_vertex_colors requires 'clear: true', 'fill', or non-empty 'updates'"
                        .into(),
                ));
            }
            let updated_count = updates.len();
            if clear {
                history.execute(
                    scene,
                    Command::Transaction(vec![
                        Command::ClearMeshVertexColors { node_id },
                        Command::SetMeshVertexColors { node_id, updates },
                    ]),
                )?;
            } else {
                history.execute(scene, Command::SetMeshVertexColors { node_id, updates })?;
            }
            Ok(json!({ "ok": true, "node_id": node_id.0, "updated_vertices": updated_count }))
        }
        "mesh_set_proxy_normals" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
            let mut normals = Vec::new();
            if let Some(arr) = args.get("normals").and_then(Value::as_array) {
                if arr.len() > 65_536 {
                    return Err(AiError::InvalidArguments(
                        "'normals' exceeds maximum of 65536 entries".into(),
                    ));
                }
                for item in arr {
                    let raw_vid = item
                        .get("vertex_id")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| AiError::InvalidArguments("missing 'vertex_id'".into()))?;
                    let vid = usize::try_from(raw_vid)
                        .map_err(|_| AiError::InvalidArguments("vertex_id overflow".into()))?;
                    let n = parse_opt_vec3(item, "normal")?.ok_or_else(|| {
                        AiError::InvalidArguments("missing 'normal' [x,y,z]".into())
                    })?;
                    normals.push((vid, n));
                }
            }
            if !clear && normals.is_empty() {
                return Err(AiError::InvalidArguments(
                    "mesh_set_proxy_normals requires 'clear: true' or non-empty 'normals'".into(),
                ));
            }
            let count = normals.len();
            history.execute(
                scene,
                Command::SetMeshProxyNormals {
                    node_id,
                    normals,
                    clear,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "updated_normals": count, "cleared": clear }))
        }
        "character_create_clip" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let duration_secs = args
                .get("duration_secs")
                .and_then(Value::as_f64)
                .unwrap_or(2.0) as f32;
            if !duration_secs.is_finite() || !(0.05..=600.0).contains(&duration_secs) {
                return Err(AiError::InvalidArguments(
                    "'duration_secs' must be finite in 0.05..=600.0".into(),
                ));
            }
            let activate = args.get("activate").and_then(Value::as_bool).unwrap_or(true);
            let clip = tessel_core::AnimationClip::new_empty(name.clone(), duration_secs);
            history.execute(scene, Command::UpsertAnimationClip { clip, activate })?;
            let clip_index = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.animations.iter().position(|c| c.name == name))
                .unwrap_or(0);
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "name": name,
                "duration_secs": duration_secs
            }))
        }
        "character_update_clip" => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                usize::try_from(idx)
                    .map_err(|_| AiError::InvalidArguments("clip_index overflow".into()))?
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                rig.animations
                    .iter()
                    .position(|c| c.name == name)
                    .ok_or_else(|| AiError::InvalidArguments(format!("clip '{name}' not found")))?
            } else {
                rig.active_animation
            };
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let duration_secs = match args.get("duration_secs") {
                None => None,
                Some(v) => {
                    let d = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'duration_secs' must be a number".into())
                    })? as f32;
                    if !d.is_finite() || !(0.05..=600.0).contains(&d) {
                        return Err(AiError::InvalidArguments(
                            "'duration_secs' must be finite in 0.05..=600.0".into(),
                        ));
                    }
                    Some(d)
                }
            };
            if name.is_none() && duration_secs.is_none() {
                return Err(AiError::InvalidArguments(
                    "character_update_clip requires 'name' and/or 'duration_secs'".into(),
                ));
            }
            history.execute(
                scene,
                Command::UpdateAnimationClipMeta {
                    clip_index,
                    name,
                    duration_secs,
                },
            )?;
            let updated = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.animations.get(clip_index));
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "name": updated.map(|c| c.name.as_str()),
                "duration_secs": updated.map(|c| c.duration_secs)
            }))
        }
        "mesh_set_crease_edges" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let arr = args
                .get("edges")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'edges' array".into()))?;
            if arr.len() > 65_536 {
                return Err(AiError::InvalidArguments(
                    "'edges' exceeds maximum of 65536 entries".into(),
                ));
            }
            let mut edges = Vec::with_capacity(arr.len());
            for item in arr {
                let pair = item.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("each edge must be [v0, v1]".into())
                })?;
                if pair.len() != 2 {
                    return Err(AiError::InvalidArguments(
                        "each edge must have 2 vertex indices".into(),
                    ));
                }
                let raw_a = pair[0]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments("edge vertex must be int".into()))?;
                let raw_b = pair[1]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments("edge vertex must be int".into()))?;
                let a = usize::try_from(raw_a)
                    .map_err(|_| AiError::InvalidArguments("edge vertex overflow".into()))?;
                let b = usize::try_from(raw_b)
                    .map_err(|_| AiError::InvalidArguments("edge vertex overflow".into()))?;
                edges.push((a, b));
            }
            let count = edges.len();
            history.execute(scene, Command::SetMeshCreaseEdges { node_id, edges })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "crease_edge_count": count }))
        }
        "viewport_set_shading_mode" => {
            let mode_str = args
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'mode'".into()))?;
            let mode = match mode_str.to_ascii_lowercase().as_str() {
                "pbr" => tessel_core::ViewportShadingMode::Pbr,
                "npr_sumie" | "npr" | "sumie" => tessel_core::ViewportShadingMode::NprSumie,
                "silhouette_check" | "silhouette" => {
                    tessel_core::ViewportShadingMode::SilhouetteCheck
                }
                "luminance_value_check" | "luminance" => {
                    tessel_core::ViewportShadingMode::LuminanceValueCheck
                }
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown shading mode '{other}', expected pbr|npr_sumie|silhouette_check|luminance_value_check"
                    )))
                }
            };
            history.execute(scene, Command::SetShadingMode(mode))?;
            Ok(json!({ "ok": true, "shading_mode": mode }))
        }
        "mesh_validate_topology" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let mesh = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("low-poly mesh not found".into()))?;
            let report = tessel_core::validate_mesh_topology(mesh);
            Ok(json!({ "ok": true, "node_id": node_id.0, "report": report }))
        }
        "character_create_bone_grid" => {
            let mut spec = tessel_core::BoneGridSpec::default();
            if let Some(p) = args.get("prefix").and_then(Value::as_str) {
                spec.prefix = p.to_string();
            }
            if let Some(c) = args.get("cols").and_then(Value::as_u64) {
                spec.cols = c as usize;
            }
            if let Some(r) = args.get("rows").and_then(Value::as_u64) {
                spec.rows = r as usize;
            }
            if let Some(v) = parse_opt_vec3(args, "origin")? {
                spec.origin = v;
            }
            if let Some(w) = args.get("width").and_then(Value::as_f64) {
                spec.width = w as f32;
            }
            if let Some(h) = args.get("height").and_then(Value::as_f64) {
                spec.height = h as f32;
            }
            if let Some(v) = parse_opt_vec3(args, "axis_u")? {
                spec.axis_u = v;
            }
            if let Some(v) = parse_opt_vec3(args, "axis_v")? {
                spec.axis_v = v;
            }
            if let Some(pb) = args.get("parent_bone").and_then(Value::as_u64) {
                spec.parent_bone = Some(BoneId(pb as u32));
            }
            let bind_node = args.get("bind_node").and_then(Value::as_u64).map(NodeId);
            let cols = spec.cols.clamp(2, 16);
            let rows = spec.rows.clamp(2, 16);
            let prefix = spec.prefix.clone();
            history.execute(scene, Command::CreateBoneGrid { spec, bind_node })?;
            Ok(json!({
                "ok": true,
                "prefix": prefix,
                "cols": cols,
                "rows": rows,
                "bone_count": cols * rows
            }))
        }
        "character_bake_cloth_wind" => {
            let bone_prefix = args
                .get("bone_prefix")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'bone_prefix'".into()))?
                .to_string();
            let mut params = tessel_core::ClothWindParams::default();
            if let Some(cn) = args.get("clip_name").and_then(Value::as_str) {
                params.clip_name = cn.to_string();
            }
            if let Some(v) = parse_opt_vec3(args, "wind_dir")? {
                params.wind_dir = v;
            }
            if let Some(v) = args.get("wind_strength").and_then(Value::as_f64) {
                params.wind_strength = v as f32;
            }
            if let Some(v) = args.get("gravity").and_then(Value::as_f64) {
                params.gravity = v as f32;
            }
            if let Some(v) = args.get("turbulence").and_then(Value::as_f64) {
                params.turbulence = v as f32;
            }
            if let Some(v) = args.get("phase_offset").and_then(Value::as_f64) {
                params.phase_offset = v as f32;
            }
            if let Some(v) = args.get("duration_secs").and_then(Value::as_f64) {
                params.duration_secs = v as f32;
            }
            if let Some(v) = args.get("fps").and_then(Value::as_f64) {
                params.fps = v as f32;
            }
            let clip_name = params.clip_name.clone();
            history.execute(
                scene,
                Command::BakeClothGridAnimation {
                    bone_prefix,
                    params,
                },
            )?;
            Ok(json!({ "ok": true, "clip_name": clip_name }))
        }
        "mesh_morph_set_deltas" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                .to_string();
            let replace = args
                .get("replace")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let parse_pairs = |key: &str| -> Result<Vec<(usize, glam::Vec3)>, AiError> {
                let mut out = Vec::new();
                if let Some(arr) = args.get(key).and_then(Value::as_array) {
                    for item in arr {
                        let vi = item
                            .get("vertex")
                            .and_then(Value::as_u64)
                            .ok_or_else(|| AiError::InvalidArguments(format!("invalid {key}.vertex")))?
                            as usize;
                        let d = parse_opt_vec3(item, "delta")?
                            .ok_or_else(|| AiError::InvalidArguments(format!("invalid {key}.delta")))?;
                        out.push((vi, d));
                    }
                }
                Ok(out)
            };
            let position_deltas = parse_pairs("position_deltas")?;
            let normal_deltas = parse_pairs("normal_deltas")?;
            let pos_count = position_deltas.len();
            let nrm_count = normal_deltas.len();
            history.execute(
                scene,
                Command::SetMorphTargetDeltas {
                    node_id,
                    name: name.clone(),
                    position_deltas,
                    normal_deltas,
                    replace,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "target": name,
                "position_deltas": pos_count,
                "normal_deltas": nrm_count
            }))
        }
        "mesh_morph_set_weight" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                .to_string();
            let weight = args
                .get("weight")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'weight'".into()))?
                as f32;
            history.execute(
                scene,
                Command::SetMorphDefaultWeight {
                    node_id,
                    name: name.clone(),
                    weight,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "target": name, "weight": weight.clamp(0.0, 1.0) }))
        }
        "mesh_proxy_cage_deform" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let divisions = if let Some(arr) = args.get("divisions").and_then(Value::as_array) {
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "divisions must have 3 integers in 2..=6".into(),
                    ));
                }
                let mut out = [2usize; 3];
                for (i, item) in arr.iter().enumerate() {
                    let v = item.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments(format!(
                            "divisions[{i}] must be an integer in 2..=6"
                        ))
                    })? as usize;
                    if !(2..=6).contains(&v) {
                        return Err(AiError::InvalidArguments(format!(
                            "divisions[{i}] must be in 2..=6, got {v}"
                        )));
                    }
                    out[i] = v;
                }
                out
            } else {
                [2, 2, 2]
            };
            let arr = args
                .get("control_deltas")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'control_deltas'".into()))?;
            let mut control_deltas = Vec::with_capacity(arr.len());
            for item in arr {
                let idx = item
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing control_deltas.index".into()))?
                    as usize;
                let d = parse_opt_vec3(item, "delta")?
                    .ok_or_else(|| AiError::InvalidArguments("missing control_deltas.delta".into()))?;
                control_deltas.push((idx, d));
            }
            let floor_y = args.get("floor_y").and_then(Value::as_f64).map(|v| v as f32);
            let bake_as_morph = args
                .get("bake_as_morph")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            history.execute(
                scene,
                Command::ApplyProxyCageDeform {
                    node_id,
                    divisions,
                    control_deltas,
                    floor_y,
                    bake_as_morph: bake_as_morph.clone(),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "baked_morph": bake_as_morph }))
        }
        "mesh_bake_vat" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let stem = args
                .get("stem")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'stem'".into()))?;
            let clip_index = args.get("clip_index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let fps = args.get("fps").and_then(Value::as_f64).unwrap_or(15.0) as f32;
            let max_frames = args
                .get("max_frames")
                .and_then(Value::as_u64)
                .unwrap_or(120) as usize;
            let report = tessel_io::export_vat_bundle(
                scene,
                &tessel_core::VatBakeParams {
                    node_id,
                    clip_index,
                    fps,
                    max_frames,
                },
                allowed_export_dir,
                stem,
            )?;
            Ok(json!({ "ok": true, "report": report }))
        }
        "mesh_create_ribbon" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Ribbon")
                .to_string();
            let source_bone = args
                .get("source_bone")
                .and_then(Value::as_u64)
                .map(|b| BoneId(b as u32));
            let source_node = args.get("source_node").and_then(Value::as_u64).map(NodeId);
            let mut explicit_points = Vec::new();
            if let Some(val) = args.get("explicit_points") {
                let arr = val.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("explicit_points must be an array of [x,y,z]".into())
                })?;
                for (i, pt) in arr.iter().enumerate() {
                    let a = pt.as_array().ok_or_else(|| {
                        AiError::InvalidArguments(format!(
                            "explicit_points[{i}] must be a 3-element array"
                        ))
                    })?;
                    if a.len() != 3 {
                        return Err(AiError::InvalidArguments(format!(
                            "explicit_points[{i}] must have exactly 3 numbers"
                        )));
                    }
                    let (Some(x), Some(y), Some(z)) =
                        (a[0].as_f64(), a[1].as_f64(), a[2].as_f64())
                    else {
                        return Err(AiError::InvalidArguments(format!(
                            "explicit_points[{i}] elements must be numbers"
                        )));
                    };
                    let v = glam::Vec3::new(x as f32, y as f32, z as f32);
                    if !v.is_finite() {
                        return Err(AiError::InvalidArguments(format!(
                            "explicit_points[{i}] must be finite"
                        )));
                    }
                    explicit_points.push(v);
                }
            }
            let clip_index = args.get("clip_index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let start_time = args.get("start_time").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let end_time = args.get("end_time").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let samples = args.get("samples").and_then(Value::as_u64).unwrap_or(16) as usize;
            let width_start = args
                .get("width_start")
                .and_then(Value::as_f64)
                .unwrap_or(0.16) as f32;
            let width_end = args
                .get("width_end")
                .and_then(Value::as_f64)
                .unwrap_or(0.02) as f32;
            let mat_idx = args.get("material").and_then(Value::as_u64).unwrap_or(1) as u8;
            let material = PaletteIndex::new(mat_idx)
                .map_err(|_| AiError::InvalidArguments("material must be 1..=255".into()))?;
            let bake_visibility_track = args
                .get("bake_visibility_track")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let params = tessel_core::RibbonParams {
                name,
                source_bone,
                source_node,
                explicit_points,
                clip_index,
                start_time,
                end_time,
                samples,
                width_start,
                width_end,
                material,
                bake_visibility_track,
            };
            history.execute(scene, Command::BakeProceduralRibbon { params })?;
            let id = scene.selected_node.map(|n| n.0);
            Ok(json!({ "ok": true, "node_id": id }))
        }
        "character_stick_gizmo_pose" => {
            let bone_id = BoneId(
                args.get("bone_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'bone_id'".into()))?
                    as u32,
            );
            let mode_str = args
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'mode'".into()))?;
            let mode = match mode_str.to_ascii_lowercase().as_str() {
                "roll" => tessel_core::StickGizmoMode::Roll,
                "pitch" => tessel_core::StickGizmoMode::Pitch,
                "yaw" => tessel_core::StickGizmoMode::Yaw,
                "twist" => tessel_core::StickGizmoMode::Twist,
                "roll_twist" | "rolltwist" => tessel_core::StickGizmoMode::RollTwist,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown stick gizmo mode '{other}'"
                    )))
                }
            };
            let angle_deg = args
                .get("angle_deg")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'angle_deg'".into()))?
                as f32;
            let chain_len = args.get("chain_len").and_then(Value::as_u64).unwrap_or(3) as usize;
            let snap_deg = args.get("snap_deg").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let mirror_lr = args
                .get("mirror_lr")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let axis_lock = args.get("axis_lock").and_then(Value::as_u64).map(|v| v as u8);
            let params = tessel_core::StickGizmoParams {
                bone_id,
                mode,
                angle_deg,
                chain_len,
                snap_deg,
                mirror_lr,
                axis_lock,
            };
            history.execute(scene, Command::ApplyStickGizmoPose { params })?;
            Ok(json!({ "ok": true, "bone_id": bone_id.0, "mode": mode }))
        }
        "character_apply_biped_rig" => {
            let preset = args
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("starter");
            let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
            let include_default_clips = match args.get("include_default_clips") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'include_default_clips' must be a boolean".into(),
                    ));
                }
            };
            if append {
                let template = match preset {
                    "starter" => tessel_core::CharacterRig::biped_template(),
                    "humanoid" => tessel_core::CharacterRig::humanoid_template(),
                    other => {
                        return Err(AiError::InvalidArguments(format!(
                            "unknown rig preset '{other}'; use 'starter' or 'humanoid'"
                        )));
                    }
                };
                let prefix = args.get("prefix").and_then(Value::as_str).unwrap_or("P2_");
                let offset = parse_opt_vec3(args, "offset")?.unwrap_or(glam::Vec3::ZERO);
                let yaw_deg = args.get("yaw_deg").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                if !yaw_deg.is_finite() {
                    return Err(AiError::InvalidArguments("'yaw_deg' must be finite".into()));
                }
                let yaw_q = glam::Quat::from_rotation_y(yaw_deg.to_radians());

                let mut cmds = Vec::with_capacity(template.bones.len() + 4);
                let base_rig_clips: Vec<String>;
                let base_id = if let Some(rig) = scene.character_rig.as_ref() {
                    base_rig_clips = rig.animations.iter().map(|c| c.name.clone()).collect();
                    rig.bones
                        .iter()
                        .map(|b| b.id.0)
                        .max()
                        .map(|m| m + 1)
                        .unwrap_or(0)
                } else {
                    let base_tmpl = tessel_core::CharacterRig::humanoid_template();
                    cmds.push(Command::ApplyHumanoidRig);
                    if !include_default_clips {
                        for _ in 0..base_tmpl.animations.len() {
                            cmds.push(Command::RemoveAnimationClip { clip_index: 0 });
                        }
                        base_rig_clips = Vec::new();
                    } else {
                        base_rig_clips = base_tmpl
                            .animations
                            .iter()
                            .map(|c| c.name.clone())
                            .collect();
                    }
                    base_tmpl
                        .bones
                        .iter()
                        .map(|b| b.id.0)
                        .max()
                        .map(|m| m + 1)
                        .unwrap_or(0)
                };
                let mut id_map = std::collections::BTreeMap::new();
                let mut bone_ids_json = serde_json::Map::new();
                let mut appended_root: Option<BoneId> = None;
                for (i, b) in template.bones.iter().enumerate() {
                    let new_id = BoneId(base_id + i as u32);
                    id_map.insert(b.id, new_id);
                    let new_parent = b.parent.and_then(|p| id_map.get(&p).copied());
                    let pos = if new_parent.is_none() {
                        appended_root = Some(new_id);
                        offset + b.rest_local.translation
                    } else {
                        // Keep child translations in character-local space; Root rest_local.rotation
                        // carries yaw_deg so all child pose/keyframe rotations stay character-local.
                        b.rest_local.translation
                    };
                    let name = format!("{prefix}{}", b.name);
                    bone_ids_json.insert(name.clone(), json!(new_id.0));
                    cmds.push(Command::AddBone {
                        name,
                        parent: new_parent,
                        position: pos,
                    });
                }
                if let Some(root_id) = appended_root {
                    if yaw_deg.abs() > 1e-4 {
                        cmds.push(Command::SetBoneRestRotation {
                            bone_id: root_id,
                            rotation: yaw_q,
                        });
                    }
                }
                let count = template.bones.len();
                if include_default_clips {
                    for tmpl_clip in &template.animations {
                        if let Some(clip_idx) =
                            base_rig_clips.iter().position(|n| n == &tmpl_clip.name)
                        {
                            for ch in &tmpl_clip.channels {
                                if let Some(&mapped_bone) = id_map.get(&ch.bone_id) {
                                    for kf in &ch.keyframes {
                                        cmds.push(Command::SetAnimationKeyframe {
                                            clip_index: clip_idx,
                                            bone_id: mapped_bone,
                                            keyframe: *kf,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                history.execute(scene, Command::Transaction(cmds))?;
                return Ok(json!({
                    "ok": true,
                    "preset": preset,
                    "appended": true,
                    "bones_added": count,
                    "bone_ids": bone_ids_json
                }));
            }
            let (apply_cmd, bones, default_clips) = match preset {
                "starter" => (
                    Command::ApplyBipedRig,
                    7usize,
                    tessel_core::CharacterRig::biped_template().animations.len(),
                ),
                "humanoid" => (
                    Command::ApplyHumanoidRig,
                    19usize,
                    tessel_core::CharacterRig::humanoid_template()
                        .animations
                        .len(),
                ),
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown rig preset '{other}'; use 'starter' or 'humanoid'"
                    )));
                }
            };
            if include_default_clips || default_clips == 0 {
                history.execute(scene, apply_cmd)?;
            } else {
                let mut cmds = Vec::with_capacity(1 + default_clips);
                cmds.push(apply_cmd);
                for _ in 0..default_clips {
                    cmds.push(Command::RemoveAnimationClip { clip_index: 0 });
                }
                history.execute(scene, Command::Transaction(cmds))?;
            }
            let clips = scene
                .character_rig
                .as_ref()
                .map(|r| r.animations.len())
                .unwrap_or(0);
            Ok(json!({ "ok": true, "preset": preset, "bones": bones, "clips": clips }))
        }
        "character_set_bone_rest_position" => {
            let bone_id = parse_bone_id(args, "bone_id")?;
            let position = parse_opt_vec3(args, "position")?;
            let has_rot = args.get("rotation_deg").is_some();
            if position.is_none() && !has_rot {
                return Err(AiError::InvalidArguments(
                    "provide 'position' and/or 'rotation_deg'".into(),
                ));
            }
            let mut cmds = Vec::new();
            if let Some(pos) = position {
                cmds.push(Command::SetBoneRestPosition {
                    bone_id,
                    position: pos,
                });
            }
            if has_rot {
                let rotation = parse_rotation_deg(args)?;
                cmds.push(Command::SetBoneRestRotation { bone_id, rotation });
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({
                "ok": true,
                "bone_id": bone_id.0,
                "rest_position": position.map(|p| p.to_array()),
                "rotation_updated": has_rot
            }))
        }
        "character_add_bone" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                .to_string();
            let parent = match args.get("parent_bone_id").and_then(Value::as_u64) {
                Some(raw) => Some(BoneId(u32::try_from(raw).map_err(|_| {
                    AiError::InvalidArguments("parent_bone_id overflow".into())
                })?)),
                None => None,
            };
            let position = parse_opt_vec3(args, "position")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'position'".into()))?;
            history.execute(
                scene,
                Command::AddBone {
                    name: name.clone(),
                    parent,
                    position,
                },
            )?;
            let bone_id = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.bones.last())
                .map_or(0, |b| b.id.0);
            Ok(json!({ "ok": true, "bone_id": bone_id, "name": name }))
        }
        "character_bind_node" => {
            let raw_node = args
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let node_id = NodeId(raw_node);
            let bone_id = match args.get("bone_id").and_then(Value::as_u64) {
                Some(raw) => {
                    Some(BoneId(u32::try_from(raw).map_err(|_| {
                        AiError::InvalidArguments("bone_id overflow".into())
                    })?))
                }
                None => None,
            };
            history.execute(scene, Command::BindNodeToBone { node_id, bone_id })?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "bone_id": bone_id.map(|b| b.0)
            }))
        }
        "character_set_node_bone_mask" => {
            let node_id = NodeId(
                args.get("node_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?,
            );
            let bone_ids = match args.get("bone_ids") {
                None | Some(Value::Null) => None,
                Some(Value::Array(arr)) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr {
                        let raw = v.as_u64().ok_or_else(|| {
                            AiError::InvalidArguments("'bone_ids' must be integers".into())
                        })?;
                        let id = u32::try_from(raw).map_err(|_| {
                            AiError::InvalidArguments("'bone_ids' entry overflows u32".into())
                        })?;
                        out.push(BoneId(id));
                    }
                    Some(out)
                }
                _ => {
                    return Err(AiError::InvalidArguments(
                        "'bone_ids' must be an array of integers or null".into(),
                    ));
                }
            };
            let count = bone_ids.as_ref().map_or(0, Vec::len);
            history.execute(scene, Command::SetNodeBoneMask { node_id, bone_ids })?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "masked_bones": count
            }))
        }
        "character_set_bone_pose" => {
            let raw_id = args
                .get("bone_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'bone_id'".into()))?;
            let bone_id = BoneId(
                u32::try_from(raw_id)
                    .map_err(|_| AiError::InvalidArguments("bone_id overflow".into()))?,
            );
            let has_rot = args.get("rotation_deg").is_some();
            let trans_delta = parse_opt_vec3(args, "translation")?;
            if !has_rot && trans_delta.is_none() {
                return Err(AiError::InvalidArguments(
                    "provide 'rotation_deg' and/or 'translation'".into(),
                ));
            }
            let dry_run = parse_opt_bool(args, "dry_run")?;
            let rot_delta = if has_rot {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            if dry_run {
                let rig = scene.character_rig.as_ref().ok_or_else(|| {
                    AiError::InvalidArguments("no character rig applied".into())
                })?;
                let existing = rig.bone(bone_id).ok_or_else(|| {
                    AiError::InvalidArguments(format!("bone {} not found", bone_id.0))
                })?;
                let before_mats = rig.world_pose_matrices();
                let world_pos_before = before_mats
                    .get(&bone_id)
                    .map(|m| m.transform_point3(glam::Vec3::ZERO))
                    .unwrap_or(glam::Vec3::ZERO);
                let mut preview_rig = rig.clone();
                if let Some(b) = preview_rig.bones.iter_mut().find(|b| b.id == bone_id) {
                    if let Some(delta) = rot_delta {
                        b.pose_local.rotation = b.rest_local.rotation * delta;
                    }
                    if let Some(td) = trans_delta {
                        b.pose_local.translation = b.rest_local.translation + td;
                    }
                }
                let after_mats = preview_rig.world_pose_matrices();
                let after_mat = after_mats
                    .get(&bone_id)
                    .copied()
                    .unwrap_or(glam::Mat4::IDENTITY);
                let world_pos_after = after_mat.transform_point3(glam::Vec3::ZERO);
                let local_rot = preview_rig
                    .bone(bone_id)
                    .map(|b| b.pose_local.rotation)
                    .unwrap_or(glam::Quat::IDENTITY);
                let local_up = (local_rot * glam::Vec3::Y).normalize_or_zero();
                let local_fwd = (local_rot * glam::Vec3::Z).normalize_or_zero();
                let world_up = after_mat
                    .transform_vector3(glam::Vec3::Y)
                    .normalize_or_zero();
                let world_fwd = after_mat
                    .transform_vector3(glam::Vec3::Z)
                    .normalize_or_zero();
                return Ok(json!({
                    "ok": true,
                    "dry_run": true,
                    "bone_id": bone_id.0,
                    "bone_name": existing.name,
                    "world_pos_before": world_pos_before.to_array(),
                    "world_pos_after": world_pos_after.to_array(),
                    "local_up_dir": local_up.to_array(),
                    "local_forward_dir": local_fwd.to_array(),
                    "world_up_dir": world_up.to_array(),
                    "world_forward_dir": world_fwd.to_array(),
                }));
            }
            let (rest_rot, rest_trans) = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.bone(bone_id))
                .map(|b| (b.rest_local.rotation, b.rest_local.translation))
                .unwrap_or((glam::Quat::IDENTITY, glam::Vec3::ZERO));
            let mut cmds = Vec::new();
            if let Some(delta) = rot_delta {
                cmds.push(Command::SetBoneRotation {
                    bone_id,
                    rotation: rest_rot * delta,
                });
            }
            if let Some(td) = trans_delta {
                cmds.push(Command::SetBoneTranslation {
                    bone_id,
                    translation: rest_trans + td,
                });
            }
            if cmds.len() == 1 {
                history.execute(scene, cmds.remove(0))?;
            } else {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({ "ok": true, "bone_id": bone_id.0 }))
        }
        "character_set_clip_layer" => {
            if args.get("clear").and_then(Value::as_bool).unwrap_or(false) {
                history.execute(scene, Command::SetClipLayers { layers: Vec::new() })?;
                return Ok(json!({ "ok": true, "layers": 0 }));
            }
            let bone_prefix = args
                .get("bone_prefix")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let time_offset = args
                .get("time_offset")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32;
            if !time_offset.is_finite() {
                return Err(AiError::InvalidArguments(
                    "'time_offset' must be finite".into(),
                ));
            }
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                rig.animations
                    .iter()
                    .position(|c| c.name == name)
                    .ok_or_else(|| AiError::InvalidArguments(format!("clip '{name}' not found")))?
            } else {
                return Err(AiError::InvalidArguments(
                    "missing 'clip_name' or 'clip_index'".into(),
                ));
            };
            if clip_index >= rig.animations.len() {
                return Err(AiError::InvalidArguments("clip_index out of range".into()));
            }
            let mut layers = rig.clip_layers.clone();
            if let Some(existing) = layers.iter_mut().find(|l| l.bone_prefix == bone_prefix) {
                existing.clip_index = clip_index;
                existing.time_offset = time_offset;
            } else {
                layers.push(tessel_core::ClipLayer {
                    clip_index,
                    bone_prefix: bone_prefix.clone(),
                    time_offset,
                });
            }
            let count = layers.len();
            history.execute(scene, Command::SetClipLayers { layers })?;
            Ok(json!({
                "ok": true,
                "bone_prefix": bone_prefix,
                "clip_index": clip_index,
                "layers": count
            }))
        }
        "character_bake_clip_layers" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let fps = args.get("fps").and_then(Value::as_f64).unwrap_or(24.0) as f32;
            if !fps.is_finite() || fps < 1.0 {
                return Err(AiError::InvalidArguments("'fps' must be >= 1".into()));
            }
            let duration_override = match args.get("duration_secs") {
                None => None,
                Some(v) => {
                    let d = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'duration_secs' must be a number".into())
                    })? as f32;
                    if !d.is_finite() || d <= 0.0 {
                        return Err(AiError::InvalidArguments(
                            "'duration_secs' must be > 0".into(),
                        ));
                    }
                    Some(d)
                }
            };
            let clamp_floor_y = match args.get("floor_y") {
                None => None,
                Some(v) => {
                    let fy = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'floor_y' must be a number".into())
                    })? as f32;
                    if !fy.is_finite() {
                        return Err(AiError::InvalidArguments("'floor_y' must be finite".into()));
                    }
                    Some(fy)
                }
            };
            let activate = args.get("activate").and_then(Value::as_bool).unwrap_or(true);
            let clear_layers = args
                .get("clear_layers")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let bounds = tessel_core::SkinnedBoundsCache::from_scene(scene, rig);
            let baked = rig
                .bake_clip_layers_with_bounds(
                    name.clone(),
                    fps,
                    duration_override,
                    clamp_floor_y,
                    bounds.as_ref(),
                )
                .ok_or_else(|| {
                    AiError::InvalidArguments("rig has no bones or animations to bake".into())
                })?;
            let duration_secs = baked.duration_secs;
            let channels = baked.channels.len();
            let visibility_tracks = baked.visibility_tracks.len();
            let mut cmds = vec![Command::UpsertAnimationClip {
                clip: baked,
                activate,
            }];
            if clear_layers {
                cmds.push(Command::SetClipLayers { layers: Vec::new() });
            }
            history.execute(scene, Command::Transaction(cmds))?;
            let clip_index = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.animations.iter().position(|c| c.name == name))
                .unwrap_or(0);
            Ok(json!({
                "ok": true,
                "name": name,
                "clip_index": clip_index,
                "duration_secs": duration_secs,
                "channels": channels,
                "visibility_tracks": visibility_tracks
            }))
        }
        "character_solve_chain_ik" => {
            let ids_arr = args
                .get("bone_ids")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'bone_ids' array".into()))?;
            if ids_arr.len() < 2 || ids_arr.len() > 32 {
                return Err(AiError::InvalidArguments(
                    "'bone_ids' must contain 2..=32 bone IDs".into(),
                ));
            }
            let mut chain = Vec::with_capacity(ids_arr.len());
            for item in ids_arr {
                let raw = item
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| AiError::InvalidArguments("invalid bone_id in 'bone_ids'".into()))?;
                chain.push(BoneId(raw));
            }
            let target = parse_opt_vec3(args, "target")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'target' [x,y,z]".into()))?;
            let max_iterations = args
                .get("max_iterations")
                .and_then(Value::as_u64)
                .map(|v| (v.min(64) as u32).max(1))
                .unwrap_or(24);
            let tolerance = args
                .get("tolerance")
                .and_then(Value::as_f64)
                .map(|v| (v as f32).clamp(1e-4, 1.0))
                .unwrap_or(1e-3);
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let rots =
                tessel_core::solve_chain_ik(rig, &chain, target, max_iterations, tolerance)?;
            let cmds: Vec<Command> = rots
                .iter()
                .map(|&(bone_id, rotation)| Command::SetBoneRotation { bone_id, rotation })
                .collect();
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else if !cmds.is_empty() {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({
                "ok": true,
                "solved_bones": rots.len(),
                "end_bone_id": chain.last().map(|b| b.0)
            }))
        }
        "character_set_clip_stepped_fps" => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                rig.animations
                    .iter()
                    .position(|c| c.name == name)
                    .ok_or_else(|| AiError::InvalidArguments(format!("clip '{name}' not found")))?
            } else {
                rig.active_animation
            };
            let mut clip = rig
                .animations
                .get(clip_index)
                .cloned()
                .ok_or_else(|| AiError::InvalidArguments(format!("clip {clip_index} not found")))?;
            let stepped_fps = match args.get("stepped_fps") {
                Some(Value::Null) => None,
                Some(v) => {
                    let f = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'stepped_fps' must be a number or null".into())
                    })? as f32;
                    if !f.is_finite() || f < 1.0 {
                        return Err(AiError::InvalidArguments(
                            "'stepped_fps' must be >= 1.0".into(),
                        ));
                    }
                    Some(f.clamp(1.0, 60.0))
                }
                None => clip.stepped_fps,
            };
            let bake_stepped = args
                .get("bake_stepped")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            clip.stepped_fps = stepped_fps;
            if bake_stepped {
                let fps = stepped_fps.unwrap_or(12.0);
                clip = clip.bake_stepped(fps);
            }
            let clip_name = clip.name.clone();
            history.execute(
                scene,
                Command::UpsertAnimationClip {
                    clip,
                    activate: false,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "clip_name": clip_name,
                "stepped_fps": stepped_fps,
                "baked": bake_stepped
            }))
        }
        "character_clamp_clip_to_floor" => {
            let floor_y = args.get("floor_y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            if !floor_y.is_finite() {
                return Err(AiError::InvalidArguments("'floor_y' must be finite".into()));
            }
            let fps = args.get("fps").and_then(Value::as_f64).unwrap_or(24.0) as f32;
            if !fps.is_finite() || fps < 1.0 {
                return Err(AiError::InvalidArguments("'fps' must be >= 1".into()));
            }
            let bone_prefix = args
                .get("bone_prefix")
                .and_then(Value::as_str)
                .unwrap_or("");
            let (surface_ids, options) = parse_grounding_args(args)?;
            let surface = if !surface_ids.is_empty() {
                Some(
                    tessel_core::SurfaceTarget::from_scene_nodes(scene, &surface_ids).ok_or_else(
                        || {
                            AiError::InvalidArguments(
                                "none of the specified surface node_ids contain faces or voxels"
                                    .into(),
                            )
                        },
                    )?,
                )
            } else {
                None
            };
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                rig.animations
                    .iter()
                    .position(|c| c.name == name)
                    .ok_or_else(|| AiError::InvalidArguments(format!("clip '{name}' not found")))?
            } else {
                rig.active_animation
            };
            let bounds =
                tessel_core::SkinnedBoundsCache::from_scene_excluding_many(scene, rig, &surface_ids);
            let clamped = rig
                .clamp_clip_to_surface_v2(
                    clip_index,
                    bone_prefix,
                    floor_y,
                    fps,
                    bounds.as_ref(),
                    surface.as_ref(),
                    &options,
                )
                .ok_or_else(|| {
                    AiError::InvalidArguments(format!(
                        "unable to clamp clip {clip_index} for prefix '{bone_prefix}'"
                    ))
                })?;
            let clip_name = clamped.name.clone();
            let duration_secs = clamped.duration_secs;
            history.execute(
                scene,
                Command::UpsertAnimationClip {
                    clip: clamped,
                    activate: false,
                },
            )?;
            let ids_json: Vec<u64> = surface_ids.iter().map(|n| n.0).collect();
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "clip_name": clip_name,
                "bone_prefix": bone_prefix,
                "floor_y": floor_y,
                "surface_node_id": surface_ids.first().map(|n| n.0),
                "surface_node_ids": ids_json,
                "solve_foot_ik": options.solve_foot_ik,
                "align_foot_normal": options.align_foot_normal,
                "pelvis_tilt": options.pelvis_tilt,
                "duration_secs": duration_secs
            }))
        }
        "character_clamp_to_floor" => {
            let floor_y = args.get("floor_y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            if !floor_y.is_finite() {
                return Err(AiError::InvalidArguments("'floor_y' must be finite".into()));
            }
            let bone_prefix = args
                .get("bone_prefix")
                .and_then(Value::as_str)
                .unwrap_or("");
            let (surface_ids, options) = parse_grounding_args(args)?;
            let surface = if !surface_ids.is_empty() {
                Some(
                    tessel_core::SurfaceTarget::from_scene_nodes(scene, &surface_ids).ok_or_else(
                        || {
                            AiError::InvalidArguments(
                                "none of the specified surface node_ids contain faces or voxels"
                                    .into(),
                            )
                        },
                    )?,
                )
            } else {
                None
            };
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let bounds =
                tessel_core::SkinnedBoundsCache::from_scene_excluding_many(scene, rig, &surface_ids);
            let mut sim = rig.clone();
            let (root_id, min_y, delta_y, target_y) = sim
                .clamp_sub_skeleton_to_surface_v2(
                    bone_prefix,
                    floor_y,
                    bounds.as_ref(),
                    surface.as_ref(),
                    &options,
                )
                .ok_or_else(|| {
                    AiError::InvalidArguments(format!(
                        "no root bone found or unable to compute bounds for prefix '{bone_prefix}'"
                    ))
                })?;
            let next_trans = sim
                .bone(root_id)
                .map(|b| b.pose_local.translation)
                .ok_or_else(|| AiError::InvalidArguments("root bone not found".into()))?;
            let mut cmds = vec![Command::SetBoneTranslation {
                bone_id: root_id,
                translation: next_trans,
            }];
            for (orig_b, sim_b) in rig.bones.iter().zip(sim.bones.iter()) {
                if orig_b
                    .pose_local
                    .rotation
                    .angle_between(sim_b.pose_local.rotation)
                    > 1e-4
                {
                    cmds.push(Command::SetBoneRotation {
                        bone_id: sim_b.id,
                        rotation: sim_b.pose_local.rotation,
                    });
                }
            }
            if cmds.len() == 1 {
                if let Some(cmd) = cmds.into_iter().next() {
                    history.execute(scene, cmd)?;
                }
            } else {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            let ids_json: Vec<u64> = surface_ids.iter().map(|n| n.0).collect();
            Ok(json!({
                "ok": true,
                "root_bone_id": root_id.0,
                "min_y_before": min_y,
                "delta_y": delta_y,
                "floor_y": target_y,
                "surface_node_id": surface_ids.first().map(|n| n.0),
                "surface_node_ids": ids_json,
                "solve_foot_ik": options.solve_foot_ik,
                "align_foot_normal": options.align_foot_normal,
                "pelvis_tilt": options.pelvis_tilt
            }))
        }
        "character_set_parent_switch_keyframe" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            if !time_secs.is_finite() || time_secs < 0.0 {
                return Err(AiError::InvalidArguments(
                    "'time_secs' must be finite and >= 0".into(),
                ));
            }
            let bone_id = match args.get("bone_id") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let raw = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments("'bone_id' must be integer or null".into())
                    })?;
                    Some(BoneId(u32::try_from(raw).map_err(|_| {
                        AiError::InvalidArguments("bone_id overflow".into())
                    })?))
                }
            };
            let position = parse_opt_vec3(args, "position")?.unwrap_or(glam::Vec3::ZERO);
            let rotation = if args.get("rotation_deg").is_some() {
                parse_rotation_deg(args)?
            } else {
                glam::Quat::IDENTITY
            };
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                match existing {
                    Some(i) => i,
                    None => {
                        let dur = args
                            .get("duration_secs")
                            .and_then(Value::as_f64)
                            .unwrap_or((time_secs as f64).max(1.0))
                            as f32;
                        history.execute(
                            scene,
                            Command::AddAnimationClip {
                                name: name.to_string(),
                                duration_secs: dur.max(0.1),
                            },
                        )?;
                        scene
                            .character_rig
                            .as_ref()
                            .map(|r| r.animations.len().saturating_sub(1))
                            .unwrap_or(0)
                    }
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map(|r| r.active_animation)
                    .unwrap_or(0)
            };
            let key = tessel_core::ParentSwitchKey {
                time_secs,
                bone_id,
                position,
                rotation,
            };
            history.execute(
                scene,
                Command::SetParentSwitchKeyframe {
                    clip_index,
                    node_id,
                    key,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "node_id": node_id.0,
                "time_secs": time_secs,
                "bone_id": bone_id.map(|b| b.0),
                "detached": bone_id.is_none()
            }))
        }
        "character_set_visibility_keyframe" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            let visible = args
                .get("visible")
                .and_then(Value::as_bool)
                .ok_or_else(|| AiError::InvalidArguments("missing boolean 'visible'".into()))?;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                match existing {
                    Some(i) => i,
                    None => {
                        let dur = args
                            .get("duration_secs")
                            .and_then(Value::as_f64)
                            .unwrap_or((time_secs as f64).max(1.0))
                            as f32;
                        history.execute(
                            scene,
                            Command::AddAnimationClip {
                                name: name.to_string(),
                                duration_secs: dur.max(0.1),
                            },
                        )?;
                        scene
                            .character_rig
                            .as_ref()
                            .map(|r| r.animations.len().saturating_sub(1))
                            .unwrap_or(0)
                    }
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map(|r| r.active_animation)
                    .unwrap_or(0)
            };
            history.execute(
                scene,
                Command::SetVisibilityKeyframe {
                    clip_index,
                    node_id,
                    time_secs,
                    visible,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "node_id": node_id.0,
                "time_secs": time_secs,
                "visible": visible
            }))
        }
        "character_set_keyframe" => {
            let parse_interp = |obj: &Value| -> Result<tessel_core::KeyframeInterpolation, AiError> {
                match obj
                    .get("interpolation")
                    .and_then(Value::as_str)
                    .unwrap_or("linear")
                {
                    "linear" => Ok(tessel_core::KeyframeInterpolation::Linear),
                    "step" => Ok(tessel_core::KeyframeInterpolation::Step),
                    "ease_in" => Ok(tessel_core::KeyframeInterpolation::EaseIn),
                    "ease_out" => Ok(tessel_core::KeyframeInterpolation::EaseOut),
                    "ease_in_out" => Ok(tessel_core::KeyframeInterpolation::EaseInOut),
                    "cubic_bezier" => {
                        let (p1x, p1y, p2x, p2y) = match obj.get("bezier") {
                            None | Some(Value::Null) => (0.25, 0.1, 0.25, 1.0),
                            Some(val) => {
                                let a = val.as_array().ok_or_else(|| {
                                    AiError::InvalidArguments(
                                        "'bezier' must be an array of 4 numbers [p1x,p1y,p2x,p2y]"
                                            .into(),
                                    )
                                })?;
                                if a.len() != 4 {
                                    return Err(AiError::InvalidArguments(
                                        "'bezier' must have exactly 4 numbers [p1x,p1y,p2x,p2y]"
                                            .into(),
                                    ));
                                }
                                let mut pts = [0.0f32; 4];
                                for (i, item) in a.iter().enumerate() {
                                    let v = item.as_f64().ok_or_else(|| {
                                        AiError::InvalidArguments(
                                            "'bezier' entries must be finite numbers".into(),
                                        )
                                    })? as f32;
                                    if !v.is_finite() {
                                        return Err(AiError::InvalidArguments(
                                            "'bezier' entries must be finite numbers".into(),
                                        ));
                                    }
                                    pts[i] = v;
                                }
                                (
                                    pts[0].clamp(0.0, 1.0),
                                    pts[1].clamp(-1.0, 2.0),
                                    pts[2].clamp(0.0, 1.0),
                                    pts[3].clamp(-1.0, 2.0),
                                )
                            }
                        };
                        Ok(tessel_core::KeyframeInterpolation::CubicBezier {
                            p1x,
                            p1y,
                            p2x,
                            p2y,
                        })
                    }
                    other => Err(AiError::InvalidArguments(format!(
                        "unsupported interpolation '{other}'; use linear|step|ease_in|ease_out|ease_in_out|cubic_bezier"
                    ))),
                }
            };
            let parse_kf_entry = |obj: &Value| -> Result<(BoneId, Keyframe), AiError> {
                let raw_id = obj
                    .get("bone_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'bone_id'".into()))?;
                let bone_id = BoneId(
                    u32::try_from(raw_id)
                        .map_err(|_| AiError::InvalidArguments("bone_id overflow".into()))?,
                );
                let time_secs = obj
                    .get("time_secs")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                    as f32;
                let rotation = parse_rotation_deg(obj)?;
                let translation = parse_opt_vec3(obj, "translation")?.unwrap_or(glam::Vec3::ZERO);
                let scale = parse_opt_vec3(obj, "scale")?.unwrap_or(glam::Vec3::ONE);
                let interpolation = parse_interp(obj)?;
                Ok((
                    bone_id,
                    Keyframe {
                        time_secs,
                        translation,
                        rotation,
                        scale,
                        interpolation,
                    },
                ))
            };

            let bulk_entries: Option<Vec<(BoneId, Keyframe)>> =
                if let Some(arr) = args.get("keyframes") {
                    let list = arr.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'keyframes' must be an array".into())
                    })?;
                    if list.is_empty() || list.len() > 2048 {
                        return Err(AiError::InvalidArguments(
                            "'keyframes' must contain between 1 and 2048 entries".into(),
                        ));
                    }
                    let mut parsed = Vec::with_capacity(list.len());
                    for item in list {
                        parsed.push(parse_kf_entry(item)?);
                    }
                    Some(parsed)
                } else {
                    None
                };

            let single_entry = if bulk_entries.is_none() {
                Some(parse_kf_entry(args)?)
            } else {
                None
            };

            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                if let Some(i) = existing {
                    i
                } else {
                    let duration_secs = args
                        .get("duration_secs")
                        .and_then(Value::as_f64)
                        .unwrap_or(2.0) as f32;
                    history.execute(
                        scene,
                        Command::AddAnimationClip {
                            name: name.to_string(),
                            duration_secs,
                        },
                    )?;
                    scene
                        .character_rig
                        .as_ref()
                        .map_or(0, |r| r.animations.len().saturating_sub(1))
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map_or(0, |r| r.active_animation)
            };

            if let Some(entries) = bulk_entries {
                let count = entries.len();
                let cmds: Vec<Command> = entries
                    .into_iter()
                    .map(|(bone_id, keyframe)| Command::SetAnimationKeyframe {
                        clip_index,
                        bone_id,
                        keyframe,
                    })
                    .collect();
                history.execute(scene, Command::Transaction(cmds))?;
                return Ok(json!({
                    "ok": true,
                    "clip_index": clip_index,
                    "count": count
                }));
            }

            let Some((bone_id, keyframe)) = single_entry else {
                return Err(AiError::InvalidArguments("missing keyframe payload".into()));
            };
            let time_secs = keyframe.time_secs;
            let translation = keyframe.translation;
            let scale = keyframe.scale;
            history.execute(
                scene,
                Command::SetAnimationKeyframe {
                    clip_index,
                    bone_id,
                    keyframe,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "bone_id": bone_id.0,
                "time_secs": time_secs,
                "translation": translation.to_array(),
                "scale": scale.to_array()
            }))
        }
        "character_solve_ik" => {
            let parse_bone = |key: &str| -> Result<BoneId, AiError> {
                let raw = args
                    .get(key)
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
                Ok(BoneId(u32::try_from(raw).map_err(|_| {
                    AiError::InvalidArguments(format!("'{key}' overflow"))
                })?))
            };
            let root = parse_bone("root_bone_id")?;
            let mid = parse_bone("mid_bone_id")?;
            let end = parse_bone("end_bone_id")?;
            let target = parse_opt_vec3(args, "target")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'target'".into()))?;
            let pole = if let Some(pt) = parse_opt_vec3(args, "pole_target")? {
                let root_world = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.world_pose_matrices().get(&root).copied())
                    .map(|m| m.transform_point3(glam::Vec3::ZERO))
                    .unwrap_or(glam::Vec3::ZERO);
                let dir = pt - root_world;
                if dir.length_squared() > 1e-8 {
                    dir.normalize()
                } else {
                    glam::Vec3::Z
                }
            } else {
                parse_opt_vec3(args, "pole_vector")?.unwrap_or(glam::Vec3::Z)
            };
            let end_world_rot_deg = parse_opt_vec3(args, "end_world_rotation_deg")?;
            let end_rot_deg = parse_opt_vec3(args, "end_rotation_deg")?;

            history.execute(
                scene,
                Command::SolveTwoBoneIk {
                    root_bone: root,
                    mid_bone: mid,
                    end_bone: end,
                    target,
                    pole,
                },
            )?;
            if let Some(wdeg) = end_world_rot_deg {
                let desired_world = glam::Quat::from_euler(
                    glam::EulerRot::XYZ,
                    wdeg.x.to_radians(),
                    wdeg.y.to_radians(),
                    wdeg.z.to_radians(),
                );
                let parent_world_rot = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.world_pose_matrices().get(&mid).copied())
                    .map(|m| m.to_scale_rotation_translation().1.normalize())
                    .unwrap_or(glam::Quat::IDENTITY);
                let local_rot = (parent_world_rot.inverse() * desired_world).normalize();
                history.execute(
                    scene,
                    Command::SetBoneRotation {
                        bone_id: end,
                        rotation: local_rot,
                    },
                )?;
            } else if let Some(deg) = end_rot_deg {
                let rest = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.bone(end))
                    .map(|b| b.rest_local.rotation)
                    .unwrap_or(glam::Quat::IDENTITY);
                let delta = glam::Quat::from_euler(
                    glam::EulerRot::XYZ,
                    deg.x.to_radians(),
                    deg.y.to_radians(),
                    deg.z.to_radians(),
                );
                history.execute(
                    scene,
                    Command::SetBoneRotation {
                        bone_id: end,
                        rotation: rest * delta,
                    },
                )?;
            }

            let bone_delta_deg = |bid: BoneId| -> [f32; 3] {
                scene
                    .character_rig
                    .as_ref()
                    .and_then(|rig| rig.bone(bid))
                    .map(|b| {
                        let d = b.rest_local.rotation.inverse() * b.pose_local.rotation;
                        let (rx, ry, rz) = d.to_euler(glam::EulerRot::XYZ);
                        [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()]
                    })
                    .unwrap_or([0.0, 0.0, 0.0])
            };
            let reached = scene
                .character_rig
                .as_ref()
                .and_then(|rig| rig.world_pose_matrices().get(&end).copied())
                .map(|m| m.transform_point3(glam::Vec3::ZERO).to_array())
                .unwrap_or([f32::NAN; 3]);
            Ok(json!({
                "ok": true,
                "root_bone_id": root.0,
                "mid_bone_id": mid.0,
                "end_bone_id": end.0,
                "reached": reached,
                "rotations_deg": {
                    "root": bone_delta_deg(root),
                    "mid": bone_delta_deg(mid),
                    "end": bone_delta_deg(end)
                }
            }))
        }
        "character_snapshot_pose_to_keyframe" => {
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            if !time_secs.is_finite() || time_secs < 0.0 {
                return Err(AiError::InvalidArguments(
                    "'time_secs' must be finite and >= 0".into(),
                ));
            }
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                match existing {
                    Some(i) => i,
                    None => {
                        let dur = args
                            .get("duration_secs")
                            .and_then(Value::as_f64)
                            .unwrap_or((time_secs as f64).max(1.0))
                            as f32;
                        history.execute(
                            scene,
                            Command::AddAnimationClip {
                                name: name.to_string(),
                                duration_secs: dur.max(0.1),
                            },
                        )?;
                        scene
                            .character_rig
                            .as_ref()
                            .map(|r| r.animations.len().saturating_sub(1))
                            .unwrap_or(0)
                    }
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map(|r| r.active_animation)
                    .unwrap_or(0)
            };

            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let filter_ids: Option<Vec<BoneId>> = match args.get("bone_ids") {
                None => None,
                Some(v) => {
                    let arr = v.as_array().ok_or_else(|| {
                        AiError::InvalidArguments("'bone_ids' must be an array".into())
                    })?;
                    let mut out = Vec::with_capacity(arr.len());
                    for item in arr {
                        let raw = item.as_u64().ok_or_else(|| {
                            AiError::InvalidArguments("'bone_ids' must contain integers".into())
                        })?;
                        out.push(BoneId(u32::try_from(raw).map_err(|_| {
                            AiError::InvalidArguments("bone_id overflow".into())
                        })?));
                    }
                    Some(out)
                }
            };
            let interpolation = match args.get("interpolation").and_then(Value::as_str) {
                None | Some("linear") => tessel_core::KeyframeInterpolation::Linear,
                Some("step") => tessel_core::KeyframeInterpolation::Step,
                Some("ease_in") => tessel_core::KeyframeInterpolation::EaseIn,
                Some("ease_out") => tessel_core::KeyframeInterpolation::EaseOut,
                Some("ease_in_out") => tessel_core::KeyframeInterpolation::EaseInOut,
                Some("cubic_bezier") => {
                    let (p1x, p1y, p2x, p2y) = match args.get("bezier") {
                        None | Some(Value::Null) => (0.25, 0.1, 0.25, 1.0),
                        Some(val) => {
                            let a = val.as_array().ok_or_else(|| {
                                AiError::InvalidArguments(
                                    "'bezier' must be an array of 4 numbers [p1x,p1y,p2x,p2y]"
                                        .into(),
                                )
                            })?;
                            if a.len() != 4 {
                                return Err(AiError::InvalidArguments(
                                    "'bezier' must have exactly 4 numbers [p1x,p1y,p2x,p2y]".into(),
                                ));
                            }
                            let mut pts = [0.0f32; 4];
                            for (i, item) in a.iter().enumerate() {
                                let v = item.as_f64().ok_or_else(|| {
                                    AiError::InvalidArguments(
                                        "'bezier' entries must be finite numbers".into(),
                                    )
                                })? as f32;
                                if !v.is_finite() {
                                    return Err(AiError::InvalidArguments(
                                        "'bezier' entries must be finite numbers".into(),
                                    ));
                                }
                                pts[i] = v;
                            }
                            (
                                pts[0].clamp(0.0, 1.0),
                                pts[1].clamp(-1.0, 2.0),
                                pts[2].clamp(0.0, 1.0),
                                pts[3].clamp(-1.0, 2.0),
                            )
                        }
                    };
                    tessel_core::KeyframeInterpolation::CubicBezier {
                        p1x,
                        p1y,
                        p2x,
                        p2y,
                    }
                }
                Some(other) => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown interpolation '{other}'; use linear|step|ease_in|ease_out|ease_in_out|cubic_bezier"
                    )));
                }
            };
            let cmds: Vec<Command> = rig
                .bones
                .iter()
                .filter(|b| {
                    filter_ids
                        .as_ref()
                        .is_none_or(|list| list.contains(&b.id))
                })
                .map(|b| {
                    let rot_delta = b.rest_local.rotation.inverse() * b.pose_local.rotation;
                    let trans_delta = b.pose_local.translation - b.rest_local.translation;
                    Command::SetAnimationKeyframe {
                        clip_index,
                        bone_id: b.id,
                        keyframe: Keyframe {
                            time_secs,
                            translation: trans_delta,
                            rotation: rot_delta,
                            scale: glam::Vec3::ONE,
                            interpolation,
                        },
                    }
                })
                .collect();
            let keyframed_bones = cmds.len();
            if keyframed_bones == 0 {
                return Err(AiError::InvalidArguments(
                    "no matching bones found to snapshot".into(),
                ));
            }
            history.execute(scene, Command::Transaction(cmds))?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "time_secs": time_secs,
                "keyframed_bones": keyframed_bones
            }))
        }
        "character_set_morph_weight" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let target = args
                .get("target")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'target'".into()))?;
            if target.is_empty() {
                return Err(AiError::InvalidArguments(
                    "'target' must not be empty".into(),
                ));
            }
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            let weight = args
                .get("weight")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'weight'".into()))?
                as f32;
            let clip_index = if let Some(idx) = args.get("clip_index").and_then(Value::as_u64) {
                idx as usize
            } else if let Some(name) = args.get("clip_name").and_then(Value::as_str) {
                let existing = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| r.animations.iter().position(|c| c.name == name));
                if let Some(i) = existing {
                    i
                } else {
                    let duration_secs = args
                        .get("duration_secs")
                        .and_then(Value::as_f64)
                        .unwrap_or(2.0) as f32;
                    history.execute(
                        scene,
                        Command::AddAnimationClip {
                            name: name.to_string(),
                            duration_secs,
                        },
                    )?;
                    scene
                        .character_rig
                        .as_ref()
                        .map_or(0, |r| r.animations.len().saturating_sub(1))
                }
            } else {
                scene
                    .character_rig
                    .as_ref()
                    .map_or(0, |r| r.active_animation)
            };
            history.execute(
                scene,
                Command::SetMorphWeightKeyframe {
                    clip_index,
                    node_id,
                    target: target.to_string(),
                    time_secs,
                    weight,
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "node_id": node_id.0,
                "target": target,
                "time_secs": time_secs,
                "weight": weight
            }))
        }
        "character_remove_morph_weight" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let target = args
                .get("target")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'target'".into()))?;
            let clip_index = args
                .get("clip_index")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'clip_index'".into()))?
                as usize;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            history.execute(
                scene,
                Command::RemoveMorphWeightKeyframe {
                    clip_index,
                    node_id,
                    target: target.to_string(),
                    time_secs,
                },
            )?;
            Ok(json!({ "ok": true, "clip_index": clip_index }))
        }
        "character_list_bones" => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig in scene".into()))?;
            let world_rest = rig.world_rest_matrices();
            let bones: Vec<Value> = rig
                .bones
                .iter()
                .map(|b| {
                    let local_pos = b.rest_local.translation.to_array();
                    let world_pos = world_rest
                        .get(&b.id)
                        .map(|m| m.w_axis.truncate().to_array())
                        .unwrap_or(local_pos);
                    json!({
                        "bone_id": b.id.0,
                        "name": b.name,
                        "parent_bone_id": b.parent.map(|p| p.0),
                        "rest_position": local_pos,
                        "local_rest_position": local_pos,
                        "world_rest_position": world_pos,
                    })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "coordinate_space": "local_rest_position is parent-relative; world_rest_position is rig world space",
                "bone_count": bones.len(),
                "bones": bones
            }))
        }
        "character_inspect_weights" => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig in scene".into()))?;
            let solver = tessel_core::AutoSkinSolver::from_rig(rig, 1.0)
                .ok_or_else(|| AiError::InvalidArguments("character rig has no bones".into()))?;
            let node_id = args.get("node_id").and_then(Value::as_u64).map(NodeId);
            let sample_point = if let Some(pt) = parse_opt_vec3(args, "point")? {
                pt
            } else if let Some(nid) = node_id {
                let mat = scene.node_world_matrix(nid);
                if let Some(m) = scene.lowpoly_meshes.get(&nid) {
                    let (lo, hi) = m.bounds();
                    mat.transform_point3((lo + hi) * 0.5)
                } else {
                    mat.transform_point3(glam::Vec3::ZERO)
                }
            } else {
                return Err(AiError::InvalidArguments(
                    "provide 'point' [x,y,z] and/or 'node_id'".into(),
                ));
            };
            let rigid_bone = node_id.and_then(|nid| {
                rig.bindings
                    .iter()
                    .find(|b| b.node_id == nid)
                    .map(|b| b.bone_id.0)
            });
            let manual_weight_vertices = node_id
                .and_then(|nid| scene.lowpoly_meshes.get(&nid))
                .map_or(0, |m| m.skin_weights.len());
            let w = solver.weights_for_vertex(node_id, sample_point);
            let mut influences = Vec::new();
            for k in 0..4 {
                if w.weights[k] > 0.0 {
                    let bone_name = rig
                        .bones
                        .get(usize::from(w.joints[k]))
                        .map(|b| b.name.as_str())
                        .unwrap_or("unknown");
                    influences.push(json!({
                        "joint_index": w.joints[k],
                        "bone_name": bone_name,
                        "weight": w.weights[k]
                    }));
                }
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.map(|n| n.0),
                "sample_point": sample_point.to_array(),
                "rigid_bone_id": rigid_bone,
                "manual_weight_vertices": manual_weight_vertices,
                "influences": influences
            }))
        }
        "character_inspect_clip" => {
            let clip_index = resolve_clip_index(args, scene, history, false)?;
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig in scene".into()))?;
            let clip = rig
                .animations
                .get(clip_index)
                .ok_or_else(|| AiError::InvalidArguments("clip_index out of range".into()))?;
            let channels: Vec<Value> = clip
                .channels
                .iter()
                .map(|ch| {
                    let keys: Vec<Value> = ch
                        .keyframes
                        .iter()
                        .map(|k| {
                            json!({
                                "time_secs": k.time_secs,
                                "translation": k.translation.to_array(),
                                "rotation_quat": k.rotation.to_array(),
                                "rotation_deg": quat_to_degrees(k.rotation),
                                "scale": k.scale.to_array(),
                            })
                        })
                        .collect();
                    json!({ "bone_id": ch.bone_id.0, "keyframes": keys })
                })
                .collect();
            let morph_tracks: Vec<Value> = clip
                .morph_tracks
                .iter()
                .map(|t| {
                    json!({
                        "node_id": t.node.0,
                        "target": t.target,
                        "keys": t.keys.iter().map(|k| json!({ "time_secs": k.time_secs, "weight": k.weight })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "name": clip.name,
                "duration_secs": clip.duration_secs,
                "is_active": rig.active_animation == clip_index,
                "channels": channels,
                "morph_tracks": morph_tracks,
            }))
        }
        "character_sample_pose" => {
            let clip_index = resolve_clip_index(args, scene, history, false)?;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            if !time_secs.is_finite() || time_secs < 0.0 {
                return Err(AiError::InvalidArguments(
                    "'time_secs' must be a finite number >= 0".into(),
                ));
            }
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig in scene".into()))?;
            let sampled = rig
                .sample_clip_pose(clip_index, time_secs)
                .ok_or_else(|| AiError::InvalidArguments("clip_index out of range".into()))?;
            let bones: Vec<Value> = sampled
                .iter()
                .map(|s| {
                    let name = rig
                        .bone(s.bone_id)
                        .map(|b| b.name.clone())
                        .unwrap_or_default();
                    json!({
                        "bone_id": s.bone_id.0,
                        "name": name,
                        "world_position": s.world_position.to_array(),
                        "local_rotation_deg": quat_to_degrees(s.local_rotation),
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "clip_index": clip_index, "time_secs": time_secs, "bones": bones }))
        }
        "character_set_active_clip" => {
            let clip_index = resolve_clip_index(args, scene, history, false)?;
            history.execute(scene, Command::SetActiveAnimationClip { clip_index })?;
            Ok(json!({ "ok": true, "clip_index": clip_index }))
        }
        "character_set_playback_time" => {
            let clip_index = if args.get("clip_index").is_some() || args.get("clip_name").is_some()
            {
                Some(resolve_clip_index(args, scene, history, false)?)
            } else {
                None
            };
            let playing = args.get("playing").and_then(Value::as_bool);
            let time_secs = if let Some(t) = args.get("time_secs").and_then(Value::as_f64) {
                t as f32
            } else if playing.is_some() {
                scene
                    .character_rig
                    .as_ref()
                    .map_or(0.0, |r| r.playback_time)
            } else {
                return Err(AiError::InvalidArguments(
                    "missing 'time_secs' or 'playing'".into(),
                ));
            };
            history.execute(
                scene,
                Command::SetAnimationPlaybackTime {
                    clip_index,
                    time_secs,
                    playing,
                },
            )?;
            let active_idx = scene
                .character_rig
                .as_ref()
                .map_or(0, |r| r.active_animation);
            let playing_now = scene.character_rig.as_ref().is_some_and(|r| r.playing);
            Ok(json!({
                "ok": true,
                "clip_index": active_idx,
                "time_secs": time_secs,
                "playing": playing_now
            }))
        }
        "character_reset_pose" => {
            history.execute(scene, Command::ResetCharacterPose)?;
            Ok(json!({ "ok": true }))
        }
        "character_remove_keyframe" => {
            let clip_index = resolve_clip_index(args, scene, history, false)?;
            let bone_id = parse_bone_id(args, "bone_id")?;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            history.execute(
                scene,
                Command::RemoveAnimationKeyframe {
                    clip_index,
                    bone_id,
                    time_secs,
                },
            )?;
            Ok(json!({ "ok": true, "clip_index": clip_index, "bone_id": bone_id.0 }))
        }
        "character_remove_clip" => {
            let clip_index = resolve_clip_index(args, scene, history, false)?;
            let removed_name = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.animations.get(clip_index))
                .map(|c| c.name.clone())
                .unwrap_or_default();
            history.execute(scene, Command::RemoveAnimationClip { clip_index })?;
            let remaining = scene
                .character_rig
                .as_ref()
                .map(|r| r.animations.len())
                .unwrap_or(0);
            Ok(json!({ "ok": true, "removed": removed_name, "remaining_clips": remaining }))
        }
        "character_remove_bone" => {
            let bone_id = parse_bone_id(args, "bone_id")?;
            let before = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig in scene".into()))?;
            let before_bones = before.bones.len();
            let before_bindings = before.bindings.len();
            let before_channels: usize = before.animations.iter().map(|c| c.channels.len()).sum();
            history.execute(scene, Command::RemoveBone { bone_id })?;
            let rig = scene.character_rig.as_ref();
            let after_bones = rig.map(|r| r.bones.len()).unwrap_or(0);
            let after_bindings = rig.map(|r| r.bindings.len()).unwrap_or(0);
            let after_channels: usize = rig
                .map(|r| r.animations.iter().map(|c| c.channels.len()).sum())
                .unwrap_or(0);
            Ok(json!({
                "ok": true,
                "removed_bones": before_bones - after_bones,
                "cleared_bindings": before_bindings - after_bindings,
                "dropped_channels": before_channels - after_channels,
            }))
        }
        "scene_duplicate_node" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no node found".into()))?;
            let name = args.get("name").and_then(Value::as_str).map(str::to_string);
            let pos_override = parse_opt_vec3(args, "position")?;
            let rot_override = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let scale_override = parse_opt_vec3(args, "scale")?;
            let has_transform =
                pos_override.is_some() || rot_override.is_some() || scale_override.is_some();

            if has_transform {
                let mut transform = scene
                    .nodes
                    .get(&node_id)
                    .ok_or_else(|| {
                        AiError::InvalidArguments(format!("node {} not found", node_id.0))
                    })?
                    .transform;
                if let Some(p) = pos_override {
                    transform.translation = p;
                } else {
                    transform.translation += glam::Vec3::new(0.5, 0.0, 0.0);
                }
                if let Some(r) = rot_override {
                    transform.rotation = r;
                }
                if let Some(s) = scale_override {
                    transform.scale = s;
                }
                history.execute(scene, Command::DuplicateNode { node_id, name })?;
                let new_id = scene.selected_node.ok_or_else(|| {
                    AiError::InvalidArguments("duplicate produced no node".into())
                })?;
                history.execute(
                    scene,
                    Command::SetNodeTransform {
                        node_id: new_id,
                        transform,
                    },
                )?;
                Ok(json!({ "ok": true, "node_id": new_id.0 }))
            } else {
                history.execute(scene, Command::DuplicateNode { node_id, name })?;
                let new_id = scene.selected_node.ok_or_else(|| {
                    AiError::InvalidArguments("duplicate produced no node".into())
                })?;
                Ok(json!({ "ok": true, "node_id": new_id.0 }))
            }
        }
        "scene_set_node_visibility" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let visible = args
                .get("visible")
                .and_then(Value::as_bool)
                .ok_or_else(|| AiError::InvalidArguments("missing boolean 'visible'".into()))?;
            history.execute(scene, Command::SetNodeVisibility { node_id, visible })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "visible": visible }))
        }
        "mesh_clear_texture" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let target = args.get("target").and_then(Value::as_str).unwrap_or("color");
            let (clear_color, clear_normal, clear_ao) = match target {
                "color" => (true, false, false),
                "normal" => (false, true, false),
                "ao" => (false, false, true),
                "both" => (true, true, false),
                "all" => (true, true, true),
                _ => {
                    return Err(AiError::InvalidArguments(
                        "target must be 'color', 'normal', 'ao', 'both', or 'all'".into(),
                    ));
                }
            };
            let mut cmds = Vec::new();
            if clear_color || clear_normal {
                cmds.push(Command::ClearMeshTexture {
                    node_id,
                    clear_color,
                    clear_normal,
                });
            }
            if clear_ao {
                cmds.push(Command::SetMeshAoMap {
                    node_id,
                    ao_map: None,
                });
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else if !cmds.is_empty() {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({ "ok": true, "node_id": node_id.0, "cleared": target }))
        }
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
        other if other.starts_with("camera_") => Err(AiError::InvalidArguments(
            "camera tools need the live GUI viewport (HTTP bridge); this headless session has no camera".into(),
        )),
        "viewport_set_preview" => Err(AiError::InvalidArguments(
            "viewport_set_preview needs the live GUI viewport (HTTP bridge)".into(),
        )),
        "viewport_screenshot" | "viewport_render_turntable_gif" => Err(AiError::InvalidArguments(
            "viewport rendering tools are served by the MCP server itself (needs its own GPU); the Direct AI path has no renderer".into(),
        )),
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
                    if let Some(m) = scene.lowpoly_meshes.get(&n.id) {
                        let (bmin, bmax) = m.bounds();
                        json!({
                            "triangles": m.triangle_count(),
                            "faces": m.faces.len(),
                            "bounds": { "min": bmin.to_array(), "max": bmax.to_array() }
                        })
                    } else {
                        json!({ "triangles": 0, "faces": 0 })
                    }
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
                "parent": n.parent.map(|p| p.0),
                "position": n.transform.translation.to_array(),
                "scale": n.transform.scale.to_array(),
                "stats": detail
            })
        })
        .collect();

    let animations: Vec<Value> = scene
        .character_rig
        .as_ref()
        .map(|rig| {
            rig.animations
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    json!({
                        "index": i,
                        "name": c.name,
                        "duration_secs": c.duration_secs,
                        "channels": c.channels.len()
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    json!({
        "active_mode": scene.active_mode.label(),
        "revision": scene.revision,
        "selected_node": scene.selected_node.map(|n| n.0),
        "light_azimuth_deg": scene.light_azimuth.to_degrees(),
        "light_elevation_deg": scene.light_elevation.to_degrees(),
        "has_character_rig": scene.character_rig.is_some(),
        "animations": animations,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_tools_pose_and_keyframe_character_rig() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_bone_pose",
            &json!({ "bone_id": 4, "rotation_deg": [0.0, 0.0, 45.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "Wave",
                "duration_secs": 1.0,
                "bone_id": 4,
                "time_secs": 0.5,
                "rotation_deg": [0.0, 0.0, 90.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        let rig = scene.character_rig.as_ref().unwrap();
        assert_eq!(rig.animations.len(), 3);
        assert_eq!(rig.animations[2].name, "Wave");
        assert_eq!(rig.animations[2].keyframe_times(Some(BoneId(4))), vec![0.5]);
    }

    #[test]
    fn ai_tool_mesh_import_loads_exported_obj_and_glb() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_ai_mesh_import_test");
        let _ = std::fs::create_dir_all(&dir);

        execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "project_export",
            &json!({ "filename": "cube_test.glb", "format": "glb" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        let res = execute_tool_call(
            "mesh_import",
            &json!({ "filename": "cube_test.glb", "name": "Imported Cube" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["faces"], 12);
        assert_eq!(scene.lowpoly_meshes.len(), 2);

        let imported_id = res["node_id"].as_u64().unwrap();
        let del = execute_tool_call(
            "scene_delete_node",
            &json!({ "node_id": imported_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(del["ok"], true);
        assert_eq!(scene.lowpoly_meshes.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ai_tools_set_node_transform_and_primitive_position() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({
                "kind": "cube",
                "size": 1.0,
                "position": [1.5, 2.0, -0.5],
                "scale": [0.5, 2.0, 0.5]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = NodeId(created["node_id"].as_u64().unwrap());
        let node = scene.nodes.get(&node_id).unwrap();
        assert_eq!(node.transform.translation, glam::Vec3::new(1.5, 2.0, -0.5));
        assert_eq!(node.transform.scale, glam::Vec3::new(0.5, 2.0, 0.5));

        execute_tool_call(
            "set_node_transform",
            &json!({
                "node_id": node_id.0,
                "position": [-2.0, 0.5, 1.0],
                "rotation_deg": [0.0, 90.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let updated = scene.nodes.get(&node_id).unwrap();
        assert_eq!(
            updated.transform.translation,
            glam::Vec3::new(-2.0, 0.5, 1.0)
        );
        assert_eq!(updated.transform.scale, glam::Vec3::new(0.5, 2.0, 0.5));
        let summary = scene_summary_json(&scene);
        assert_eq!(summary["nodes"][0]["position"], json!([-2.0, 0.5, 1.0]));

        // Verify palette_set_color, mesh_inspect, mesh_inset_face, mesh_extrude_face, scene_inspect
        let pal_res = execute_tool_call(
            "palette_set_color",
            &json!({ "palette_index": 12, "name": "CamoOlive", "hex": "#4B5320" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(pal_res["palette_index"], 12);

        let minspect = execute_tool_call(
            "mesh_inspect",
            &json!({ "node_id": node_id.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(minspect["face_count"], 6);

        let inset = execute_tool_call(
            "mesh_inset_face",
            &json!({ "node_id": node_id.0, "face_id": 0, "factor": 0.25 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(inset["total_faces"], 10);

        let extrude = execute_tool_call(
            "mesh_extrude_face",
            &json!({ "node_id": node_id.0, "face_id": 0, "distance": 0.2 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(extrude["total_faces"], 14);

        let sinspect =
            execute_tool_call("scene_inspect", &json!({}), &mut scene, &mut history, &dir).unwrap();
        assert_eq!(sinspect["ok"], true);
        assert_eq!(
            sinspect["export_dir"],
            crate::translator::canonical_export_dir_string(&dir)
        );

        let beveled = execute_tool_call(
            "mesh_bevel",
            &json!({ "node_id": node_id.0, "amount": 0.15 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(beveled["ok"], true);

        let sphere = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "sphere", "size": 1.0, "position": [0.0, 2.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(sphere["ok"], true);
        let sphere_id = sphere["node_id"].as_u64().unwrap();

        let parented = execute_tool_call(
            "scene_set_parent",
            &json!({ "node_id": sphere_id, "parent_node_id": node_id.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(parented["parent_node_id"], node_id.0);

        let strap = execute_tool_call(
            "mesh_create_curve_tube",
            &json!({
                "name": "ShoulderStrap",
                "points": [[0.0, 0.2, 0.0], [0.2, 0.6, 0.2], [0.0, 1.0, 0.0]],
                "radius": 0.08,
                "flatness": 0.25
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(strap["ok"], true);
        let strap_id = strap["node_id"].as_u64().unwrap();

        let dec = execute_tool_call(
            "mesh_decimate",
            &json!({ "node_id": strap_id, "ratio": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(dec["ok"], true);

        let bone = execute_tool_call(
            "character_add_bone",
            &json!({ "name": "FlapHinge", "position": [0.0, 1.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(bone["bone_id"], 0);

        let bind = execute_tool_call(
            "character_bind_node",
            &json!({ "node_id": strap_id, "bone_id": 0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(bind["bone_id"], 0);

        let light = execute_tool_call(
            "scene_set_lighting",
            &json!({ "azimuth_deg": 45.0, "elevation_deg": 60.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(light["ok"], true);
    }

    #[test]
    fn ai_tools_move_vertices_set_vertex_and_face_uvs() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        let before_v0 = mesh.vertices[0];
        let before_v1 = mesh.vertices[1];

        let moved = execute_tool_call(
            "mesh_move_vertices",
            &json!({ "node_id": node_id, "vertex_ids": [0], "delta": [0.25, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(moved["ok"], true);
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!((mesh.vertices[0].x - (before_v0.x + 0.25)).abs() < 1e-6);
        assert_eq!(mesh.vertices[1], before_v1);

        let placed = execute_tool_call(
            "mesh_set_vertex",
            &json!({ "node_id": node_id, "vertex_id": 1, "position": [0.1, 0.2, 0.3] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(placed["ok"], true);
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert_eq!(mesh.vertices[1], glam::Vec3::new(0.1, 0.2, 0.3));

        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let unwrapped = execute_tool_call(
            "mesh_set_face_uvs",
            &json!({ "node_id": node_id, "face_id": 0, "uvs": uvs }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(unwrapped["ok"], true);
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert_eq!(mesh.faces[0].uvs, uvs);

        // Rejected inputs surface as errors, not silent no-ops.
        assert!(execute_tool_call(
            "mesh_move_vertices",
            &json!({ "node_id": node_id, "vertex_ids": [], "delta": [0.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "mesh_set_face_uvs",
            &json!({ "node_id": node_id, "face_id": 0, "uvs": [[0.0, 0.0]] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // Undo restores the mesh snapshot taken before the last command.
        history.undo(&mut scene).unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!(mesh.faces[0].uvs.is_empty());
    }

    #[test]
    fn ai_tool_set_keyframe_stores_trs() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let res = execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "Bounce",
                "duration_secs": 2.0,
                "bone_id": 1,
                "time_secs": 0.5,
                "rotation_deg": [0.0, 0.0, 45.0],
                "translation": [0.0, 0.1, 0.0],
                "scale": [1.0, 1.0, 2.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        // f32 JSON round-trip is approximate: compare with tolerance.
        let got_t: Vec<f32> = serde_json::from_value(res["translation"].clone()).unwrap();
        let got_s: Vec<f32> = serde_json::from_value(res["scale"].clone()).unwrap();
        for (got, want) in got_t.iter().zip([0.0f32, 0.1, 0.0]) {
            assert!((got - want).abs() < 1e-6);
        }
        for (got, want) in got_s.iter().zip([1.0f32, 1.0, 2.0]) {
            assert!((got - want).abs() < 1e-6);
        }

        let rig = scene.character_rig.as_ref().unwrap();
        let clip = rig.animations.iter().find(|c| c.name == "Bounce").unwrap();
        let ch = clip
            .channels
            .iter()
            .find(|c| c.bone_id == BoneId(1))
            .unwrap();
        assert_eq!(ch.keyframes.len(), 1);
        assert_eq!(ch.keyframes[0].translation, glam::Vec3::new(0.0, 0.1, 0.0));
        assert_eq!(ch.keyframes[0].scale, glam::Vec3::new(1.0, 1.0, 2.0));

        // Rotation-only callers keep working with TRS defaults.
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "Bounce",
                "bone_id": 1,
                "time_secs": 1.0,
                "rotation_deg": [0.0, 0.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let clip = rig.animations.iter().find(|c| c.name == "Bounce").unwrap();
        let ch = clip
            .channels
            .iter()
            .find(|c| c.bone_id == BoneId(1))
            .unwrap();
        assert_eq!(ch.keyframes.len(), 2);

        // Malformed TRS arrays are rejected, not stored.
        assert!(execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "Bounce",
                "bone_id": 1,
                "time_secs": 1.5,
                "rotation_deg": [0.0, 0.0, 0.0],
                "translation": [0.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
    }

    #[test]
    fn ai_tool_camera_calls_need_live_viewport() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // Headless dispatch has no camera: every camera_* tool must fail
        // closed with a clear message instead of misrouting to UnknownTool.
        for (name, args) in [
            ("camera_get", json!({})),
            (
                "camera_set_pose",
                json!({ "yaw_deg": 45.0, "pitch_deg": 20.0 }),
            ),
            ("camera_look_at", json!({ "target": [0.0, 0.5, 0.0] })),
            ("camera_zoom", json!({ "factor": 0.5 })),
            ("camera_orbit", json!({ "dx": 10.0, "dy": 0.0 })),
            ("camera_pan", json!({ "dx": 10.0, "dy": 0.0 })),
            ("camera_reset", json!({})),
            ("camera_preset", json!({ "name": "front" })),
        ] {
            let err = execute_tool_call(name, &args, &mut scene, &mut history, &dir)
                .expect_err("camera tools must fail without a live viewport");
            let msg = format!("{err:?}");
            assert!(
                msg.contains("live GUI viewport"),
                "unexpected error for {name}: {msg}"
            );
        }
    }

    #[test]
    fn ai_tool_solve_ik_reaches_target() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // Straight 3-bone leg: hip(0,1,0) -> knee(0,0.5,0) -> ankle(0,0,0).
        for (name, parent, pos) in [
            ("Hip", None, [0.0, 1.0, 0.0]),
            ("Knee", Some(0), [0.0, -0.5, 0.0]),
            ("Ankle", Some(1), [0.0, -0.5, 0.0]),
        ] {
            let mut args = serde_json::Map::new();
            args.insert("name".into(), json!(name));
            args.insert("position".into(), json!(pos));
            if let Some(pid) = parent {
                args.insert("parent_bone_id".into(), json!(pid));
            }
            execute_tool_call(
                "character_add_bone",
                &Value::Object(args),
                &mut scene,
                &mut history,
                &dir,
            )
            .unwrap();
        }

        let solved = execute_tool_call(
            "character_solve_ik",
            &json!({
                "root_bone_id": 0,
                "mid_bone_id": 1,
                "end_bone_id": 2,
                "target": [0.0, 0.5, 0.5]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(solved["ok"], true);
        let reached: Vec<f32> = serde_json::from_value(solved["reached"].clone()).unwrap();
        let target = glam::Vec3::new(0.0, 0.5, 0.5);
        assert!(
            (glam::Vec3::from_array([reached[0], reached[1], reached[2]]) - target).length() < 1e-3,
            "reached {reached:?}"
        );

        // Undo restores the straight pose.
        history.undo(&mut scene).unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let world = rig.world_pose_matrices();
        let ankle = world
            .get(&BoneId(2))
            .unwrap()
            .transform_point3(glam::Vec3::ZERO);
        assert!((ankle - glam::Vec3::new(0.0, 0.0, 0.0)).length() < 1e-5);

        // Non-linked triple is rejected, not solved.
        assert!(execute_tool_call(
            "character_solve_ik",
            &json!({
                "root_bone_id": 0,
                "mid_bone_id": 2,
                "end_bone_id": 1,
                "target": [0.0, 0.5, 0.5]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
    }

    #[test]
    fn ai_tool_vertex_weights_paint_and_clear() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();

        let painted = execute_tool_call(
            "mesh_set_vertex_weights",
            &json!({ "node_id": node_id, "vertex_ids": [0, 1], "joints": [2, 0, 0, 0], "weights": [3.0, 1.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(painted["ok"], true);
        assert_eq!(painted["painted"], 2);
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!((mesh.skin_weights[&0].weights[0] - 0.75).abs() < 1e-6);

        let inspect = execute_tool_call(
            "mesh_inspect",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(inspect["manual_weights"], 2);

        // Undo restores the unpainted mesh.
        history.undo(&mut scene).unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!(mesh.skin_weights.is_empty());

        // Re-paint then clear one vertex.
        execute_tool_call(
            "mesh_set_vertex_weights",
            &json!({ "node_id": node_id, "vertex_ids": [0, 1], "joints": [2, 0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_clear_vertex_weights",
            &json!({ "node_id": node_id, "vertex_ids": [0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert_eq!(mesh.skin_weights.len(), 1);

        // Rejected inputs: empty ids, short quads, negative weights.
        for bad in [
            json!({ "node_id": node_id, "vertex_ids": [], "joints": [0, 0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
            json!({ "node_id": node_id, "vertex_ids": [0], "joints": [0, 0, 0], "weights": [1.0, 0.0, 0.0, 0.0] }),
            json!({ "node_id": node_id, "vertex_ids": [0], "joints": [0, 0, 0, 0], "weights": [-1.0, 1.0, 0.0, 0.0] }),
        ] {
            assert!(execute_tool_call(
                "mesh_set_vertex_weights",
                &bad,
                &mut scene,
                &mut history,
                &dir
            )
            .is_err());
        }
    }

    #[test]
    fn ai_tool_rig_introspection_and_small_batch() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // No rig yet: introspection fails closed.
        assert!(execute_tool_call(
            "character_list_bones",
            &json!({}),
            &mut scene,
            &mut history,
            &dir
        )
        .is_err());

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // List bones: 7-bone biped with parents.
        let listed = execute_tool_call(
            "character_list_bones",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(listed["bone_count"], 7);
        assert_eq!(listed["bones"][1]["name"], "Torso");
        assert_eq!(listed["bones"][1]["parent_bone_id"], 0);
        assert!(listed["coordinate_space"].as_str().is_some());
        let torso_local = listed["bones"][1]["local_rest_position"]
            .as_array()
            .unwrap();
        let torso_world = listed["bones"][1]["world_rest_position"]
            .as_array()
            .unwrap();
        assert_eq!(torso_local.len(), 3);
        assert_eq!(torso_world.len(), 3);
        let head_world_y = listed["bones"][2]["world_rest_position"][1]
            .as_f64()
            .unwrap();
        let head_local_y = listed["bones"][2]["local_rest_position"][1]
            .as_f64()
            .unwrap();
        assert!(head_world_y > head_local_y);

        // Inspect the Idle clip (index 0): Torso channel has 3 keys.
        let insp = execute_tool_call(
            "character_inspect_clip",
            &json!({ "clip_name": "Idle" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(insp["name"], "Idle");
        assert!(insp["is_active"].as_bool().unwrap());
        let torso_ch = insp["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["bone_id"] == 1)
            .unwrap();
        assert_eq!(torso_ch["keyframes"].as_array().unwrap().len(), 3);

        // Sample the pose halfway: torso lifted by half the breathing offset.
        let sampled = execute_tool_call(
            "character_sample_pose",
            &json!({ "clip_name": "Idle", "time_secs": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let torso = sampled["bones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["bone_id"] == 1)
            .unwrap();
        assert!((torso["world_position"][1].as_f64().unwrap() - 1.115).abs() < 1e-3);

        // Switch active clip to Walk, then remove a keyframe off it.
        execute_tool_call(
            "character_set_active_clip",
            &json!({ "clip_name": "Walk" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        assert_eq!(rig.animations[rig.active_animation].name, "Walk");
        let left_arm_keys = |scene: &SceneState| {
            scene.character_rig.as_ref().unwrap().animations[1]
                .channels
                .iter()
                .find(|c| c.bone_id == BoneId(3))
                .map(|c| c.keyframes.len())
                .unwrap_or(0)
        };
        assert_eq!(left_arm_keys(&scene), 3);
        execute_tool_call(
            "character_remove_keyframe",
            &json!({ "clip_index": 1, "bone_id": 3, "time_secs": 0.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(left_arm_keys(&scene), 2);

        // Duplicate a low-poly node: cube -> copy with offset + inherited name.
        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();
        let dup = execute_tool_call(
            "scene_duplicate_node",
            &json!({ "node_id": node_id, "name": "Twin" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_ne!(dup["node_id"], node_id);
        let twin = scene
            .lowpoly_meshes
            .get(&NodeId(dup["node_id"].as_u64().unwrap()))
            .unwrap();
        assert_eq!(twin.vertices.len(), 8);
        // Undo drops the copy.
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes.len(), 1);

        // Visibility toggle with undo.
        execute_tool_call(
            "scene_set_node_visibility",
            &json!({ "node_id": node_id, "visible": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(!scene.nodes.get(&NodeId(node_id)).unwrap().visible);
        history.undo(&mut scene).unwrap();
        assert!(scene.nodes.get(&NodeId(node_id)).unwrap().visible);

        // Clear texture round-trips through undo.
        execute_tool_call(
            "mesh_fill_texture",
            &json!({ "node_id": node_id, "palette_index": 5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(scene
            .lowpoly_meshes
            .get(&NodeId(node_id))
            .unwrap()
            .texture
            .is_some());
        execute_tool_call(
            "mesh_clear_texture",
            &json!({ "node_id": node_id, "target": "color" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(scene
            .lowpoly_meshes
            .get(&NodeId(node_id))
            .unwrap()
            .texture
            .is_none());
        history.undo(&mut scene).unwrap();
        assert!(scene
            .lowpoly_meshes
            .get(&NodeId(node_id))
            .unwrap()
            .texture
            .is_some());
        assert!(execute_tool_call(
            "mesh_clear_texture",
            &json!({ "node_id": node_id, "target": "bogus" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // Remove one clip, then one leaf bone (RightArm has no children;
        // removing Torso would drop the body chain).
        let clips_before = scene.character_rig.as_ref().unwrap().animations.len();
        execute_tool_call(
            "character_remove_clip",
            &json!({ "clip_name": "Idle" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(
            scene.character_rig.as_ref().unwrap().animations.len(),
            clips_before - 1
        );
        let removed = execute_tool_call(
            "character_remove_bone",
            &json!({ "bone_id": 4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(removed["removed_bones"], 1);
        let listed = execute_tool_call(
            "character_list_bones",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(listed["bone_count"], 6);
        assert!(scene
            .character_rig
            .as_ref()
            .unwrap()
            .bones
            .iter()
            .all(|b| b.name != "RightArm"));
        // Unknown bone / clip fail cleanly.
        assert!(execute_tool_call(
            "character_remove_bone",
            &json!({ "bone_id": 99 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "character_remove_clip",
            &json!({ "clip_name": "Nope" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
    }

    #[test]
    fn ai_tool_morph_delta_and_clear() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();

        let added = execute_tool_call(
            "mesh_morph_delta",
            &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(added["ok"], true);
        assert_eq!(added["morph"], "Smile");

        let inspect = execute_tool_call(
            "mesh_inspect",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(inspect["morphs"], json!(["Smile"]));

        // Undo drops the morph; re-add then clear by name.
        history.undo(&mut scene).unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!(mesh.morphs.is_empty());
        execute_tool_call(
            "mesh_morph_delta",
            &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_morph_clear",
            &json!({ "node_id": node_id, "name": "Smile" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let mesh = scene.lowpoly_meshes.get(&NodeId(node_id)).unwrap();
        assert!(mesh.morphs.is_empty());

        // Rejected inputs: empty name, empty ids, short delta.
        for bad in [
            json!({ "node_id": node_id, "name": "", "vertex_ids": [0], "delta": [0.0, 0.0, 0.0] }),
            json!({ "node_id": node_id, "name": "X", "vertex_ids": [], "delta": [0.0, 0.0, 0.0] }),
            json!({ "node_id": node_id, "name": "X", "vertex_ids": [0], "delta": [0.0, 0.0] }),
        ] {
            assert!(
                execute_tool_call("mesh_morph_delta", &bad, &mut scene, &mut history, &dir)
                    .is_err()
            );
        }
    }

    #[test]
    fn ai_tool_morph_weight_keyframes() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();
        execute_tool_call(
            "mesh_morph_delta",
            &json!({ "node_id": node_id, "name": "Smile", "vertex_ids": [0], "delta": [0.0, 0.1, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        let set = execute_tool_call(
            "character_set_morph_weight",
            &json!({ "clip_name": "Talk", "node_id": node_id, "target": "Smile", "time_secs": 0.5, "weight": 0.75 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(set["ok"], true);
        let clip_index = set["clip_index"].as_u64().unwrap() as usize;
        let rig = scene.character_rig.as_ref().unwrap();
        let clip = rig.animations.iter().find(|c| c.name == "Talk").unwrap();
        assert_eq!(clip.morph_tracks.len(), 1);
        assert_eq!(clip.morph_tracks[0].keys.len(), 1);

        let removed = execute_tool_call(
            "character_remove_morph_weight",
            &json!({ "clip_index": clip_index, "node_id": node_id, "target": "Smile", "time_secs": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(removed["ok"], true);
        let rig = scene.character_rig.as_ref().unwrap();
        let clip = rig.animations.iter().find(|c| c.name == "Talk").unwrap();
        assert!(clip.morph_tracks.is_empty());

        // Unknown clip index is rejected.
        assert!(execute_tool_call(
            "character_remove_morph_weight",
            &json!({ "clip_index": 99, "node_id": node_id, "target": "Smile", "time_secs": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
    }

    #[test]
    fn ai_tool_list_vertices_paginates_cube_corners() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();

        let first = execute_tool_call(
            "mesh_list_vertices",
            &json!({ "node_id": node_id, "offset": 0, "limit": 4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(first["vertex_count"], 8);
        assert_eq!(first["vertices"].as_array().unwrap().len(), 4);
        assert_eq!(first["vertices"][0]["vertex_id"], 0);
        assert_eq!(first["vertices"][0]["position"], json!([-0.5, 0.0, 0.5]));

        let rest = execute_tool_call(
            "mesh_list_vertices",
            &json!({ "node_id": node_id, "offset": 6, "limit": 64 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(rest["vertices"].as_array().unwrap().len(), 2);
        assert_eq!(rest["vertices"][1]["vertex_id"], 7);

        assert!(execute_tool_call(
            "mesh_list_vertices",
            &json!({ "node_id": 9999 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
    }

    #[test]
    fn lowpoly_1to1_modeling_and_humanoid_rig_tools_roundtrip_and_undo() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. mesh_create_raw with per-face materials
        let raw = execute_tool_call(
            "mesh_create_raw",
            &json!({
                "name": "CustomVisor",
                "vertices": [
                    [-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], [0.1, 0.2, 0.0], [-0.1, 0.2, 0.0]
                ],
                "faces": [
                    { "vertices": [0, 1, 2, 3], "palette_index": 2 }
                ],
                "smooth": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let raw_id = raw["node_id"].as_u64().unwrap();
        assert_eq!(raw["vertices"], 4);
        assert_eq!(raw["faces"], 1);

        // 2. mesh_set_vertices (batch) & mesh_set_face_material
        let batch = execute_tool_call(
            "mesh_set_vertices",
            &json!({
                "node_id": raw_id,
                "updates": [
                    { "vertex_id": 2, "position": [0.12, 0.22, 0.02] },
                    { "vertex_id": 3, "position": [-0.12, 0.22, 0.02] }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(batch["updated"], 2);

        let mat = execute_tool_call(
            "mesh_set_face_material",
            &json!({ "node_id": raw_id, "face_ids": [0], "palette_index": 5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mat["palette_index"], 5);

        // 3. mesh_taper, mesh_bend, mesh_mirror, mesh_set_origin
        execute_tool_call(
            "mesh_taper",
            &json!({ "node_id": raw_id, "axis": "y", "start_scale": 0.8, "end_scale": 1.1 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_bend",
            &json!({ "node_id": raw_id, "bend_axis": "x", "curve_axis": "z", "angle_deg": 35.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let mir = execute_tool_call(
            "mesh_mirror",
            &json!({ "node_id": raw_id, "axis": "x", "merge_original": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mir["faces"], 2);
        execute_tool_call(
            "mesh_set_origin",
            &json!({ "node_id": raw_id, "mode": "center" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // 4. Thin primitive (size 0.015m) is NOT clamped to 0.1m anymore!
        let thin = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Tie", "kind": "cube", "size": 0.015, "palette_index": 1 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(thin["clamped"], false);
        let thin_id = thin["node_id"].as_u64().unwrap();

        // 5. mesh_join merges thin into raw_id and deletes thin_id; undo restores both
        let joined = execute_tool_call(
            "mesh_join",
            &json!({ "target_node_id": raw_id, "source_node_ids": [thin_id] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(joined["faces"], 8);
        assert!(!scene.nodes.contains_key(&NodeId(thin_id)));
        history.undo(&mut scene).unwrap();
        assert!(scene.nodes.contains_key(&NodeId(thin_id)));

        // 6. Humanoid 19-bone rig (Left=+X, Right=-X) + character_set_bone_rest_position
        let rig = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(rig["bones"], 19);
        let bones = execute_tool_call(
            "character_list_bones",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let left_shoulder = bones["bones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "LeftShoulder")
            .unwrap();
        assert!(left_shoulder["rest_position"][0].as_f64().unwrap() > 0.0);

        execute_tool_call(
            "character_set_bone_rest_position",
            &json!({ "bone_id": 4, "position": [0.0, 0.15, 0.01] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(
            (scene.character_rig.as_ref().unwrap().bones[4]
                .rest_local
                .translation
                .y
                - 0.15)
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn audit_p1_input_validation_and_transaction_undo_revision() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. i64 wrapping bypass in parse_ivec3 must be rejected (4_294_967_296 wraps to 0 as i32)
        assert!(execute_tool_call(
            "voxel_fill_box",
            &json!({ "min": [4_294_967_296_i64, 0, 0], "max": [1, 1, 1], "palette_index": 4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // 2. Oversized ellipsoid fails fast before allocating edits
        assert!(matches!(
            execute_tool_call(
                "voxel_fill_ellipsoid",
                &json!({ "center": [0, 0, 0], "radii": [200.0, 200.0, 200.0], "palette_index": 4 }),
                &mut scene,
                &mut history,
                &dir,
            ),
            Err(AiError::BatchLimitExceeded)
        ));

        // 3. Out-of-range rgb in palette_set_color is rejected (no silent unwrap_or(0))
        assert!(execute_tool_call(
            "palette_set_color",
            &json!({ "palette_index": 9, "rgb": [300, 10, 10] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // 4. Undo of a Transaction bumps scene.revision and sets dirty_viewport
        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Box", "kind": "cube", "size": 1.0, "palette_index": 4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let nid = created["node_id"].as_u64().unwrap();
        execute_tool_call(
            "mesh_set_origin",
            &json!({ "node_id": nid, "mode": "center" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        scene.dirty_viewport = false;
        let rev_before = scene.revision;
        history.undo(&mut scene).unwrap();
        assert!(scene.revision > rev_before);
        assert!(scene.dirty_viewport);
    }

    #[test]
    fn texture_paint_line_rect_uv_project_and_local_face_subdivide_roundtrip() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Torso", "kind": "cube", "size": 1.0, "palette_index": 3 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let nid = created["node_id"].as_u64().unwrap();

        // Project cylindrical UVs
        let proj = execute_tool_call(
            "mesh_project_uvs",
            &json!({ "node_id": nid, "mode": "cylindrical" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(proj["faces_projected"], 6);
        assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].faces[0].uvs.len(), 4);

        // Paint line & rect on RGBA texture + undo
        execute_tool_call(
            "mesh_paint_line",
            &json!({
                "node_id": nid,
                "uv_start": [0.1, 0.2],
                "uv_end": [0.9, 0.2],
                "width_px": 6.0,
                "palette_index": 1
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_paint_rect",
            &json!({
                "node_id": nid,
                "uv_min": [0.4, 0.4],
                "uv_max": [0.6, 0.6],
                "palette_index": 5
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(scene.lowpoly_meshes[&NodeId(nid)].texture.is_some());

        // Local face subdivision: 6 faces -> 1 quad replaced by 4 quads = 9 faces
        let sub = execute_tool_call(
            "mesh_subdivide_face",
            &json!({ "node_id": nid, "face_id": 0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(sub["faces"], 9);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].faces.len(), 6);

        // Apply transform bakes world offset + scale into vertices so mirror_x works across world X=0
        execute_tool_call(
            "scene_set_node_transform",
            &json!({ "node_id": nid, "position": [0.3, 0.0, 0.0], "scale": [0.1, 0.1, 0.1] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_apply_transform",
            &json!({ "node_id": nid }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(
            scene.nodes[&NodeId(nid)].transform,
            tessel_core::Transform::default()
        );
        execute_tool_call(
            "mesh_mirror",
            &json!({ "node_id": nid, "axis": "x", "merge_original": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let (lo, hi) = scene.lowpoly_meshes[&NodeId(nid)].bounds();
        assert!((lo.x + 0.35).abs() < 1e-4);
        assert!((hi.x - 0.35).abs() < 1e-4);
    }

    #[test]
    fn mcp_project_save_open_and_node_bone_mask_roundtrip() {
        let dir = std::env::temp_dir().join("tessel_mcp_proj_io_test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Legs", "kind": "cube", "size": 0.4, "palette_index": 7, "position": [-0.15, 0.7, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let nid = created["node_id"].as_u64().unwrap();
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // Restrict Legs node to Root(0) + leg bones (13..=18) so hanging RightHand(12) never pulls hip
        execute_tool_call(
            "character_set_node_bone_mask",
            &json!({ "node_id": nid, "bone_ids": [0, 13, 14, 15, 16, 17, 18] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let inspected = execute_tool_call(
            "character_inspect_weights",
            &json!({ "node_id": nid, "point": [-0.25, 0.85, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        for inf in inspected["influences"].as_array().unwrap() {
            let j = inf["joint_index"].as_u64().unwrap();
            assert!(
                j == 0 || (13..=18).contains(&j),
                "unexpected arm bone influence {j} on masked legs node"
            );
        }

        // Save .tessel atomically inside allowed_export_dir
        let saved = execute_tool_call(
            "project_save",
            &json!({ "filename": "hero_checkpoint.tessel" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(saved["file_size_bytes"].as_u64().unwrap() > 0);

        // Path traversal must be denied on both save and open
        assert!(execute_tool_call(
            "project_save",
            &json!({ "filename": "../escape.tessel" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
        assert!(execute_tool_call(
            "project_open",
            &json!({ "filename": "../escape.tessel" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // Load into a fresh SceneState and verify rig + bone mask survived round-trip
        let mut fresh_scene = SceneState::new();
        let mut fresh_history = HistoryStack::new();
        let opened = execute_tool_call(
            "project_open",
            &json!({ "filename": "hero_checkpoint.tessel" }),
            &mut fresh_scene,
            &mut fresh_history,
            &dir,
        )
        .unwrap();
        assert_eq!(opened["has_character_rig"], true);
        assert!(fresh_scene
            .character_rig
            .as_ref()
            .unwrap()
            .node_bone_masks
            .contains_key(&NodeId(nid)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_v18_animation_scrub_pose_reset_rename_and_clear_roundtrip() {
        let dir = std::env::temp_dir();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // 1. scene_clear removes all nodes atomically and is undoable
        execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "StarterA", "kind": "cube", "size": 0.5, "palette_index": 3 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "StarterB", "kind": "sphere", "size": 0.5, "palette_index": 4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let before = scene.nodes.len();
        assert_eq!(before, 2);
        let cleared =
            execute_tool_call("scene_clear", &json!({}), &mut scene, &mut history, &dir).unwrap();
        assert_eq!(cleared["removed_nodes"], before);
        assert!(scene.nodes.is_empty());
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes.len(), before);

        // 2. scene_rename_node renames cleanly + undo
        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "TempPart", "kind": "cube", "size": 0.5, "palette_index": 3 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let nid = created["node_id"].as_u64().unwrap();
        execute_tool_call(
            "scene_rename_node",
            &json!({ "node_id": nid, "name": "PUBG_Helmet" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(scene.nodes[&NodeId(nid)].name, "PUBG_Helmet");
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes[&NodeId(nid)].name, "TempPart");

        // 3. character_set_playback_time scrubs Walk clip live + character_reset_pose resets
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "Walk", "time_secs": 0.25 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        assert_eq!(rig.active_animation, 1);
        assert!(
            rig.bone(BoneId(13))
                .unwrap()
                .pose_local
                .rotation
                .angle_between(glam::Quat::IDENTITY)
                > 0.1
        );

        execute_tool_call(
            "character_reset_pose",
            &json!({}),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        assert!(
            rig.bone(BoneId(13))
                .unwrap()
                .pose_local
                .rotation
                .angle_between(glam::Quat::IDENTITY)
                < 1e-5
        );
    }

    #[test]
    fn mcp_v19_batch_atomic_rollback_weld_and_flip_normals() {
        let dir = std::env::temp_dir();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // 1. Atomic scene_batch succeeds and returns per-call results
        let ok = execute_tool_call(
            "scene_batch",
            &json!({
                "calls": [
                    { "name": "mesh_create_primitive", "arguments": { "name": "Half", "kind": "cube", "size": 1.0, "palette_index": 4, "position": [0.5, 0.0, 0.0] } },
                    { "name": "mesh_apply_transform", "arguments": {} },
                    { "name": "mesh_mirror", "arguments": { "axis": "x", "merge_original": true } }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(ok["count"], 3);
        let nid = ok["results"][0]["node_id"].as_u64().unwrap();
        assert_eq!(scene.lowpoly_meshes[&NodeId(nid)].vertices.len(), 16);

        // 2. mesh_weld_vertices collapses the 4 duplicated center-seam vertices at X=0 (16 -> 12)
        let welded = execute_tool_call(
            "mesh_weld_vertices",
            &json!({ "node_id": nid, "tolerance": 0.001 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(welded["removed"], 4);
        assert_eq!(welded["vertices_after"], 12);

        // 3. mesh_flip_normals reverses face 0 winding
        let orig_f0 = scene.lowpoly_meshes[&NodeId(nid)].faces[0].vertices.clone();
        execute_tool_call(
            "mesh_flip_normals",
            &json!({ "node_id": nid, "face_ids": [0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let mut expected = orig_f0;
        expected.reverse();
        assert_eq!(
            scene.lowpoly_meshes[&NodeId(nid)].faces[0].vertices,
            expected
        );

        // 4. Failed scene_batch rolls back completely (scene node count unchanged!)
        let count_before = scene.nodes.len();
        let failed = execute_tool_call(
            "scene_batch",
            &json!({
                "calls": [
                    { "name": "mesh_create_primitive", "arguments": { "name": "WillRollback", "kind": "cube", "size": 1.0, "palette_index": 2 } },
                    { "name": "scene_delete_node", "arguments": { "node_id": 999999 } }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(failed.is_err());
        assert_eq!(scene.nodes.len(), count_before);
    }

    #[test]
    fn v27_mcp_friction_fixes_ik_morph_and_glb_roundtrip() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v27_mcp_roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 1. scene_set_mode accepts "Low-Poly" (matching scene_inspect output)
        let mode_res = execute_tool_call(
            "scene_set_mode",
            &json!({ "mode": "Low-Poly" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mode_res["active_mode"], "Low-Poly");

        // 2. mesh_create_primitive accepts non-uniform size [sx, sy, sz]
        let prim = execute_tool_call(
            "mesh_create_primitive",
            &json!({
                "name": "Torso",
                "kind": "cube",
                "size": [0.42, 0.56, 0.24],
                "palette_index": 2
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = prim["node_id"].as_u64().unwrap();
        let (lo, hi) = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].bounds();
        assert!(((hi.x - lo.x) - 0.42).abs() < 1e-4 && ((hi.y - lo.y) - 0.56).abs() < 1e-4);

        // 3. Apply humanoid rig & verify character_solve_ik moves arm bones
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let upper = rig
            .bones
            .iter()
            .find(|b| b.name == "RightUpperArm")
            .unwrap()
            .id
            .0;
        let mid = rig
            .bones
            .iter()
            .find(|b| b.name == "RightForearm")
            .unwrap()
            .id
            .0;
        let end = rig
            .bones
            .iter()
            .find(|b| b.name == "RightHand")
            .unwrap()
            .id
            .0;
        let before_rot = rig
            .bone(tessel_core::BoneId(upper))
            .unwrap()
            .pose_local
            .rotation;
        execute_tool_call(
            "character_solve_ik",
            &json!({
                "root_bone_id": upper,
                "mid_bone_id": mid,
                "end_bone_id": end,
                "target": [-0.35, 1.35, 0.45],
                "pole": [-0.5, 1.1, -0.3]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let after_rot = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(tessel_core::BoneId(upper))
            .unwrap()
            .pose_local
            .rotation;
        assert!(before_rot.angle_between(after_rot) > 1e-3);

        // 4. Verify morph target creation & weight application
        execute_tool_call(
            "mesh_morph_delta",
            &json!({
                "node_id": node_id,
                "name": "Breathe",
                "vertex_ids": [0, 1],
                "delta": [0.0, 0.04, 0.02]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let morph_res = execute_tool_call(
            "character_set_morph_weight",
            &json!({
                "clip_name": "Idle",
                "node_id": node_id,
                "target": "Breathe",
                "time_secs": 0.5,
                "weight": 0.75
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(morph_res["ok"], true);
        let idle = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "Idle")
            .unwrap();
        assert!(idle
            .morph_tracks
            .iter()
            .any(|t| t.target == "Breathe" && !t.keys.is_empty()));

        // 5. Export .glb (using absolute path inside exports dir) & round-trip parse it
        let abs_out = dir.join("v27_character.glb");
        let exp = execute_tool_call(
            "project_export",
            &json!({
                "format": "glb",
                "filename": abs_out.to_string_lossy(),
                "preset": "godot4"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(exp["file_size_bytes"].as_u64().unwrap() > 0);
        assert!(exp["triangles"].as_u64().unwrap() > 0);
        let bytes = std::fs::read(&abs_out).unwrap();
        let parsed =
            tessel_io::import_glb_bytes(&bytes, tessel_core::PaletteIndex::new(1).unwrap())
                .unwrap();
        assert!(!parsed.vertices.is_empty());
        assert!(!parsed.faces.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mesh_create_lathe_builds_helmet_dome_and_visor() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let dome = execute_tool_call(
            "mesh_create_lathe",
            &json!({
                "name": "AltynDome",
                "profile": [[0.14, 0.0], [0.15, 0.08], [0.11, 0.16], [0.0, 0.19]],
                "segments": 16,
                "angle_deg": 360.0,
                "cap_ends": true,
                "palette_index": 3,
                "position": [0.0, 1.55, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(dome["ok"], true);
        assert_eq!(dome["vertices"], 49);
        assert_eq!(dome["faces"], 49);

        let visor = execute_tool_call(
            "mesh_create_lathe",
            &json!({
                "name": "AltynVisor",
                "profile": [[0.155, 0.0], [0.155, 0.10]],
                "segments": 8,
                "angle_deg": 160.0,
                "palette_index": 5
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(visor["ok"], true);
        assert_eq!(visor["faces"], 8);
    }

    #[test]
    fn v29_ik_end_rotation_snapshot_keyframe_and_morph_bounds() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let upper = rig
            .bones
            .iter()
            .find(|b| b.name == "RightUpperArm")
            .unwrap()
            .id
            .0;
        let mid = rig
            .bones
            .iter()
            .find(|b| b.name == "RightForearm")
            .unwrap()
            .id
            .0;
        let end = rig
            .bones
            .iter()
            .find(|b| b.name == "RightHand")
            .unwrap()
            .id
            .0;

        // 1. Solve IK with pole direction + end_rotation_deg and inspect returned rotations_deg
        let ik = execute_tool_call(
            "character_solve_ik",
            &json!({
                "root_bone_id": upper,
                "mid_bone_id": mid,
                "end_bone_id": end,
                "target": [-0.25, 1.30, 0.40],
                "pole_vector": [0.0, -1.0, -1.0],
                "end_rotation_deg": [0.0, 20.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(ik["rotations_deg"]["root"].is_array());
        assert!((ik["rotations_deg"]["end"][1].as_f64().unwrap() - 20.0).abs() < 1e-2);

        // 2. Snapshot solved pose directly into a new clip keyframe
        let snap = execute_tool_call(
            "character_snapshot_pose_to_keyframe",
            &json!({
                "clip_name": "AimClip",
                "duration_secs": 1.0,
                "time_secs": 0.5,
                "bone_ids": [upper, mid, end]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(snap["ok"], true);
        assert_eq!(snap["keyframed_bones"], 3);
        let aim_clip = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "AimClip")
            .unwrap();
        assert_eq!(aim_clip.channels.len(), 3);

        // 3. Spatial bounding-box morph target selection (without manual vertex_ids)
        let mesh_res = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Head", "kind": "cube", "size": 1.0, "palette_index": 2 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = mesh_res["node_id"].as_u64().unwrap();
        let morph = execute_tool_call(
            "mesh_morph_delta",
            &json!({
                "node_id": node_id,
                "name": "RaiseTop",
                "bounds_min": [-1.0, 0.5, -1.0],
                "bounds_max": [1.0, 1.5, 1.0],
                "delta": [0.0, 0.2, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(morph["ok"], true);
        assert_eq!(morph["affected_vertices"], 4);
    }

    #[test]
    fn v30_extruded_polygon_and_ik_world_end_rotation() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. Create necktie silhouette via mesh_create_extruded_polygon
        let tie = execute_tool_call(
            "mesh_create_extruded_polygon",
            &json!({
                "name": "Necktie",
                "points": [[-0.03, 0.0], [0.03, 0.0], [0.045, -0.32], [0.0, -0.38], [-0.045, -0.32]],
                "depth": 0.015,
                "palette_index": 1,
                "position": [0.0, 1.42, 0.12]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(tie["ok"], true);
        assert_eq!(tie["vertices"], 10);
        assert_eq!(tie["faces"], 11); // 3 front tris + 3 back tris + 5 side quads

        // 2. IK with pole_target (world point) + end_world_rotation_deg (world-space aim)
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let upper = rig
            .bones
            .iter()
            .find(|b| b.name == "RightUpperArm")
            .unwrap()
            .id;
        let mid = rig
            .bones
            .iter()
            .find(|b| b.name == "RightForearm")
            .unwrap()
            .id;
        let end = rig.bones.iter().find(|b| b.name == "RightHand").unwrap().id;

        execute_tool_call(
            "character_solve_ik",
            &json!({
                "root_bone_id": upper.0,
                "mid_bone_id": mid.0,
                "end_bone_id": end.0,
                "target": [-0.18, 1.25, 0.35],
                "pole_target": [-0.35, 0.90, -0.15],
                "end_world_rotation_deg": [0.0, 0.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let end_world_mat = scene.character_rig.as_ref().unwrap().world_pose_matrices()[&end];
        let end_world_rot = glam::Quat::from_mat4(&end_world_mat).normalize();
        assert!(
            end_world_rot.angle_between(glam::Quat::IDENTITY) < 1e-2,
            "end_bone_id world rotation should match identity [0,0,0] regardless of forearm IK"
        );
    }

    #[test]
    fn v32_loft_mesh_and_multi_character_subrig_append() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. Create tapered torso via mesh_create_loft
        let loft = execute_tool_call(
            "mesh_create_loft",
            &json!({
                "name": "TaperedTorso",
                "sections": [
                    [[0.0, 0.0, 0.2], [0.25, 0.0, 0.0], [0.0, 0.0, -0.2], [-0.25, 0.0, 0.0]],
                    [[0.0, 0.5, 0.16], [0.20, 0.5, 0.0], [0.0, 0.5, -0.16], [-0.20, 0.5, 0.0]]
                ],
                "cap_ends": true,
                "palette_index": 3
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(loft["ok"], true);
        assert_eq!(loft["vertices"], 8);
        assert_eq!(loft["faces"], 6);

        // 2. Apply P1 humanoid rig, then append P2 humanoid sub-rig at z=2.0 facing -Z (yaw 180)
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({
                "preset": "humanoid",
                "append": true,
                "prefix": "P2_",
                "offset": [0.0, 0.0, 2.0],
                "yaw_deg": 180.0
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(p2["ok"], true);
        assert_eq!(p2["bones_added"], 19);
        assert_eq!(p2["bone_ids"]["P2_Root"], 19);
        assert_eq!(scene.character_rig.as_ref().unwrap().bones.len(), 38);
    }

    #[test]
    fn v33_scene_batch_chains_prior_results_and_bakes_vector_size() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        let batch = execute_tool_call(
            "scene_batch",
            &json!({
                "calls": [
                    {
                        "name": "mesh_create_primitive",
                        "arguments": {
                            "name": "Helmet",
                            "kind": "cube",
                            "size": [0.3, 0.2, 0.25],
                            "palette_index": 3
                        }
                    },
                    {
                        "name": "character_apply_biped_rig",
                        "arguments": { "preset": "humanoid" }
                    },
                    {
                        "name": "character_bind_node",
                        "arguments": {
                            "node_id": "$0.node_id",
                            "bone_id": 4
                        }
                    }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(batch["ok"], true);
        assert_eq!(batch["count"], 3);
        let helmet_id = batch["results"][0]["node_id"].as_u64().unwrap();
        let actual_sz = batch["results"][0]["actual_size"].as_array().unwrap();
        assert!((actual_sz[0].as_f64().unwrap() - 0.3).abs() < 1e-5);
        assert!((actual_sz[1].as_f64().unwrap() - 0.2).abs() < 1e-5);
        assert!((actual_sz[2].as_f64().unwrap() - 0.25).abs() < 1e-5);
        let (lo, hi) = scene.lowpoly_meshes[&tessel_core::NodeId(helmet_id)].bounds();
        assert!(((hi.x - lo.x) - 0.3).abs() < 1e-4);
        assert!(((hi.y - lo.y) - 0.2).abs() < 1e-4);
        assert!(((hi.z - lo.z) - 0.25).abs() < 1e-4);
        assert!(scene
            .character_rig
            .as_ref()
            .unwrap()
            .bindings
            .iter()
            .any(|b| b.node_id == tessel_core::NodeId(helmet_id) && b.bone_id == BoneId(4)));
    }

    #[test]
    fn v34_closes_all_gaps_v2_items() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 2.2 & 2.3: Append P2 humanoid facing -Z (yaw 180) with default clips remapped
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({
                "preset": "humanoid",
                "append": true,
                "prefix": "P2_",
                "offset": [0.0, 0.0, 2.0],
                "yaw_deg": 180.0
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;
        let p2_left_thigh = p2["bone_ids"]["P2_LeftThigh"].as_u64().unwrap() as u32;

        // Walk clip now animates P2's LeftThigh automatically (#2.2)
        let walk = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "Walk")
            .unwrap();
        assert!(walk
            .channels
            .iter()
            .any(|c| c.bone_id == BoneId(p2_left_thigh)));

        // P2_Root carries yaw=180 rest rotation (#2.3) so child local axes are character-local
        let p2_root_bone = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(BoneId(p2_root))
            .unwrap();
        assert!(
            p2_root_bone
                .rest_local
                .rotation
                .angle_between(glam::Quat::from_rotation_y(std::f32::consts::PI))
                < 1e-4
        );

        // 2.5: Pose supports translation + snapshot preserves translation delta
        execute_tool_call(
            "character_set_bone_pose",
            &json!({
                "bone_id": p2_root,
                "translation": [0.25, -0.1, 0.0],
                "rotation_deg": [0.0, 15.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_snapshot_pose_to_keyframe",
            &json!({
                "clip_name": "Dodge",
                "duration_secs": 1.0,
                "time_secs": 0.4,
                "bone_ids": [p2_root]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let dodge = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "Dodge")
            .unwrap();
        let kf = dodge.sample_bone(BoneId(p2_root), 0.4).unwrap();
        assert!((kf.translation.x - 0.25).abs() < 1e-4);
        assert!((kf.translation.y - (-0.1)).abs() < 1e-4);

        // 2.4: Per-node visibility keyframes toggle live visibility on playback scrub
        let flash = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "MuzzleFlash", "kind": "sphere", "size": 0.08, "palette_index": 6 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let flash_id = flash["node_id"].as_u64().unwrap();
        execute_tool_call(
            "character_set_visibility_keyframe",
            &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.0, "visible": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_visibility_keyframe",
            &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.3, "visible": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_visibility_keyframe",
            &json!({ "clip_name": "Dodge", "node_id": flash_id, "time_secs": 0.5, "visible": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "Dodge", "time_secs": 0.1 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(!scene.character_rig.as_ref().unwrap().node_visibility[&NodeId(flash_id)]);
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "Dodge", "time_secs": 0.35 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(scene.character_rig.as_ref().unwrap().node_visibility[&NodeId(flash_id)]);
    }

    #[test]
    fn v36_clip_layers_interpolation_and_floor_clamp() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({
                "preset": "humanoid",
                "append": true,
                "prefix": "P2_",
                "offset": [0.0, 0.0, 2.0],
                "yaw_deg": 180.0
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;

        // 1. Step & ease_in_out keyframe interpolation on ShotFall clip
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "ShotFall",
                "duration_secs": 1.0,
                "bone_id": p2_root,
                "time_secs": 0.0,
                "rotation_deg": [0.0, 0.0, 0.0],
                "interpolation": "step"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "ShotFall",
                "bone_id": p2_root,
                "time_secs": 0.5,
                "rotation_deg": [-85.0, 0.0, 0.0],
                "interpolation": "ease_out"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let shot = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "ShotFall")
            .unwrap();
        // Step interpolation holds 0° until t=0.5s
        let mid_kf = shot.sample_bone(BoneId(p2_root), 0.25).unwrap();
        assert!(mid_kf.rotation.angle_between(glam::Quat::IDENTITY) < 1e-4);

        // 2. Simultaneous per-character clip layers: P1 Walk + P2 ShotFall
        execute_tool_call(
            "character_set_clip_layer",
            &json!({ "clip_name": "ShotFall", "bone_prefix": "P2_" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "Walk", "time_secs": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig = scene.character_rig.as_ref().unwrap();
        let p2_bone = rig.bone(BoneId(p2_root)).unwrap();
        let p2_delta = p2_bone.rest_local.rotation.inverse() * p2_bone.pose_local.rotation;
        assert!(p2_delta.angle_between(glam::Quat::IDENTITY) > 1.0);

        // 3. Floor clamp shifts P2_Root Y so lowest joint sits flush on y = 0.0
        let clamp = execute_tool_call(
            "character_clamp_to_floor",
            &json!({ "floor_y": 0.0, "bone_prefix": "P2_" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(clamp["ok"], true);
        let mats = scene.character_rig.as_ref().unwrap().world_pose_matrices();
        let min_y = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bones
            .iter()
            .filter(|b| b.name.starts_with("P2_"))
            .map(|b| mats[&b.id].transform_point3(glam::Vec3::ZERO).y)
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_y.abs() < 1e-3,
            "min_y after floor clamp should be ~0, got {min_y}"
        );
    }

    #[test]
    fn v39_snapshot_interpolation_bake_clip_layers_and_subframe_floor_clamp() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "HeroBody", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({
                "preset": "starter",
                "append": true,
                "prefix": "P2_",
                "offset": [0.0, 0.0, 2.0],
                "yaw_deg": 180.0
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let p2_root = p2["bone_ids"]["P2_Root"].as_u64().unwrap() as u32;

        // 1. character_snapshot_pose_to_keyframe with interpolation: "ease_out"
        execute_tool_call(
            "character_set_bone_pose",
            &json!({ "bone_id": p2_root, "rotation_deg": [-86.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_snapshot_pose_to_keyframe",
            &json!({
                "clip_name": "FallArc",
                "duration_secs": 1.0,
                "time_secs": 0.0,
                "bone_ids": [p2_root],
                "interpolation": "ease_out"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        // Reset root to upright at t=0 and fallen at t=1.0
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "FallArc",
                "bone_id": p2_root,
                "time_secs": 0.0,
                "rotation_deg": [0.0, 0.0, 0.0],
                "interpolation": "ease_out"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "FallArc",
                "bone_id": p2_root,
                "time_secs": 1.0,
                "rotation_deg": [-86.0, 0.0, 0.0],
                "interpolation": "linear"
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let fall_clip = scene
            .character_rig
            .as_ref()
            .unwrap()
            .animations
            .iter()
            .find(|c| c.name == "FallArc")
            .unwrap();
        let ch = fall_clip
            .channels
            .iter()
            .find(|c| c.bone_id == BoneId(p2_root))
            .unwrap();
        assert_eq!(
            ch.keyframes[0].interpolation,
            tessel_core::KeyframeInterpolation::EaseOut
        );

        // 2. Sub-frame floor clamp keeps P2 flush on y=0.0 even at mid-arc t=0.5s
        let clamped = execute_tool_call(
            "character_clamp_clip_to_floor",
            &json!({ "clip_name": "FallArc", "bone_prefix": "P2_", "floor_y": 0.0, "fps": 20 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(clamped["ok"], true);
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "FallArc", "time_secs": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let mats = scene.character_rig.as_ref().unwrap().world_pose_matrices();
        let mid_min_y = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bones
            .iter()
            .filter(|b| b.name.starts_with("P2_"))
            .map(|b| mats[&b.id].transform_point3(glam::Vec3::ZERO).y)
            .fold(f32::INFINITY, f32::min);
        assert!(
            mid_min_y.abs() < 0.02,
            "mid-arc min_y after sub-frame floor clamp should be ~0, got {mid_min_y}"
        );

        // 3. Bake multi-character clip_layers into 1 standalone clip ready for .glb
        execute_tool_call(
            "character_set_clip_layer",
            &json!({ "clip_name": "Walk", "bone_prefix": "" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_clip_layer",
            &json!({ "clip_name": "FallArc", "bone_prefix": "P2_" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let baked = execute_tool_call(
            "character_bake_clip_layers",
            &json!({ "name": "CombinedBattle", "fps": 12, "floor_y": 0.0, "clear_layers": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(baked["ok"], true);
        assert_eq!(baked["name"], "CombinedBattle");
        assert!(baked["channels"].as_u64().unwrap() >= 14);
        assert!(scene.character_rig.as_ref().unwrap().clip_layers.is_empty());

        // Verify .glb export includes the baked clip cleanly
        execute_tool_call(
            "scene_set_mode",
            &json!({ "mode": "character" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let (glb, _) =
            tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
        assert!(glb.starts_with(b"glTF"));
        let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
        let glb_json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(glb_json.contains("\"CombinedBattle\""));
    }

    #[test]
    fn v40_dynamic_parent_switch_weapon_drop_and_glb_export() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        execute_tool_call(
            "scene_set_mode",
            &json!({ "mode": "character" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rifle = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Rifle", "kind": "cube", "size": [0.1, 0.1, 0.8] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rifle_id = rifle["node_id"].as_u64().unwrap();
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let right_arm = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bones
            .iter()
            .find(|b| b.name == "RightArm")
            .unwrap()
            .id
            .0 as u64;

        // Hold rifle in RightArm at t=0.0, detach & drop to floor at t=0.5 -> t=1.0
        execute_tool_call(
            "character_set_parent_switch_keyframe",
            &json!({
                "clip_name": "WeaponDrop",
                "duration_secs": 1.0,
                "node_id": rifle_id,
                "time_secs": 0.0,
                "bone_id": right_arm
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_parent_switch_keyframe",
            &json!({
                "clip_name": "WeaponDrop",
                "node_id": rifle_id,
                "time_secs": 0.5,
                "bone_id": null,
                "position": [0.5, 0.8, 0.3],
                "rotation_deg": [0.0, 0.0, 0.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_parent_switch_keyframe",
            &json!({
                "clip_name": "WeaponDrop",
                "node_id": rifle_id,
                "time_secs": 1.0,
                "bone_id": null,
                "position": [0.8, 0.05, 0.6],
                "rotation_deg": [0.0, 45.0, 90.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // At t=0.25 rifle is still attached to RightArm
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "WeaponDrop", "time_secs": 0.25 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let ov_hold = scene
            .character_rig
            .as_ref()
            .unwrap()
            .node_parent_overrides
            .get(&NodeId(rifle_id))
            .copied()
            .unwrap();
        assert_eq!(ov_hold.bone_id, Some(BoneId(right_arm as u32)));

        // At t=0.75 rifle is detached and interpolated halfway along drop arc
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "WeaponDrop", "time_secs": 0.75 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let ov_drop = scene
            .character_rig
            .as_ref()
            .unwrap()
            .node_parent_overrides
            .get(&NodeId(rifle_id))
            .copied()
            .unwrap();
        assert_eq!(ov_drop.bone_id, None);
        assert!((ov_drop.position.x - 0.65).abs() < 1e-3);
        assert!((ov_drop.position.y - 0.425).abs() < 1e-3);

        // Export .glb includes WeaponDrop animation with translation + rotation channels
        let (glb, _) =
            tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
        assert!(glb.starts_with(b"glTF"));
        let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
        let glb_json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(glb_json.contains("\"WeaponDrop\""));
        assert!(glb_json.contains("\"translation\""));
        assert!(glb_json.contains("\"rotation\""));
    }

    #[test]
    fn floor_clamp_uses_lowest_skinned_mesh_vertex_below_foot_joint() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // Create a boot mesh whose sole sits at y = -0.06 (below humanoid foot joint).
        let boot = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "LeftBoot", "kind": "cube", "size": [0.18, 0.14, 0.28] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let boot_id = boot["node_id"].as_u64().unwrap();
        execute_tool_call(
            "scene_set_node_transform",
            &json!({ "node_id": boot_id, "position": [0.11, -0.06, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // 1. Live pose floor clamp lifts Root by +0.06 so boot sole sits at y = 0.0.
        let clamp = execute_tool_call(
            "character_clamp_to_floor",
            &json!({ "floor_y": 0.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(clamp["ok"], true);
        let min_y_before = clamp["min_y_before"].as_f64().unwrap() as f32;
        assert!(
            (min_y_before - (-0.06)).abs() < 1e-3,
            "expected skinned boot sole min_y_before = -0.06, got {min_y_before}"
        );
        let cache = tessel_core::SkinnedBoundsCache::from_scene(
            &scene,
            scene.character_rig.as_ref().unwrap(),
        )
        .unwrap();
        let min_y_after = cache
            .sub_skeleton_min_y(scene.character_rig.as_ref().unwrap(), "")
            .unwrap();
        assert!(
            min_y_after.abs() < 1e-3,
            "expected skinned boot sole min_y_after ~ 0.0, got {min_y_after}"
        );

        // 2. Clip floor clamp keeps skinned boot sole flush on y = 0.0 across Walk frames.
        let clip_clamp = execute_tool_call(
            "character_clamp_clip_to_floor",
            &json!({ "clip_name": "Walk", "floor_y": 0.0, "fps": 12 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(clip_clamp["ok"], true);
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "Walk", "time_secs": 0.25 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let walk_min_y = cache
            .sub_skeleton_min_y(scene.character_rig.as_ref().unwrap(), "")
            .unwrap();
        assert!(
            walk_min_y.abs() < 0.02,
            "expected clamped Walk skinned boot sole min_y ~ 0.0, got {walk_min_y}"
        );
    }

    #[test]
    fn mcp_quick_wins_batch_schema_double_sided_duplicate_bulk_keyframes_and_playing() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. scene_batch schema maxItems is synchronized to 512
        let batch_def = crate::tools::tool_definitions()
            .into_iter()
            .find(|t| t["name"] == "scene_batch")
            .unwrap();
        assert_eq!(
            batch_def["inputSchema"]["properties"]["calls"]["maxItems"],
            512
        );

        // 2. mesh_set_material supports double_sided and exports glTF doubleSided: true
        let grass = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "GrassBlade", "kind": "plane", "size": 0.4 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let grass_id = grass["node_id"].as_u64().unwrap();
        let mat_res = execute_tool_call(
            "mesh_set_material",
            &json!({ "node_id": grass_id, "double_sided": true, "roughness": 0.9 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mat_res["double_sided"], true);
        assert!(
            scene.lowpoly_meshes[&NodeId(grass_id)]
                .material
                .double_sided
        );

        // 3. scene_duplicate_node accepts position, rotation_deg, scale directly
        let dup = execute_tool_call(
            "scene_duplicate_node",
            &json!({
                "node_id": grass_id,
                "name": "GrassBlade_2",
                "position": [1.5, 0.0, -2.0],
                "rotation_deg": [0.0, 45.0, 0.0],
                "scale": [1.2, 1.5, 1.2]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let dup_id = NodeId(dup["node_id"].as_u64().unwrap());
        let dup_tr = scene.nodes[&dup_id].transform;
        assert!((dup_tr.translation - glam::Vec3::new(1.5, 0.0, -2.0)).length() < 1e-4);
        assert!((dup_tr.scale - glam::Vec3::new(1.2, 1.5, 1.2)).length() < 1e-4);

        // 4. character_set_keyframe supports bulk keyframes in 1 atomic call
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let bulk = execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "WindSway",
                "duration_secs": 2.0,
                "keyframes": [
                    { "bone_id": 1, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                    { "bone_id": 1, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, 12.0] },
                    { "bone_id": 2, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                    { "bone_id": 2, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, -8.0] }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(bulk["ok"], true);
        assert_eq!(bulk["count"], 4);

        // 5. character_set_playback_time supports playing: true/false even while in Low-Poly mode
        execute_tool_call(
            "scene_set_mode",
            &json!({ "mode": "lowpoly" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let play_res = execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "WindSway", "time_secs": 1.0, "playing": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(play_res["playing"], true);
        assert!(scene.character_rig.as_ref().unwrap().playing);
        let pause_res = execute_tool_call(
            "character_set_playback_time",
            &json!({ "playing": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(pause_res["playing"], false);
        assert!(!scene.character_rig.as_ref().unwrap().playing);
    }

    #[test]
    fn scene_group_and_per_node_transform_animation_without_bones_exports_valid_glb() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. Create an empty Group folder "Trees" at [2, 0, 0]
        let grp = execute_tool_call(
            "scene_create_group",
            &json!({ "name": "Trees", "position": [2.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let grp_id = grp["node_id"].as_u64().unwrap();
        assert_eq!(
            scene.nodes[&NodeId(grp_id)].kind,
            tessel_core::NodeKind::Group
        );

        // 2. Create a Pine mesh under "Trees" and a static Rock at origin
        let pine = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Pine_1", "kind": "cone", "size": 1.0, "position": [0.5, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let pine_id = pine["node_id"].as_u64().unwrap();
        execute_tool_call(
            "scene_set_parent",
            &json!({ "node_id": pine_id, "parent_node_id": grp_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rock = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Rock_Static", "kind": "cube", "size": 0.6 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rock_id = rock["node_id"].as_u64().unwrap();

        // 3. Animate the Group and Pine directly via scene_set_node_keyframe (0 bones!)
        let kf_res = execute_tool_call(
            "scene_set_node_keyframe",
            &json!({
                "clip_name": "WindBreeze",
                "duration_secs": 2.0,
                "keyframes": [
                    { "node_id": grp_id, "time_secs": 0.0, "translation": [0.0, 0.0, 0.0] },
                    { "node_id": grp_id, "time_secs": 1.0, "translation": [0.0, 0.5, 0.0] },
                    { "node_id": pine_id, "time_secs": 0.0, "rotation_deg": [0.0, 0.0, 0.0] },
                    { "node_id": pine_id, "time_secs": 1.0, "rotation_deg": [0.0, 0.0, 15.0] }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(kf_res["ok"], true);
        assert_eq!(kf_res["count"], 4);
        assert!(scene.character_rig.as_ref().unwrap().bones.is_empty());

        // 4. Scrub to t=1.0: Pine world Y lifts by +0.5 from parent Group; Rock stays at Y=0
        execute_tool_call(
            "character_set_playback_time",
            &json!({ "clip_name": "WindBreeze", "time_secs": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let pine_world = scene
            .node_world_matrix(NodeId(pine_id))
            .transform_point3(glam::Vec3::ZERO);
        assert!((pine_world.x - 2.5).abs() < 1e-3);
        assert!((pine_world.y - 0.5).abs() < 1e-3);
        let rock_world = scene
            .node_world_matrix(NodeId(rock_id))
            .transform_point3(glam::Vec3::ZERO);
        assert!(rock_world.length() < 1e-4);

        // 5. Export .glb and validate Group hierarchy + WindBreeze animation channels
        let (glb, _) =
            tessel_io::export_glb_bytes(&scene, tessel_io::ExportPreset::Godot4).unwrap();
        assert!(glb.starts_with(b"glTF"));
        let json_len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
        let glb_json: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let nodes = glb_json["nodes"].as_array().unwrap();
        let trees_node = nodes.iter().find(|n| n["name"] == "Trees").unwrap();
        let pine_idx = nodes.iter().position(|n| n["name"] == "Pine_1").unwrap();
        let trees_children = trees_node["children"].as_array().unwrap();
        assert!(
            trees_children
                .iter()
                .any(|c| c.as_u64() == Some(pine_idx as u64)),
            "Trees group must parent Pine_1 in glTF"
        );
        let anims = glb_json["animations"].as_array().unwrap();
        let wind = anims.iter().find(|a| a["name"] == "WindBreeze").unwrap();
        assert!(wind["channels"].as_array().unwrap().len() >= 6);
    }

    #[test]
    fn v41_terrain_generator_and_surface_raycast_grounding_with_foot_ik() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();

        // 1. Procedural fBm terrain with palette bands
        let island = execute_tool_call(
            "mesh_create_terrain",
            &json!({
                "name": "IslandDiorama",
                "width": 8.0,
                "depth": 8.0,
                "subdiv_x": 8,
                "subdiv_z": 8,
                "height_scale": 2.0,
                "seed": 7,
                "island_falloff": 0.6,
                "palette_bands": [2, 3, 4]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(island["ok"], true);
        assert_eq!(island["vertices"], 81);
        assert_eq!(island["triangles"], 128);

        // 2. Explicit sloped ramp terrain: y rises linearly from 0.5 at x=-2 to 2.5 at x=+2
        // (subdiv_x=2, subdiv_z=2 -> 3x3 grid; at x=0 y=1.5, at x=+0.11 y=1.555, at x=-0.11 y=1.445)
        let ramp = execute_tool_call(
            "mesh_create_terrain",
            &json!({
                "name": "SlopedRamp",
                "width": 4.0,
                "depth": 4.0,
                "subdiv_x": 2,
                "subdiv_z": 2,
                "height_scale": 1.0,
                "heights": [
                    0.5, 1.5, 2.5,
                    0.5, 1.5, 2.5,
                    0.5, 1.5, 2.5
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let ramp_id = ramp["node_id"].as_u64().unwrap();

        // 3. Apply humanoid rig at origin and ground onto SlopedRamp with solve_foot_ik=true
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let grounded = execute_tool_call(
            "character_clamp_to_floor",
            &json!({
                "surface_node_id": ramp_id,
                "solve_foot_ik": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(grounded["ok"], true);
        let floor_y = grounded["floor_y"].as_f64().unwrap() as f32;
        assert!(
            floor_y > 1.3 && floor_y < 1.6,
            "expected character grounded onto sloped ramp near y ~ 1.445, got {floor_y}"
        );

        // Verify uphill LeftFoot (+X) was raised higher than downhill RightFoot (-X) by leg IK
        let rig = scene.character_rig.as_ref().unwrap();
        let mats = rig.world_pose_matrices();
        let lf = rig.bones.iter().find(|b| b.name == "LeftFoot").unwrap().id;
        let rf = rig.bones.iter().find(|b| b.name == "RightFoot").unwrap().id;
        let ly = mats[&lf].transform_point3(glam::Vec3::ZERO).y;
        let ry = mats[&rf].transform_point3(glam::Vec3::ZERO).y;
        assert!(
            ly > ry + 0.05,
            "expected uphill LeftFoot ({ly}) higher than downhill RightFoot ({ry}) via leg IK"
        );
    }

    #[test]
    fn mesh_bake_maps_bakes_attaches_ao_and_exports_pngs() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v42_bake_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Crate", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = NodeId(created["node_id"].as_u64().unwrap());

        let res = execute_tool_call(
            "mesh_bake_maps",
            &json!({
                "node_id": node_id.0,
                "maps": ["ao", "curvature", "thickness"],
                "resolution": 32,
                "samples": 8,
                "dilation_px": 2,
                "output_prefix": "crate_baked",
                "attach_ao": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["ao_attached"], true);
        assert!(res["covered_texels"].as_u64().unwrap() > 0);
        assert!(scene.lowpoly_meshes[&node_id].ao_map.is_some());
        assert!(dir.join("crate_baked_ao.png").exists());
        assert!(dir.join("crate_baked_curvature.png").exists());
        assert!(dir.join("crate_baked_thickness.png").exists());

        // Undo detaches the AO map cleanly.
        history.undo(&mut scene).unwrap();
        assert!(scene.lowpoly_meshes[&node_id].ao_map.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mesh_paint_spot_across_seams_with_camera_culling_and_undo() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v43_paint_seams_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "SeamCube", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = NodeId(created["node_id"].as_u64().unwrap());

        // Paint right on the +X (x=0.5) / +Y (y=1.0) seam edge with camera visibility culling enabled.
        let res = execute_tool_call(
            "mesh_paint_spot",
            &json!({
                "node_id": node_id.0,
                "point": [0.5, 1.0, 0.0],
                "radius_world": 0.25,
                "across_seams": true,
                "camera_pos": [3.0, 3.0, 0.0],
                "palette_index": 5
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["across_seams"], true);
        assert_eq!(res["camera_culled"], true);

        let mesh = &scene.lowpoly_meshes[&node_id];
        let tex = mesh.texture.as_ref().expect("texture created by 3D paint");
        let (min_b, max_b) = mesh.bounds();
        let expected_color = scene
            .palette
            .get(PaletteIndex::new(5).unwrap())
            .unwrap()
            .color;
        let expected_px = [
            expected_color.r,
            expected_color.g,
            expected_color.b,
            expected_color.a,
        ];
        let sample = |uv: [f32; 2]| -> [u8; 4] {
            let px = ((uv[0].clamp(0.0, 1.0) * (tex.width - 1) as f32).round() as usize)
                .min(tex.width as usize - 1);
            let py = ((uv[1].clamp(0.0, 1.0) * (tex.height - 1) as f32).round() as usize)
                .min(tex.height as usize - 1);
            let off = (py * tex.width as usize + px) * 4;
            [
                tex.rgba[off],
                tex.rgba[off + 1],
                tex.rgba[off + 2],
                tex.rgba[off + 3],
            ]
        };
        let uv_x =
            tessel_core::box_atlas_uv(glam::Vec3::new(0.5, 0.92, 0.0), glam::Vec3::X, min_b, max_b);
        let uv_y =
            tessel_core::box_atlas_uv(glam::Vec3::new(0.42, 1.0, 0.0), glam::Vec3::Y, min_b, max_b);
        let uv_back = tessel_core::box_atlas_uv(
            glam::Vec3::new(-0.5, 0.92, 0.0),
            -glam::Vec3::X,
            min_b,
            max_b,
        );
        assert_eq!(sample(uv_x), expected_px);
        assert_eq!(sample(uv_y), expected_px);
        assert_ne!(sample(uv_back), expected_px);

        // Undo removes the newly created texture cleanly.
        history.undo(&mut scene).unwrap();
        assert!(scene.lowpoly_meshes[&node_id].texture.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mesh_fill_smart_material_generates_texture_attaches_ao_and_undoes() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v44_smart_mat_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "WornCrate", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = NodeId(created["node_id"].as_u64().unwrap());

        let res = execute_tool_call(
            "mesh_fill_smart_material",
            &json!({
                "node_id": node_id.0,
                "base_palette_index": 3,
                "edge_strength": 0.7,
                "cavity_strength": 0.6,
                "resolution": 32,
                "samples": 8,
                "attach_ao": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["width"], 32);
        assert_eq!(res["ao_attached"], true);
        assert!(res["covered_texels"].as_u64().unwrap() > 0);
        assert!(scene.lowpoly_meshes[&node_id].texture.is_some());
        assert!(scene.lowpoly_meshes[&node_id].ao_map.is_some());

        // Single undo reverts both texture and ao_map atomically.
        history.undo(&mut scene).unwrap();
        assert!(scene.lowpoly_meshes[&node_id].texture.is_none());
        assert!(scene.lowpoly_meshes[&node_id].ao_map.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mesh_paint_layers_non_destructive_workflow_and_undo() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v45_paint_layers_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let created = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "LayeredCube", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = NodeId(created["node_id"].as_u64().unwrap());

        // Base solid fill (palette 3), then add transparent Decal layer at 50% opacity.
        execute_tool_call(
            "mesh_fill_texture",
            &json!({ "node_id": node_id.0, "palette_index": 3 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let added = execute_tool_call(
            "mesh_layer_add",
            &json!({ "node_id": node_id.0, "name": "Decal", "opacity": 0.5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(added["layer_count"], 2);
        let decal_id = added["layer_id"].as_u64().unwrap();

        // Paint palette 5 on the Decal layer at uv [0.5, 0.5].
        execute_tool_call(
            "mesh_paint_spot",
            &json!({
                "node_id": node_id.0,
                "uv": [0.5, 0.5],
                "radius_px": 12.0,
                "palette_index": 5
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        let base_c = scene
            .palette
            .get(PaletteIndex::new(3).unwrap())
            .unwrap()
            .color;
        let decal_c = scene
            .palette
            .get(PaletteIndex::new(5).unwrap())
            .unwrap()
            .color;
        let sample_center = |s: &SceneState| -> [u8; 4] {
            let tex = s.lowpoly_meshes[&node_id].texture.as_ref().unwrap();
            let px = (tex.width as usize) / 2;
            let py = (tex.height as usize) / 2;
            let off = (py * tex.width as usize + px) * 4;
            [
                tex.rgba[off],
                tex.rgba[off + 1],
                tex.rgba[off + 2],
                tex.rgba[off + 3],
            ]
        };

        // At 50% opacity, composited pixel is blended between base_c and decal_c.
        let blended = sample_center(&scene);
        assert_ne!(blended, [base_c.r, base_c.g, base_c.b, base_c.a]);
        assert_ne!(blended, [decal_c.r, decal_c.g, decal_c.b, decal_c.a]);

        // Hide Decal layer -> composited pixel reverts cleanly to base_c.
        execute_tool_call(
            "mesh_layer_set",
            &json!({ "node_id": node_id.0, "layer_id": decal_id, "visible": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(
            sample_center(&scene),
            [base_c.r, base_c.g, base_c.b, base_c.a]
        );

        // List reports both layers.
        let listed = execute_tool_call(
            "mesh_layer_list",
            &json!({ "node_id": node_id.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(listed["layers"].as_array().unwrap().len(), 2);

        // Delete Decal layer, then undo restores it.
        let del = execute_tool_call(
            "mesh_layer_delete",
            &json!({ "node_id": node_id.0, "layer_id": decal_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(del["layer_count"], 1);
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.lowpoly_meshes[&node_id].paint_layers.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn v49_grounding_v2_multi_surface_overhang_and_foot_normal() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_v49_grounding_v2_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Sloped ramp at y ~ 0.0..1.0 + overhead bridge at y = 3.0
        let ramp = execute_tool_call(
            "mesh_create_terrain",
            &json!({
                "name": "Slope",
                "width": 4.0,
                "depth": 4.0,
                "subdiv_x": 2,
                "subdiv_z": 2,
                "heights": [0.0, 0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.0]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let ramp_id = ramp["node_id"].as_u64().unwrap();

        let bridge = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Bridge", "kind": "plane", "size": 6.0, "position": [0.0, 3.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let bridge_id = bridge["node_id"].as_u64().unwrap();

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        let res = execute_tool_call(
            "character_clamp_to_floor",
            &json!({
                "surface_node_ids": [ramp_id, bridge_id],
                "solve_foot_ik": true,
                "align_foot_normal": true,
                "pelvis_tilt": true,
                "step_up_clearance": 1.0
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["align_foot_normal"], true);
        let floor_y = res["floor_y"].as_f64().unwrap() as f32;
        // Must land on the lower sloped ramp (~0.45), NOT snap onto the y=3.0 bridge!
        assert!(
            floor_y < 1.2,
            "character under y=3.0 bridge must stay on lower ramp, got {floor_y}"
        );
        // LeftFoot rotation should be tilted to align with the sloped normal.
        let rig = scene.character_rig.as_ref().unwrap();
        let lf = rig.bones.iter().find(|b| b.name == "LeftFoot").unwrap();
        assert!(
            lf.pose_local.rotation.angle_between(glam::Quat::IDENTITY) > 0.05,
            "expected LeftFoot tilted to match slope normal"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn v5_1_to_v5_3_mcp_tools_chain_ik_stepped_npr_and_shading_mode() {
        let dir = std::env::temp_dir().join("tessel_v5_1_to_5_3_mcp_test");
        let _ = std::fs::create_dir_all(&dir);
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // v5.2: vertex colors, crease edges, topology validator, alpha_mode + unlit
        let cube = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "InkCube", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = cube["node_id"].as_u64().unwrap();

        let vc = execute_tool_call(
            "mesh_set_vertex_colors",
            &json!({ "node_id": node_id, "fill": [0.7, 0.5, 0.9, 1.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(vc["ok"], true);
        assert!(vc["updated_vertices"].as_u64().unwrap() >= 8);

        let ce = execute_tool_call(
            "mesh_set_crease_edges",
            &json!({ "node_id": node_id, "edges": [[0, 1], [1, 2]] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(ce["crease_edge_count"], 2);

        let mat = execute_tool_call(
            "mesh_set_material",
            &json!({
                "node_id": node_id,
                "alpha_mode": "mask",
                "alpha_cutoff": 0.4,
                "unlit": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mat["unlit"], true);

        let topo = execute_tool_call(
            "mesh_validate_topology",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(topo["report"]["is_clean"], true);

        // v5.3: viewport_set_shading_mode
        let mode = execute_tool_call(
            "viewport_set_shading_mode",
            &json!({ "mode": "npr_sumie" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mode["ok"], true);
        assert_eq!(
            scene.shading_mode,
            tessel_core::ViewportShadingMode::NprSumie
        );

        // v5.1: chain IK + stepped FPS
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let chain = execute_tool_call(
            "character_solve_chain_ik",
            &json!({
                "bone_ids": [0, 1, 2, 3],
                "target": [0.2, 1.5, 0.3],
                "max_iterations": 16
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(chain["ok"], true);

        let stepped = execute_tool_call(
            "character_set_clip_stepped_fps",
            &json!({ "clip_name": "Idle", "stepped_fps": 12.0, "bake_stepped": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(stepped["ok"], true);
        assert_eq!(stepped["baked"], true);

        // Audit regressions:
        // 1. mesh_set_material updates alpha_cutoff without resending alpha_mode
        let mat2 = execute_tool_call(
            "mesh_set_material",
            &json!({ "node_id": node_id, "alpha_cutoff": 0.72 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(mat2["alpha_mode"]["mask"]["cutoff"], 0.72_f32 as f64);

        // 2. SetMeshVertexColors is atomic on error (no partial mutation)
        let before_vc = scene.lowpoly_meshes[&tessel_core::NodeId(node_id)]
            .vertex_colors
            .clone();
        assert!(execute_tool_call(
            "mesh_set_vertex_colors",
            &json!({
                "node_id": node_id,
                "updates": [
                    { "vertex_id": 0, "rgba": [0.1, 0.2, 0.3, 1.0] },
                    { "vertex_id": 9999, "rgba": [1.0, 1.0, 1.0, 1.0] }
                ]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());
        assert_eq!(
            scene.lowpoly_meshes[&tessel_core::NodeId(node_id)].vertex_colors,
            before_vc
        );

        // 3. cubic_bezier rejects malformed bezier array
        assert!(execute_tool_call(
            "character_set_keyframe",
            &json!({
                "clip_name": "Idle",
                "bone_id": 1,
                "time_secs": 0.5,
                "rotation_deg": [10.0, 0.0, 0.0],
                "interpolation": "cubic_bezier",
                "bezier": [0.25, 0.1]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pixel_tools_create_layer_draw_animate_and_export() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_pixel_tools_test");
        let _ = std::fs::create_dir_all(&dir);

        // 1. Create a 32x32 canvas with one transparent layer.
        let created = execute_tool_call(
            "pixel_create_canvas",
            &json!({ "name": "Ronin Fox", "width": 32, "height": 32 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let node_id = created["node_id"].as_u64().unwrap();
        assert_eq!(created["width"].as_u64().unwrap(), 32);
        assert_eq!(created["layers"].as_array().unwrap().len(), 1);

        // 2. Oversized canvas is rejected before allocation.
        assert!(execute_tool_call(
            "pixel_create_canvas",
            &json!({ "width": 513, "height": 32 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // 3. Add two layers (ground + scarf) on top of the base layer.
        let ground = execute_tool_call(
            "pixel_add_layer",
            &json!({ "node_id": node_id, "name": "Ground & Glow" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let scarf = execute_tool_call(
            "pixel_add_layer",
            &json!({ "node_id": node_id, "name": "Scarf & Katana" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_ne!(
            ground["layer_id"].as_u64().unwrap(),
            scarf["layer_id"].as_u64().unwrap()
        );

        // 4. Set palette + paint one pixel per layer, erase with null.
        execute_tool_call(
            "palette_set_color",
            &json!({ "palette_index": 5, "hex": "#C8102E" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let drawn = execute_tool_call(
            "pixel_draw",
            &json!({
                "node_id": node_id,
                "layer_id": scarf["layer_id"],
                "pixels": [{ "x": 16, "y": 16, "palette_index": 5 }]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(drawn["pixels_painted"].as_u64().unwrap(), 1);

        // 5. Out-of-bounds pixels are rejected, nothing is painted.
        assert!(execute_tool_call(
            "pixel_draw",
            &json!({
                "node_id": node_id,
                "pixels": [{ "x": 32, "y": 0, "palette_index": 5 }]
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .is_err());

        // 6. Append a frame, switch active frame, set fps, inspect.
        let framed = execute_tool_call(
            "pixel_add_frame",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(framed["frame"].as_u64().unwrap(), 1);
        execute_tool_call(
            "pixel_set_active_frame",
            &json!({ "node_id": node_id, "frame": 0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "pixel_set_fps",
            &json!({ "node_id": node_id, "fps": 12 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let inspected = execute_tool_call(
            "pixel_inspect",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(inspected["frame_count"].as_u64().unwrap(), 2);
        assert_eq!(inspected["fps"].as_u64().unwrap(), 12);
        assert_eq!(inspected["active_frame"].as_u64().unwrap(), 0);

        // 7. Undo restores default fps, redo reapplies it (single command bus).
        history.undo(&mut scene).unwrap();
        let after_undo = execute_tool_call(
            "pixel_inspect",
            &json!({ "node_id": node_id }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(after_undo["fps"].as_u64().unwrap(), 8);
        history.redo(&mut scene).unwrap();
        execute_tool_call(
            "pixel_set_fps",
            &json!({ "node_id": node_id, "fps": 8 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();

        // 8. Export animated GIF + sprite sheet through the export dir.
        let gif = execute_tool_call(
            "project_export",
            &json!({ "filename": "ronin_fox.gif", "format": "gif" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(gif["file_size_bytes"].as_u64().unwrap() > 0);
        let sheet = execute_tool_call(
            "pixel_export_sprite_sheet",
            &json!({ "node_id": node_id, "filename": "ronin_fox_sheet.png" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(sheet["sheet_width"].as_u64().unwrap(), 64);
        assert_eq!(sheet["sheet_height"].as_u64().unwrap(), 32);
        assert!(dir.join("ronin_fox.gif").exists());
        assert!(dir.join("ronin_fox_sheet.png").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pixel_and_voxel_mcp_parity_tools_and_non_mutating_inspect() {
        let dir = std::env::temp_dir().join("tessel_pixel_voxel_mcp_parity_test");
        let _ = std::fs::create_dir_all(&dir);
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // Empty scene: read-only inspect & convert MUST error without mutating scene!
        let rev_before = scene.revision;
        assert!(
            execute_tool_call("pixel_inspect", &json!({}), &mut scene, &mut history, &dir).is_err()
        );
        assert!(
            execute_tool_call("voxel_inspect", &json!({}), &mut scene, &mut history, &dir).is_err()
        );
        assert_eq!(scene.revision, rev_before);

        // Pixel full workflow: create, draw_line + mirror_x, draw_rect, flood_fill, set_layer, onion_skin, convert_to_voxel
        let pc = execute_tool_call(
            "pixel_create_canvas",
            &json!({ "name": "HeroSprite", "width": 16, "height": 16 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let pid = pc["node_id"].as_u64().unwrap();
        execute_tool_call(
            "pixel_draw_line",
            &json!({ "node_id": pid, "start": [2, 2], "end": [2, 10], "palette_index": 3, "mirror_x": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "pixel_draw_rect",
            &json!({ "node_id": pid, "min": [4, 4], "max": [11, 11], "palette_index": 4, "filled": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let ff = execute_tool_call(
            "pixel_flood_fill",
            &json!({ "node_id": pid, "x": 6, "y": 6, "palette_index": 5 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(ff["pixels_filled"].as_u64().unwrap() > 0);

        execute_tool_call(
            "pixel_set_layer",
            &json!({ "node_id": pid, "layer_id": 1, "visible": true, "opacity": 0.75 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "pixel_set_onion_skin",
            &json!({ "node_id": pid, "enabled": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let pinsp = execute_tool_call(
            "pixel_inspect",
            &json!({ "node_id": pid }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(pinsp["onion_skin"], true);
        assert_eq!(pinsp["frames"][0]["layers"][0]["opacity"], 0.75);

        let pvox = execute_tool_call(
            "pixel_convert_to_voxel",
            &json!({ "node_id": pid, "depth": 3, "name": "SpriteVoxel" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(pvox["new_node_id"].as_u64().is_some());

        // Voxel full workflow: create_volume, set_voxels + mirror_x, fill_box carve (null), inspect, export+import .vox
        let vv = execute_tool_call(
            "voxel_create_volume",
            &json!({ "name": "CarvedBlock" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let vid = vv["node_id"].as_u64().unwrap();
        execute_tool_call(
            "voxel_fill_box",
            &json!({ "node_id": vid, "min": [0, 0, 0], "max": [2, 2, 2], "palette_index": 2 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "voxel_fill_box",
            &json!({ "node_id": vid, "min": [1, 1, 1], "max": [1, 1, 1], "palette_index": null }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let sv = execute_tool_call(
            "voxel_set_voxels",
            &json!({ "node_id": vid, "voxels": [{ "x": 4, "y": 0, "z": 0, "palette_index": 6 }], "mirror_x": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(sv["voxels_modified"], 2);
        let vinsp = execute_tool_call(
            "voxel_inspect",
            &json!({ "node_id": vid }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(vinsp["voxel_count"], 28); // 27 - 1 carved + 2 mirrored

        // Project/Low-Poly/Animation new MCP tools
        execute_tool_call(
            "scene_select_node",
            &json!({ "node_id": vid }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(scene.selected_node, Some(NodeId(vid)));

        let pal = execute_tool_call(
            "palette_set_color",
            &json!({ "palette_index": 9, "hex": "#FF4422CC", "roughness": 0.4, "metallic": 0.2 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(pal["alpha"], 204);

        let cube = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "ProxyCube", "kind": "cube", "size": 1.0 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let cid = cube["node_id"].as_u64().unwrap();
        let pn = execute_tool_call(
            "mesh_set_proxy_normals",
            &json!({ "node_id": cid, "normals": [{ "vertex_id": 0, "normal": [0.0, 1.0, 0.0] }] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(pn["updated_normals"], 1);

        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let cc = execute_tool_call(
            "character_create_clip",
            &json!({ "name": "Slash", "duration_secs": 1.2 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let cidx = cc["clip_index"].as_u64().unwrap();
        let uc = execute_tool_call(
            "character_update_clip",
            &json!({ "clip_index": cidx, "name": "HeavySlash", "duration_secs": 0.9 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(uc["name"], "HeavySlash");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn b2_pin_primitive_bottom_anchor_bone_rotation_sign_and_png_export_mode() {
        let dir = std::env::temp_dir().join("tessel_b2_quirk_pin_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // 1. Primitive bottom-anchoring: y in [0, size_y], centered in X/Z.
        for kind in ["cube", "cylinder", "sphere", "cone", "capsule"] {
            let res = execute_tool_call(
                "mesh_create_primitive",
                &json!({ "name": format!("Pin_{kind}"), "kind": kind, "size": [0.4, 1.2, 0.6] }),
                &mut scene,
                &mut history,
                &dir,
            )
            .unwrap();
            let nid = NodeId(res["node_id"].as_u64().unwrap());
            let (bmin, bmax) = scene.lowpoly_meshes[&nid].bounds();
            assert!(
                bmin.y.abs() < 1e-4,
                "{kind} min.y must be 0.0, got {}",
                bmin.y
            );
            assert!(
                (bmax.y - 1.2).abs() < 1e-4,
                "{kind} max.y must be 1.2, got {}",
                bmax.y
            );
            assert!((bmin.x + bmax.x).abs() < 1e-4, "{kind} X must be centered");
            assert!((bmin.z + bmax.z).abs() < 1e-4, "{kind} Z must be centered");
        }

        // 2. Bone rotation sign: Right-Handed +Y Up, +Z Front, +X Left.
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "character_set_bone_pose",
            &json!({ "bone_id": 2, "rotation_deg": [45.0, 0.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let pitch_rot = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(BoneId(2))
            .unwrap()
            .pose_local
            .rotation;
        assert!(
            (pitch_rot * glam::Vec3::Y).z > 0.5,
            "+X pitch must tilt +Y forward (+Z)"
        );

        execute_tool_call(
            "character_set_bone_pose",
            &json!({ "bone_id": 2, "rotation_deg": [0.0, 45.0, 0.0] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let yaw_rot = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(BoneId(2))
            .unwrap()
            .pose_local
            .rotation;
        assert!(
            (yaw_rot * glam::Vec3::Z).x > 0.5,
            "+Y yaw must turn +Z toward +X (Left)"
        );

        // 3. Mode-dependent PNG export: LowPoly mode rejects PNG (requires PixelCanvas), allows GLB.
        let png_err = execute_tool_call(
            "project_export",
            &json!({ "filename": "should_fail.png", "format": "png" }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(png_err.is_err(), "PNG export in LowPoly mode must error");

        execute_tool_call(
            "pixel_create_canvas",
            &json!({ "name": "PinSprite", "width": 8, "height": 8 }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        execute_tool_call(
            "pixel_draw",
            &json!({ "pixels": [{ "x": 0, "y": 0, "palette_index": 4 }] }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let png_ok = execute_tool_call(
            "project_export",
            &json!({ "filename": "pixel_ok.png", "format": "png" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert!(png_ok.is_object());
        let bytes = std::fs::read(dir.join("pixel_ok.png")).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn b3_explicit_primitive_anchor_and_dry_run_previews() {
        let dir = std::env::temp_dir().join("tessel_b3_anchor_dry_run_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // 1. Explicit anchor='center' and 'top' on scalar and non-uniform primitives.
        let center = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "CenteredCube", "kind": "cube", "size": 2.0, "anchor": "center" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(center["anchor"], "center");
        let cid = NodeId(center["node_id"].as_u64().unwrap());
        let (clo, chi) = scene.lowpoly_meshes[&cid].bounds();
        assert!((clo.y + 1.0).abs() < 1e-4 && (chi.y - 1.0).abs() < 1e-4);

        let top = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "HangingCyl", "kind": "cylinder", "size": [0.4, 1.5, 0.4], "anchor": "top" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(top["anchor"], "top");
        let tid = NodeId(top["node_id"].as_u64().unwrap());
        let (tlo, thi) = scene.lowpoly_meshes[&tid].bounds();
        assert!((tlo.y + 1.5).abs() < 1e-4 && thi.y.abs() < 1e-4);

        let bad_anchor = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "Bad", "kind": "cube", "anchor": "middle" }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(bad_anchor.is_err());
        let non_str_anchor = execute_tool_call(
            "mesh_create_primitive",
            &json!({ "name": "BadType", "kind": "cube", "anchor": 123 }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(non_str_anchor.is_err());

        // Non-boolean dry_run must fail closed without mutating scene
        let rev_guard = scene.revision;
        let bad_dry = execute_tool_call(
            "scene_set_node_transform",
            &json!({ "node_id": cid.0, "position": [9.0, 9.0, 9.0], "dry_run": "true" }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(bad_dry.is_err());
        assert_eq!(scene.revision, rev_guard);

        // 2. scene_set_node_transform dry_run previews world position/bounds without mutating state.
        let rev_before = scene.revision;
        let transform_before = scene.nodes[&cid].transform;
        let tx_preview = execute_tool_call(
            "scene_set_node_transform",
            &json!({
                "node_id": cid.0,
                "position": [1.0, 2.0, 3.0],
                "rotation_deg": [0.0, 90.0, 0.0],
                "dry_run": true
            }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(tx_preview["dry_run"], true);
        assert_eq!(tx_preview["world_position"], json!([1.0, 2.0, 3.0]));
        assert!(tx_preview["world_forward_dir"][0].as_f64().unwrap() > 0.99);
        assert!(
            (tx_preview["world_bounds"]["min"][1].as_f64().unwrap() - 1.0).abs() < 1e-4
                && (tx_preview["world_bounds"]["max"][1].as_f64().unwrap() - 3.0).abs() < 1e-4
        );
        assert_eq!(scene.revision, rev_before);
        assert_eq!(scene.nodes[&cid].transform, transform_before);

        // 3. character_set_bone_pose dry_run previews bone direction without mutating rig or undo stack.
        execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        let rig_rev_before = scene.revision;
        let pose_before = scene
            .character_rig
            .as_ref()
            .unwrap()
            .bone(BoneId(2))
            .unwrap()
            .pose_local;
        let bone_preview = execute_tool_call(
            "character_set_bone_pose",
            &json!({ "bone_id": 2, "rotation_deg": [45.0, 0.0, 0.0], "dry_run": true }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(bone_preview["dry_run"], true);
        assert!(bone_preview["local_up_dir"][2].as_f64().unwrap() > 0.5);
        assert_eq!(scene.revision, rig_rev_before);
        assert_eq!(
            scene
                .character_rig
                .as_ref()
                .unwrap()
                .bone(BoneId(2))
                .unwrap()
                .pose_local,
            pose_before
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn character_apply_biped_rig_respects_include_default_clips_false_without_append() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir().join("tessel_test_rig_no_default_clips");
        let _ = std::fs::create_dir_all(&dir);

        // Starter preset with include_default_clips: false -> 7 bones, 0 clips
        let res = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter", "include_default_clips": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["bones"], 7);
        assert_eq!(res["clips"], 0);
        assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 0);

        // Single undo restores no-rig state cleanly
        history.undo(&mut scene).unwrap();
        assert!(scene.character_rig.is_none());

        // Humanoid preset with include_default_clips: false -> 19 bones, 0 clips
        let res2 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "humanoid", "include_default_clips": false }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res2["bones"], 19);
        assert_eq!(res2["clips"], 0);
        assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 0);

        // Default (omitted) still includes Idle & Walk
        let res3 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter" }),
            &mut scene,
            &mut history,
            &dir,
        )
        .unwrap();
        assert_eq!(res3["clips"], 2);
        assert_eq!(scene.character_rig.as_ref().unwrap().animations.len(), 2);

        // Non-boolean include_default_clips is rejected fail-closed
        let bad = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter", "include_default_clips": "false" }),
            &mut scene,
            &mut history,
            &dir,
        );
        assert!(bad.is_err());

        // append=true + include_default_clips=false on an empty scene is atomic and leaves 0 clips
        let mut empty_scene = SceneState::new();
        let mut empty_history = HistoryStack::new();
        let res4 = execute_tool_call(
            "character_apply_biped_rig",
            &json!({ "preset": "starter", "append": true, "include_default_clips": false }),
            &mut empty_scene,
            &mut empty_history,
            &dir,
        )
        .unwrap();
        assert_eq!(res4["ok"], true);
        assert_eq!(
            empty_scene.character_rig.as_ref().unwrap().animations.len(),
            0
        );
        empty_history.undo(&mut empty_scene).unwrap();
        assert!(empty_scene.character_rig.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
