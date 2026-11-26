#![allow(dead_code)]
use std::path::Path;

use serde_json::{json, Value};
use tessel_core::{BoneId, Command, HistoryStack, NodeId, PaletteIndex, SceneState, VoxelCoord};

use crate::error::AiError;

pub(super) const MAX_COORD_ABS: i32 = 256;
pub(super) const MAX_BATCH_VOXELS: usize = 50_000;
pub(super) const MAX_RAW_VERTICES: usize = 65_536;
pub(super) const MAX_RAW_FACES: usize = 65_536;
pub(super) const MAX_VERTEX_EDIT_BATCH: usize = 65_536;
pub(super) const MAX_RIBBON_POINTS: usize = 2_048;

pub(super) fn parse_opt_bool_default(
    args: &Value,
    field: &str,
    default: bool,
) -> Result<bool, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(AiError::InvalidArguments(format!(
            "'{field}' must be a boolean"
        ))),
    }
}

pub(super) fn parse_opt_f32_finite(
    args: &Value,
    field: &str,
    default: f32,
) -> Result<f32, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v
                .as_f64()
                .ok_or_else(|| AiError::InvalidArguments(format!("'{field}' must be a number")))?
                as f32;
            if !n.is_finite() {
                return Err(AiError::InvalidArguments(format!(
                    "'{field}' must be finite"
                )));
            }
            Ok(n)
        }
    }
}

pub(super) fn parse_opt_u32(args: &Value, field: &str, default: u32) -> Result<u32, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| {
                AiError::InvalidArguments(format!("'{field}' must be a non-negative integer"))
            })?;
            u32::try_from(raw)
                .map_err(|_| AiError::InvalidArguments(format!("'{field}' out of u32 range")))
        }
    }
}

pub(super) fn parse_opt_usize(args: &Value, field: &str, default: usize) -> Result<usize, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| {
                AiError::InvalidArguments(format!("'{field}' must be a non-negative integer"))
            })?;
            usize::try_from(raw)
                .map_err(|_| AiError::InvalidArguments(format!("'{field}' out of usize range")))
        }
    }
}

pub(super) fn bone_world_rest_transform(
    rig: &tessel_core::CharacterRig,
    bone_id: BoneId,
) -> Option<(glam::Vec3, glam::Quat)> {
    let mut chain = Vec::new();
    let mut cur = Some(bone_id);
    while let Some(id) = cur {
        if chain.len() > rig.bones.len() {
            return None;
        }
        let b = rig.bones.iter().find(|b| b.id == id)?;
        chain.push(b);
        cur = b.parent;
    }
    let mut world_pos = glam::Vec3::ZERO;
    let mut world_rot = glam::Quat::IDENTITY;
    for b in chain.into_iter().rev() {
        world_pos += world_rot * b.rest_local.translation;
        world_rot = (world_rot * b.rest_local.rotation).normalize();
    }
    Some((world_pos, world_rot))
}

pub(super) fn is_disk_write_tool(name: &str) -> bool {
    matches!(
        name,
        "project_save"
            | "project_export"
            | "mesh_bake_vat"
            | "mesh_bake_maps"
            | "pixel_export_sprite_sheet"
            | "viewport_render_turntable_gif"
    )
}

/// Returns a canonical (or absolute fallback) display string for the sandboxed export directory.
pub(crate) fn canonical_export_dir_string(allowed_export_dir: &Path) -> String {
    std::fs::canonicalize(allowed_export_dir)
        .or_else(|_| std::path::absolute(allowed_export_dir))
        .unwrap_or_else(|_| allowed_export_dir.to_path_buf())
        .display()
        .to_string()
}

pub(super) fn parse_opt_bool(args: &Value, field: &str) -> Result<bool, AiError> {
    match args.get(field) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(AiError::InvalidArguments(format!(
            "'{field}' must be a boolean"
        ))),
    }
}

pub(super) fn check_coord(c: VoxelCoord) -> Result<(), AiError> {
    if c.x.abs() > MAX_COORD_ABS || c.y.abs() > MAX_COORD_ABS || c.z.abs() > MAX_COORD_ABS {
        return Err(AiError::OutOfBounds);
    }
    Ok(())
}

pub(super) fn parse_ivec3(val: &Value, field: &str) -> Result<VoxelCoord, AiError> {
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

pub(super) fn parse_opt_vec3(val: &Value, field: &str) -> Result<Option<glam::Vec3>, AiError> {
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

pub(super) fn parse_grounding_args(
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

pub(super) fn parse_rotation_deg(val: &Value) -> Result<glam::Quat, AiError> {
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
pub(super) fn parse_bone_id(args: &Value, key: &str) -> Result<BoneId, AiError> {
    let raw = args
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}'")))?;
    Ok(BoneId(u32::try_from(raw).map_err(|_| {
        AiError::InvalidArguments(format!("'{key}' overflows u32"))
    })?))
}

/// Converts a quaternion to XYZ euler degrees (matches `rotation_deg` input order).
pub(super) fn quat_to_degrees(q: glam::Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

pub(super) fn parse_palette_index(val: &Value, default: u8) -> Result<PaletteIndex, AiError> {
    let raw = val
        .get("palette_index")
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(default));
    let byte =
        u8::try_from(raw).map_err(|_| AiError::InvalidArguments("palette_index > 255".into()))?;
    Ok(PaletteIndex::new(byte)?)
}

pub(super) fn parse_opt_palette_index(
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

pub(super) fn find_voxel_node(args: &Value, scene: &SceneState) -> Result<NodeId, AiError> {
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

pub(super) fn find_pixel_node(args: &Value, scene: &SceneState) -> Result<NodeId, AiError> {
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

pub(super) fn resolve_voxel_node(
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
pub(super) fn resolve_pixel_node(
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
pub(super) fn resolve_pixel_frame(
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
pub(super) fn pixel_frame_summary(frame: &tessel_core::PixelFrame) -> Vec<Value> {
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
pub(super) fn resolve_clip_index(
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
pub(super) fn resolve_batch_refs(value: &Value, results: &[Value]) -> Result<Value, AiError> {
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

pub(super) fn parse_symmetry_3d(args: &Value) -> Result<tessel_core::Symmetry3D, AiError> {
    Ok(tessel_core::Symmetry3D {
        x: parse_opt_bool(args, "mirror_x")?,
        y: parse_opt_bool(args, "mirror_y")?,
        z: parse_opt_bool(args, "mirror_z")?,
    })
}

pub(super) fn parse_symmetry_2d(args: &Value) -> Result<tessel_core::Symmetry2D, AiError> {
    Ok(tessel_core::Symmetry2D {
        x: parse_opt_bool(args, "mirror_x")?,
        y: parse_opt_bool(args, "mirror_y")?,
    })
}
