#![allow(unused_imports, unused_variables)]
use glam::Quat;
use serde::{Deserialize, Serialize};

use crate::character::{
    AnimationClip, Bone, BoneId, CharacterRig, ClipLayer, Keyframe, SkinBinding,
};
use crate::error::CoreError;
use crate::mesh::{
    ops as mesh_ops, FaceId, LowPolyMesh, MeshMaterial, MeshPaintLayer, MeshTexture, PrimitiveKind,
    UvProjection, DEFAULT_MESH_TEX_SIZE,
};
use crate::palette::{PaletteEntry, PaletteIndex, Rgba8};
use crate::pixel::{FrameIndex, LayerId, PixelCanvas, PixelEdit, PixelFrame, PixelLayer};
use crate::scene::{AppMode, NodeId, NodeKind, SceneNode, SceneState, Transform};
use crate::voxel::{ops as voxel_ops, VoxelCoord, VoxelEdit, VoxelVolume};

use super::{insert_validated_scene_node, Command, UndoDelta};

pub(super) fn apply(cmd: &Command, scene: &mut SceneState) -> Result<UndoDelta, CoreError> {
    let delta = match cmd {
        Command::CreatePixelCanvas {
            name,
            width,
            height,
        } => {
            let canvas = PixelCanvas::new(*width, *height)?;
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::PixelCanvas,
                None,
                Transform::default(),
            )?;
            scene.pixel_canvases.insert(id, canvas);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::ResizePixelCanvas {
            node_id,
            width,
            height,
        } => {
            if *width == 0
                || *height == 0
                || *width > crate::pixel::MAX_PIXEL_CANVAS_DIM
                || *height > crate::pixel::MAX_PIXEL_CANVAS_DIM
            {
                return Err(CoreError::OutOfBounds);
            }
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            if canvas.width == *width && canvas.height == *height {
                return Ok(UndoDelta::Transaction(Vec::new()));
            }
            let previous = Box::new(canvas.clone());
            canvas.resize(*width, *height)?;
            UndoDelta::RestorePixelCanvas {
                node_id: *node_id,
                previous,
            }
        }
        Command::DrawPixels {
            node_id,
            frame,
            layer_id,
            edits,
        } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            let frame_ref = canvas
                .frame_mut(*frame)
                .ok_or(CoreError::PixelFrameNotFound(*frame))?;
            let layer = frame_ref
                .layer_mut(*layer_id)
                .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;

            if edits.len() > layer.pixels().len() / 8 {
                let rle_pixels = layer.encode_rle();
                for edit in edits {
                    let _ = layer.set(edit.coord, edit.color);
                }
                UndoDelta::RestorePixelLayerRle {
                    node_id: *node_id,
                    frame: *frame,
                    layer_id: *layer_id,
                    rle_pixels,
                }
            } else {
                let mut previous_pixels = Vec::with_capacity(edits.len());
                for edit in edits {
                    if let Some(prev) = layer.set(edit.coord, edit.color) {
                        previous_pixels.push(PixelEdit {
                            coord: edit.coord,
                            color: prev,
                        });
                    }
                }
                UndoDelta::RestorePixels {
                    node_id: *node_id,
                    frame: *frame,
                    layer_id: *layer_id,
                    previous_pixels,
                }
            }
        }
        Command::FloodFillPixels {
            node_id,
            frame,
            layer_id,
            start,
            color,
        } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            if start.x >= canvas.width || start.y >= canvas.height {
                return Err(CoreError::OutOfBounds);
            }
            let frame_ref = canvas
                .frame_mut(*frame)
                .ok_or(CoreError::PixelFrameNotFound(*frame))?;
            let layer = frame_ref
                .layer_mut(*layer_id)
                .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;
            if layer.get(*start) == *color {
                return Ok(UndoDelta::Transaction(Vec::new()));
            }
            let rle_pixels = layer.encode_rle();
            crate::pixel::ops::flood_fill_layer_in_place(layer, *start, *color);
            UndoDelta::RestorePixelLayerRle {
                node_id: *node_id,
                frame: *frame,
                layer_id: *layer_id,
                rle_pixels,
            }
        }
        Command::AddPixelLayer {
            node_id,
            frame,
            name,
        } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            if canvas.frames.len() >= 128 {
                return Err(CoreError::OutOfBounds);
            }
            let (w, h) = (canvas.width, canvas.height);
            {
                let frame_check = canvas
                    .frame(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                if frame_check.layers.len() >= 32 {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let new_id = canvas.allocate_layer_id();
            let frame_ref = canvas
                .frame_mut(*frame)
                .ok_or(CoreError::PixelFrameNotFound(*frame))?;
            let previous_active = frame_ref.active_layer;
            frame_ref.layers.push(PixelLayer::new(new_id, name, w, h));
            frame_ref.active_layer = frame_ref.layers.len().saturating_sub(1);
            UndoDelta::RemovePixelLayer {
                node_id: *node_id,
                frame: *frame,
                layer_id: new_id,
                previous_active,
            }
        }
        Command::SetPixelLayerVisible {
            node_id,
            frame,
            layer_id,
            visible,
        } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            let frame_ref = canvas
                .frame_mut(*frame)
                .ok_or(CoreError::PixelFrameNotFound(*frame))?;
            let layer = frame_ref
                .layer_mut(*layer_id)
                .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;
            let previous = layer.visible;
            layer.visible = *visible;
            UndoDelta::RestorePixelLayerVisible {
                node_id: *node_id,
                frame: *frame,
                layer_id: *layer_id,
                previous,
            }
        }
        Command::AddPixelFrame { node_id } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            if canvas.frames.len() >= 128 {
                return Err(CoreError::OutOfBounds);
            }
            let (w, h) = (canvas.width, canvas.height);
            let layer_id = canvas.allocate_layer_id();
            let previous_active = canvas.active_frame;
            canvas.frames.push(PixelFrame::new(layer_id, w, h));
            canvas.active_frame = canvas.frames.len().saturating_sub(1);
            UndoDelta::PopPixelFrame {
                node_id: *node_id,
                previous_active,
            }
        }
        Command::SetActivePixelFrame { node_id, frame } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            if frame.0 >= canvas.frames.len() {
                return Err(CoreError::PixelFrameNotFound(*frame));
            }
            let previous_active = canvas.active_frame;
            canvas.active_frame = frame.0;
            UndoDelta::RestoreActivePixelFrame {
                node_id: *node_id,
                previous_active,
            }
        }
        Command::SetPixelFps { node_id, fps } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            let previous = canvas.fps;
            canvas.fps = *fps;
            UndoDelta::RestorePixelFps {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetPixelLayerOpacity {
            node_id,
            frame,
            layer_id,
            opacity,
        } => {
            if !opacity.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            let frame_ref = canvas
                .frame_mut(*frame)
                .ok_or(CoreError::PixelFrameNotFound(*frame))?;
            let layer = frame_ref
                .layer_mut(*layer_id)
                .ok_or(CoreError::PixelLayerNotFound(*layer_id))?;
            let previous = layer.opacity;
            layer.opacity = opacity.clamp(0.0, 1.0);
            UndoDelta::RestorePixelLayerOpacity {
                node_id: *node_id,
                frame: *frame,
                layer_id: *layer_id,
                previous,
            }
        }
        Command::SetPixelOnionSkin { node_id, enabled } => {
            let canvas = scene
                .pixel_canvases
                .get_mut(node_id)
                .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
            let previous = canvas.onion_skin;
            canvas.onion_skin = *enabled;
            UndoDelta::RestorePixelOnionSkin {
                node_id: *node_id,
                previous,
            }
        }
        _ => return Err(CoreError::OutOfBounds),
    };
    scene.mark_modified();
    Ok(delta)
}
