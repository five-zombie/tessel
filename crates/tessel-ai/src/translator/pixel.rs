#![allow(unused_imports, unused_variables)]

use std::path::Path;

use serde_json::{json, Value};
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    apply_primitive_anchor, bake_mesh_maps, generate_smart_material_texture, AppMode, BakeMapKind,
    BakeParams, BoneId, Command, FrameIndex, HistoryStack, Keyframe, LayerId, NodeId, PaletteEntry,
    PaletteIndex, PixelCoord, PixelEdit, PrimitiveAnchor, PrimitiveKind, Rgba8, SceneState,
    SmartMaterialParams, VoxelCoord,
};
use tessel_io::{
    export_mesh_texture_png_bytes, export_scene_to_dir, export_sprite_sheet_bytes,
    import_mesh_from_path, import_mesh_texture_from_path, load_tessel_project,
    resolve_safe_export_path, save_tessel_project_atomic, ExportFormat, ExportPreset,
};

use super::helpers::*;
use super::{execute_tool_call, palette_summary_json, scene_summary_json};
use crate::error::AiError;

pub(super) fn execute(
    name: &str,
    args: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &Path,
) -> Result<Value, AiError> {
    match name {
        "pixel_create_canvas" => {
            let max_dim = u64::from(tessel_core::MAX_PIXEL_CANVAS_DIM);
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Pixel Sprite")
                .to_string();
            let width = match args.get("width") {
                None | Some(Value::Null) => 32,
                Some(v) => v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments(format!(
                        "'width' must be a positive integer in 1..={max_dim}"
                    ))
                })?,
            };
            let height = match args.get("height") {
                None | Some(Value::Null) => 32,
                Some(v) => v.as_u64().ok_or_else(|| {
                    AiError::InvalidArguments(format!(
                        "'height' must be a positive integer in 1..={max_dim}"
                    ))
                })?,
            };
            if width == 0 || height == 0 || width > max_dim || height > max_dim {
                return Err(AiError::InvalidArguments(format!(
                    "pixel canvas dimensions must be 1..={max_dim}"
                )));
            }
            history.execute(
                scene,
                Command::CreatePixelCanvas {
                    name,
                    width: width as u32,
                    height: height as u32,
                },
            )?;
            let node_id = scene
                .active_pixel_node()
                .ok_or_else(|| AiError::InvalidArguments("failed to create pixel canvas".into()))?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("failed to create pixel canvas".into()))?;
            let frame = canvas
                .current_frame()
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas has no frames".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": canvas.width,
                "height": canvas.height,
                "active_frame": canvas.active_frame,
                "layers": pixel_frame_summary(frame),
            }))
        }
        "pixel_resize_canvas" => {
            let max_dim = u64::from(tessel_core::MAX_PIXEL_CANVAS_DIM);
            let node_id = match args.get("node_id") {
                None | Some(Value::Null) => scene.active_pixel_node().ok_or_else(|| {
                    AiError::InvalidArguments("no pixel canvas found in scene".into())
                })?,
                Some(v) => {
                    let raw = v.as_u64().ok_or_else(|| {
                        AiError::InvalidArguments("'node_id' must be a positive integer".into())
                    })?;
                    let id = NodeId(raw);
                    if !scene.pixel_canvases.contains_key(&id) {
                        return Err(AiError::InvalidArguments(format!(
                            "pixel canvas node {raw} not found"
                        )));
                    }
                    id
                }
            };
            let width = args
                .get("width")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'width'".into()))?;
            let height = args
                .get("height")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'height'".into()))?;
            if width == 0 || height == 0 || width > max_dim || height > max_dim {
                return Err(AiError::InvalidArguments(format!(
                    "pixel canvas dimensions must be 1..={max_dim}"
                )));
            }
            let (previous_width, previous_height) = scene
                .pixel_canvases
                .get(&node_id)
                .map(|c| (c.width, c.height))
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            if (width as u32, height as u32) != (previous_width, previous_height) {
                history.execute(
                    scene,
                    Command::ResizePixelCanvas {
                        node_id,
                        width: width as u32,
                        height: height as u32,
                    },
                )?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": width,
                "height": height,
                "previous_width": previous_width,
                "previous_height": previous_height,
            }))
        }
        "pixel_add_layer" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let (frame_count, active_frame) = {
                let canvas = scene
                    .pixel_canvases
                    .get(&node_id)
                    .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
                (canvas.frames.len(), canvas.active_frame)
            };
            let frame_idx = resolve_pixel_frame(args, frame_count, active_frame)?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Layer")
                .to_string();
            history.execute(
                scene,
                Command::AddPixelLayer {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    name,
                },
            )?;
            let layer_id = scene
                .pixel_canvases
                .get(&node_id)
                .and_then(|c| c.frame(FrameIndex(frame_idx)))
                .and_then(|f| f.layers.last())
                .map(|l| l.id.0)
                .ok_or_else(|| AiError::InvalidArguments("pixel layer not found".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id,
            }))
        }
        "pixel_draw" => {
            const MAX_PIXEL_EDITS: usize = 16_384;
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (u64::from(canvas.width), u64::from(canvas.height));
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            if frame.layer(layer_id).is_none() {
                return Err(AiError::InvalidArguments(format!(
                    "pixel layer {} not found in frame {frame_idx}",
                    layer_id.0
                )));
            }
            let raw_pixels = args
                .get("pixels")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::InvalidArguments("missing 'pixels' array".into()))?;
            if raw_pixels.is_empty() || raw_pixels.len() > MAX_PIXEL_EDITS {
                return Err(AiError::InvalidArguments(format!(
                    "pixel_draw needs 1..={MAX_PIXEL_EDITS} pixels"
                )));
            }
            let sym = parse_symmetry_2d(args)?;
            let mut edits = Vec::with_capacity(raw_pixels.len());
            for (i, item) in raw_pixels.iter().enumerate() {
                let x = item
                    .get("x")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("pixels[{i}].x missing")))?;
                let y = item
                    .get("y")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AiError::InvalidArguments(format!("pixels[{i}].y missing")))?;
                if x >= width || y >= height {
                    return Err(AiError::InvalidArguments(format!(
                        "pixels[{i}] ({x},{y}) outside {width}x{height} canvas"
                    )));
                }
                let color = parse_opt_palette_index(item, None)?;
                let pts = tessel_core::pixel::ops::expand_symmetry(
                    PixelCoord::new(x as u32, y as u32),
                    width as u32,
                    height as u32,
                    sym,
                );
                for coord in pts {
                    if edits.len() >= MAX_PIXEL_EDITS {
                        return Err(AiError::InvalidArguments(format!(
                            "pixel_draw exceeds {MAX_PIXEL_EDITS} pixel edits after symmetry"
                        )));
                    }
                    edits.push(PixelEdit { coord, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted,
            }))
        }
        "pixel_set_layer" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let raw_lid = args
                .get("layer_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'layer_id'".into()))?;
            let layer_id = LayerId(
                u32::try_from(raw_lid)
                    .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
            );
            let mut cmds = Vec::new();
            if let Some(vis) = args.get("visible").and_then(Value::as_bool) {
                cmds.push(Command::SetPixelLayerVisible {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    visible: vis,
                });
            }
            if let Some(op_val) = args.get("opacity") {
                let op = op_val.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments("'opacity' must be a finite number 0.0..=1.0".into())
                })? as f32;
                if !op.is_finite() {
                    return Err(AiError::InvalidArguments(
                        "'opacity' must be a finite number 0.0..=1.0".into(),
                    ));
                }
                cmds.push(Command::SetPixelLayerOpacity {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    opacity: op.clamp(0.0, 1.0),
                });
            }
            if cmds.is_empty() {
                return Err(AiError::InvalidArguments(
                    "pixel_set_layer requires 'visible' and/or 'opacity'".into(),
                ));
            }
            if cmds.len() == 1 {
                if let Some(c) = cmds.into_iter().next() {
                    history.execute(scene, c)?;
                }
            } else {
                history.execute(scene, Command::Transaction(cmds))?;
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0
            }))
        }
        "pixel_set_onion_skin" => {
            let node_id = find_pixel_node(args, scene)?;
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| AiError::InvalidArguments("missing boolean 'enabled'".into()))?;
            history.execute(scene, Command::SetPixelOnionSkin { node_id, enabled })?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "onion_skin": enabled }))
        }
        "pixel_flood_fill" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let layer = frame
                .layer(layer_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel layer not found".into()))?;
            let x = args
                .get("x")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'x'".into()))?;
            let y = args
                .get("y")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'y'".into()))?;
            if x >= u64::from(canvas.width) || y >= u64::from(canvas.height) {
                return Err(AiError::InvalidArguments(
                    "flood fill coordinate outside canvas".into(),
                ));
            }
            let color = parse_opt_palette_index(args, None)?;
            let start = PixelCoord::new(x as u32, y as u32);
            let filled = if layer.get(start) == color {
                0
            } else {
                let before_count = layer.pixels().iter().filter(|&&c| c == color).count();
                history.execute(
                    scene,
                    Command::FloodFillPixels {
                        node_id,
                        frame: FrameIndex(frame_idx),
                        layer_id,
                        start,
                        color,
                    },
                )?;
                let after_count = scene
                    .pixel_canvases
                    .get(&node_id)
                    .and_then(|c| c.frame(FrameIndex(frame_idx)))
                    .and_then(|f| f.layer(layer_id))
                    .map(|l| l.pixels().iter().filter(|&&c| c == color).count())
                    .unwrap_or(before_count);
                after_count.saturating_sub(before_count)
            };
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_filled": filled
            }))
        }
        "pixel_draw_line" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (canvas.width, canvas.height);
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let parse_pt = |key: &str| -> Result<PixelCoord, AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() == 2)
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}' must be [x, y]")))?;
                let px = arr[0]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[0]' must be int")))?;
                let py = arr[1]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[1]' must be int")))?;
                if px >= u64::from(width) || py >= u64::from(height) {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' ({px},{py}) outside {width}x{height} canvas"
                    )));
                }
                Ok(PixelCoord::new(px as u32, py as u32))
            };
            let start = parse_pt("start")?;
            let end = parse_pt("end")?;
            let color = parse_opt_palette_index(args, None)?;
            let sym = parse_symmetry_2d(args)?;
            let mut edits = Vec::new();
            for pt in tessel_core::pixel::ops::line_points(start, end) {
                for mc in tessel_core::pixel::ops::expand_symmetry(pt, width, height, sym) {
                    edits.push(PixelEdit { coord: mc, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted
            }))
        }
        "pixel_draw_rect" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (width, height) = (canvas.width, canvas.height);
            let frame_idx = resolve_pixel_frame(args, canvas.frames.len(), canvas.active_frame)?;
            let frame = canvas
                .frame(FrameIndex(frame_idx))
                .ok_or_else(|| AiError::InvalidArguments("pixel frame not found".into()))?;
            let layer_id = match args.get("layer_id").and_then(Value::as_u64) {
                Some(raw) => LayerId(
                    u32::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("layer_id out of range".into()))?,
                ),
                None => frame
                    .active_layer_id()
                    .ok_or_else(|| AiError::InvalidArguments("pixel frame has no layers".into()))?,
            };
            let parse_pt = |key: &str| -> Result<PixelCoord, AiError> {
                let arr = args
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|a| a.len() == 2)
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}' must be [x, y]")))?;
                let px = arr[0]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[0]' must be int")))?;
                let py = arr[1]
                    .as_u64()
                    .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[1]' must be int")))?;
                if px >= u64::from(width) || py >= u64::from(height) {
                    return Err(AiError::InvalidArguments(format!(
                        "'{key}' ({px},{py}) outside {width}x{height} canvas"
                    )));
                }
                Ok(PixelCoord::new(px as u32, py as u32))
            };
            let p0 = parse_pt("min")?;
            let p1 = parse_pt("max")?;
            let filled = args.get("filled").and_then(Value::as_bool).unwrap_or(false);
            let color = parse_opt_palette_index(args, None)?;
            let sym = parse_symmetry_2d(args)?;
            const MAX_RECT_PIXEL_EDITS: usize = 16_384;
            let pts: Vec<PixelCoord> = if filled {
                let (x0, x1) = (p0.x.min(p1.x), p0.x.max(p1.x));
                let (y0, y1) = (p0.y.min(p1.y), p0.y.max(p1.y));
                let rect_w = x1.saturating_sub(x0) as usize + 1;
                let rect_h = y1.saturating_sub(y0) as usize + 1;
                if rect_w.saturating_mul(rect_h) > MAX_RECT_PIXEL_EDITS {
                    return Err(AiError::InvalidArguments(format!(
                        "pixel_draw_rect exceeds {MAX_RECT_PIXEL_EDITS} pixels"
                    )));
                }
                let mut v = Vec::with_capacity(rect_w * rect_h);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        v.push(PixelCoord::new(x, y));
                    }
                }
                v
            } else {
                tessel_core::pixel::ops::rect_outline_points(p0, p1)
            };
            let mut edits = Vec::with_capacity(pts.len());
            for pt in pts {
                for mc in tessel_core::pixel::ops::expand_symmetry(pt, width, height, sym) {
                    if edits.len() >= MAX_RECT_PIXEL_EDITS {
                        return Err(AiError::InvalidArguments(format!(
                            "pixel_draw_rect exceeds {MAX_RECT_PIXEL_EDITS} pixels"
                        )));
                    }
                    edits.push(PixelEdit { coord: mc, color });
                }
            }
            let painted = edits.len();
            history.execute(
                scene,
                Command::DrawPixels {
                    node_id,
                    frame: FrameIndex(frame_idx),
                    layer_id,
                    edits,
                },
            )?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": frame_idx,
                "layer_id": layer_id.0,
                "pixels_painted": painted
            }))
        }
        "pixel_convert_to_voxel" => {
            let pixel_node_id = find_pixel_node(args, scene)?;
            let depth = args
                .get("depth")
                .and_then(Value::as_u64)
                .map(|d| (d.min(32) as u32).max(1))
                .unwrap_or(4);
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Converted Voxel")
                .to_string();
            history.execute(
                scene,
                Command::ConvertPixelToVoxel {
                    pixel_node_id,
                    depth,
                    name,
                },
            )?;
            Ok(json!({ "ok": true, "new_node_id": scene.selected_node.map(|n| n.0) }))
        }
        "pixel_add_frame" => {
            let node_id = resolve_pixel_node(args, scene, history)?;
            history.execute(scene, Command::AddPixelFrame { node_id })?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let frame = canvas
                .current_frame()
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas has no frames".into()))?;
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "frame": canvas.active_frame,
                "layers": pixel_frame_summary(frame),
            }))
        }
        "pixel_set_active_frame" => {
            let node_id = find_pixel_node(args, scene)?;
            let frame_count = scene
                .pixel_canvases
                .get(&node_id)
                .map(|c| c.frames.len())
                .unwrap_or(0);
            let raw_frame = args
                .get("frame")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'frame'".into()))?;
            let frame = usize::try_from(raw_frame)
                .map_err(|_| AiError::InvalidArguments("frame overflow".into()))?;
            if frame >= frame_count {
                return Err(AiError::InvalidArguments(format!(
                    "pixel frame {frame} out of range (0..{frame_count})"
                )));
            }
            history.execute(
                scene,
                Command::SetActivePixelFrame {
                    node_id,
                    frame: FrameIndex(frame),
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "frame": frame }))
        }
        "pixel_set_fps" => {
            let node_id = find_pixel_node(args, scene)?;
            let fps = args
                .get("fps")
                .and_then(Value::as_u64)
                .ok_or_else(|| AiError::InvalidArguments("missing 'fps'".into()))?;
            if fps == 0 || fps > 60 {
                return Err(AiError::InvalidArguments("fps must be 1..=60".into()));
            }
            history.execute(
                scene,
                Command::SetPixelFps {
                    node_id,
                    fps: fps as u16,
                },
            )?;
            Ok(json!({ "ok": true, "node_id": node_id.0, "fps": fps }))
        }
        "pixel_inspect" => {
            let node_id = find_pixel_node(args, scene)?;
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let only_frame = match args.get("frame").and_then(Value::as_u64) {
                Some(raw) => Some(
                    usize::try_from(raw)
                        .map_err(|_| AiError::InvalidArguments("frame overflow".into()))?,
                ),
                None => None,
            };
            let mut frames = Vec::with_capacity(canvas.frames.len());
            for (idx, frame) in canvas.frames.iter().enumerate() {
                if only_frame.is_some_and(|f| f != idx) {
                    continue;
                }
                frames.push(json!({
                    "index": idx,
                    "layers": pixel_frame_summary(frame),
                }));
            }
            if frames.is_empty() {
                return Err(AiError::InvalidArguments("pixel frame not found".into()));
            }
            Ok(json!({
                "ok": true,
                "node_id": node_id.0,
                "width": canvas.width,
                "height": canvas.height,
                "fps": canvas.fps,
                "onion_skin": canvas.onion_skin,
                "active_frame": canvas.active_frame,
                "frame_count": canvas.frames.len(),
                "frames": frames,
            }))
        }
        "pixel_export_sprite_sheet" => {
            let node_id = find_pixel_node(args, scene)?;
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("sprite_sheet.png");
            let canvas = scene
                .pixel_canvases
                .get(&node_id)
                .ok_or_else(|| AiError::InvalidArguments("pixel canvas not found".into()))?;
            let (bytes, meta) = export_sprite_sheet_bytes(canvas, &scene.palette)?;
            let target = resolve_safe_export_path(allowed_export_dir, filename)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(tessel_io::IoError::from)?;
            }
            std::fs::write(&target, &bytes).map_err(tessel_io::IoError::from)?;
            Ok(json!({
                "ok": true,
                "output_path": target.display().to_string(),
                "file_size_bytes": bytes.len(),
                "sheet_width": meta.sheet_width,
                "sheet_height": meta.sheet_height,
                "fps": meta.fps,
                "frames": meta.frames,
            }))
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}
