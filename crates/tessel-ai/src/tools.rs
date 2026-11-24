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
            "description": "Execute up to 512 tool calls atomically in one MCP request. If any call fails, the entire scene and undo history roll back cleanly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 128,
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
            "description": "Create a low-poly primitive mesh (cube, plane, cylinder, cone, sphere, torus, capsule) with optional world position, rotation_deg, and scale. 'size' accepts either a scalar (>= 0.005) or [sx, sy, sz] non-uniform dimensions. Local pivot is bottom-centered: y in [0, size] (torus y in [0, 0.28*size]); x,z in [-size/2, size/2] (capsule x,z in [-size/4, size/4]).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["cube", "plane", "cylinder", "cone", "sphere", "torus", "capsule"] },
                    "size": {
                        "oneOf": [
                            { "type": "number", "minimum": 0.005 },
                            { "type": "array", "items": { "type": "number", "minimum": 0.005 }, "minItems": 3, "maxItems": 3 }
                        ]
                    },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "mesh_create_lathe",
            "description": "Create a surface-of-revolution (lathe) Low-Poly mesh by revolving a 2D profile [[radius, y], ...] (bottom-to-top) around the +Y axis with automatic outward CCW winding and pole collapse. Use angle_deg=360 for domes/helmets/boots/props or partial arcs (e.g. 160-180) for curved visors and shields.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "profile": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 128,
                        "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
                    },
                    "segments": { "type": "integer", "minimum": 3, "maximum": 64 },
                    "angle_deg": { "type": "number", "minimum": 1.1, "maximum": 360.0 },
                    "cap_ends": { "type": "boolean" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["profile"]
            }
        }),
        json!({
            "name": "mesh_create_extruded_polygon",
            "description": "Create a 3D Low-Poly mesh by extruding a 2D XY polygon [[x, y], ...] (3..=64 points) by 'depth' along Z (centered on [-depth/2, +depth/2]). Automatically normalizes CW/CCW winding so front/back caps and side walls always face outward. Ideal for neckties, armor plates, buckles, weapon silhouettes, blades, and emblems.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "points": {
                        "type": "array",
                        "minItems": 3,
                        "maxItems": 64,
                        "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
                    },
                    "depth": { "type": "number", "minimum": 0.005 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["points", "depth"]
            }
        }),
        json!({
            "name": "mesh_create_loft",
            "description": "Create a lofted Low-Poly mesh by connecting 2..=32 closed 3D cross-section rings of equal vertex count (3..=64) with automatic outward CCW winding and optional end caps. Ideal for tapered torsos, limbs, boots, backpacks, capes, and rifle stocks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "sections": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 32,
                        "items": {
                            "type": "array",
                            "minItems": 3,
                            "maxItems": 64,
                            "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                        }
                    },
                    "cap_ends": { "type": "boolean" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["sections"]
            }
        }),
        json!({
            "name": "mesh_create_raw",
            "description": "Create a custom Low-Poly mesh directly from explicit 3D vertices [[x,y,z], ...] and faces (triangles/quads/n-gons with optional per-face palette_index and per-corner uvs). Ideal for bespoke 1:1 character anatomy, helmets, clothing, and organic silhouettes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "vertices": {
                        "type": "array",
                        "minItems": 3,
                        "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                    },
                    "faces": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertices": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 3 },
                                "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                                "uvs": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "minItems": 3 }
                            },
                            "required": ["vertices"]
                        }
                    },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "smooth": { "type": "boolean" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["vertices", "faces"]
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
            "name": "mesh_paint_line",
            "description": "Paint a crisp line segment (seams, stitching, eyebrows, stripes) onto a Low-Poly mesh's RGBA texture between uv_start and uv_end.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "uv_start": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "uv_end": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "width_px": { "type": "number", "minimum": 1.0, "maximum": 96.0 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["uv_start", "uv_end", "palette_index"]
            }
        }),
        json!({
            "name": "mesh_paint_rect",
            "description": "Paint a solid axis-aligned UV rectangle (pockets, panels, camo blocks, visor frames) onto a Low-Poly mesh's RGBA texture.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "uv_min": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "uv_max": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["uv_min", "uv_max", "palette_index"]
            }
        }),
        json!({
            "name": "mesh_project_uvs",
            "description": "Automatically project per-corner UVs onto every face of a Low-Poly mesh using 'box', 'planar_x', 'planar_y', 'planar_z', 'cylindrical', or 'spherical' projection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["box", "planar_x", "planar_y", "planar_z", "cylindrical", "spherical"] }
                },
                "required": ["mode"]
            }
        }),
        json!({
            "name": "mesh_subdivide_face",
            "description": "Locally subdivide one Low-Poly face into N quads around its centroid and edge midpoints (adds local joint/face topology density without subdividing the whole mesh).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "face_id": { "type": "integer", "minimum": 0 }
                },
                "required": ["face_id"]
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
            "name": "mesh_set_vertices",
            "description": "Batch-update multiple Low-Poly vertex positions in a single atomic undoable command.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "updates": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertex_id": { "type": "integer", "minimum": 0 },
                                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                            },
                            "required": ["vertex_id", "position"]
                        }
                    }
                },
                "required": ["updates"]
            }
        }),
        json!({
            "name": "mesh_set_face_material",
            "description": "Assign a palette_index color to one or more faces on a Low-Poly mesh (enables multi-color unified meshes without splitting nodes).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "face_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                },
                "required": ["face_ids", "palette_index"]
            }
        }),
        json!({
            "name": "mesh_join",
            "description": "Merge source_node_ids Low-Poly meshes into target_node_id (baking world transforms into target local space, preserving per-face materials/UVs, deleting source nodes).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target_node_id": { "type": "integer", "minimum": 1 },
                    "source_node_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "minItems": 1 }
                },
                "required": ["target_node_id", "source_node_ids"]
            }
        }),
        json!({
            "name": "mesh_mirror",
            "description": "Mirror a Low-Poly mesh across axis ('x', 'y', or 'z'), preserving outward normals. Set merge_original=true (default) for symmetrical modeling.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "axis": { "type": "string", "enum": ["x", "y", "z"] },
                    "merge_original": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "mesh_taper",
            "description": "Linearly taper a Low-Poly mesh along axis ('x', 'y', or 'z') from start_scale (min) to end_scale (max) — ideal for forearms, calves, thighs, and V-tapered torsos.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "axis": { "type": "string", "enum": ["x", "y", "z"] },
                    "start_scale": { "type": "number", "minimum": 0.05 },
                    "end_scale": { "type": "number", "minimum": 0.05 }
                },
                "required": ["start_scale", "end_scale"]
            }
        }),
        json!({
            "name": "mesh_bend",
            "description": "Smoothly bend a Low-Poly mesh along bend_axis ('x'|'y'|'z') curving toward curve_axis ('x'|'y'|'z') by angle_deg — ideal for curved helmet visors, collars, and armor plates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "bend_axis": { "type": "string", "enum": ["x", "y", "z"] },
                    "curve_axis": { "type": "string", "enum": ["x", "y", "z"] },
                    "angle_deg": { "type": "number" }
                },
                "required": ["bend_axis", "curve_axis", "angle_deg"]
            }
        }),
        json!({
            "name": "mesh_set_origin",
            "description": "Move a Low-Poly mesh's local pivot/origin to 'center', 'bottom', or an explicit local point [x,y,z] while keeping its world position unchanged.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "mode": { "type": "string", "enum": ["center", "bottom"] },
                    "origin": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
            }
        }),
        json!({
            "name": "mesh_apply_transform",
            "description": "Bake a Low-Poly node's position, rotation, and scale directly into its mesh vertices and reset the node transform to identity (use before mesh_mirror or mesh_bend for exact 1:1 world-space symmetry).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "mesh_weld_vertices",
            "description": "Merge coincident vertices within tolerance (default 0.001) and remap faces to remove center-seam shading creases after mesh_mirror or mesh_join.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "tolerance": { "type": "number", "minimum": 0.00001, "maximum": 0.5 }
                }
            }
        }),
        json!({
            "name": "mesh_flip_normals",
            "description": "Reverse face winding (flip outward normals) on all faces of a Low-Poly mesh or on explicit face_ids to fix inside-out faces.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "face_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 }
                }
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
            "description": "Add (accumulate) a position delta [dx, dy, dz] onto a named morph target (blend shape), creating the target if missing. Select vertices either by explicit 'vertex_ids' OR by local-space bounding box ('bounds_min' + 'bounds_max'). Exports as glTF morph targets; bevel/subdivide/decimate clear them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "name": { "type": "string" },
                    "vertex_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1 },
                    "bounds_min": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Local-space AABB min [x, y, z] for spatial vertex selection (alternative to vertex_ids)" },
                    "bounds_max": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Local-space AABB max [x, y, z] for spatial vertex selection (alternative to vertex_ids)" },
                    "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["name", "delta"]
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
            "description": "Apply a humanoid skeleton rig (Right-Handed +Y Up, +Z Front: Left=+X, Right=-X). Pass preset='starter' (default, 7 bones) or preset='humanoid' (19-bone 1:1 humanoid). For multi-character scenes, pass append=true with prefix (e.g. 'P2_'), offset [x,y,z], and yaw_deg (e.g. 180) to spawn an additional prefixed skeleton into the active rig in 1 call and receive its bone_ids map.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["starter", "humanoid"] },
                    "append": { "type": "boolean", "description": "When true, append a new sub-skeleton into the existing CharacterRig instead of replacing it (ideal for 2+ character scenes)" },
                    "prefix": { "type": "string", "description": "Bone name prefix when append=true (e.g. 'P2_')" },
                    "offset": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space root translation [x, y, z] for appended sub-skeleton" },
                    "yaw_deg": { "type": "number", "description": "Yaw rotation in degrees around +Y for appended sub-skeleton (e.g. 180 to face -Z)" }
                }
            }
        }),
        json!({
            "name": "character_set_bone_rest_position",
            "description": "Move an existing bone's local rest position [x, y, z] without deleting its subtree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id", "position"]
            }
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
            "name": "character_set_node_bone_mask",
            "description": "Restrict which bones may auto-skin a scene node (e.g. allow only leg/root bones on trousers so hanging arms never pull the hips). Pass null or empty array to clear the mask.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "bone_ids": { "type": ["array", "null"], "items": { "type": "integer", "minimum": 0 } }
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
            "description": "Analytic two-bone IK: rotate root and mid bones so the end bone reaches target [x, y, z]. Use 'pole_target' for a world-space bend point OR 'pole_vector' for a world-space bend direction. Use 'end_world_rotation_deg' to orient end_bone_id in world space (auto-compensating forearm rotation) or 'end_rotation_deg' for local rotation. Returns reached position and resulting bone delta rotations_deg.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root_bone_id": { "type": "integer", "minimum": 0 },
                    "mid_bone_id": { "type": "integer", "minimum": 0 },
                    "end_bone_id": { "type": "integer", "minimum": 0 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space target position [x, y, z] for end_bone_id" },
                    "pole_target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space POINT [x, y, z] that the elbow/knee bends toward (auto-converted to direction from root_bone_id)" },
                    "pole_vector": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space DIRECTION vector (not a world point) steering the elbow/knee bend plane; e.g. [0, -1, -1] for elbow down-back, [0, 0, 1] for knee forward" },
                    "end_world_rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Optional WORLD-SPACE Euler XYZ degrees for end_bone_id after solving IK (automatically compensates for mid_bone_id world rotation)" },
                    "end_rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Optional LOCAL Euler XYZ degrees (relative to rest pose) applied to end_bone_id after solving IK" }
                },
                "required": ["root_bone_id", "mid_bone_id", "end_bone_id", "target"]
            }
        }),
        json!({
            "name": "character_snapshot_pose_to_keyframe",
            "description": "Bake/snapshot the current live bone pose (e.g. after character_solve_ik or manual posing) into an animation clip at time_secs as a single atomic transaction. Creates the clip automatically if clip_name is new.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "bone_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "description": "Optional subset of bone IDs to snapshot (defaults to all bones in the rig)" }
                },
                "required": ["time_secs"]
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
            "name": "character_list_bones",
            "description": "List all bones of the character rig (id, name, parent, rest position). Fails when the scene has no rig.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "character_inspect_weights",
            "description": "Inspect skinning bone influences at a 3D world point [x,y,z] and/or for a scene node_id (reports rigid binding, manual weight count, and resolved bone names + weights).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "point": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
            }
        }),
        json!({
            "name": "character_inspect_clip",
            "description": "Inspect an animation clip in full: keyframes per bone channel (time, translation, rotation quat + degrees, scale) and morph weight tracks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_sample_pose",
            "description": "Sample an animation clip at time_secs without touching live pose. Returns per-bone world positions and local XYZ-euler degrees.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["time_secs"]
            }
        }),
        json!({
            "name": "character_set_active_clip",
            "description": "Set the active animation clip (viewport playback target).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_set_playback_time",
            "description": "Scrub an animation clip to time_secs and immediately apply its bone poses and morph weights to the live viewport for visual inspection or viewport_screenshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["time_secs"]
            }
        }),
        json!({
            "name": "character_reset_pose",
            "description": "Reset all bones to their rest pose and clear live morph weights in a single undoable command.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "character_remove_keyframe",
            "description": "Remove a bone keyframe matching time_secs (within 1ms) from an animation clip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["bone_id", "time_secs"]
            }
        }),
        json!({
            "name": "character_remove_clip",
            "description": "Remove an animation clip by index or name (active clip index is repaired when needed).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_remove_bone",
            "description": "Remove a bone and its whole subtree (children, bindings, and bone channels referencing it). Reports counts of removed items.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 }
                },
                "required": ["bone_id"]
            }
        }),
        json!({
            "name": "scene_duplicate_node",
            "description": "Duplicate a scene node (mesh/voxel/pixel payload plus material, inheriting rig bindings) with a +0.5 x offset. Selects the copy.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string" }
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
            "name": "mesh_clear_texture",
            "description": "Clear a low-poly mesh's color texture, normal map, or both (undoable).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "target": { "type": "string", "enum": ["color", "normal", "both"] }
                }
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
        json!({
            "name": "camera_get",
            "description": "Read the live viewport camera state (target, yaw, pitch, distance). Requires the live GUI viewport; unavailable headless.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "camera_look_at",
            "description": "Point the live viewport camera at an absolute world position [x, y, z]. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "camera_zoom",
            "description": "Zoom the live viewport camera. Pass factor (> 0, < 1 zooms in) or an absolute distance (> 0). Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "factor": { "type": "number" },
                    "distance": { "type": "number", "minimum": 0 }
                }
            }
        }),
        json!({
            "name": "camera_orbit",
            "description": "Orbit the live viewport camera by pixel deltas (dx turns yaw, dy turns pitch). Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dx": { "type": "number" },
                    "dy": { "type": "number" }
                },
                "required": ["dx", "dy"]
            }
        }),
        json!({
            "name": "camera_pan",
            "description": "Pan the live viewport camera target by pixel deltas. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dx": { "type": "number" },
                    "dy": { "type": "number" }
                },
                "required": ["dx", "dy"]
            }
        }),
        json!({
            "name": "camera_reset",
            "description": "Reset the live viewport camera to its default view. Requires the live GUI viewport.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "camera_preset",
            "description": "Move the live viewport camera to a named preset view. Requires the live GUI viewport.",
            "inputSchema": {
                "type": "object",
                "properties": {
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
            "description": "Render the current scene offscreen (headless GPU) and return an sRGB PNG image. Prefer yaw_deg/pitch_deg (degrees); yaw/pitch are in radians. Needs a GPU (software GL works).",
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
                    "distance": { "type": "number", "minimum": 0 },
                    "preview": { "type": "boolean" }
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
                    "clip_name": { "type": "string" },
                    "clip_index": { "type": "integer", "minimum": 0 }
                },
                "required": ["filename"]
            }
        }),
    ]
}
