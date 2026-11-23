use std::path::Path;

use serde_json::{json, Value};
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    AppMode, BoneId, Command, HistoryStack, Keyframe, NodeId, PaletteEntry, PaletteIndex,
    PrimitiveKind, Rgba8, SceneState, VoxelCoord,
};
use tessel_io::{
    export_scene_to_dir, import_mesh_from_path, import_mesh_texture_from_path,
    resolve_safe_export_path, ExportFormat, ExportPreset,
};

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
        "scene_inspect" | "scene_list_nodes" => Ok(json!({
            "ok": true,
            "scene": scene_summary_json(scene),
            "palette": palette_summary_json(scene)["colors"]
        })),
        "palette_set_color" => {
            let index = parse_palette_index(args, 0)?;
            let color = if let Some(arr) = args.get("rgb").and_then(Value::as_array) {
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'rgb' must have 3 integers [r, g, b]".into(),
                    ));
                }
                let r = arr[0].as_u64().unwrap_or(0).min(255) as u8;
                let g = arr[1].as_u64().unwrap_or(0).min(255) as u8;
                let b = arr[2].as_u64().unwrap_or(0).min(255) as u8;
                Rgba8::rgb(r, g, b)
            } else if let Some(hex_raw) = args.get("hex").and_then(Value::as_str) {
                let hex = hex_raw.trim().trim_start_matches('#');
                if hex.len() != 6 {
                    return Err(AiError::InvalidArguments(
                        "'hex' must be 6 hex digits '#RRGGBB'".into(),
                    ));
                }
                let r = u8::from_str_radix(&hex[0..2], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex red".into()))?;
                let g = u8::from_str_radix(&hex[2..4], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex green".into()))?;
                let b = u8::from_str_radix(&hex[4..6], 16)
                    .map_err(|_| AiError::InvalidArguments("invalid hex blue".into()))?;
                Rgba8::rgb(r, g, b)
            } else {
                return Err(AiError::InvalidArguments(
                    "provide either 'rgb': [r,g,b] or 'hex': '#RRGGBB'".into(),
                ));
            };
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("Color {}", index.get()));
            history.execute(
                scene,
                Command::SetPaletteEntry {
                    index,
                    entry: PaletteEntry::matte(name.clone(), color),
                },
            )?;
            Ok(json!({
                "ok": true,
                "palette_index": index.get(),
                "name": name,
                "hex": format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
            }))
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
                "sphere" => PrimitiveKind::Sphere,
                "torus" => PrimitiveKind::Torus,
                "capsule" => PrimitiveKind::Capsule,
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
            let pos = parse_opt_vec3(args, "position")?;
            let rot = if args.get("rotation_deg").is_some() {
                Some(parse_rotation_deg(args)?)
            } else {
                None
            };
            let sc = parse_opt_vec3(args, "scale")?;
            history.execute(
                scene,
                Command::CreatePrimitiveMesh {
                    name,
                    kind,
                    size,
                    material,
                },
            )?;
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
            Ok(json!({ "ok": true, "node_id": created_id.map(|n| n.0) }))
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
            Ok(json!({ "ok": true, "node_id": node_id.0, "morph": name }))
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
                mat.emissive = [
                    arr[0].as_f64().unwrap_or(0.0) as f32,
                    arr[1].as_f64().unwrap_or(0.0) as f32,
                    arr[2].as_f64().unwrap_or(0.0) as f32,
                ];
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
                "emissive": mat.emissive
            }))
        }
        "character_apply_biped_rig" => {
            history.execute(scene, Command::ApplyBipedRig)?;
            Ok(json!({ "ok": true, "bones": 7 }))
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
        "character_set_bone_pose" => {
            let raw_id = args
                .get("bone_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'bone_id'".into()))?;
            let bone_id = BoneId(
                u32::try_from(raw_id)
                    .map_err(|_| AiError::InvalidArguments("bone_id overflow".into()))?,
            );
            let delta = parse_rotation_deg(args)?;
            let rest = scene
                .character_rig
                .as_ref()
                .and_then(|r| r.bone(bone_id))
                .map(|b| b.rest_local.rotation)
                .unwrap_or(glam::Quat::IDENTITY);
            history.execute(
                scene,
                Command::SetBoneRotation {
                    bone_id,
                    rotation: rest * delta,
                },
            )?;
            Ok(json!({ "ok": true, "bone_id": bone_id.0 }))
        }
        "character_set_keyframe" => {
            let raw_id = args
                .get("bone_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'bone_id'".into()))?;
            let bone_id = BoneId(
                u32::try_from(raw_id)
                    .map_err(|_| AiError::InvalidArguments("bone_id overflow".into()))?,
            );
            let time_secs = args
                .get("time_secs")
                .and_then(Value::as_f64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'time_secs'".into()))?
                as f32;
            let rotation = parse_rotation_deg(args)?;
            let translation = parse_opt_vec3(args, "translation")?.unwrap_or(glam::Vec3::ZERO);
            let scale = parse_opt_vec3(args, "scale")?.unwrap_or(glam::Vec3::ONE);
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
                Command::SetAnimationKeyframe {
                    clip_index,
                    bone_id,
                    keyframe: Keyframe {
                        time_secs,
                        translation,
                        rotation,
                        scale,
                    },
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
            let pole = parse_opt_vec3(args, "pole_vector")?.unwrap_or(glam::Vec3::Z);
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
                "reached": reached
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
            let bones: Vec<Value> = rig
                .bones
                .iter()
                .map(|b| {
                    json!({
                        "bone_id": b.id.0,
                        "name": b.name,
                        "parent_bone_id": b.parent.map(|p| p.0),
                        "rest_position": b.rest_local.translation.to_array(),
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "bone_count": bones.len(), "bones": bones }))
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
            history.execute(scene, Command::DuplicateNode { node_id, name })?;
            let new_id = scene.selected_node.ok_or_else(|| {
                AiError::InvalidArguments("duplicate produced no node".into())
            })?;
            Ok(json!({ "ok": true, "node_id": new_id.0 }))
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
            let (clear_color, clear_normal) = match target {
                "color" => (true, false),
                "normal" => (false, true),
                "both" => (true, true),
                _ => {
                    return Err(AiError::InvalidArguments(
                        "target must be 'color', 'normal', or 'both'".into(),
                    ));
                }
            };
            history.execute(
                scene,
                Command::ClearMeshTexture {
                    node_id,
                    clear_color,
                    clear_normal,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "cleared": target }))
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
        "viewport_screenshot" => Err(AiError::InvalidArguments(
            "viewport_screenshot is served by the MCP server itself (needs its own GPU); the Direct AI path has no renderer".into(),
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
}
