use glam::Quat;
use serde::{Deserialize, Serialize};

use crate::character::{BoneId, CharacterRig};
use crate::error::CoreError;
use crate::mesh::{ops as mesh_ops, FaceId, LowPolyMesh, PrimitiveKind};
use crate::palette::{PaletteEntry, PaletteIndex};
use crate::pixel::{FrameIndex, LayerId, PixelCanvas, PixelEdit, PixelFrame, PixelLayer};
use crate::scene::{AppMode, NodeId, NodeKind, SceneNode, SceneState, Transform};
use crate::voxel::{ops as voxel_ops, VoxelCoord, VoxelEdit, VoxelVolume};

/// Single Command Bus: every mutation from UI, Direct AI, or MCP flows through this enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    SetMode(AppMode),
    CreateNode {
        name: String,
        kind: NodeKind,
        transform: Transform,
    },
    CreatePixelCanvas {
        name: String,
        width: u32,
        height: u32,
    },
    DeleteNode(NodeId),
    SelectNode(Option<NodeId>),
    SetPaletteEntry {
        index: PaletteIndex,
        entry: PaletteEntry,
    },
    DrawPixels {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        edits: Vec<PixelEdit>,
    },
    AddPixelLayer {
        node_id: NodeId,
        frame: FrameIndex,
        name: String,
    },
    SetPixelLayerVisible {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        visible: bool,
    },
    AddPixelFrame {
        node_id: NodeId,
    },
    SetActivePixelFrame {
        node_id: NodeId,
        frame: FrameIndex,
    },
    CreateVoxelVolume {
        name: String,
    },
    SetVoxels {
        node_id: NodeId,
        edits: Vec<VoxelEdit>,
    },
    FillVoxelBox {
        node_id: NodeId,
        min: VoxelCoord,
        max: VoxelCoord,
        color: Option<PaletteIndex>,
        hollow: bool,
    },
    ConvertPixelToVoxel {
        pixel_node_id: NodeId,
        depth: u32,
        name: String,
    },
    CreatePrimitiveMesh {
        name: String,
        kind: PrimitiveKind,
        size: f32,
        material: PaletteIndex,
    },
    ConvertVoxelToLowPoly {
        voxel_node_id: NodeId,
        name: String,
    },
    ExtrudeMeshFace {
        node_id: NodeId,
        face_id: FaceId,
        distance: f32,
    },
    ApplyBipedRig,
    SetBoneRotation {
        bone_id: BoneId,
        rotation: Quat,
    },
    Transaction(Vec<Command>),
}

/// Minimal inverse operation captured when applying a `Command`.
#[derive(Debug, Clone, PartialEq)]
pub enum UndoDelta {
    RestoreMode(AppMode),
    RemoveNode {
        id: NodeId,
        previous_selection: Option<NodeId>,
    },
    RestoreNode {
        node: SceneNode,
        pixel_canvas: Option<PixelCanvas>,
        voxel_volume: Option<VoxelVolume>,
        lowpoly_mesh: Option<LowPolyMesh>,
        previous_selection: Option<NodeId>,
    },
    RestoreSelection(Option<NodeId>),
    RestorePaletteEntry {
        index: PaletteIndex,
        previous: Option<PaletteEntry>,
    },
    RestorePixels {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        previous_pixels: Vec<PixelEdit>,
    },
    RemovePixelLayer {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        previous_active: usize,
    },
    RestorePixelLayerVisible {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        previous: bool,
    },
    PopPixelFrame {
        node_id: NodeId,
        previous_active: usize,
    },
    RestoreActivePixelFrame {
        node_id: NodeId,
        previous_active: usize,
    },
    RestoreVoxels {
        node_id: NodeId,
        previous_voxels: Vec<VoxelEdit>,
    },
    RestoreLowPolyMesh {
        node_id: NodeId,
        previous_mesh: LowPolyMesh,
    },
    RestoreCharacterRig(Option<CharacterRig>),
    RestoreBoneRotation {
        bone_id: BoneId,
        previous: Quat,
    },
    Transaction(Vec<UndoDelta>),
}

