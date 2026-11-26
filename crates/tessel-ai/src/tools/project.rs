use serde_json::{json, Value};

pub(super) fn project_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "viewport_set_shading_mode",
            "description": "Switch 3D viewport & screenshot shading preview mode between 'pbr' (default), 'npr_sumie' (ink-wash ramp, dark Fresnel rim, inverted-hull outlines, triplanar washi grain, depth ink-fade), 'silhouette_check', and 'luminance_value_check'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["pbr", "npr_sumie", "silhouette_check", "luminance_value_check"]
                    }
                },
                "required": ["mode"]
            }
        }),
        json!({
            "name": "project_save",
            "description": "Atomically save the current editable scene (nodes, palette, rig, animations) to a .tessel project archive inside the allowed export directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "project_open",
            "description": "Load an editable .tessel project archive from the allowed export directory, replacing the active scene and resetting undo history.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "project_export",
            "description": "Export the current scene inside the allowed export directory. Format matrix: glb/obj/stl export 3D geometry (Voxel/LowPoly/Character); vox exports the active VoxelVolume; png/gif export ONLY the active PixelCanvas (active frame for png, all frames for gif — fails with EmptyCanvas in 3D-only scenes; use viewport_screenshot for 3D PNG or viewport_render_turntable_gif for 3D GIF).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string" },
                    "format": { "type": "string", "enum": ["glb", "obj", "stl", "vox", "png", "gif"] },
                    "preset": { "type": "string", "enum": ["godot4", "unity", "unreal", "threejs", "print3d"] }
                },
                "required": ["filename", "format"]
            }
        }),
        json!({
            "name": "history_undo",
            "description": "Undo the most recent command or AI transaction.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "history_redo",
            "description": "Redo the most recently undone command.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "camera_get",
            "description": "Read the live viewport and scene/projector camera state (target, yaw, pitch, distance). Pass camera: 'viewport' (default) or 'scene'. Requires the live GUI viewport; unavailable headless.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] }
                }
            }
        }),
        json!({
            "name": "camera_set_pose",
            "description": "Set target, yaw_deg (or yaw), pitch_deg (or pitch), and/or distance of the live viewport or scene/projector camera in one smooth call. Requires the live GUI viewport (HTTP bridge); fails in headless MCP sessions and will roll back an entire scene_batch if included.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "yaw_deg": { "type": "number" },
                    "pitch_deg": { "type": "number" },
                    "yaw": { "type": "number" },
                    "pitch": { "type": "number" },
                    "distance": { "type": "number", "minimum": 0.25 },
                    "orthographic": { "type": "boolean", "description": "Use orthographic projection (flat perspective for NPR/turnarounds)" },
                    "telephoto": { "type": "boolean", "description": "Use narrow 22-degree telephoto FOV (false resets to 45 deg)" }
                }
            }
        }),
        json!({
            "name": "camera_look_at",
            "description": "Point the live viewport or scene/projector camera at an absolute world position [x, y, z]. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "camera_zoom",
            "description": "Zoom the live viewport or scene/projector camera. Pass factor (> 0, < 1 zooms in) or an absolute distance (> 0). Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "factor": { "type": "number" },
                    "distance": { "type": "number", "minimum": 0 }
                }
            }
        }),
        json!({
            "name": "camera_orbit",
            "description": "Orbit the live viewport or scene/projector camera by pixel deltas (dx turns yaw, dy turns pitch). Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "dx": { "type": "number" },
                    "dy": { "type": "number" }
                },
                "required": ["dx", "dy"]
            }
        }),
        json!({
            "name": "camera_pan",
            "description": "Pan the live viewport or scene/projector camera target by pixel deltas. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "dx": { "type": "number" },
                    "dy": { "type": "number" }
                },
                "required": ["dx", "dy"]
            }
        }),
        json!({
            "name": "camera_reset",
            "description": "Reset the live viewport or scene/projector camera to its default view. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] }
                }
            }
        }),
        json!({
            "name": "camera_preset",
            "description": "Move the live viewport or scene/projector camera to a named preset view. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "camera": { "type": "string", "enum": ["viewport", "scene"] },
                    "name": { "type": "string", "enum": ["iso", "front", "back", "right", "left", "top"] }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "viewport_set_preview",
            "description": "Toggle clean Render Preview mode (hides wireframe overlay and editor gizmos in the live GUI viewport and tessel://viewport/preview). Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preview": { "type": "boolean" }
                },
                "required": ["preview"]
            }
        }),
        json!({
            "name": "viewport_screenshot",
            "description": "Render the current scene offscreen (headless GPU) and return an sRGB PNG image. Output is a two-block MCP content array: content[0] is a non-JSON text summary ('viewport WxH sRGB PNG') and content[1] is an image block (base64 PNG). Prefer yaw_deg/pitch_deg (degrees); yaw/pitch are in radians. Needs a GPU (software GL works).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "minimum": 16, "maximum": 1024 },
                    "height": { "type": "integer", "minimum": 16, "maximum": 1024 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "yaw_deg": { "type": "number", "description": "Camera yaw in degrees (e.g. 25)" },
                    "pitch_deg": { "type": "number", "description": "Camera pitch in degrees (e.g. 15)" },
                    "yaw": { "type": "number", "description": "Camera yaw in radians (use yaw_deg for degrees)" },
                    "pitch": { "type": "number", "description": "Camera pitch in radians (use pitch_deg for degrees)" },
                    "distance": { "type": "number", "minimum": 0.25 },
                    "preview": { "type": "boolean" },
                    "orthographic": { "type": "boolean", "description": "Render with orthographic projection" },
                    "telephoto": { "type": "boolean", "description": "Render with narrow 22-degree telephoto FOV" }
                }
            }
        }),
        json!({
            "name": "viewport_render_turntable_gif",
            "description": "Render a 360-degree 3D turntable animated .gif (optionally playing clip_name/clip_index across the loop) and save it inside the allowed export directory (relative filename or absolute path inside exports/). Prefer yaw_deg/pitch_deg (degrees); yaw/pitch are in radians.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string", "description": "Relative filename inside exports/ (e.g. 'turntable.gif')" },
                    "width": { "type": "integer", "minimum": 64, "maximum": 512 },
                    "height": { "type": "integer", "minimum": 64, "maximum": 512 },
                    "frames": { "type": "integer", "minimum": 4, "maximum": 36 },
                    "fps": { "type": "integer", "minimum": 2, "maximum": 30 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "yaw_deg": { "type": "number", "description": "Starting camera yaw in degrees" },
                    "pitch_deg": { "type": "number", "description": "Camera pitch in degrees" },
                    "yaw": { "type": "number", "description": "Starting camera yaw in radians (use yaw_deg for degrees)" },
                    "pitch": { "type": "number", "description": "Camera pitch in radians (use pitch_deg for degrees)" },
                    "distance": { "type": "number", "minimum": 0.25 },
                    "preview": { "type": "boolean" },
                    "orthographic": { "type": "boolean", "description": "Render turntable with orthographic projection" },
                    "telephoto": { "type": "boolean", "description": "Render turntable with narrow 22-degree telephoto FOV" },
                    "clip_name": { "type": "string" },
                    "clip_index": { "type": "integer", "minimum": 0 }
                },
                "required": ["filename"]
            }
        }),
    ]
}
