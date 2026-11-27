use serde_json::{json, Value};

pub(super) fn mesh_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "mesh_create_primitive",
            "description": "Create a low-poly primitive mesh (cube, plane, cylinder, cone, sphere, torus, capsule) with optional world position, rotation_deg, scale, and vertical pivot anchor ('bottom' default, 'center', or 'top'). 'size' accepts either a scalar (>= 0.005) or [sx, sy, sz] non-uniform dimensions.",
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
                    "anchor": { "type": "string", "enum": ["bottom", "center", "top"], "description": "Local vertical pivot anchor (default 'bottom': y in [0, size_y]; 'center': y in [-size_y/2, +size_y/2]; 'top': y in [-size_y, 0])" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World position of the selected pivot anchor [x, y, z]" },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "mesh_create_terrain",
            "description": "Generate a faceted (or smooth) low-poly terrain / diorama mesh centered on X/Z from either an explicit 'heights' grid ((subdiv_x+1)*(subdiv_z+1)) or deterministic seeded 2D fBm noise, with planar UVs, optional 'island_falloff', and elevation-banded 'palette_bands'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "width": { "type": "number", "minimum": 0.05, "description": "Terrain width along X (default 10.0)" },
                    "depth": { "type": "number", "minimum": 0.05, "description": "Terrain depth along Z (default 10.0)" },
                    "subdiv_x": { "type": "integer", "minimum": 1, "maximum": 128, "description": "Grid cells along X (default 16)" },
                    "subdiv_z": { "type": "integer", "minimum": 1, "maximum": 128, "description": "Grid cells along Z (default 16)" },
                    "height_scale": { "type": "number", "description": "Vertical amplitude multiplier (default 2.0)" },
                    "seed": { "type": "integer", "minimum": 0, "description": "Deterministic noise seed (default 42)" },
                    "octaves": { "type": "integer", "minimum": 1, "maximum": 8, "description": "fBm octaves (default 4)" },
                    "frequency": { "type": "number", "minimum": 0.001, "description": "Noise spatial frequency (default 0.35)" },
                    "island_falloff": { "type": "number", "minimum": 0.0, "description": "Radial edge attenuation for island dioramas (default 0.0)" },
                    "heights": { "type": "array", "items": { "type": "number" }, "description": "Optional explicit heightmap values of length (subdiv_x+1)*(subdiv_z+1)" },
                    "palette_bands": { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 255 }, "description": "Optional elevation-ordered palette indices (low -> high)" },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "smooth_shading": { "type": "boolean" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
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
                        "description": "2..=32 closed cross-section rings, each an array of 3..=64 3D points [[x,y,z], ...]. Example (2 square rings): [[[-0.2, 0.0, -0.2], [0.2, 0.0, -0.2], [0.2, 0.0, 0.2], [-0.2, 0.0, 0.2]], [[-0.1, 1.0, -0.1], [0.1, 1.0, -0.1], [0.1, 1.0, 0.1], [-0.1, 1.0, 0.1]]].",
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
            "name": "mesh_fill_smart_material",
            "description": "Generate and apply a procedural Smart Material Lite RGBA texture (base color + curvature-driven convex edge wear + AO/concavity cavity dirt) onto a Low-Poly mesh in one atomic undoable command.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "base_palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "edge_palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "cavity_palette_index": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "edge_strength": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "edge_threshold": { "type": "number", "minimum": 0.5, "maximum": 0.95 },
                    "cavity_strength": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "cavity_threshold": { "type": "number", "minimum": 0.05, "maximum": 1.0 },
                    "ao_darken": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "resolution": { "type": "integer", "minimum": 16, "maximum": 1024 },
                    "samples": { "type": "integer", "minimum": 4, "maximum": 64 },
                    "dilation_px": { "type": "integer", "minimum": 0, "maximum": 16 },
                    "attach_ao": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "mesh_load_texture",
            "description": "Load a custom .png, .jpg/.jpeg, .tga, .tif/.tiff, or .exr image texture (e.g. from TextureCraft/Blender/Poly Haven) from the allowed export/workspace directory onto a Low-Poly mesh.",
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
            "description": "Load a tangent-space Normal Map (.png, .jpg, .tga, .tif/.tiff, or .exr) from the allowed export/workspace directory onto a Low-Poly mesh for micro surface detail (skin pores, fabric weave, metal scratches).",
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
            "name": "mesh_bake_maps",
            "description": "Deterministically bake CPU surface maps (Ambient Occlusion, Curvature, Thickness) in UV space for a Low-Poly mesh. Optionally saves PNGs to the export directory and attaches the baked AO map to the mesh for glTF occlusionTexture export.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "maps": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["ao", "curvature", "thickness"] }
                    },
                    "resolution": { "type": "integer", "minimum": 16, "maximum": 1024 },
                    "samples": { "type": "integer", "minimum": 4, "maximum": 64 },
                    "max_distance": { "type": "number", "minimum": 0.001, "maximum": 100.0 },
                    "bias": { "type": "number", "minimum": 0.00001, "maximum": 1.0 },
                    "dilation_px": { "type": "integer", "minimum": 0, "maximum": 16 },
                    "output_prefix": { "type": "string" },
                    "attach_ao": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "mesh_layer_add",
            "description": "Add a non-destructive RGBA paint layer to a Low-Poly mesh and make it active (promotes any existing mesh texture to a 'Base' layer automatically).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "name": { "type": "string" },
                    "opacity": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "palette_index": { "type": "integer", "minimum": 1, "maximum": 255 }
                }
            }
        }),
        json!({
            "name": "mesh_layer_set",
            "description": "Update a non-destructive paint layer's visibility, opacity, name, or active target state and recomposite the mesh texture.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string" },
                    "visible": { "type": "boolean" },
                    "opacity": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "active": { "type": "boolean" }
                },
                "required": ["layer_id"]
            }
        }),
        json!({
            "name": "mesh_layer_delete",
            "description": "Delete a non-destructive paint layer from a Low-Poly mesh and recomposite the remaining layers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "layer_id": { "type": "integer", "minimum": 1 }
                },
                "required": ["layer_id"]
            }
        }),
        json!({
            "name": "mesh_layer_list",
            "description": "List all non-destructive paint layers on a Low-Poly mesh with their id, name, visibility, opacity, and active state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "mesh_paint_spot",
            "description": "Paint a circular spot onto a Low-Poly mesh's RGBA texture at a 3D surface point+normal (or direct UV), with optional X mirror symmetry, seam-crossing 3D brush (`across_seams`/`radius_world`), and camera-ray visibility culling (`camera_pos`) so strokes never bleed onto back-facing or occluded geometry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "point": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "uv": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "radius_px": { "type": "number", "minimum": 1.0, "maximum": 96.0 },
                    "radius_world": { "type": "number", "minimum": 0.0001, "maximum": 100.0 },
                    "across_seams": { "type": "boolean" },
                    "camera_pos": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
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
            "description": "Set PBR & optical material properties (metallic 0..1, roughness 0.05..1, emissive [r,g,b] 0..1, emissive_intensity 0..20, transmission 0..1, ior 1..2.5, double_sided, cast_shadow, receive_shadow) on a Low-Poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "metallic": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "roughness": { "type": "number", "minimum": 0.05, "maximum": 1.0 },
                    "emissive": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "emissive_intensity": { "type": "number", "minimum": 0.0, "maximum": 20.0, "description": "HDR emissive strength multiplier (0.0..=20.0, default 1.0; exports KHR_materials_emissive_strength)" },
                    "transmission": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Glass/dielectric transmission factor (0.0..=1.0, default 0.0; exports KHR_materials_transmission)" },
                    "ior": { "type": "number", "minimum": 1.0, "maximum": 2.5, "description": "Index of Refraction (1.0..=2.5, default 1.5; exports KHR_materials_ior)" },
                    "double_sided": { "type": "boolean", "description": "Disable back-face culling on export (glTF doubleSided: true; ideal for thin grass blades, foliage, cloth)" },
                    "cast_shadow": { "type": "boolean", "description": "Whether this mesh casts shadows into the directional shadow map (default true)" },
                    "receive_shadow": { "type": "boolean", "description": "Whether this mesh receives shadows from the directional shadow map (default true)" },
                    "alpha_mode": { "type": "string", "enum": ["opaque", "mask", "blend"], "description": "glTF 2.0 alphaMode (default opaque)" },
                    "alpha_cutoff": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Alpha cutoff threshold when alpha_mode is 'mask' (default 0.5)" },
                    "unlit": { "type": "boolean", "description": "Export with glTF KHR_materials_unlit extension for stylized/NPR shading (default false)" }
                }
            }
        }),
        json!({
            "name": "mesh_set_vertex_colors",
            "description": "Set or clear per-vertex RGBA attributes (glTF COLOR_0: R=outline weight, G=shadow bias, B=ink density, A=cavity) on a Low-Poly mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "clear": { "type": "boolean", "description": "Clear all COLOR_0 vertex colors on the mesh" },
                    "fill": { "type": "array", "items": { "type": "number", "minimum": 0.0, "maximum": 1.0 }, "minItems": 4, "maxItems": 4 },
                    "updates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertex_id": { "type": "integer", "minimum": 0 },
                                "rgba": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                            },
                            "required": ["vertex_id", "rgba"]
                        }
                    }
                }
            }
        }),
        json!({
            "name": "mesh_set_crease_edges",
            "description": "Tag explicit interior ink-crease edges [[v0, v1], ...] on a Low-Poly mesh for NPR / stylized line-art authoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "edges": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 }
                    }
                },
                "required": ["edges"]
            }
        }),
        json!({
            "name": "mesh_set_proxy_normals",
            "description": "Set or clear per-vertex transferred/proxy normals on a low-poly mesh (for clean NPR/anime face shading and smooth outline normals).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "clear": { "type": "boolean", "description": "Clear existing proxy normals before applying 'normals'" },
                    "normals": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertex_id": { "type": "integer", "minimum": 0 },
                                "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                            },
                            "required": ["vertex_id", "normal"]
                        }
                    }
                }
            }
        }),
        json!({
            "name": "mesh_validate_topology",
            "description": "Validate Low-Poly mesh silhouette hygiene (sliver triangles, degenerate normals, non-manifold edges) and UV texel-density uniformity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "mesh_clear_texture",
            "description": "Clear a low-poly mesh's color texture, normal map, ao map, both (color+normal), or all (color+normal+ao) (undoable).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "target": { "type": "string", "enum": ["color", "normal", "ao", "both", "all"] }
                }
            }
        }),
        json!({
            "name": "mesh_morph_set_deltas",
            "description": "Author sparse per-vertex position_deltas and normal_deltas on a named MorphTarget (exported 1:1 to glTF targets[].POSITION & targets[].NORMAL).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string" },
                    "position_deltas": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertex": { "type": "integer", "minimum": 0 },
                                "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                            },
                            "required": ["vertex", "delta"]
                        }
                    },
                    "normal_deltas": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "vertex": { "type": "integer", "minimum": 0 },
                                "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                            },
                            "required": ["vertex", "delta"]
                        }
                    },
                    "replace": { "type": "boolean" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "mesh_morph_set_weight",
            "description": "Set the default rest weight (0.0..=1.0) and live viewport weight for a named MorphTarget (exported to glTF mesh.weights[]).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "name": { "type": "string" },
                    "weight": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                },
                "required": ["name", "weight"]
            }
        }),
        json!({
            "name": "mesh_proxy_cage_deform",
            "description": "Apply deterministic kinematic 3D FFD Proxy Cage deformation to a low-poly mesh with optional floor_y clamp; optionally bake as a MorphTarget instead of mutating rest vertices.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "divisions": { "type": "array", "items": { "type": "integer", "minimum": 2, "maximum": 6 }, "minItems": 3, "maxItems": 3 },
                    "control_deltas": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "index": { "type": "integer", "minimum": 0 },
                                "delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                            },
                            "required": ["index", "delta"]
                        }
                    },
                    "floor_y": { "type": "number" },
                    "bake_as_morph": { "type": "string" }
                },
                "required": ["control_deltas"]
            }
        }),
        json!({
            "name": "mesh_bake_vat",
            "description": "Bake deterministic per-frame Vertex Animation Texture (position + normal RGBA8 PNGs and JSON metadata) for a low-poly mesh across an animation clip into exports/.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "stem": { "type": "string", "description": "Output filename stem inside exports/ (e.g. 'cape_vat')" },
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "fps": { "type": "number", "minimum": 1.0, "maximum": 60.0 },
                    "max_frames": { "type": "integer", "minimum": 2, "maximum": 512 }
                },
                "required": ["stem"]
            }
        }),
        json!({
            "name": "mesh_create_ribbon",
            "description": "Generate a deterministic procedural ribbon/trail strip mesh following a bone/node animation trajectory or explicit_points, with optional STEP visibility track for glTF export.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "source_bone": { "type": "integer", "minimum": 0 },
                    "source_node": { "type": "integer", "minimum": 1 },
                    "explicit_points": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                    },
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "start_time": { "type": "number", "minimum": 0.0 },
                    "end_time": { "type": "number", "minimum": 0.01 },
                    "samples": { "type": "integer", "minimum": 2, "maximum": 128 },
                    "width_start": { "type": "number", "minimum": 0.0 },
                    "width_end": { "type": "number", "minimum": 0.0 },
                    "material": { "type": "integer", "minimum": 1, "maximum": 255 },
                    "bake_visibility_track": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "mesh_cleanup",
            "description": "Run one-command mesh cleanup (weld near-duplicate vertices and remove degenerate faces, v5.8).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "weld_epsilon": { "type": "number", "minimum": 0.00001, "maximum": 1.0 }
                }
            }
        }),
        json!({
            "name": "mesh_add_modifier",
            "description": "Append a non-destructive modifier (mirror, bevel, solidify, decimate) to a LowPolyMesh modifier stack (v5.9).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "kind": { "type": "string", "enum": ["mirror", "bevel", "solidify", "decimate"] },
                    "axis": { "type": "integer", "minimum": 0, "maximum": 2 },
                    "width": { "type": "number", "minimum": 0.001, "maximum": 0.5 },
                    "thickness": { "type": "number", "minimum": -1.0, "maximum": 1.0 },
                    "ratio": { "type": "number", "minimum": 0.05, "maximum": 0.99 }
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "mesh_preflight",
            "description": "Run unified pre-export QA gate (polycount budget, degenerate/non-manifold topology, missing UVs, texture resolution, duplicate node names) returning pass/warn/fail (v5.9).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_triangles": { "type": "integer", "minimum": 1, "maximum": 500000 }
                }
            }
        }),
        json!({
            "name": "mesh_set_npr_slot",
            "description": "Attach or clear an NPR texture slot (matcap, face_sdf, hatching_ramp) on a LowPolyMesh and optionally tune matcap_strength, hatching_scale, or face_sdf_light_override (Phase L2).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "slot": { "type": "string", "enum": ["matcap", "face_sdf", "hatching_ramp"] },
                    "filename": { "type": "string", "description": "Image filename inside exports/ (.png/.jpg/.tga/.tiff/.exr)" },
                    "clear": { "type": "boolean" },
                    "matcap_strength": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "hatching_scale": { "type": "number", "minimum": 0.1, "maximum": 64.0 },
                    "face_sdf_light_override": {
                        "type": "array",
                        "items": { "type": "number" },
                        "minItems": 3,
                        "maxItems": 3
                    }
                },
                "required": ["slot"]
            }
        }),
        json!({
            "name": "mesh_set_uv_seams",
            "description": "Mark or replace manual UV seam edges ([v0, v1]) on a LowPolyMesh and optionally unwrap UV islands along those seams (Phase M1).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "edges": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                            "minItems": 2,
                            "maxItems": 2
                        }
                    },
                    "replace": { "type": "boolean" },
                    "unwrap": { "type": "boolean" }
                },
                "required": ["edges"]
            }
        }),
        json!({
            "name": "mesh_normalize_texel_density",
            "description": "Equalize UV island scales across a LowPolyMesh for uniform texel density and repack cleanly into [0, 1] (Phase M1).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "texture_size": { "type": "integer", "minimum": 16, "maximum": 4096 },
                    "target_px_per_unit": { "type": "number", "minimum": 0.1 },
                    "padding_uv": { "type": "number", "minimum": 0.0, "maximum": 0.05 }
                }
            }
        }),
        json!({
            "name": "mesh_sculpt_stroke",
            "description": "Apply a deterministic organic sculpting brush stroke (smooth, inflate, grab, crease) with falloff radius, strength, and optional mirror_x symmetry (Phase M2).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "brush": { "type": "string", "enum": ["smooth", "inflate", "grab", "crease"] },
                    "center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "normal": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "drag_delta": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "radius": { "type": "number", "minimum": 0.001, "maximum": 50.0 },
                    "strength": { "type": "number", "minimum": -1.0, "maximum": 1.0 },
                    "mirror_x": { "type": "boolean" }
                },
                "required": ["brush", "center", "radius"]
            }
        }),
    ]
}