impl Command {
    pub fn apply(&self, scene: &mut SceneState) -> Result<UndoDelta, CoreError> {
        let delta = match self {
            Self::SetMode(mode) => {
                let prev = scene.active_mode;
                scene.active_mode = *mode;
                UndoDelta::RestoreMode(prev)
            }
            Self::CreateNode {
                name,
                kind,
                transform,
            } => {
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: *kind,
                        visible: true,
                        transform: *transform,
                    },
                );
                if *kind == NodeKind::PixelCanvas {
                    let canvas = PixelCanvas::new(32, 32)?;
                    scene.pixel_canvases.insert(id, canvas);
                } else if *kind == NodeKind::VoxelVolume {
                    scene.voxel_volumes.insert(id, VoxelVolume::new());
                }
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::CreatePixelCanvas {
                name,
                width,
                height,
            } => {
                let canvas = PixelCanvas::new(*width, *height)?;
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::PixelCanvas,
                        visible: true,
                        transform: Transform::default(),
                    },
                );
                scene.pixel_canvases.insert(id, canvas);
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::DeleteNode(id) => {
                let node = scene.nodes.remove(id).ok_or(CoreError::NodeNotFound(*id))?;
                let pixel_canvas = scene.pixel_canvases.remove(id);
                let voxel_volume = scene.voxel_volumes.remove(id);
                let lowpoly_mesh = scene.lowpoly_meshes.remove(id);
                let previous_selection = scene.selected_node;
                if scene.selected_node == Some(*id) {
                    scene.selected_node = None;
                }
                UndoDelta::RestoreNode {
                    node,
                    pixel_canvas,
                    voxel_volume,
                    lowpoly_mesh,
                    previous_selection,
                }
            }
            Self::SelectNode(target) => {
                if let Some(id) = target {
                    if !scene.nodes.contains_key(id) {
                        return Err(CoreError::NodeNotFound(*id));
                    }
                }
                let prev = scene.selected_node;
                scene.selected_node = *target;
                UndoDelta::RestoreSelection(prev)
            }
            Self::SetPaletteEntry { index, entry } => {
                let previous = scene.palette.set(*index, entry.clone());
                UndoDelta::RestorePaletteEntry {
                    index: *index,
                    previous,
                }
            }
            Self::DrawPixels {
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
            Self::AddPixelLayer {
                node_id,
                frame,
                name,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                let (w, h) = (canvas.width, canvas.height);
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
            Self::SetPixelLayerVisible {
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
            Self::AddPixelFrame { node_id } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
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
            Self::SetActivePixelFrame { node_id, frame } => {
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
            Self::CreateVoxelVolume { name } => {
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::VoxelVolume,
                        visible: true,
                        transform: Transform::default(),
                    },
                );
                scene.voxel_volumes.insert(id, VoxelVolume::new());
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::SetVoxels { node_id, edits } => {
                let volume = scene
                    .voxel_volumes
                    .get_mut(node_id)
                    .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
                let mut previous_voxels = Vec::with_capacity(edits.len());
                for edit in edits {
                    let prev = volume.set(edit.coord, edit.color);
                    previous_voxels.push(VoxelEdit {
                        coord: edit.coord,
                        color: prev,
                    });
                }
                UndoDelta::RestoreVoxels {
                    node_id: *node_id,
                    previous_voxels,
                }
            }
            Self::FillVoxelBox {
                node_id,
                min,
                max,
                color,
                hollow,
            } => {
                let edits = voxel_ops::box_fill_edits(*min, *max, *color, *hollow);
                let volume = scene
                    .voxel_volumes
                    .get_mut(node_id)
                    .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
                let mut previous_voxels = Vec::with_capacity(edits.len());
                for edit in edits {
                    let prev = volume.set(edit.coord, edit.color);
                    previous_voxels.push(VoxelEdit {
                        coord: edit.coord,
                        color: prev,
                    });
                }
                UndoDelta::RestoreVoxels {
                    node_id: *node_id,
                    previous_voxels,
                }
            }
            Self::ConvertPixelToVoxel {
                pixel_node_id,
                depth,
                name,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get(pixel_node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*pixel_node_id))?;
                let vol = voxel_ops::pixel_to_voxel_volume(canvas, *depth);
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::VoxelVolume,
                        visible: true,
                        transform: Transform::default(),
                    },
                );
                scene.voxel_volumes.insert(id, vol);
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::CreatePrimitiveMesh {
                name,
                kind,
                size,
                material,
            } => {
                let mesh = mesh_ops::create_primitive(*kind, *size, *material);
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::LowPolyMesh,
                        visible: true,
                        transform: Transform::default(),
                    },
                );
                scene.lowpoly_meshes.insert(id, mesh);
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::ConvertVoxelToLowPoly {
                voxel_node_id,
                name,
            } => {
                let vol = scene
                    .voxel_volumes
                    .get(voxel_node_id)
                    .ok_or(CoreError::VoxelVolumeNotFound(*voxel_node_id))?;
                let mesh = mesh_ops::voxel_to_lowpoly(vol, scene.unit_scale);
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::LowPolyMesh,
                        visible: true,
                        transform: Transform::default(),
                    },
                );
                scene.lowpoly_meshes.insert(id, mesh);
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::ExtrudeMeshFace {
                node_id,
                face_id,
                distance,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::extrude_face(&current, *face_id, *distance)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::ApplyBipedRig => {
                let prev = scene.character_rig.replace(CharacterRig::biped_template());
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::SetBoneRotation { bone_id, rotation } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                let previous = bone.pose_local.rotation;
                bone.pose_local.rotation = *rotation;
                UndoDelta::RestoreBoneRotation {
                    bone_id: *bone_id,
                    previous,
                }
            }
            Self::Transaction(commands) => {
                if commands.is_empty() {
                    return Err(CoreError::EmptyTransaction);
                }
                let mut applied = Vec::with_capacity(commands.len());
                for cmd in commands {
                    match cmd.apply(scene) {
                        Ok(d) => applied.push(d),
                        Err(err) => {
                            for prev_delta in applied.into_iter().rev() {
                                let _ = prev_delta.revert(scene);
                            }
                            return Err(err);
                        }
                    }
                }
                return Ok(UndoDelta::Transaction(applied));
            }
        };

        scene.mark_modified();
        Ok(delta)
    }
}

