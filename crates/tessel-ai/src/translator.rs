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
                "bounds": { "min": min_b.to_array(), "max": max_b.to_array() },
                "faces_preview": faces
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
                    keyframe: Keyframe::rotation_only(time_secs, rotation),
                },
            )?;
            Ok(json!({
                "ok": true,
                "clip_index": clip_index,
                "bone_id": bone_id.0,
                "time_secs": time_secs
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
    }
}
