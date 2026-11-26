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
        "character_create_clip" | "scene_create_clip" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let duration_secs = parse_opt_f32_finite(args, "duration_secs", 2.0)?;
            if !(0.05..=600.0).contains(&duration_secs) {
                return Err(AiError::InvalidArguments(
                    "'duration_secs' must be finite in 0.05..=600.0".into(),
                ));
            }
            let activate = parse_opt_bool_default(args, "activate", true)?;
            if scene.character_rig.is_none() {
                history.execute(
                    scene,
                    Command::AddAnimationClip {
                        name: name.clone(),
                        duration_secs,
                    },
                )?;
            } else {
                let clip = tessel_core::AnimationClip::new_empty(name.clone(), duration_secs);
                history.execute(scene, Command::UpsertAnimationClip { clip, activate })?;
            }
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
        "character_create_bone_grid" => {
            let mut spec = tessel_core::BoneGridSpec::default();
            if let Some(p) = args.get("prefix").and_then(Value::as_str) {
                spec.prefix = p.to_string();
            }
            spec.cols = parse_opt_usize(args, "cols", spec.cols)?.clamp(2, 16);
            spec.rows = parse_opt_usize(args, "rows", spec.rows)?.clamp(2, 16);
            if let Some(v) = parse_opt_vec3(args, "origin")? {
                spec.origin = v;
            }
            spec.width = parse_opt_f32_finite(args, "width", spec.width)?;
            spec.height = parse_opt_f32_finite(args, "height", spec.height)?;
            if spec.width <= 0.0 || spec.height <= 0.0 {
                return Err(AiError::InvalidArguments(
                    "'width' and 'height' must be > 0".into(),
                ));
            }
            if let Some(v) = parse_opt_vec3(args, "axis_u")? {
                spec.axis_u = v;
            }
            if let Some(v) = parse_opt_vec3(args, "axis_v")? {
                spec.axis_v = v;
            }
            if let Some(raw_pb) = args.get("parent_bone").filter(|v| !v.is_null()) {
                let pb = raw_pb
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        AiError::InvalidArguments(
                            "'parent_bone' must be a valid u32 bone id".into(),
                        )
                    })?;
                spec.parent_bone = Some(BoneId(pb));
            }
            let bind_node = match args.get("bind_node") {
                None | Some(Value::Null) => None,
                Some(v) => Some(NodeId(v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'bind_node' must be a valid node id".into())
                })?)),
            };
            let cols = spec.cols;
            let rows = spec.rows;
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
            params.wind_strength =
                parse_opt_f32_finite(args, "wind_strength", params.wind_strength)?;
            params.gravity = parse_opt_f32_finite(args, "gravity", params.gravity)?;
            params.turbulence = parse_opt_f32_finite(args, "turbulence", params.turbulence)?;
            params.phase_offset = parse_opt_f32_finite(args, "phase_offset", params.phase_offset)?;
            params.duration_secs =
                parse_opt_f32_finite(args, "duration_secs", params.duration_secs)?;
            if !(0.05..=600.0).contains(&params.duration_secs) {
                return Err(AiError::InvalidArguments(
                    "'duration_secs' must be in 0.05..=600.0".into(),
                ));
            }
            params.fps = parse_opt_f32_finite(args, "fps", params.fps)?;
            if !(1.0..=120.0).contains(&params.fps) {
                return Err(AiError::InvalidArguments(
                    "'fps' must be in 1.0..=120.0".into(),
                ));
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
            let axis_lock = args
                .get("axis_lock")
                .and_then(Value::as_u64)
                .map(|v| v as u8);
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
                    let (base_tmpl, base_cmd) = match preset {
                        "starter" => (
                            tessel_core::CharacterRig::biped_template(),
                            Command::ApplyBipedRig,
                        ),
                        _ => (
                            tessel_core::CharacterRig::humanoid_template(),
                            Command::ApplyHumanoidRig,
                        ),
                    };
                    cmds.push(base_cmd);
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
            let parent = match args.get("parent_bone_id") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let raw = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments(
                            "'parent_bone_id' must be a non-negative integer".into(),
                        )
                    })?;
                    Some(BoneId(u32::try_from(raw).map_err(|_| {
                        AiError::InvalidArguments("parent_bone_id overflow".into())
                    })?))
                }
            };
            let input_pos = parse_opt_vec3(args, "position")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'position'".into()))?;
            let space = match args.get("space") {
                None | Some(Value::Null) => "local",
                Some(Value::String(s)) => s.as_str(),
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'space' must be 'local' or 'world'".into(),
                    ))
                }
            };
            let position = match (space, parent) {
                ("local", _) => input_pos,
                ("world", None) => input_pos,
                ("world", Some(pid)) => {
                    let rig = scene.character_rig.as_ref().ok_or_else(|| {
                        AiError::InvalidArguments(
                            "cannot use space='world' with parent_bone_id before a rig exists"
                                .into(),
                        )
                    })?;
                    let (parent_world, parent_rot) = bone_world_rest_transform(rig, pid)
                        .ok_or_else(|| {
                            AiError::InvalidArguments(format!("parent bone {} not found", pid.0))
                        })?;
                    parent_rot.inverse() * (input_pos - parent_world)
                }
                (other, _) => {
                    return Err(AiError::InvalidArguments(format!(
                        "invalid 'space' '{other}'; expected 'local' or 'world'"
                    )))
                }
            };
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
            let world_rest = scene
                .character_rig
                .as_ref()
                .and_then(|r| bone_world_rest_transform(r, BoneId(bone_id)).map(|(p, _)| p))
                .unwrap_or(position);
            Ok(json!({
                "ok": true,
                "bone_id": bone_id,
                "name": name,
                "local_rest_position": position.to_array(),
                "world_rest_position": world_rest.to_array()
            }))
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
                let rig = scene
                    .character_rig
                    .as_ref()
                    .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
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
            let fps = parse_opt_f32_finite(args, "fps", 24.0)?;
            if !(1.0..=120.0).contains(&fps) {
                return Err(AiError::InvalidArguments(
                    "'fps' must be in 1.0..=120.0".into(),
                ));
            }
            let duration_override = match args.get("duration_secs") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let d = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'duration_secs' must be a number".into())
                    })? as f32;
                    if !d.is_finite() || !(0.05..=600.0).contains(&d) {
                        return Err(AiError::InvalidArguments(
                            "'duration_secs' must be in 0.05..=600.0".into(),
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
            let activate = args
                .get("activate")
                .and_then(Value::as_bool)
                .unwrap_or(true);
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
                    .ok_or_else(|| {
                        AiError::InvalidArguments("invalid bone_id in 'bone_ids'".into())
                    })?;
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
            let rots = tessel_core::solve_chain_ik(rig, &chain, target, max_iterations, tolerance)?;
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
            let mut clip =
                rig.animations.get(clip_index).cloned().ok_or_else(|| {
                    AiError::InvalidArguments(format!("clip {clip_index} not found"))
                })?;
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
            let floor_y = parse_opt_f32_finite(args, "floor_y", 0.0)?;
            let fps = parse_opt_f32_finite(args, "fps", 24.0)?;
            if !(1.0..=120.0).contains(&fps) {
                return Err(AiError::InvalidArguments(
                    "'fps' must be in 1.0..=120.0".into(),
                ));
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
            let bounds = tessel_core::SkinnedBoundsCache::from_scene_excluding_many(
                scene,
                rig,
                &surface_ids,
            );
            let clamped = rig
                .clamp_clip_to_surface_with(tessel_core::ClipGroundingRequest {
                    clip_index,
                    bone_prefix,
                    fallback_floor_y: floor_y,
                    fps,
                    bounds: bounds.as_ref(),
                    surface: surface.as_ref(),
                    options: &options,
                })
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
            let bounds = tessel_core::SkinnedBoundsCache::from_scene_excluding_many(
                scene,
                rig,
                &surface_ids,
            );
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
                    tessel_core::KeyframeInterpolation::CubicBezier { p1x, p1y, p2x, p2y }
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
                .filter(|b| filter_ids.as_ref().is_none_or(|list| list.contains(&b.id)))
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
            Ok(
                json!({ "ok": true, "clip_index": clip_index, "time_secs": time_secs, "bones": bones }),
            )
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
        "character_set_bone_scale" => {
            let bone_id = parse_bone_id(args, "bone_id")?;
            let scale = parse_opt_vec3(args, "scale")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'scale'".into()))?;
            history.execute(scene, Command::SetBoneScale { bone_id, scale })?;
            Ok(json!({ "ok": true, "bone_id": bone_id.0, "scale": scale.to_array() }))
        }
        "character_set_bone_properties" => {
            let bone_id = parse_bone_id(args, "bone_id")?;
            let rotation_limits_deg = match (
                parse_opt_vec3(args, "min_deg")?,
                parse_opt_vec3(args, "max_deg")?,
            ) {
                (Some(mn), Some(mx)) => {
                    if mn.x > mx.x || mn.y > mx.y || mn.z > mx.z {
                        return Err(AiError::InvalidArguments(
                            "'min_deg' must be <= 'max_deg' on all axes".into(),
                        ));
                    }
                    Some(Some((mn.to_array(), mx.to_array())))
                }
                (None, None) => None,
                _ => {
                    return Err(AiError::InvalidArguments(
                        "both 'min_deg' and 'max_deg' must be provided together".into(),
                    ))
                }
            };
            let opt_b = |k: &str| -> Result<Option<bool>, AiError> {
                match args.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::Bool(b)) => Ok(Some(*b)),
                    Some(_) => Err(AiError::InvalidArguments(format!(
                        "'{k}' must be a boolean"
                    ))),
                }
            };
            let spring_stiffness = match args.get("spring_stiffness") {
                None | Some(Value::Null) => None,
                Some(_) => {
                    let v = parse_opt_f32_finite(args, "spring_stiffness", 0.0)?;
                    if !(0.0..=200.0).contains(&v) {
                        return Err(AiError::InvalidArguments(
                            "'spring_stiffness' must be in 0.0..=200.0".into(),
                        ));
                    }
                    Some(v)
                }
            };
            let spring_damping = match args.get("spring_damping") {
                None | Some(Value::Null) => None,
                Some(_) => {
                    let v = parse_opt_f32_finite(args, "spring_damping", 0.5)?;
                    if !(0.01..=50.0).contains(&v) {
                        return Err(AiError::InvalidArguments(
                            "'spring_damping' must be in 0.01..=50.0".into(),
                        ));
                    }
                    Some(v)
                }
            };
            let lod_level = match args.get("lod_level") {
                None | Some(Value::Null) => None,
                Some(_) => {
                    let v = parse_opt_u32(args, "lod_level", 0)?;
                    if v > 8 {
                        return Err(AiError::InvalidArguments(
                            "'lod_level' must be in 0..=8".into(),
                        ));
                    }
                    Some(v as u8)
                }
            };
            history.execute(
                scene,
                Command::SetBoneProperties {
                    bone_id,
                    rotation_limits_deg,
                    inherit_rotation: opt_b("inherit_rotation")?,
                    inherit_scale: opt_b("inherit_scale")?,
                    connected: opt_b("connected")?,
                    deform: opt_b("deform")?,
                    spring_stiffness,
                    spring_damping,
                    lod_level,
                    physics_collider: None,
                },
            )?;
            Ok(json!({ "ok": true, "bone_id": bone_id.0 }))
        }
        "character_set_constraint" => {
            let cname = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'kind'".into()))?;
            let weight = parse_opt_f32_finite(args, "weight", 1.0)?.clamp(0.0, 1.0);
            let constraint = match kind {
                "ik_fk_blend" => tessel_core::BoneConstraint::IkFkBlend {
                    name: cname.clone(),
                    root: parse_bone_id(args, "root")?,
                    mid: parse_bone_id(args, "mid")?,
                    end: parse_bone_id(args, "end")?,
                    target: parse_opt_vec3(args, "target")?
                        .ok_or_else(|| AiError::InvalidArguments("missing 'target'".into()))?,
                    pole: parse_opt_vec3(args, "pole")?,
                    weight,
                },
                "look_at" => tessel_core::BoneConstraint::LookAt {
                    name: cname.clone(),
                    bone_id: parse_bone_id(args, "bone_id")?,
                    target: parse_opt_vec3(args, "target")?
                        .ok_or_else(|| AiError::InvalidArguments("missing 'target'".into()))?,
                    up: glam::Vec3::Y,
                    influence: weight,
                },
                "copy_transform" => tessel_core::BoneConstraint::CopyTransform {
                    name: cname.clone(),
                    bone_id: parse_bone_id(args, "bone_id")?,
                    source_bone_id: parse_bone_id(args, "source_bone_id")?,
                    influence: weight,
                },
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unsupported constraint kind '{other}'"
                    )))
                }
            };
            history.execute(scene, Command::UpsertRigConstraint { constraint })?;
            Ok(json!({ "ok": true, "name": cname, "kind": kind }))
        }
        "character_set_socket" => {
            let sname = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let bone_id = parse_bone_id(args, "bone_id")?;
            let pos = parse_opt_vec3(args, "position")?.unwrap_or(glam::Vec3::ZERO);
            history.execute(
                scene,
                Command::UpsertRigSocket {
                    socket: tessel_core::RigSocket {
                        name: sname.clone(),
                        bone_id,
                        local_offset: tessel_core::Transform {
                            translation: pos,
                            ..tessel_core::Transform::default()
                        },
                    },
                },
            )?;
            Ok(json!({ "ok": true, "name": sname, "bone_id": bone_id.0 }))
        }
        "character_pose_preset" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'action'".into()))?;
            match action {
                "save" => {
                    let pname = args
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                        .to_string();
                    history.execute(
                        scene,
                        Command::SavePosePreset {
                            name: pname.clone(),
                        },
                    )?;
                    Ok(json!({ "ok": true, "action": "save", "name": pname }))
                }
                "apply" => {
                    let pname = args
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| AiError::InvalidArguments("missing 'name'".into()))?
                        .to_string();
                    let flipped = parse_opt_bool_default(args, "flipped", false)?;
                    history.execute(
                        scene,
                        Command::ApplyPosePreset {
                            name: pname.clone(),
                            flipped,
                        },
                    )?;
                    Ok(json!({ "ok": true, "action": "apply", "name": pname, "flipped": flipped }))
                }
                "mirror" => {
                    let left_to_right = parse_opt_bool_default(args, "left_to_right", true)?;
                    history.execute(scene, Command::MirrorRigPose { left_to_right })?;
                    Ok(json!({ "ok": true, "action": "mirror", "left_to_right": left_to_right }))
                }
                other => Err(AiError::InvalidArguments(format!(
                    "unsupported action '{other}'; use save|apply|mirror"
                ))),
            }
        }
        "character_bake_spring_bones" => {
            let clip_index = parse_opt_usize(args, "clip_index", 0)?;
            let fps = parse_opt_f32_finite(args, "fps", 24.0)?;
            if !(1.0..=120.0).contains(&fps) {
                return Err(AiError::InvalidArguments(
                    "'fps' must be in 1.0..=120.0".into(),
                ));
            }
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or_else(|| AiError::InvalidArguments("no character rig applied".into()))?;
            let baked = tessel_core::bake_spring_bones(rig, clip_index, fps).ok_or_else(|| {
                AiError::InvalidArguments(
                    "no bones with spring_stiffness > 0 or invalid clip_index".into(),
                )
            })?;
            let clip_name = baked.name.clone();
            history.execute(
                scene,
                Command::UpsertAnimationClip {
                    clip: baked,
                    activate: false,
                },
            )?;
            Ok(json!({ "ok": true, "clip_index": clip_index, "clip_name": clip_name }))
        }
        "character_add_event" => {
            let clip_index = parse_opt_usize(args, "clip_index", 0)?;
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    AiError::InvalidArguments("missing finite numeric 'time_secs'".into())
                })?;
            if time_secs < 0.0 {
                return Err(AiError::InvalidArguments("'time_secs' must be >= 0".into()));
            }
            let ename = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AiError::InvalidArguments("missing non-empty 'name'".into()))?
                .to_string();
            let payload = match args.get("payload") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(_) => {
                    return Err(AiError::InvalidArguments(
                        "'payload' must be a string".into(),
                    ))
                }
            };
            history.execute(
                scene,
                Command::AddAnimationEvent {
                    clip_index,
                    event: tessel_core::AnimationEventKey {
                        time_secs,
                        name: ename.clone(),
                        payload,
                    },
                },
            )?;
            Ok(
                json!({ "ok": true, "clip_index": clip_index, "name": ename, "time_secs": time_secs }),
            )
        }
        "character_apply_foot_roll" => {
            let heel_id = parse_bone_id(args, "heel_id")?;
            let ball_id = parse_bone_id(args, "ball_id")?;
            let toe_id = parse_bone_id(args, "toe_id")?;
            let roll_deg = args
                .get("roll_deg")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    AiError::InvalidArguments("missing finite numeric 'roll_deg'".into())
                })?;
            if !(-60.0..=90.0).contains(&roll_deg) {
                return Err(AiError::InvalidArguments(
                    "'roll_deg' must be in -60.0..=90.0".into(),
                ));
            }
            history.execute(
                scene,
                Command::ApplyFootRoll {
                    heel_id,
                    ball_id,
                    toe_id,
                    roll_deg,
                },
            )?;
            Ok(json!({ "ok": true, "roll_deg": roll_deg }))
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
