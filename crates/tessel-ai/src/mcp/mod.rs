pub mod http;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use base64::Engine as _;
use serde_json::{json, Value};
use tessel_core::{HistoryStack, SceneState};

use crate::tools::tool_definitions;
use crate::translator::{execute_tool_call, palette_summary_json, scene_summary_json};

pub use http::{LiveMcpRequest, LiveMcpServerHandle};

const MCP_SERVER_INSTRUCTIONS: &str = "\
Tessel is a 3D & 2D asset creation studio (Pixel, Voxel, Low-Poly, Character) using a Right-Handed +Y Up, +Z Front coordinate system (1 voxel = 0.1m).
Recommended workflow:
1. Inspect `tessel://scene/summary` and `tessel://palette/current` before editing so you know existing node IDs and palette indices (1..=255).
2. Remove unwanted starter nodes (such as 'Starter Voxel') with `scene_delete_node` before sculpting a new asset.
3. Voxel mode: use `voxel_fill_box` and `voxel_fill_ellipsoid` (bounds ±256), then optionally `voxel_convert_to_lowpoly`.
4. Low-Poly mode: create primitives (`mesh_create_primitive`) or import `.obj`/`.glb` (`mesh_import`). To apply custom `.png`/`.jpg` textures (wood, fur, fabric), call `mesh_load_texture`. To paint patterns/spots, call `mesh_fill_texture` then `mesh_paint_spot` with 3D `point` + `normal` (or `uv`) and `radius_px`. Adjust surface finish (metallic/roughness/emissive glow) with `mesh_set_material`. To round sharp corners, call `mesh_smooth_subdivide` (1-3 iterations) and `mesh_set_shading` (`smooth: true`).
5. Character mode: call `character_apply_biped_rig`, pose bones with `character_set_bone_pose`, and author custom animations with `character_set_keyframe`.
6. Read `tessel://viewport/preview` (when GUI is running) to visually inspect results, use `history_undo`/`history_redo` if needed, and export via `project_export` (`.glb`, `.obj`, `.stl`, `.vox`, `.png`, `.gif`).";

/// Processes a single JSON-RPC 2.0 MCP request directly against any live `SceneState` & `HistoryStack`.
pub fn dispatch_mcp_message(
    request: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &std::path::Path,
) -> Option<Value> {
    dispatch_mcp_message_with_preview(request, scene, history, allowed_export_dir, None)
}

