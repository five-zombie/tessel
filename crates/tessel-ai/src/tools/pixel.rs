use serde_json::{json, Value};

pub(super) fn pixel_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "pixel_create_canvas",
            "description": "Create a layered frame-by-frame pixel canvas (default 32x32) with one transparent layer. Each frame owns its own layers; use pixel_add_layer / pixel_add_frame to grow the sprite.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "width": { "type": "integer", "minimum": 1, "maximum": 512 },
                    "height": { "type": "integer", "minimum": 1, "maximum": 512 }
                }
            }
        }),
        json!({
            "name": "pixel_add_layer",
            "description": "Add a transparent pixel layer on top of a canvas frame. Returns the new layer_id for pixel_draw.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "pixel_draw",
            "description": "Paint pixels on one canvas layer (origin top-left). Each entry takes x, y and palette_index (1-255); omit palette_index or pass null to erase. Supports optional mirror_x / mirror_y symmetry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" },
                    "pixels": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 16384,
                        "items": {
                            "type": "object",
                            "properties": {
                                "x": { "type": "integer", "minimum": 0 },
                                "y": { "type": "integer", "minimum": 0 },
                                "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 }
                            },
                            "required": ["x", "y"]
                        }
                    }
                },
                "required": ["pixels"]
            }
        }),
        json!({
            "name": "pixel_set_layer",
            "description": "Update a pixel layer's visibility and/or opacity (0.0..=1.0) on a canvas frame.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "visible": { "type": "boolean" },
                    "opacity": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                },
                "required": ["layer_id"]
            }
        }),
        json!({
            "name": "pixel_set_onion_skin",
            "description": "Toggle onion skinning on a pixel canvas.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["enabled"]
            }
        }),
        json!({
            "name": "pixel_flood_fill",
            "description": "4-connected flood fill starting at (x, y) on a pixel canvas layer. Pass palette_index: null to erase the connected region.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "x": { "type": "integer", "minimum": 0 },
                    "y": { "type": "integer", "minimum": 0 },
                    "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 }
                },
                "required": ["x", "y"]
            }
        }),
        json!({
            "name": "pixel_draw_line",
            "description": "Draw a Bresenham pixel line from start [x,y] to end [x,y] on a pixel canvas layer. Pass palette_index: null to erase. Supports mirror_x / mirror_y.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "start": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 },
                    "end": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 },
                    "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" }
                },
                "required": ["start", "end"]
            }
        }),
        json!({
            "name": "pixel_draw_rect",
            "description": "Draw a pixel rectangle (outline or filled) from min [x,y] to max [x,y] on a pixel canvas layer. Pass palette_index: null to erase. Supports mirror_x / mirror_y.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 },
                    "layer_id": { "type": "integer", "minimum": 1 },
                    "min": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 },
                    "max": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 },
                    "palette_index": { "type": ["integer", "null"], "minimum": 1, "maximum": 255 },
                    "filled": { "type": "boolean" },
                    "mirror_x": { "type": "boolean" },
                    "mirror_y": { "type": "boolean" }
                },
                "required": ["min", "max"]
            }
        }),
        json!({
            "name": "pixel_convert_to_voxel",
            "description": "Convert a 2D pixel canvas active frame into a 3D extruded voxel volume node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "depth": { "type": "integer", "minimum": 1, "maximum": 32 },
                    "name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "pixel_add_frame",
            "description": "Append a blank animation frame (with one transparent layer) to a pixel canvas and make it active.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "pixel_set_active_frame",
            "description": "Select which canvas frame is active for drawing, inspection, and single-frame PNG export.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 }
                },
                "required": ["frame"]
            }
        }),
        json!({
            "name": "pixel_set_fps",
            "description": "Set canvas playback speed (frames per second, 1-60); controls animated GIF frame delay. Default is 8.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "fps": { "type": "integer", "minimum": 1, "maximum": 60 }
                },
                "required": ["fps"]
            }
        }),
        json!({
            "name": "pixel_inspect",
            "description": "Report canvas dimensions, fps, frames, and per-frame layers with painted-pixel counts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "frame": { "type": "integer", "minimum": 0 }
                }
            }
        }),
        json!({
            "name": "pixel_export_sprite_sheet",
            "description": "Pack all canvas frames side-by-side into one PNG sprite sheet inside the export dir and return frame rects with durations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer" },
                    "filename": { "type": "string" }
                }
            }
        }),
    ]
}
