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
                        AiError::InvalidArguments("'size' must be a number or [sx, sy, sz]".into())
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
                history.execute(
                    scene,
                    Command::ImportLowPolyMesh {
                        name,
                        mesh: Box::new(mesh),
                    },
                )?;
                let was_clamped = (actual_v - v).abs().max_element() > 1e-6;
                (json!(actual_v.to_array()), was_clamped)
            } else if anchor != PrimitiveAnchor::Bottom {
                let actual_s = size.max(min_s);
                let mut mesh = tessel_core::mesh::ops::create_primitive(kind, actual_s, material);
                apply_primitive_anchor(&mut mesh, anchor);
                history.execute(
                    scene,
                    Command::ImportLowPolyMesh {
                        name,
                        mesh: Box::new(mesh),
                    },
                )?;
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
            let width = parse_opt_f32_finite(args, "width", 10.0)?;
            let depth = parse_opt_f32_finite(args, "depth", 10.0)?;
            if width <= 0.0 || depth <= 0.0 {
                return Err(AiError::InvalidArguments(
                    "'width' and 'depth' must be > 0".into(),
                ));
            }
            let subdiv_x = parse_opt_u32(args, "subdiv_x", 16)?;
            let subdiv_z = parse_opt_u32(args, "subdiv_z", 16)?;
            if !(1..=128).contains(&subdiv_x) || !(1..=128).contains(&subdiv_z) {
                return Err(AiError::InvalidArguments(
                    "'subdiv_x' and 'subdiv_z' must be in 1..=128".into(),
                ));
            }
            let height_scale = parse_opt_f32_finite(args, "height_scale", 2.0)?;
            let seed = match args.get("seed") {
                None | Some(Value::Null) => 42,
                Some(v) => v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'seed' must be a non-negative integer".into())
                })?,
            };
            let octaves = parse_opt_u32(args, "octaves", 4)?.clamp(1, 8);
            let frequency = parse_opt_f32_finite(args, "frequency", 0.35)?;
            if frequency <= 0.0 {
                return Err(AiError::InvalidArguments("'frequency' must be > 0".into()));
            }
            let island_falloff = parse_opt_f32_finite(args, "island_falloff", 0.0)?;
            if island_falloff < 0.0 {
                return Err(AiError::InvalidArguments(
                    "'island_falloff' must be >= 0".into(),
                ));
            }
            let smooth_shading = parse_opt_bool_default(args, "smooth_shading", false)?;
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
            history.execute(
                scene,
                Command::ImportLowPolyMesh {
                    name,
                    mesh: Box::new(mesh),
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
            history.execute(
                scene,
                Command::ImportLowPolyMesh {
                    name,
                    mesh: Box::new(mesh),
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
                        AiError::InvalidArguments(format!("sections[{si}][{vi}] must be [x, y, z]"))
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
            history.execute(
                scene,
                Command::ImportLowPolyMesh {
                    name,
                    mesh: Box::new(mesh),
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
            if verts_arr.len() > MAX_RAW_VERTICES {
                return Err(AiError::InvalidArguments(format!(
                    "'vertices' exceeds maximum of {MAX_RAW_VERTICES}"
                )));
            }
            if faces_arr.len() > MAX_RAW_FACES {
                return Err(AiError::InvalidArguments(format!(
                    "'faces' exceeds maximum of {MAX_RAW_FACES}"
                )));
            }
            let default_mat = parse_palette_index(args, 4)?;
            let smooth = parse_opt_bool_default(args, "smooth", false)?;
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
                let v = glam::Vec3::new(x, y, z);
                if !v.is_finite() {
                    return Err(AiError::InvalidArguments(format!(
                        "'vertices[{i}]' must be finite"
                    )));
                }
                vertices.push(v);
            }
            let mut faces = Vec::with_capacity(faces_arr.len());
            for (fi, item) in faces_arr.iter().enumerate() {
                let v_ids_arr =
                    item.get("vertices")
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
            history.execute(
                scene,
                Command::ImportLowPolyMesh {
                    name,
                    mesh: Box::new(mesh),
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
            history.execute(
                scene,
                Command::ImportLowPolyMesh {
                    name,
                    mesh: Box::new(mesh),
                },
            )?;
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
                let color = scene.palette.get(idx).map(|e| e.color).ok_or_else(|| {
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
        "mesh_set_npr_slot" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let slot_str = args
                .get("slot")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'slot'".into()))?;
            let slot = match slot_str.to_ascii_lowercase().as_str() {
                "matcap" => tessel_core::NprTextureSlot::MatCap,
                "face_sdf" => tessel_core::NprTextureSlot::FaceSdf,
                "hatching_ramp" => tessel_core::NprTextureSlot::HatchingRamp,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown NPR slot '{other}', expected matcap|face_sdf|hatching_ramp"
                    )))
                }
            };
            let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
            let mut loaded_dims: Option<(u32, u32)> = None;
            if clear {
                history.execute(
                    scene,
                    Command::SetMeshNprSlot {
                        node_id,
                        slot,
                        texture: None,
                    },
                )?;
            } else if let Some(filename) = args.get("filename").and_then(Value::as_str) {
                let target = resolve_safe_export_path(allowed_export_dir, filename)?;
                let tex = import_mesh_texture_from_path(&target)?;
                loaded_dims = Some((tex.width, tex.height));
                history.execute(
                    scene,
                    Command::SetMeshNprSlot {
                        node_id,
                        slot,
                        texture: Some(tex),
                    },
                )?;
            }
            let mut mat = scene
                .lowpoly_meshes
                .get(&node_id)
                .ok_or(tessel_core::CoreError::LowPolyMeshNotFound(node_id))?
                .material;
            let mut mat_changed = false;
            if let Some(val) = args.get("matcap_strength").filter(|v| !v.is_null()) {
                let v = val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'matcap_strength' must be a finite number".into())
                })? as f32;
                if !v.is_finite() {
                    return Err(AiError::InvalidArguments(
                        "'matcap_strength' must be finite".into(),
                    ));
                }
                mat.matcap_strength = v.clamp(0.0, 1.0);
                mat_changed = true;
            }
            if let Some(val) = args.get("hatching_scale").filter(|v| !v.is_null()) {
                let v = val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'hatching_scale' must be a finite number".into())
                })? as f32;
                if !v.is_finite() {
                    return Err(AiError::InvalidArguments(
                        "'hatching_scale' must be finite".into(),
                    ));
                }
                mat.hatching_scale = v.clamp(0.1, 64.0);
                mat_changed = true;
            }
            if let Some(val) = args.get("face_sdf_light_override").filter(|v| !v.is_null()) {
                let arr = val.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("'face_sdf_light_override' must be [x, y, z]".into())
                })?;
                if arr.len() != 3 {
                    return Err(AiError::InvalidArguments(
                        "'face_sdf_light_override' must be [x, y, z]".into(),
                    ));
                }
                let x = arr[0].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("face_sdf_light_override[0] must be number".into())
                })? as f32;
                let y = arr[1].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("face_sdf_light_override[1] must be number".into())
                })? as f32;
                let z = arr[2].as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("face_sdf_light_override[2] must be number".into())
                })? as f32;
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err(AiError::InvalidArguments(
                        "'face_sdf_light_override' must be finite".into(),
                    ));
                }
                mat.face_sdf_light_override = Some([x, y, z]);
                mat_changed = true;
            }
            if mat_changed {
                history.execute(
                    scene,
                    Command::SetMeshMaterial {
                        node_id,
                        material: mat,
                    },
                )?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "slot": slot_str,
                "cleared": clear,
                "width": loaded_dims.map(|d| d.0),
                "height": loaded_dims.map(|d| d.1),
                "matcap_strength": mat.matcap_strength,
                "hatching_scale": mat.hatching_scale,
                "face_sdf_light_override": mat.face_sdf_light_override
            }))
        }
        "mesh_set_uv_seams" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let raw_edges = args
                .get("edges")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'edges' array".into()))?;
            if raw_edges.len() > 65_536 {
                return Err(AiError::InvalidArguments(
                    "'edges' exceeds 65536 limit".into(),
                ));
            }
            let mut edges = Vec::with_capacity(raw_edges.len());
            for entry in raw_edges {
                let pair = entry.as_array().ok_or_else(|| {
                    AiError::InvalidArguments("each seam edge must be [v0, v1]".into())
                })?;
                if pair.len() != 2 {
                    return Err(AiError::InvalidArguments(
                        "each seam edge must have exactly 2 vertex indices".into(),
                    ));
                }
                let a = pair[0].as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("seam vertex index must be non-negative int".into())
                })? as usize;
                let b = pair[1].as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("seam vertex index must be non-negative int".into())
                })? as usize;
                edges.push((a, b));
            }
            let replace = match args.get("replace").filter(|v| !v.is_null()) {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| {
                    AiError::InvalidArguments("'replace' must be a boolean".into())
                })?,
            };
            let unwrap = match args.get("unwrap").filter(|v| !v.is_null()) {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| {
                    AiError::InvalidArguments("'unwrap' must be a boolean".into())
                })?,
            };
            history.execute(
                scene,
                Command::SetMeshUvSeams {
                    node_id,
                    edges,
                    replace,
                    unwrap,
                },
            )?;
            let seam_count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.uv_seams.len());
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "seam_count": seam_count,
                "unwrapped": unwrap
            }))
        }
        "mesh_normalize_texel_density" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let texture_size = match args.get("texture_size").filter(|v| !v.is_null()) {
                None => 256u32,
                Some(v) => {
                    let raw = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments("'texture_size' must be an integer".into())
                    })?;
                    if !(16..=4096).contains(&raw) {
                        return Err(AiError::InvalidArguments(
                            "'texture_size' must be within 16..=4096".into(),
                        ));
                    }
                    raw as u32
                }
            };
            let target_px_per_unit = match args.get("target_px_per_unit").filter(|v| !v.is_null()) {
                None => None,
                Some(v) => {
                    let f = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'target_px_per_unit' must be a number".into())
                    })? as f32;
                    if !f.is_finite() || f <= 0.0 {
                        return Err(AiError::InvalidArguments(
                            "'target_px_per_unit' must be > 0".into(),
                        ));
                    }
                    Some(f)
                }
            };
            let padding_uv = match args.get("padding_uv").filter(|v| !v.is_null()) {
                None => 0.01f32,
                Some(v) => {
                    let f = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'padding_uv' must be a number".into())
                    })? as f32;
                    if !f.is_finite() {
                        return Err(AiError::InvalidArguments(
                            "'padding_uv' must be finite".into(),
                        ));
                    }
                    f.clamp(0.0, 0.05)
                }
            };
            history.execute(
                scene,
                Command::NormalizeMeshTexelDensity {
                    node_id,
                    texture_size,
                    target_px_per_unit,
                    padding_uv,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "texture_size": texture_size,
                "padding_uv": padding_uv
            }))
        }
        "mesh_sculpt_stroke" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let brush_str = args
                .get("brush")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'brush'".into()))?;
            let brush = match brush_str.to_ascii_lowercase().as_str() {
                "smooth" => tessel_core::SculptBrushKind::Smooth,
                "inflate" => tessel_core::SculptBrushKind::Inflate,
                "grab" => tessel_core::SculptBrushKind::Grab,
                "crease" => tessel_core::SculptBrushKind::Crease,
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unknown sculpt brush '{other}', expected smooth|inflate|grab|crease"
                    )))
                }
            };
            let parse_vec3 = |key: &str, default: Option<[f32; 3]>| -> Result<[f32; 3], AiError> {
                match args.get(key).filter(|v| !v.is_null()) {
                    None => default.ok_or_else(|| {
                        AiError::InvalidArguments(format!("missing '{key}' [x, y, z]"))
                    }),
                    Some(val) => {
                        let arr = val.as_array().ok_or_else(|| {
                            AiError::InvalidArguments(format!("'{key}' must be [x, y, z]"))
                        })?;
                        if arr.len() != 3 {
                            return Err(AiError::InvalidArguments(format!(
                                "'{key}' must have 3 numbers"
                            )));
                        }
                        let mut out = [0.0f32; 3];
                        for (i, item) in arr.iter().enumerate() {
                            let f = item.as_f64().ok_or_else(|| {
                                AiError::InvalidArguments(format!("'{key}[{i}]' must be number"))
                            })? as f32;
                            if !f.is_finite() {
                                return Err(AiError::InvalidArguments(format!(
                                    "'{key}[{i}]' must be finite"
                                )));
                            }
                            out[i] = f;
                        }
                        Ok(out)
                    }
                }
            };
            let center = parse_vec3("center", None)?;
            let normal = parse_vec3("normal", Some([0.0, 1.0, 0.0]))?;
            let drag_delta = parse_vec3("drag_delta", Some([0.0, 0.0, 0.0]))?;
            let radius = args
                .get("radius")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .ok_or_else(|| AiError::InvalidArguments("missing finite 'radius'".into()))?;
            if !radius.is_finite() || radius <= 0.0 {
                return Err(AiError::InvalidArguments("'radius' must be > 0".into()));
            }
            let strength = match args.get("strength").filter(|v| !v.is_null()) {
                None => 0.5f32,
                Some(v) => {
                    let f = v.as_f64().ok_or_else(|| {
                        AiError::InvalidArguments("'strength' must be a number".into())
                    })? as f32;
                    if !f.is_finite() {
                        return Err(AiError::InvalidArguments(
                            "'strength' must be finite".into(),
                        ));
                    }
                    f.clamp(-1.0, 1.0)
                }
            };
            let mirror_x = match args.get("mirror_x").filter(|v| !v.is_null()) {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| {
                    AiError::InvalidArguments("'mirror_x' must be a boolean".into())
                })?,
            };
            history.execute(
                scene,
                Command::SculptMeshStroke {
                    node_id,
                    params: tessel_core::SculptStrokeParams {
                        brush,
                        center,
                        normal,
                        drag_delta,
                        radius,
                        strength,
                        mirror_x,
                    },
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "brush": brush_str,
                "radius": radius,
                "strength": strength,
                "mirror_x": mirror_x
            }))
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
                let c =
                    scene.palette.get(idx).map(|e| e.color).ok_or_else(|| {
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
            let set_active = args.get("active").and_then(Value::as_bool).unwrap_or(false);
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
            let mirror_x = match args.get("mirror_x").filter(|v| !v.is_null()) {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| {
                    AiError::InvalidArguments("'mirror_x' must be a boolean".into())
                })?,
            };
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
                    let mirrored_normal = brush_normal.map(|n| glam::Vec3::new(-n.x, n.y, n.z));
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
                    return Err(AiError::InvalidArguments(format!("'{key}' must be finite")));
                }
                Ok([u, v])
            };
            let uv_start = parse_uv("uv_start")?;
            let uv_end = parse_uv("uv_end")?;
            let width_px = args.get("width_px").and_then(Value::as_f64).unwrap_or(4.0) as f32;
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
                    return Err(AiError::InvalidArguments(format!("'{key}' must be finite")));
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
            if ids_arr.len() > MAX_VERTEX_EDIT_BATCH {
                return Err(AiError::InvalidArguments(format!(
                    "'vertex_ids' exceeds maximum of {MAX_VERTEX_EDIT_BATCH}"
                )));
            }
            let mut vertex_ids = Vec::with_capacity(ids_arr.len());
            for id in ids_arr {
                vertex_ids.push(id.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'vertex_ids' must be integers".into())
                })? as usize);
            }
            let delta = parse_opt_vec3(args, "delta")?
                .ok_or_else(|| AiError::InvalidArguments("missing 'delta'".into()))?;
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
            history.execute(scene, Command::WeldMeshVertices { node_id, tolerance })?;
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
            if ids_arr.len() > MAX_VERTEX_EDIT_BATCH {
                return Err(AiError::InvalidArguments(format!(
                    "'vertex_ids' exceeds maximum of {MAX_VERTEX_EDIT_BATCH}"
                )));
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
            let vertex_ids: Vec<usize> =
                if let Some(ids_arr) = args.get("vertex_ids").and_then(Value::as_array) {
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
                        AiError::InvalidArguments(
                            "'emissive' entries must be finite numbers".into(),
                        )
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
                            AiError::InvalidArguments(
                                "'rgba' entries must be finite numbers".into(),
                            )
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
                return Ok(
                    json!({ "ok": true, "node_id": node_id.0, "cleared": true, "updated_vertices": 0 }),
                );
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
            Ok(
                json!({ "ok": true, "node_id": node_id.0, "updated_normals": count, "cleared": clear }),
            )
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
                    if arr.len() > MAX_VERTEX_EDIT_BATCH {
                        return Err(AiError::InvalidArguments(format!(
                            "'{key}' exceeds maximum of {MAX_VERTEX_EDIT_BATCH}"
                        )));
                    }
                    for item in arr {
                        let vi = item
                            .get("vertex")
                            .and_then(Value::as_u64)
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| {
                                AiError::InvalidArguments(format!("invalid {key}.vertex"))
                            })?;
                        let d = parse_opt_vec3(item, "delta")?.ok_or_else(|| {
                            AiError::InvalidArguments(format!("invalid {key}.delta"))
                        })?;
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
            Ok(
                json!({ "ok": true, "node_id": node_id.0, "target": name, "weight": weight.clamp(0.0, 1.0) }),
            )
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
                let idx = item.get("index").and_then(Value::as_u64).ok_or_else(|| {
                    AiError::InvalidArguments("missing control_deltas.index".into())
                })? as usize;
                let d = parse_opt_vec3(item, "delta")?.ok_or_else(|| {
                    AiError::InvalidArguments("missing control_deltas.delta".into())
                })?;
                control_deltas.push((idx, d));
            }
            let floor_y = args
                .get("floor_y")
                .and_then(Value::as_f64)
                .map(|v| v as f32);
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
            let clip_index = parse_opt_usize(args, "clip_index", 0)?;
            let fps = parse_opt_f32_finite(args, "fps", 15.0)?;
            if !(1.0..=120.0).contains(&fps) {
                return Err(AiError::InvalidArguments(
                    "'fps' must be in 1.0..=120.0".into(),
                ));
            }
            let max_frames = parse_opt_usize(args, "max_frames", 120)?;
            if !(1..=512).contains(&max_frames) {
                return Err(AiError::InvalidArguments(
                    "'max_frames' must be in 1..=512".into(),
                ));
            }
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
                if arr.len() > MAX_RIBBON_POINTS {
                    return Err(AiError::InvalidArguments(format!(
                        "'explicit_points' exceeds maximum of {MAX_RIBBON_POINTS}"
                    )));
                }
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
                    let (Some(x), Some(y), Some(z)) = (a[0].as_f64(), a[1].as_f64(), a[2].as_f64())
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
            let clip_index = parse_opt_usize(args, "clip_index", 0)?;
            let start_time = parse_opt_f32_finite(args, "start_time", 0.0)?;
            let end_time = parse_opt_f32_finite(args, "end_time", 1.0)?;
            let samples = parse_opt_usize(args, "samples", 16)?;
            if !(2..=256).contains(&samples) {
                return Err(AiError::InvalidArguments(
                    "'samples' must be in 2..=256".into(),
                ));
            }
            let width_start = parse_opt_f32_finite(args, "width_start", 0.16)?;
            let width_end = parse_opt_f32_finite(args, "width_end", 0.02)?;
            let mat_raw = parse_opt_u32(args, "material", 1)?;
            let mat_idx = u8::try_from(mat_raw)
                .map_err(|_| AiError::InvalidArguments("material must be 1..=255".into()))?;
            let material = PaletteIndex::new(mat_idx)
                .map_err(|_| AiError::InvalidArguments("material must be 1..=255".into()))?;
            let bake_visibility_track =
                parse_opt_bool_default(args, "bake_visibility_track", false)?;
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
        "mesh_clear_texture" => {
            let node_id = args
                .get("node_id")
                .and_then(Value::as_u64)
                .map(NodeId)
                .or_else(|| scene.active_lowpoly_node())
                .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?;
            let target = args
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("color");
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
        "mesh_cleanup" => {
            let node_id = match args.get("node_id") {
                None | Some(Value::Null) => scene
                    .active_lowpoly_node()
                    .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?,
                Some(v) => NodeId(v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'node_id' must be a non-negative integer".into())
                })?),
            };
            let weld_epsilon = parse_opt_f32_finite(args, "weld_epsilon", 1e-4)?;
            if !(1e-5..=1.0).contains(&weld_epsilon) {
                return Err(AiError::InvalidArguments(
                    "'weld_epsilon' must be in 1e-5..=1.0".into(),
                ));
            }
            history.execute(
                scene,
                Command::CleanupMesh {
                    node_id,
                    weld_epsilon,
                },
            )?;
            let mesh = scene.lowpoly_meshes.get(&node_id);
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "vertices": mesh.map_or(0, |m| m.vertices.len()),
                "faces": mesh.map_or(0, |m| m.faces.len())
            }))
        }
        "mesh_add_modifier" => {
            let node_id = match args.get("node_id") {
                None | Some(Value::Null) => scene
                    .active_lowpoly_node()
                    .ok_or_else(|| AiError::InvalidArguments("no low-poly mesh found".into()))?,
                Some(v) => NodeId(v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments("'node_id' must be a non-negative integer".into())
                })?),
            };
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::InvalidArguments("missing 'kind'".into()))?;
            let modifier = match kind {
                "mirror" => {
                    let axis = parse_opt_u32(args, "axis", 0)?;
                    if axis > 2 {
                        return Err(AiError::InvalidArguments(
                            "'axis' must be 0, 1, or 2".into(),
                        ));
                    }
                    tessel_core::MeshModifier::Mirror {
                        axis: axis as u8,
                        enabled: true,
                    }
                }
                "bevel" => {
                    let width = parse_opt_f32_finite(args, "width", 0.05)?;
                    if !(0.001..=0.5).contains(&width) {
                        return Err(AiError::InvalidArguments(
                            "'width' must be in 0.001..=0.5".into(),
                        ));
                    }
                    tessel_core::MeshModifier::Bevel {
                        width,
                        enabled: true,
                    }
                }
                "solidify" => {
                    let thickness = parse_opt_f32_finite(args, "thickness", 0.05)?;
                    if !(-1.0..=1.0).contains(&thickness) {
                        return Err(AiError::InvalidArguments(
                            "'thickness' must be in -1.0..=1.0".into(),
                        ));
                    }
                    tessel_core::MeshModifier::Solidify {
                        thickness,
                        enabled: true,
                    }
                }
                "decimate" => {
                    let ratio = parse_opt_f32_finite(args, "ratio", 0.5)?;
                    if !(0.05..=0.99).contains(&ratio) {
                        return Err(AiError::InvalidArguments(
                            "'ratio' must be in 0.05..=0.99".into(),
                        ));
                    }
                    tessel_core::MeshModifier::Decimate {
                        ratio,
                        enabled: true,
                    }
                }
                other => {
                    return Err(AiError::InvalidArguments(format!(
                        "unsupported modifier kind '{other}'"
                    )))
                }
            };
            history.execute(scene, Command::AddMeshModifier { node_id, modifier })?;
            let count = scene
                .lowpoly_meshes
                .get(&node_id)
                .map_or(0, |m| m.modifiers.len());
            Ok(json!({ "ok": true, "node_id": node_id.0, "modifier_count": count }))
        }
        "mesh_preflight" => {
            let max_tris = parse_opt_usize(args, "max_triangles", 10_000)?;
            if !(1..=500_000).contains(&max_tris) {
                return Err(AiError::InvalidArguments(
                    "'max_triangles' must be in 1..=500000".into(),
                ));
            }
            let report = tessel_core::preflight_check_scene(scene, max_tris);
            Ok(serde_json::to_value(report)?)
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