/// Processes a single JSON-RPC 2.0 MCP request with an optional PNG viewport snapshot for `tessel://viewport/preview`.
pub fn dispatch_mcp_message_with_preview(
    request: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &std::path::Path,
    preview_png: Option<&[u8]>,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    if method.starts_with("notifications/") {
        return None;
    }

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {}
            },
            "serverInfo": {
                "name": "tessel-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": MCP_SERVER_INSTRUCTIONS
        })),
        "tools/list" => Ok(json!({
            "tools": tool_definitions()
        })),
        "tools/call" => {
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match execute_tool_call(tool_name, &args, scene, history, allowed_export_dir) {
                Ok(val) => Ok(json!({
                    "content": [{ "type": "text", "text": val.to_string() }],
                    "isError": false
                })),
                Err(err) => Ok(json!({
                    "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                    "isError": true
                })),
            }
        }
        "resources/list" => Ok(json!({
            "resources": [
                {
                    "uri": "tessel://scene/summary",
                    "name": "Scene Summary",
                    "mimeType": "application/json"
                },
                {
                    "uri": "tessel://palette/current",
                    "name": "Active Palette",
                    "mimeType": "application/json"
                },
                {
                    "uri": "tessel://viewport/preview",
                    "name": "Viewport Preview Snapshot",
                    "mimeType": "image/png"
                }
            ]
        })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            match uri {
                "tessel://scene/summary" => Ok(json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": scene_summary_json(scene).to_string()
                    }]
                })),
                "tessel://palette/current" => Ok(json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": palette_summary_json(scene).to_string()
                    }]
                })),
                "tessel://viewport/preview" => match preview_png {
                    Some(bytes) => Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "image/png",
                            "blob": base64::engine::general_purpose::STANDARD.encode(bytes)
                        }]
                    })),
                    None => Err((
                        -32603,
                        "viewport preview is only available when the Tessel GUI is running".into(),
                    )),
                },
                _ => Err((-32602, format!("unknown resource URI '{uri}'"))),
            }
        }
        "prompts/list" => Ok(json!({
            "prompts": [
                {
                    "name": "create-voxel-character",
                    "description": "Skill: Build a proportional voxel humanoid character, apply biped rig, inspect preview, and export .glb.",
                    "arguments": [
                        { "name": "character_concept", "description": "Character theme (e.g. knight, robot, mage)", "required": false }
                    ]
                },
                {
                    "name": "create-lowpoly-prop",
                    "description": "Skill: Create a clean game-ready low-poly prop and export .glb.",
                    "arguments": [
                        { "name": "prop_name", "description": "Prop description", "required": false }
                    ]
                },
                {
                    "name": "inspect-and-refine",
                    "description": "Skill: Inspect active scene summary, palette, and viewport PNG before refining.",
                    "arguments": []
                },
                {
                    "name": "paint-and-smooth-lowpoly",
                    "description": "Skill: Paint custom patterns/spots onto a Low-Poly mesh via auto-UV texture painting, apply smooth subdivision + Shade Smooth, and export .glb.",
                    "arguments": [
                        { "name": "pattern_description", "description": "Desired surface pattern or colors", "required": false }
                    ]
                }
            ]
        })),
        "prompts/get" => {
            let prompt_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match prompt_name {
                "create-voxel-character" => {
                    let concept = args
                        .get("character_concept")
                        .and_then(Value::as_str)
                        .unwrap_or("stylized voxel adventurer");
                    Ok(json!({
                        "description": "Tessel Voxel Character Creation Skill",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Create a '{concept}' in Tessel.\n\
                                     Rules:\n\
                                     1. Coordinate system is Right-Handed, +Y Up, +Z Front, 1 voxel = 0.1m.\n\
                                     2. Read `tessel://palette/current` first so colors match the active palette.\n\
                                     3. Use `voxel_fill_box` and `voxel_fill_ellipsoid` to build legs (y=0..6), torso (y=7..13), arms, and head (y=14..18) facing +Z.\n\
                                     4. Call `character_apply_biped_rig` to attach the starter skeleton.\n\
                                     5. Inspect `tessel://viewport/preview` if running in GUI, then call `project_export` with format `glb`."
                                )
                            }
                        }]
                    }))
                }
                "create-lowpoly-prop" => {
                    let prop = args
                        .get("prop_name")
                        .and_then(Value::as_str)
                        .unwrap_or("game prop");
                    Ok(json!({
                        "description": "Tessel Low-Poly Prop Skill",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Build a low-poly '{prop}' in Tessel.\n\
                                     1. Read `tessel://palette/current`.\n\
                                     2. Either compose primitives with `mesh_create_primitive` or block out voxels and run `voxel_convert_to_lowpoly`.\n\
                                     3. Export via `project_export` (.glb)."
                                )
                            }
                        }]
                    }))
                }
                "inspect-and-refine" => Ok(json!({
                    "description": "Tessel Scene Inspection Skill",
                    "messages": [{
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "Read `tessel://scene/summary`, `tessel://palette/current`, and `tessel://viewport/preview`. Summarize proportions and suggest or apply targeted voxel/mesh refinements."
                        }
                    }]
                })),
                "paint-and-smooth-lowpoly" => {
                    let pattern = args
                        .get("pattern_description")
                        .and_then(Value::as_str)
                        .unwrap_or("custom painted surface pattern");
                    Ok(json!({
                        "description": "Tessel Low-Poly 3D Texture Painting & Smoothing Skill",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Paint and smooth the active Low-Poly mesh with '{pattern}':\n\
                                     1. Read `tessel://scene/summary` and `tessel://palette/current` to get the target `node_id` and palette indices.\n\
                                     2. Call `mesh_fill_texture` to apply the base coat color.\n\
                                     3. Call `mesh_paint_spot` with 3D `point` + outward `normal` and `radius_px` to paint markings, stripes, or circular spots.\n\
                                     4. Call `mesh_smooth_subdivide` (1-2 iterations) and `mesh_set_shading` (`smooth: true`) to round sharp corners.\n\
                                     5. Call `project_export` with `format: \"glb\"` to embed the painted PNG texture."
                                )
                            }
                        }]
                    }))
                }
                other => Err((-32602, format!("unknown prompt '{other}'"))),
            }
        }
        other => Err((-32601, format!("method not found: '{other}'"))),
    };

    let response = match result {
        Ok(res) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": res
        }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": { "code": code, "message": message }
        }),
    };

    Some(response)
}

