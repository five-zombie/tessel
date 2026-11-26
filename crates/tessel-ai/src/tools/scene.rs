use serde_json::{json, Value};

pub(super) fn scene_tools() -> Vec<Value> {
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
            "name": "scene_inspect",
            "description": "List and inspect all scene nodes (id, name, kind, position, scale, bounds, face/voxel counts) and active palette colors (1..=255).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "palette_set_color",
            "description": "Add or update a color in the 255-slot palette (1..=255) using rgb [r,g,b] or hex '#RRGGBB'/'#RRGGBBAA', with optional alpha, roughness, metallic, and emissive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "name": { "type": "string" },
                    "rgb": { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 }, "minItems": 3, "maxItems": 3 },
                    "hex": { "type": "string" },
                    "alpha": { "type": "integer", "minimum": 0, "maximum": 255 },
                    "roughness": { "type": "number", "minimum": 0.05, "maximum": 1.0 },
                    "metallic": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "emissive": { "type": "number", "minimum": 0.0, "maximum": 10.0 }
                },
                "required": ["palette_index"]
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
            "name": "scene_rename_node",
            "description": "Rename an existing scene node by node_id (supports Undo/Redo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string", "minLength": 1 }
                },
                "required": ["node_id", "name"]
            }
        }),
        json!({
            "name": "scene_clear",
            "description": "Remove all nodes and reset the active character rig to start a clean empty scene (undoable as a single transaction).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "scene_batch",
            "description": "Execute up to 512 tool calls atomically in one MCP request. Supports referencing prior call results via \"$N.field\" or \"$N.nested.field\" placeholders (e.g. \"$0.node_id\", \"$1.bone_ids.P2_Root\"). If any call fails, the entire scene and undo history roll back cleanly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 512,
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": { "type": "object" }
                            },
                            "required": ["name"]
                        }
                    }
                },
                "required": ["calls"]
            }
        }),
        json!({
            "name": "scene_create_group",
            "description": "Create an empty Group / folder node for organizing scene hierarchies (e.g. Trees, Grass, Props) and animating entire subtrees without bones. Exports as a standard glTF 2.0 parent node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name for the group node (default 'Group')" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "parent_node_id": { "type": ["integer", "null"], "minimum": 1 }
                }
            }
        }),
        json!({
            "name": "scene_set_node_keyframe",
            "description": "Animate any scene node's local TRS (position/rotation/scale delta relative to its base transform) directly in an animation clip WITHOUT needing any CharacterRig bones (prevents auto-skinning static props). Supports single keyframe OR bulk 'keyframes': [{ node_id, time_secs, translation?, rotation_deg?, scale?, interpolation? }, ...] in 1 atomic transaction. Works on meshes, voxels, and Group folders; exports as standard glTF 2.0 TRS channels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Delta offset [dx, dy, dz] added to the node's base position (NOT absolute world coordinates; use [0,0,0] to stay at base position)" },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "interpolation": { "type": "string", "enum": ["linear", "step", "ease_in", "ease_out", "ease_in_out", "cubic_bezier"] },
                    "bezier": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "Optional [p1x, p1y, p2x, p2y] control points when interpolation is 'cubic_bezier'" },
                    "keyframes": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 2048,
                        "description": "Optional bulk list of per-node TRS keyframes to upsert atomically into the target clip",
                        "items": {
                            "type": "object",
                            "properties": {
                                "node_id": { "type": "integer", "minimum": 1 },
                                "time_secs": { "type": "number", "minimum": 0.0 },
                                "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Delta offset [dx, dy, dz] added to the node's base position (NOT absolute world coordinates; use [0,0,0] to stay at base position)" },
                                "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                                "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                                "interpolation": { "type": "string", "enum": ["linear", "step", "ease_in", "ease_out", "ease_in_out", "cubic_bezier"] },
                                "bezier": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                            },
                            "required": ["node_id", "time_secs"]
                        }
                    }
                }
            }
        }),
        json!({
            "name": "scene_set_node_transform",
            "description": "Set a scene node's world position [x,y,z], rotation_deg [pitch,yaw,roll], and/or scale [sx,sy,sz] (omitted fields keep their current values). Pass dry_run=true to preview world transform, orientation axes, and bounds without mutating the scene.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "dry_run": { "type": "boolean", "description": "When true, previews resulting world position, orientation vectors, and bounds without mutating the scene or undo history (default false)" }
                }
            }
        }),
        json!({
            "name": "scene_set_parent",
            "description": "Parent or group a child node under a parent_node_id (or null to unparent) so moving/rotating the parent moves all children together.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "parent_node_id": { "type": ["integer", "null"] }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "scene_set_lighting",
            "description": "Set the studio directional key light azimuth_deg (-180..180) and elevation_deg (5..85) for viewport and Render Preview ground shadows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "azimuth_deg": { "type": "number" },
                    "elevation_deg": { "type": "number", "minimum": 5.0, "maximum": 85.0 }
                }
            }
        }),
        json!({
            "name": "scene_select_node",
            "description": "Select an active scene node by node_id (or pass null to clear selection).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": ["integer", "null"], "minimum": 1 }
                }
            }
        }),
        json!({
            "name": "scene_create_clip",
            "description": "Create a new empty animation clip for node TRS or character animations (works even before any bones or character rig exist).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.05, "maximum": 600.0 },
                    "activate": { "type": "boolean" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "scene_duplicate_node",
            "description": "Duplicate a scene node (mesh/voxel/pixel payload plus material, inheriting rig bindings). Defaults to a +0.5 X offset unless position is provided; supports optional position, rotation_deg, and scale in one atomic call. Selects the copy.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
            }
        }),
        json!({
            "name": "scene_set_node_visibility",
            "description": "Show or hide a scene node (undoable).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "visible": { "type": "boolean" }
                },
                "required": ["node_id", "visible"]
            }
        }),
        json!({
            "name": "scene_set_node_metadata",
            "description": "Attach glTF extras key-value metadata and/or a collision hull (box/capsule/none) to a scene node (v5.8).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "extras": { "type": "object", "additionalProperties": { "type": "string" } },
                    "collision_shape": { "type": "string", "enum": ["box", "capsule", "none"] },
                    "half_extents": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "radius": { "type": "number", "minimum": 0.01 },
                    "height": { "type": "number", "minimum": 0.01 }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "scene_set_lookdev",
            "description": "Configure Look-Dev environment preset (daylight/night/indoor/sunset), save a camera bookmark, or tune NPR look parameters (v5.9/E8).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["daylight", "night", "indoor", "sunset"] },
                    "bookmark_name": { "type": "string" },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "yaw": { "type": "number" },
                    "pitch": { "type": "number" },
                    "distance": { "type": "number", "minimum": 0.25 },
                    "hatching_strength": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "bloom_strength": { "type": "number", "minimum": 0.0, "maximum": 2.0 },
                    "msaa_samples": { "type": "integer", "enum": [1, 4] }
                }
            }
        }),
    ]
}
