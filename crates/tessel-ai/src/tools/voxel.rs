use serde_json::{json, Value};

pub(super) fn voxel_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "voxel_create_volume",
            "description": "Create a new empty 3D voxel volume node and make it active.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "voxel_set_voxels",
            "description": "Place or erase individual 3D voxels in the active or specified voxel node. Omit palette_index or pass null to erase. Supports optional X/Y/Z mirror symmetry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "voxels": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 50000,
                        "items": {
                            "type": "object",
                            "properties": {
                                "x": { "type": "integer" },
                                "y": { "type": "integer" },
                                "z": { "type": "integer" },
                                "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 }
                            },
                            "required": ["x", "y", "z"]
                        }
                    },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" },
                    "mirror_z": { "type": "boolean" }
                },
                "required": ["voxels"]
            }
        }),
        json!({
            "name": "voxel_inspect",
            "description": "Inspect a voxel volume node (voxel count, chunk count, and inclusive min/max bounds) without mutating the scene.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "voxel_import",
            "description": "Import a MagicaVoxel .vox file from inside the allowed exports directory as a new voxel volume node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string", "description": "Relative .vox filename inside exports/" },
                    "name": { "type": "string" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "voxel_fill_box",
            "description": "Fill or carve an axis-aligned 3D voxel box (solid or hollow) in the active or specified voxel node. Pass palette_index: null to erase/carve. Supports mirror_x/y/z.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "min": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "max": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 },
                    "hollow": { "type": "boolean" },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" },
                    "mirror_z": { "type": "boolean" }
                },
                "required": ["min", "max"]
            }
        }),
        json!({
            "name": "voxel_fill_ellipsoid",
            "description": "Fill or carve a 3D voxel ellipsoid for organic parts (head, torso, boulders). Pass palette_index: null to erase/carve. Supports mirror_x/y/z.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "center": { "type": "array", "items": { "type": "integer" }, "minItems": 3, "maxItems": 3 },
                    "radii": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" },
                    "mirror_z": { "type": "boolean" }
                },
                "required": ["center", "radii"]
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
    ]
}
