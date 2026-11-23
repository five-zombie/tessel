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
            "name": "scene_inspect",
            "description": "List and inspect all scene nodes (id, name, kind, position, scale, bounds, face/voxel counts) and active palette colors (1..=255).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "palette_set_color",
            "description": "Add or update a color in the 255-slot palette (1..=255) using either rgb [r,g,b] (0..255) or hex '#RRGGBB'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "name": { "type": "string" },
                    "rgb": { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 }, "minItems": 3, "maxItems": 3 },
                    "hex": { "type": "string" }
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
            "name": "scene_set_node_transform",
            "description": "Set a scene node's world position [x,y,z], rotation_deg [pitch,yaw,roll], and/or scale [sx,sy,sz] (omitted fields keep their current values).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
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
            "description": "Create a low-poly primitive mesh (cube, plane, cylinder, cone, sphere, torus, capsule) with optional world position, rotation_deg, and scale.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["cube", "plane", "cylinder", "cone", "sphere", "torus", "capsule"] },
                    "size": { "type": "number" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "mesh_create_curve_tube",
            "description": "Create a smooth curved strap, shoulder belt, cable, pipe, or handle following 3D control points [[x,y,z], ...]. Set flatness=0.25 for flat backpack straps or flatness=1.0 for round cables/tubes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "points": {
                        "type": "array",
                        "minItems": 2,
                        "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                    },
                    "radius": { "type": "number", "minimum": 0.005 },
                    "flatness": { "type": "number", "minimum": 0.1, "maximum": 1.0 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["points"]
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
            "description": "Paint a circular spot onto a Low-Poly mesh's auto-UV texture at a 3D surface point+normal (or direct UV), with optional X mirror symmetry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "point": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "uv": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "radius_px": { "type": "number", "minimum": 1.0, "maximum": 96.0 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "mirror_x": { "type": "boolean" }
                },
                "required": ["palette_index"]
            }
        }),
        json!({
            "name": "mesh_inspect",
            "description": "Inspect a Low-Poly mesh's bounding box and list its faces (face_id, center [x,y,z], outward normal [nx,ny,nz]) for targeted extrude, inset, translate_face, or paint_spot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 }
                }
            }
        }),
        json!({
            "name": "mesh_extrude_face",
            "description": "Extrude a Low-Poly face outward (distance > 0) or inward (distance < 0) along its face normal, creating side quads (ideal for pockets, flaps, straps, details).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "face_id": { "type": "integer", "minimum": 0 },
                    "distance": { "type": "number" }
                },
                "required": ["face_id", "distance"]
            }
        }),
        json!({
            "name": "mesh_inset_face",
            "description": "Inset a Low-Poly face toward its center by factor (0.05..0.9) and create border quads (ideal for bevels, seams, zippers, panels before extruding).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "face_id": { "type": "integer", "minimum": 0 },
                    "factor": { "type": "number", "minimum": 0.05, "maximum": 0.9 }
                },
                "required": ["face_id", "factor"]
            }
        }),
        json!({
            "name": "mesh_list_vertices",
            "description": "List Low-Poly vertex indices and positions (paginated; use to target mesh_move_vertices / mesh_set_vertex precisely).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 512 }
                },
                "required": []
            }
        }),
        json!({
            "name": "mesh_translate_face",
            "description": "Translate all vertices of a selected Low-Poly face by a 3D delta [dx, dy, dz].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "face_id": { "type": "integer", "minimum": 0 },
                    "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["face_id", "delta"]
            }
        }),
        json!({
            "name": "mesh_move_vertices",
            "description": "Move an explicit list of Low-Poly vertices by a 3D delta [dx, dy, dz] (per-vertex editing for precise 1:1 shaping).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "vertex_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 },
                    "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["vertex_ids", "delta"]
            }
        }),
        json!({
            "name": "mesh_set_vertex",
            "description": "Set one Low-Poly vertex to an absolute 3D position [x, y, z] (precise 1:1 corner placement).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "vertex_id": { "type": "integer", "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["vertex_id", "position"]
            }
        }),
        json!({
            "name": "mesh_set_face_uvs",
            "description": "Set manual per-corner UVs on a Low-Poly face (manual UV unwrap; one [u, v] pair per face corner, values clamped to 0..1, overrides auto box-atlas UV on export).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "face_id": { "type": "integer", "minimum": 0 },
                    "uvs": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "minItems": 3 }
                },
                "required": ["face_id", "uvs"]
            }
        }),
        json!({
            "name": "mesh_set_vertex_weights",
            "description": "Paint manual skin weights onto Low-Poly vertices (weight paint for smooth skinning). joints is 4 bone indices, weights is 4 influences (normalized to sum 1, negatives rejected). Export prefers these over auto weights; bevel/subdivide/decimate clear them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "vertex_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 },
                    "joints": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 4, "maxItems": 4 },
                    "weights": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                },
                "required": ["vertex_ids", "joints", "weights"]
            }
        }),
        json!({
            "name": "mesh_morph_delta",
            "description": "Add (accumulate) a position delta [dx, dy, dz] onto a named morph target (blend shape) for explicit vertices, creating the target if missing. Exports as glTF morph targets; bevel/subdivide/decimate clear them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "name": { "type": "string" },
                    "vertex_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 },
                    "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["name", "vertex_ids", "delta"]
            }
        }),
        json!({
            "name": "mesh_morph_clear",
            "description": "Clear morph targets on a Low-Poly mesh (all when name is omitted, otherwise only the named target).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "name": { "type": "string" }
                },
                "required": []
            }
        }),
        json!({
            "name": "mesh_clear_vertex_weights",
            "description": "Clear manual skin-weight overrides on a Low-Poly mesh (all when vertex_ids is omitted, otherwise only the listed vertices; falls back to auto weights).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "vertex_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 }
                },
                "required": []
            }
        }),
        json!({
            "name": "mesh_bevel",
            "description": "Chamfer/bevel all shared edges and corners of a Low-Poly mesh by amount (0.05..0.45, default 0.18) while keeping main surfaces flat (ideal for backpacks, pouches, weapons, hard-surface props).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "amount": { "type": "number", "minimum": 0.05, "maximum": 0.45 }
                }
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
            "name": "mesh_decimate",
            "description": "Simplify/decimate a Low-Poly mesh for LOD reduction by target vertex ratio (0.1..0.9, default 0.5).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "ratio": { "type": "number", "minimum": 0.1, "maximum": 0.9 }
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
            "name": "character_add_bone",
            "description": "Add a custom bone to the active rig (or initialize a custom prop rig for backpacks, chests, weapons) with optional parent_bone_id and local rest position [x,y,z].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "parent_bone_id": { "type": ["integer", "null"], "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["name", "position"]
            }
        }),
        json!({
            "name": "character_bind_node",
            "description": "Bind a scene node (e.g. backpack flap, pouch, weapon part) rigidly to a bone_id (or null to unbind for smooth auto-weight skinning).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "bone_id": { "type": ["integer", "null"], "minimum": 0 }
                },
                "required": ["node_id"]
            }
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
            "description": "Insert or update a bone TRS keyframe in an animation clip (creates clip if clip_name is new). rotation_deg is required; translation defaults to [0,0,0] (offset added to the rest pose) and scale defaults to [1,1,1] (multiplier).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id", "time_secs", "rotation_deg"]
            }
        }),
        json!({
            "name": "character_solve_ik",
            "description": "Analytic two-bone IK: rotate root and mid bones so the end bone reaches target [x, y, z]. Chain must be parent-linked root <- mid <- end (build with character_add_bone). pole_vector steers the elbow/knee bend, defaults to [0,0,1].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root_bone_id": { "type": "integer", "minimum": 0 },
                    "mid_bone_id": { "type": "integer", "minimum": 0 },
                    "end_bone_id": { "type": "integer", "minimum": 0 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "pole_vector": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["root_bone_id", "mid_bone_id", "end_bone_id", "target"]
            }
        }),
        json!({
            "name": "character_set_morph_weight",
            "description": "Insert or update a morph-target weight keyframe on a mesh node in an animation clip (creates clip if clip_name is new). Animate blend shapes over time; export writes glTF weight channels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "target": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "weight": { "type": "number" }
                },
                "required": ["node_id", "target", "time_secs", "weight"]
            }
        }),
        json!({
            "name": "character_remove_morph_weight",
            "description": "Remove a morph-target weight keyframe matching time_secs (within 1ms) from an animation clip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "target": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["clip_index", "node_id", "target", "time_secs"]
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