impl UndoDelta {
    pub fn revert(&self, scene: &mut SceneState) -> Result<(), CoreError> {
        match self {
            Self::RestoreMode(mode) => {
                scene.active_mode = *mode;
            }
            Self::RemoveNode {
                id,
                previous_selection,
            } => {
                scene.nodes.remove(id);
                scene.pixel_canvases.remove(id);
                scene.voxel_volumes.remove(id);
                scene.lowpoly_meshes.remove(id);
                scene.selected_node = *previous_selection;
            }
            Self::RestoreNode {
                node,
                pixel_canvas,
                voxel_volume,
                lowpoly_mesh,
                previous_selection,
            } => {
                if let Some(canvas) = pixel_canvas {
                    scene.pixel_canvases.insert(node.id, canvas.clone());
                }
                if let Some(volume) = voxel_volume {
                    scene.voxel_volumes.insert(node.id, volume.clone());
                }
                if let Some(mesh) = lowpoly_mesh {
                    scene.lowpoly_meshes.insert(node.id, mesh.clone());
                }
                scene.nodes.insert(node.id, node.clone());
                scene.selected_node = *previous_selection;
            }
            Self::RestoreSelection(prev) => {
                scene.selected_node = *prev;
            }
            Self::RestorePaletteEntry { index, previous } => match previous {
                Some(entry) => {
                    scene.palette.set(*index, entry.clone());
                }
                None => {
                    scene.palette.remove(*index);
                }
            },
            Self::RestorePixels {
                node_id,
                frame,
                layer_id,
                previous_pixels,
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
                for prev in previous_pixels.iter().rev() {
                    layer.set(prev.coord, prev.color);
                }
            }
            Self::RemovePixelLayer {
                node_id,
                frame,
                layer_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                let frame_ref = canvas
                    .frame_mut(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                frame_ref.layers.retain(|l| l.id != *layer_id);
                frame_ref.active_layer =
                    (*previous_active).min(frame_ref.layers.len().saturating_sub(1));
            }
            Self::RestorePixelLayerVisible {
                node_id,
                frame,
                layer_id,
                previous,
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
                layer.visible = *previous;
            }
            Self::PopPixelFrame {
                node_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                canvas.frames.pop();
                canvas.active_frame = (*previous_active).min(canvas.frames.len().saturating_sub(1));
            }
            Self::RestoreActivePixelFrame {
                node_id,
                previous_active,
            } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                canvas.active_frame = *previous_active;
            }
            Self::RestoreVoxels {
                node_id,
                previous_voxels,
            } => {
                let volume = scene
                    .voxel_volumes
                    .get_mut(node_id)
                    .ok_or(CoreError::VoxelVolumeNotFound(*node_id))?;
                for prev in previous_voxels.iter().rev() {
                    volume.set(prev.coord, prev.color);
                }
            }
            Self::RestoreLowPolyMesh {
                node_id,
                previous_mesh,
            } => {
                scene.lowpoly_meshes.insert(*node_id, previous_mesh.clone());
            }
            Self::RestoreCharacterRig(prev) => {
                scene.character_rig = prev.clone();
            }
            Self::RestoreBoneRotation { bone_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.pose_local.rotation = *previous;
            }
            Self::Transaction(deltas) => {
                for delta in deltas.iter().rev() {
                    delta.revert(scene)?;
                }
                return Ok(());
            }
        }
        scene.mark_modified();
        Ok(())
    }
}
