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
Tessel is a 3D & 2D asset creation studio (Pixel, Voxel, Low-Poly, Character) using a Right-Handed +Y Up, +Z Front coordinate system (1 voxel = 0.1m; Left = +X, Right = -X).
Recommended workflow:
1. Call `scene_inspect` (or read `tessel://scene/summary` and `tessel://palette/current`) before editing, or call `scene_clear` to start fresh. Add custom colors to slots 1..=255 with `palette_set_color` (`rgb` or `hex`). Use `scene_batch` (up to 512 calls, atomic rollback, supports `\"$N.field\"` chaining like `\"$0.node_id\"`) for fast multi-step builds, `scene_rename_node` to label parts, `scene_set_node_transform` to pose nodes, `scene_set_parent` to group, and `scene_set_lighting` for studio key light.
2. Voxel mode: use `voxel_fill_box` and `voxel_fill_ellipsoid` (bounds ±256), then optionally `voxel_convert_to_lowpoly`.
3. Low-Poly mode: build rotational shapes (helmets, visors, boots, domes) with `mesh_create_lathe` (`profile`, `segments`, `angle_deg`, `cap_ends`), build flat/extruded silhouettes (neckties, armor plates, buckles, weapons) with `mesh_create_extruded_polygon` (`points`, `depth`), build tapered organic shapes (torsos, limbs, backpacks, stocks) with `mesh_create_loft` (`sections`, `cap_ends`), create bespoke geometry via `mesh_create_raw` (`vertices`, `faces` with per-face `palette_index` & optional `uvs`), compose primitives via `mesh_create_primitive` (`cube`, `plane`, `cylinder`, `cone`, `sphere`, `torus`, `capsule`; `size` accepts scalar >= 0.005 or `[sx, sy, sz]`; bottom-anchored), generate curved straps/cables with `mesh_create_curve_tube`, join parts with `mesh_join`, bake transforms before mirroring/bending via `mesh_apply_transform`, mirror symmetrically with `mesh_mirror`, collapse center seams with `mesh_weld_vertices`, fix inside-out faces with `mesh_flip_normals`, deform with `mesh_taper` / `mesh_bend`, shift pivots with `mesh_set_origin`, subdivide locally with `mesh_subdivide_face`, paint per-face colors with `mesh_set_face_material`, batch-update vertices with `mesh_set_vertices`, inspect with `mesh_inspect`, bevel/inset/extrude (`mesh_inset_face`, `mesh_extrude_face`, `mesh_bevel`), sculpt (`mesh_translate_face`, `mesh_move_vertices`, `mesh_set_vertex`), author blend shapes (`mesh_morph_delta` with `vertex_ids` or spatial `bounds_min`/`bounds_max`), project UVs (`mesh_project_uvs`), paint 3D textures (`mesh_fill_texture`, `mesh_paint_spot` with `mirror_x: true`, `mesh_paint_line`, `mesh_paint_rect`, `mesh_load_texture`, `mesh_load_normal_map`), tune PBR (`mesh_set_material`), and smooth (`mesh_smooth_subdivide`, `mesh_set_shading`).
4. Character & Prop Rigging mode: call `character_apply_biped_rig` (`preset: \"starter\"` 7-bone or `preset: \"humanoid\"` 19-bone with built-in Idle & Walk; Left=+X, Right=-X; pass `append: true`, `prefix`, `offset`, `yaw_deg`, `include_default_clips: true` for multi-character scenes), adjust rest positions (`character_set_bone_rest_position`), isolate limbs (`character_set_node_bone_mask`), bind rigid gear (`character_bind_node`), inspect weights (`character_inspect_weights`), pose (`character_set_bone_pose` with `rotation_deg` and/or `translation`, `character_clamp_to_floor`, `character_solve_ik` with `pole_target` world point or `pole_vector` direction + `end_world_rotation_deg` world aim), bake live/IK poses into keyframes (`character_snapshot_pose_to_keyframe` with `interpolation`), clamp in-between clip frames to floor (`character_clamp_clip_to_floor`), layer per-character clips (`character_set_clip_layer`) and bake them into one exportable clip (`character_bake_clip_layers`), toggle prop/muzzle visibility over time (`character_set_visibility_keyframe`), scrub live animations (`character_set_playback_time`, `character_reset_pose`), author keyframes (`character_set_keyframe`), and animate blend shapes (`character_set_morph_weight`). Both live viewport rendering and `viewport_screenshot` deform meshes in real time.
5. Verify visually with `viewport_screenshot` (`width`, `height`, `target`, `yaw_deg`/`pitch_deg` in degrees or `yaw`/`pitch` in radians, `distance >= 0.25`, `preview: true` for clean PBR without wireframe) or render 360° animated showcases with `viewport_render_turntable_gif` (`filename`, `frames`, `fps`, `clip_name`). Save editable projects with `project_save` / `project_open` (`.tessel`) and export game-ready assets with `project_export` (`.glb` with automatic PBR material deduplication, `.obj`, `.stl`, `.vox`, `.png`, `.gif`).
6. Live GUI viewport only: frame subjects smoothly with `camera_get`, `camera_set_pose`, `camera_look_at`, `camera_zoom`, `camera_orbit`, `camera_pan`, `camera_preset`, `camera_reset` (pass `camera: \"viewport\" | \"scene\"` to move either the live viewport or the 3D projector camera), and `viewport_set_preview`.";

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
                "tools": { "listChanged": true },
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
            let started = std::time::Instant::now();
            match execute_tool_call(tool_name, &args, scene, history, allowed_export_dir) {
                Ok(val) => {
                    tracing::debug!(
                        tool = tool_name,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        is_error = false,
                        "MCP tool call succeeded"
                    );
                    Ok(json!({
                        "content": [{ "type": "text", "text": val.to_string() }],
                        "isError": false
                    }))
                }
                Err(err) => {
                    tracing::warn!(
                        tool = tool_name,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        error = %err,
                        "MCP tool call failed"
                    );
                    Ok(json!({
                        "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                        "isError": true
                    }))
                }
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
                    "name": "create-lowpoly-character",
                    "description": "Skill: Build a 1:1 low-poly humanoid character using mesh_create_raw, mesh_apply_transform + mesh_mirror + mesh_weld_vertices, 19-bone humanoid rig, bone masks, live animation scrubbing, and .tessel/.glb export.",
                    "arguments": [
                        { "name": "character_concept", "description": "Character theme (e.g. tactical survivor, sci-fi trooper)", "required": false }
                    ]
                },
                {
                    "name": "create-voxel-character",
                    "description": "Skill: Build a proportional voxel humanoid character, apply biped rig, inspect preview, and export .glb.",
                    "arguments": [
                        { "name": "character_concept", "description": "Character theme (e.g. knight, robot, mage)", "required": false }
                    ]
                },
                {
                    "name": "create-lowpoly-prop",
                    "description": "Skill: Compose a game-ready low-poly prop using positioned primitives/sculpting, PBR materials/textures, and export .glb.",
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
                    "description": "Skill: Paint symmetrical 3D textures or load custom albedo/normal maps, tune PBR material, smooth subdivide + Shade Smooth, and export .glb.",
                    "arguments": [
                        { "name": "pattern_description", "description": "Desired surface pattern or colors", "required": false }
                    ]
                },
                {
                    "name": "rig-and-animate-character",
                    "description": "Skill: Attach the biped skeleton, pose bones, author custom keyframe animations, and export a smooth-skinned .glb for Unity/Godot/Unreal/Blender.",
                    "arguments": [
                        { "name": "animation_style", "description": "Desired animation (e.g. wave, attack, run, idle)", "required": false }
                    ]
                }
            ]
        })),
        "prompts/get" => {
            let prompt_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match prompt_name {
                "create-lowpoly-character" => {
                    let concept = args
                        .get("character_concept")
                        .and_then(Value::as_str)
                        .unwrap_or("1:1 tactical low-poly character");
                    Ok(json!({
                        "description": "Tessel 1:1 Low-Poly Character Creation Skill",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Create a 1:1 low-poly '{concept}' in Tessel (~1.82m tall, Right-Handed +Y Up, +Z Front, Left=+X, Right=-X).\n\
                                     1. Call `scene_clear` and populate custom palette colors with `palette_set_color` (batch up to 512 calls with `scene_batch`).\n\
                                     2. Build helmets/visors/boots with `mesh_create_lathe`, neckties/plates/weapons with `mesh_create_extruded_polygon`, scaled primitives with `mesh_create_primitive` (`size: [sx, sy, sz]`), and bespoke anatomy with `mesh_create_raw` (per-face `palette_index`); build left limbs once, then run `mesh_join` -> `mesh_apply_transform` -> `mesh_mirror` (`axis: \"x\"`) -> `mesh_weld_vertices`.\n\
                                     3. Apply `character_apply_biped_rig` (`preset: \"humanoid\"`, 19 bones with built-in Idle & Walk), rigidly bind helmets/props via `character_bind_node`, isolate trousers/torso from arms via `character_set_node_bone_mask`, and pose/bake IK with `character_solve_ik` (`pole_target`, `end_world_rotation_deg`) + `character_snapshot_pose_to_keyframe`.\n\
                                     4. Scrub `character_set_playback_time` (e.g. `Walk` at `0.25`) and verify visually with `viewport_screenshot` (`yaw_deg`, `pitch_deg`, `preview: true`) or export a 360° showcase via `viewport_render_turntable_gif`.\n\
                                     5. Save editable `.tessel` via `project_save` and export game-ready `.glb` via `project_export`."
                                )
                            }
                        }]
                    }))
                }
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
                                     4. Call `character_apply_biped_rig` to attach the starter skeleton, and optionally pose/animate with `character_set_bone_pose` and `character_set_keyframe`.\n\
                                     5. Inspect `tessel://viewport/preview` if running in GUI, then call `project_export` with format `glb` (auto-weight smooth skinning `skins` + `JOINTS_0`/`WEIGHTS_0` is embedded automatically)."
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
                                     1. Read `tessel://palette/current` and delete unwanted starter nodes with `scene_delete_node`.\n\
                                     2. Compose multi-part primitives with `mesh_create_primitive` using `position`, `rotation_deg`, and `scale` (or adjust existing nodes with `scene_set_node_transform` and sculpt faces with `mesh_translate_face`), or import `.obj`/`.glb` via `mesh_import`.\n\
                                     3. Tune PBR finish via `mesh_set_material` (`metallic`, `roughness`, `emissive`) and optionally load `.png`/`.jpg` textures or normal maps via `mesh_load_texture` / `mesh_load_normal_map`.\n\
                                     4. Export via `project_export` (.glb) — all node transforms are baked cleanly into the exported mesh."
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
                                     2. Call `mesh_fill_texture` for a base coat (or `mesh_load_texture` / `mesh_load_normal_map` for custom `.png`/`.jpg` PBR maps).\n\
                                     3. Call `mesh_paint_spot` with 3D `point` + outward `normal`, `radius_px`, and `mirror_x: true` for symmetrical markings, stripes, or eyes.\n\
                                     4. Tune `mesh_set_material` (`metallic`, `roughness`, `emissive`), then call `mesh_smooth_subdivide` (1-2 iterations) and `mesh_set_shading` (`smooth: true`).\n\
                                     5. Call `project_export` with `format: \"glb\"` to embed the painted PNG texture and PBR maps."
                                )
                            }
                        }]
                    }))
                }
                "rig-and-animate-character" => {
                    let style = args
                        .get("animation_style")
                        .and_then(Value::as_str)
                        .unwrap_or("idle and wave");
                    Ok(json!({
                        "description": "Tessel Character Rigging & Smooth Skinning Animation Skill",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "Rig and animate the active character with '{style}' in Tessel:\n\
                                     1. Read `tessel://scene/summary` to confirm the character mesh stands on y=0 facing +Z (adjust with `scene_set_node_transform` if needed).\n\
                                     2. Call `character_apply_biped_rig` to attach the 7-bone skeleton (0:Root, 1:Torso, 2:Head, 3:LeftArm, 4:RightArm, 5:LeftLeg, 6:RightLeg).\n\
                                     3. Use `character_set_bone_pose` to verify joint angles in degrees `[pitch, yaw, roll]`.\n\
                                     4. Author custom keyframes with `character_set_keyframe` (`clip_name`, `duration_secs`, `bone_id`, `time_secs`, `rotation_deg`).\n\
                                     5. Inspect `tessel://viewport/preview` and export via `project_export` (`format: \"glb\"`), which automatically embeds smooth auto-weight skinning (`skins`, `JOINTS_0`, `WEIGHTS_0`, `inverseBindMatrices`) and animation clips."
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
    headless: Option<crate::headless::HeadlessRenderer>,
}

impl McpServer {
    pub fn new(allowed_export_dir: PathBuf) -> Self {
        Self {
            scene: SceneState::new(),
            history: HistoryStack::new(),
            allowed_export_dir,
            headless: None,
        }
    }

    /// Processes a single JSON-RPC 2.0 request and returns an optional response (`None` for notifications).
    pub fn handle_message(&mut self, request: &Value) -> Option<Value> {
        if request.get("method").and_then(Value::as_str) == Some("tools/call") {
            match request
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
            {
                Some("viewport_screenshot") => return Some(self.call_viewport_screenshot(request)),
                Some("viewport_render_turntable_gif") => {
                    return Some(self.call_viewport_turntable_gif(request));
                }
                _ => {}
            }
        }
        dispatch_mcp_message(
            request,
            &mut self.scene,
            &mut self.history,
            &self.allowed_export_dir,
        )
    }

    /// Renders the current scene offscreen (headless GPU) and returns the PNG
    /// as an MCP image content block. The renderer is created lazily so
    /// machines without a GPU keep working for every other tool.
    fn call_viewport_screenshot(&mut self, request: &Value) -> Value {
        use crate::error::AiError;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let args = request
            .get("params")
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = (|| -> Result<Value, AiError> {
            let w = args
                .get("width")
                .and_then(Value::as_u64)
                .unwrap_or(640)
                .clamp(16, 1024) as u32;
            let h = args
                .get("height")
                .and_then(Value::as_u64)
                .unwrap_or(480)
                .clamp(16, 1024) as u32;
            let preview = args
                .get("preview")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let cam = crate::headless::screenshot_camera(&args)?;
            if self.headless.is_none() {
                self.headless = Some(crate::headless::HeadlessRenderer::new(640, 480)?);
            }
            let png = self
                .headless
                .as_mut()
                .expect("headless renderer just created")
                .render_png(&self.scene, &cam, w, h, preview)?;
            Ok(json!([
                { "type": "text", "text": format!("viewport {w}x{h} sRGB PNG") },
                { "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png" },
            ]))
        })();
        match result {
            Ok(content) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": content, "isError": false },
            }),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                    "isError": true,
                },
            }),
        }
    }

    /// Renders a 360-degree animated turntable GIF offscreen and saves it inside `allowed_export_dir`.
    fn call_viewport_turntable_gif(&mut self, request: &Value) -> Value {
        use crate::error::AiError;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let args = request
            .get("params")
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = (|| -> Result<Value, AiError> {
            let parsed = crate::headless::parse_turntable_args(&self.allowed_export_dir, &args)?;
            if self.headless.is_none() {
                self.headless = Some(crate::headless::HeadlessRenderer::new(320, 320)?);
            }
            let Some(renderer) = self.headless.as_mut() else {
                return Err(AiError::InvalidArguments(
                    "headless renderer unavailable".into(),
                ));
            };
            renderer.render_turntable_gif(&self.scene, &parsed)
        })();
        match result {
            Ok(val) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": val.to_string() }],
                    "isError": false,
                },
            }),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                    "isError": true,
                },
            }),
        }
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
    fn mcp_viewport_screenshot_needs_gpu_but_validates_args_first() {
        let export_dir = std::env::temp_dir().join("tessel_mcp_shot_test");
        let _ = std::fs::remove_dir_all(&export_dir);
        let mut server = McpServer::new(export_dir);

        // Bad camera args fail before any GPU is touched.
        let bad = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "viewport_screenshot",
                    "arguments": { "target": [0.0, 0.0] }
                }
            }))
            .unwrap();
        assert_eq!(bad["result"]["isError"], true);
        assert!(bad["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("target"));

        // A real render needs a GPU; CI sandboxes without one get a clear
        // error instead of a hang (run with a GPU, e.g. lavapipe, for pixels).
        let shot = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "viewport_screenshot",
                    "arguments": { "width": 64, "height": 64 }
                }
            }))
            .unwrap();
        if shot["result"]["isError"] == true {
            let msg = shot["result"]["content"][0]["text"].as_str().unwrap();
            assert!(
                msg.contains("GPU"),
                "GPU-less failure must say so, got: {msg}"
            );
            eprintln!("SKIP pixels: no GPU adapter in this environment");
            return;
        }
        let content = shot["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        let b64 = content[1]["data"].as_str().unwrap();
        assert_eq!(content[1]["mimeType"], "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(&bytes[1..4], b"PNG", "must be a real PNG file");
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
        assert!(instructions.contains("scene_set_node_transform"));
        assert!(instructions.contains("mesh_translate_face"));
        assert!(instructions.contains("mirror_x"));
        assert!(instructions.contains("mesh_paint_spot"));
        assert!(instructions.contains("mesh_smooth_subdivide"));
        assert!(instructions.contains("mesh_create_lathe"));
        assert!(instructions.contains("mesh_create_extruded_polygon"));
        assert!(instructions.contains("character_snapshot_pose_to_keyframe"));
        assert!(instructions.contains("end_world_rotation_deg"));
        assert!(instructions.contains("viewport_render_turntable_gif"));
        assert!(instructions.contains("512"));

        let prompts = list["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 6);

        let lp_skill = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "prompts/get",
                "params": {
                    "name": "create-lowpoly-character",
                    "arguments": { "character_concept": "PUBG Lone Survivor" }
                }
            }))
            .unwrap();
        let lp_text = lp_skill["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(lp_text.contains("PUBG Lone Survivor"));
        assert!(lp_text.contains("mesh_create_raw"));
        assert!(lp_text.contains("mesh_create_lathe"));
        assert!(lp_text.contains("mesh_create_extruded_polygon"));
        assert!(lp_text.contains("character_snapshot_pose_to_keyframe"));
        assert!(lp_text.contains("viewport_render_turntable_gif"));
        assert!(lp_text.contains("character_set_node_bone_mask"));

        let rig_skill = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "prompts/get",
                "params": {
                    "name": "rig-and-animate-character",
                    "arguments": { "animation_style": "combat ready" }
                }
            }))
            .unwrap();
        let rig_text = rig_skill["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(rig_text.contains("combat ready"));
        assert!(rig_text.contains("character_set_keyframe"));

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