/// Headless JSON-RPC 2.0 MCP Server driving `SceneState` through the single Command Bus.
pub struct McpServer {
    pub scene: SceneState,
    pub history: HistoryStack,
    pub allowed_export_dir: PathBuf,
}

impl McpServer {
    pub fn new(allowed_export_dir: PathBuf) -> Self {
        Self {
            scene: SceneState::new(),
            history: HistoryStack::new(),
            allowed_export_dir,
        }
    }

    /// Processes a single JSON-RPC 2.0 request and returns an optional response (`None` for notifications).
    pub fn handle_message(&mut self, request: &Value) -> Option<Value> {
        dispatch_mcp_message(
            request,
            &mut self.scene,
            &mut self.history,
            &self.allowed_export_dir,
        )
    }

    /// Runs the line-delimited JSON-RPC stdio transport loop until EOF.
    pub fn run_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(trimmed) {
                Ok(req) => self.handle_message(&req),
                Err(err) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {err}") }
                })),
            };
            if let Some(resp) = response {
                writeln!(stdout, "{resp}")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_lifecycle_fill_box_and_export_glb() {
        let export_dir = std::env::temp_dir().join("tessel_mcp_test");
        let _ = std::fs::remove_dir_all(&export_dir);
        let mut server = McpServer::new(export_dir.clone());

        let init = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "tessel-mcp");

        let list = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap();
        assert!(!list["result"]["tools"].as_array().unwrap().is_empty());

        let fill = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "voxel_fill_box",
                    "arguments": {
                        "min": [0, 0, 0],
                        "max": [2, 2, 2],
                        "palette_index": 4
                    }
                }
            }))
            .unwrap();
        assert_eq!(fill["result"]["isError"], false);

        let exp = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "project_export",
                    "arguments": {
                        "filename": "mcp_cube.glb",
                        "format": "glb",
                        "preset": "godot4"
                    }
                }
            }))
            .unwrap();
        assert_eq!(exp["result"]["isError"], false);
        assert!(export_dir.join("mcp_cube.glb").exists());

        // Path traversal must be rejected cleanly
        let denied = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "project_export",
                    "arguments": {
                        "filename": "../escape.glb",
                        "format": "glb"
                    }
                }
            }))
            .unwrap();
        assert_eq!(denied["result"]["isError"], true);
        assert!(denied["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("EXPORT_PATH_DENIED"));

        let _ = std::fs::remove_dir_all(&export_dir);
    }

    #[test]
    fn mcp_rejects_out_of_bounds_and_excessive_batches() {
        let mut server = McpServer::new(std::env::temp_dir());
        let oob = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "voxel_fill_box",
                    "arguments": {
                        "min": [0, 0, 0],
                        "max": [300, 1, 1],
                        "palette_index": 4
                    }
                }
            }))
            .unwrap();
        assert_eq!(oob["result"]["isError"], true);
        assert!(oob["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("OUT_OF_BOUNDS"));
    }

    #[test]
    fn mcp_viewport_preview_returns_base64_png_when_available() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();
        let fake_png = [0x89u8, 0x50, 0x4E, 0x47];
        let resp = dispatch_mcp_message_with_preview(
            &json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "resources/read",
                "params": { "uri": "tessel://viewport/preview" }
            }),
            &mut scene,
            &mut history,
            &dir,
            Some(&fake_png),
        )
        .unwrap();
        assert_eq!(resp["result"]["contents"][0]["mimeType"], "image/png");
        assert_eq!(resp["result"]["contents"][0]["blob"], "iVBORw==");
    }

    #[test]
    fn mcp_prompts_list_and_get_return_skills() {
        let mut server = McpServer::new(std::env::temp_dir());
        let list = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "prompts/list"
            }))
            .unwrap();
        let init = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "initialize"
            }))
            .unwrap();
        let instructions = init["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("mesh_paint_spot"));
        assert!(instructions.contains("mesh_smooth_subdivide"));

        let prompts = list["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 4);

        let get = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "prompts/get",
                "params": {
                    "name": "create-voxel-character",
                    "arguments": { "character_concept": "cyber knight" }
                }
            }))
            .unwrap();
        let text = get["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("cyber knight"));
        assert!(text.contains("voxel_fill_box"));
    }
}
