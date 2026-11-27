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
            if let Some(val) = args.get("emissive").filter(|v| !v.is_null()) {
                let ef = val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'emissive' must be a finite number".into())
                })? as f32;
                if !ef.is_finite() || !(0.0..=20.0).contains(&ef) {
                    return Err(AiError::InvalidArguments(
                        "'emissive' must be a finite number in 0.0..=20.0".into(),
                    ));
                }
                entry.emissive = ef;
            }
            if let Some(val) = args.get("transmission").filter(|v| !v.is_null()) {
                let tf = val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'transmission' must be a finite number".into())
                })? as f32;
                if !tf.is_finite() || !(0.0..=1.0).contains(&tf) {
                    return Err(AiError::InvalidArguments(
                        "'transmission' must be a finite number in 0.0..=1.0".into(),
                    ));
                }
                entry.transmission = tf;
            }
            if let Some(val) = args.get("ior").filter(|v| !v.is_null()) {
                let ior = val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'ior' must be a finite number".into())
                })? as f32;
                if !ior.is_finite() || !(1.0..=2.5).contains(&ior) {
                    return Err(AiError::InvalidArguments(
                        "'ior' must be a finite number in 1.0..=2.5".into(),
                    ));
                }
                entry.ior = ior;
            }
            let out_rough = entry.roughness;
            let out_metal = entry.metallic;
            let out_emissive = entry.emissive;
            let out_trans = entry.transmission;
            let out_ior = entry.ior;
            history.execute(scene, Command::SetPaletteEntry { index, entry })?;
            Ok(json!({
                "ok": true,
                "palette_index": index.get(),
                "name": name,
                "alpha": color.a,
                "hex": format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b),
                "roughness": out_rough,
                "metallic": out_metal,
                "emissive": out_emissive,
                "transmission": out_trans,
                "ior": out_ior
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
            let mut disk_side_effects: Vec<String> = Vec::new();
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
                        let note = if disk_side_effects.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "; note: in-memory scene was rolled back, but prior disk-write calls already committed files: [{}]",
                                disk_side_effects.join(", ")
                            )
                        };
                        return Err(AiError::InvalidArguments(format!(
                            "batch aborted at calls[{idx}] ({sub_name}): {err}{note}"
                        )));
                    }
                };
                match execute_tool_call(sub_name, &sub_args, scene, history, allowed_export_dir) {
                    Ok(val) => {
                        if is_disk_write_tool(sub_name) {
                            let target = val
                                .get("path")
                                .or_else(|| val.get("output_path"))
                                .and_then(Value::as_str)
                                .unwrap_or(sub_name);
                            disk_side_effects.push(format!("{sub_name}({target})"));
                        }
                        results.push(val);
                    }
                    Err(err) => {
                        *scene = prev_scene;
                        *history = prev_history;
                        let note = if disk_side_effects.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "; note: in-memory scene was rolled back, but prior disk-write calls already committed files: [{}]",
                                disk_side_effects.join(", ")
                            )
                        };
                        return Err(AiError::InvalidArguments(format!(
                            "batch aborted at calls[{idx}] ({sub_name}): {err}{note}"
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
            let parse_interp =
                |obj: &Value| -> Result<tessel_core::KeyframeInterpolation, AiError> {
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
        "history_undo" => {
            history.undo(scene)?;
            Ok(json!({ "ok": true, "undo_remaining": history.undo_len() }))
        }
        "history_redo" => {
            history.redo(scene)?;
            Ok(json!({ "ok": true, "redo_remaining": history.redo_len() }))
        }
        "scene_set_node_metadata" => {
            let raw_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'node_id'".into()))?;
            let node_id = NodeId(raw_id);
            if !scene.nodes.contains_key(&node_id) {
                return Err(AiError::InvalidArguments(format!(
                    "node {} not found",
                    node_id.0
                )));
            }
            let mut cmds = Vec::new();
            match args.get("extras") {
                None | Some(Value::Null) => {}
                Some(Value::Object(obj)) => {
                    if obj.len() > 64 {
                        return Err(AiError::InvalidArguments(
                            "'extras' must contain at most 64 keys".into(),
                        ));
                    }
                    let mut map = std::collections::BTreeMap::new();
                    for (k, v) in obj {
                        let s = v.as_str().ok_or_else(|| {
                            AiError::InvalidArguments(format!("extras['{k}'] must be a string"))
                        })?;
                        map.insert(k.clone(), s.to_string());
                    }
                    cmds.push(Command::SetNodeExtras {
                        node_id,
                        extras: map,
                    });
                }
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'extras' must be a JSON object of string values".into(),
                    ));
                }
            }
            match args.get("collision_shape") {
                None | Some(Value::Null) => {}
                Some(Value::String(shape)) => {
                    let hull = match shape.as_str() {
                        "none" => None,
                        "box" => {
                            let he = parse_opt_vec3(args, "half_extents")?
                                .unwrap_or(glam::Vec3::splat(0.5));
                            if !he.is_finite() || he.x <= 0.0 || he.y <= 0.0 || he.z <= 0.0 {
                                return Err(AiError::InvalidArguments(
                                    "'half_extents' must have finite components > 0".into(),
                                ));
                            }
                            Some(tessel_core::CollisionHull::Box {
                                half_extents: he.to_array(),
                                offset: [0.0, 0.0, 0.0],
                            })
                        }
                        "capsule" => {
                            let radius = parse_opt_f32_finite(args, "radius", 0.35)?;
                            let height = parse_opt_f32_finite(args, "height", 1.8)?;
                            if radius <= 0.0 || height <= 0.0 {
                                return Err(AiError::InvalidArguments(
                                    "'radius' and 'height' must be > 0".into(),
                                ));
                            }
                            Some(tessel_core::CollisionHull::Capsule {
                                radius,
                                height,
                                offset: [0.0, 0.0, 0.0],
                            })
                        }
                        other => {
                            return Err(AiError::InvalidArguments(format!(
                                "unsupported collision_shape '{other}'"
                            )))
                        }
                    };
                    cmds.push(Command::SetCollisionHull { node_id, hull });
                }
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'collision_shape' must be a string".into(),
                    ));
                }
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else if !cmds.is_empty() {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({ "ok": true, "node_id": node_id.0 }))
        }
        "scene_set_lookdev" => {
            let mut cmds = Vec::new();
            match args.get("preset") {
                None | Some(Value::Null) => {}
                Some(Value::String(p)) => {
                    let preset = match p.as_str() {
                        "daylight" => tessel_core::LookDevPreset::Daylight,
                        "night" => tessel_core::LookDevPreset::Night,
                        "indoor" => tessel_core::LookDevPreset::Indoor,
                        "sunset" => tessel_core::LookDevPreset::Sunset,
                        other => {
                            return Err(AiError::InvalidArguments(format!(
                                "unsupported lookdev preset '{other}'"
                            )))
                        }
                    };
                    cmds.push(Command::SetLookDevPreset { preset });
                }
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'preset' must be a string".into(),
                    ));
                }
            }
            match args.get("bookmark_name") {
                None | Some(Value::Null) => {}
                Some(Value::String(raw_name)) => {
                    let bname = raw_name.trim();
                    if bname.is_empty() {
                        return Err(AiError::InvalidArguments(
                            "'bookmark_name' must not be empty".into(),
                        ));
                    }
                    let target = parse_opt_vec3(args, "target")?.unwrap_or(glam::Vec3::ZERO);
                    let yaw = parse_opt_f32_finite(args, "yaw", 0.6)?;
                    let pitch = parse_opt_f32_finite(args, "pitch", 0.35)?;
                    let distance = parse_opt_f32_finite(args, "distance", 3.5)?.max(0.25);
                    cmds.push(Command::SaveCameraBookmark {
                        bookmark: tessel_core::CameraBookmark {
                            name: bname.to_string(),
                            target: target.to_array(),
                            yaw,
                            pitch,
                            distance,
                        },
                    });
                }
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'bookmark_name' must be a string".into(),
                    ));
                }
            }
            if args.get("hatching_strength").is_some()
                || args.get("bloom_strength").is_some()
                || args.get("msaa_samples").is_some()
            {
                let mut params = scene.npr_look;
                if args.get("hatching_strength").is_some() {
                    params.hatching_strength =
                        parse_opt_f32_finite(args, "hatching_strength", params.hatching_strength)?
                            .clamp(0.0, 1.0);
                }
                if args.get("bloom_strength").is_some() {
                    params.bloom_strength =
                        parse_opt_f32_finite(args, "bloom_strength", params.bloom_strength)?
                            .clamp(0.0, 2.0);
                }
                if args.get("msaa_samples").is_some() {
                    let s = parse_opt_u32(args, "msaa_samples", params.msaa_samples)?;
                    if s != 1 && s != 4 {
                        return Err(AiError::InvalidArguments(
                            "'msaa_samples' must be 1 or 4".into(),
                        ));
                    }
                    params.msaa_samples = s;
                }
                cmds.push(Command::SetNprLookParams { params });
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else if !cmds.is_empty() {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(
                json!({ "ok": true, "lookdev_preset": scene.lookdev_preset, "bookmarks": scene.camera_bookmarks.len() }),
            )
        }
        "scene_set_post_process" => {
            let mut settings = scene.post_process;
            let read_bool = |key: &str| -> Result<Option<bool>, AiError> {
                match args.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::Bool(b)) => Ok(Some(*b)),
                    Some(_) => Err(AiError::InvalidArguments(format!(
                        "'{key}' must be a boolean"
                    ))),
                }
            };
            let read_bounded = |key: &str, min: f32, max: f32| -> Result<Option<f32>, AiError> {
                match args.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => {
                        let n = v.as_f64().ok_or_else(|| {
                            AiError::InvalidArguments(format!("'{key}' must be a finite number"))
                        })? as f32;
                        if !n.is_finite() || !(min..=max).contains(&n) {
                            return Err(AiError::InvalidArguments(format!(
                                "'{key}' must be a finite number in {min}..={max}"
                            )));
                        }
                        Ok(Some(n))
                    }
                }
            };
            if let Some(v) = read_bool("bloom_enabled")? {
                settings.bloom_enabled = v;
            }
            if let Some(v) = read_bounded("bloom_threshold", 0.0, 10.0)? {
                settings.bloom_threshold = v;
            }
            if let Some(v) = read_bounded("bloom_intensity", 0.0, 4.0)? {
                settings.bloom_intensity = v;
            }
            if let Some(v) = read_bounded("exposure", 0.05, 8.0)? {
                settings.exposure = v;
            }
            if let Some(v) = read_bool("aces_enabled")? {
                settings.aces_enabled = v;
            }
            if let Some(v) = read_bool("fog_enabled")? {
                settings.fog_enabled = v;
            }
            if let Some(val) = args.get("fog_color").filter(|v| !v.is_null()) {
                let arr = val.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("'fog_color' must be [r, g, b] in 0.0..=1.0".into())
                })?;
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'fog_color' must have 3 numbers [r, g, b]".into(),
                    ));
                }
                let mut col = [0.0f32; 3];
                for (i, item) in arr.iter().enumerate() {
                    let c = item.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments(
                            "'fog_color' entries must be finite numbers".into(),
                        )
                    })? as f32;
                    if !c.is_finite() || !(0.0..=1.0).contains(&c) {
                        return Err(AiError::InvalidArguments(
                            "'fog_color' entries must be in 0.0..=1.0".into(),
                        ));
                    }
                    col[i] = c;
                }
                settings.fog_color = col;
            }
            if let Some(v) = read_bounded("fog_density", 0.0, 1.0)? {
                settings.fog_density = v;
            }
            if let Some(v) = read_bounded("fog_start", 0.0, 500.0)? {
                settings.fog_start = v;
            }
            if let Some(v) = read_bounded("fog_height_falloff", 0.0, 5.0)? {
                settings.fog_height_falloff = v;
            }
            history.execute(scene, Command::SetPostProcess(settings))?;
            Ok(json!({
                "ok": true,
                "post_process": scene.post_process
            }))
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
