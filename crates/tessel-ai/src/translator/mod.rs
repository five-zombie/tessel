mod character;
mod helpers;
mod mesh;
mod pixel;
mod project;
mod scene;
#[cfg(test)]
mod tests;
mod voxel;

use std::path::Path;

use serde_json::{json, Value};
use tessel_core::{HistoryStack, SceneState};

use crate::error::AiError;

pub(crate) use helpers::canonical_export_dir_string;

/// Translates an AI / MCP tool invocation into deterministic `Command` executions on `SceneState`.
pub fn execute_tool_call(
    name: &str,
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &Path,
) -> Result<Value, AiError> {
    if name.starts_with("voxel_") {
        return voxel::execute(name, args, scene, history, allowed_export_dir);
    }
    if name.starts_with("pixel_") {
        return pixel::execute(name, args, scene, history, allowed_export_dir);
    }
    if name.starts_with("mesh_") {
        return mesh::execute(name, args, scene, history, allowed_export_dir);
    }
    if name.starts_with("character_") || name == "scene_create_clip" {
        return character::execute(name, args, scene, history, allowed_export_dir);
    }
    if name.starts_with("project_") {
        return project::execute(name, args, scene, history, allowed_export_dir);
    }
    if name.starts_with("scene_")
        || name.starts_with("palette_")
        || name.starts_with("history_")
        || name == "viewport_set_shading_mode"
        || name == "set_node_transform"
    {
        return scene::execute(name, args, scene, history, allowed_export_dir);
    }
    match name {
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
