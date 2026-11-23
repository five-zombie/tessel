use serde_json::{json, Value};

/// Single source of truth for tool schemas shared by both Direct AI and MCP `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "scene_set_mode",
            "description": "Switch the active Tessel workspace mode (pixel, voxel, lowpoly, character).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["pixel", "voxel", "lowpoly", "character"] }
                },
                "required": ["mode"]
            }
        }),
        json!({
            "name": "scene_delete_node",
            "description": "Delete a scene node by its node_id (supports Undo/Redo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "voxel_fill_box",
            "description": "Fill an axis-aligned 3D voxel box (solid or hollow) in the active or specified voxel node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "min": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "max": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "hollow": { "type": "boolean" }
                },
                "required": ["min", "max", "palette_index"]
            }
        }),
        json!({
            "name": "voxel_fill_ellipsoid",
            "description": "Fill a 3D voxel ellipsoid for organic parts (head, torso, boulders).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "center": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "radii": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["center", "radii", "palette_index"]
            }
        }),
        json!({
            "name": "voxel_convert_to_lowpoly",
            "description": "Convert a voxel volume into an optimized welded low-poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "mesh_create_primitive",
            "description": "Create a low-poly primitive mesh (cube, plane, cylinder, cone).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["cube", "plane", "cylinder", "cone"] },
                    "size": { "type": "number" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "mesh_import",
            "description": "Import a .obj or .glb 3D mesh from the allowed workspace/export directory into Low-Poly mode.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string" },
                    "name": { "type": "string" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "mesh_fill_texture",
            "description": "Fill the entire auto-UV RGBA texture of a Low-Poly mesh with a solid palette color.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["palette_index"]
            }
        }),
        json!({
            "name": "mesh_load_texture",
            "description": "Load a custom .png or .jpg/.jpeg image texture (e.g. wood bark, fur, fabric from Blender/Poly Haven) from the allowed export/workspace directory onto a Low-Poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "filename": { "type": "string" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "mesh_load_normal_map",
            "description": "Load a tangent-space Normal Map (.png or .jpg) from the allowed export/workspace directory onto a Low-Poly mesh for micro surface detail (skin pores, fabric weave, metal scratches).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "filename": { "type": "string" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "mesh_paint_spot",
            "description": "Paint a circular spot onto a Low-Poly mesh's auto-UV texture at a 3D surface point+normal (or direct UV).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "point": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "uv": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "radius_px": { "type": "number", "minimum": 1.0, "maximum": 96.0 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["palette_index"]
            }
        }),
        json!({
            "name": "mesh_smooth_subdivide",
            "description": "Apply Catmull-Clark smooth subdivision to round sharp edges on a Low-Poly mesh (1-3 iterations).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "iterations": { "type": "integer", "minimum": 1, "maximum": 3 }
                }
            }
        }),
        json!({
            "name": "mesh_set_shading",
            "description": "Toggle smooth vertex-normal shading (true = Shade Smooth, false = Flat) on a Low-Poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "smooth": { "type": "boolean" }
                },
                "required": ["smooth"]
            }
        }),
        json!({
            "name": "mesh_set_material",
            "description": "Set PBR material properties (metallic 0..1, roughness 0.05..1, emissive [r,g,b] 0..1) on a Low-Poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "metallic": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "roughness": { "type": "number", "minimum": 0.05, "maximum": 1.0 },
                    "emissive": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
            }
        }),
        json!({
            "name": "character_apply_biped_rig",
            "description": "Apply the starter 7-bone biped humanoid skeleton with Idle & Walk animations.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "character_set_bone_pose",
            "description": "Set a bone's pose rotation in degrees [pitch, yaw, roll] on the active character rig.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id", "rotation_deg"]
            }
        }),
        json!({
            "name": "character_set_keyframe",
            "description": "Insert or update a bone rotation keyframe in an animation clip (creates clip if clip_name is new).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id", "time_secs", "rotation_deg"]
            }
        }),
        json!({
            "name": "project_export",
            "description": "Export the current scene to .glb, .obj, .stl, .vox, .png, or .gif inside the allowed export directory.",
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
    ]
}
