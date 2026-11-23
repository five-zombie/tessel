use glam::Quat;
use serde::{Deserialize, Serialize};

use crate::character::{AnimationClip, BoneId, CharacterRig, Keyframe, SkinBinding};
use crate::error::CoreError;
use crate::mesh::{
    ops as mesh_ops, FaceId, LowPolyMesh, MeshMaterial, MeshTexture, PrimitiveKind,
    DEFAULT_MESH_TEX_SIZE,
};
use crate::palette::{PaletteEntry, PaletteIndex, Rgba8};
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
    SetNodeTransform {
        node_id: NodeId,
        transform: Transform,
    },
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
    ImportLowPolyMesh {
        name: String,
        mesh: LowPolyMesh,
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
    InsetMeshFace {
        node_id: NodeId,
        face_id: FaceId,
        factor: f32,
    },
    SmoothSubdivideMesh {
        node_id: NodeId,
    },
    SetMeshSmoothShading {
        node_id: NodeId,
        smooth: bool,
    },
    SetMeshMaterial {
        node_id: NodeId,
        material: MeshMaterial,
    },
    PaintMeshTexture {
        node_id: NodeId,
        uv: [f32; 2],
        radius_px: f32,
        color: Rgba8,
    },
    FillMeshTexture {
        node_id: NodeId,
        color: Rgba8,
    },
    SetMeshTexture {
        node_id: NodeId,
        texture: Option<MeshTexture>,
    },
    SetMeshNormalMap {
        node_id: NodeId,
        normal_map: Option<MeshTexture>,
    },
    ApplyBipedRig,
    SetBoneRotation {
        bone_id: BoneId,
        rotation: Quat,
    },
    BindNodeToBone {
        node_id: NodeId,
        bone_id: Option<BoneId>,
    },
    AddAnimationClip {
        name: String,
        duration_secs: f32,
    },
    SetAnimationKeyframe {
        clip_index: usize,
        bone_id: BoneId,
        keyframe: Keyframe,
    },
    RemoveAnimationKeyframe {
        clip_index: usize,
        bone_id: BoneId,
        time_secs: f32,
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
        node: Box<SceneNode>,
        pixel_canvas: Option<Box<PixelCanvas>>,
        voxel_volume: Option<Box<VoxelVolume>>,
        lowpoly_mesh: Option<Box<LowPolyMesh>>,
        previous_selection: Option<NodeId>,
    },
    RestoreSelection(Option<NodeId>),
    RestoreNodeTransform {
        node_id: NodeId,
        previous: Transform,
    },
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
    RestoreMeshTexturePixels {
        node_id: NodeId,
        was_none: bool,
        previous_pixels: Vec<(usize, [u8; 4])>,
    },
    RestoreMeshTextureFull {
        node_id: NodeId,
        previous: Option<MeshTexture>,
    },
    RestoreMeshNormalMap {
        node_id: NodeId,
        previous: Option<MeshTexture>,
    },
    RestoreMeshSmoothShading {
        node_id: NodeId,
        previous: bool,
    },
    RestoreMeshMaterial {
        node_id: NodeId,
        previous: MeshMaterial,
    },
    RestoreCharacterRig(Option<CharacterRig>),
    RestoreBoneRotation {
        bone_id: BoneId,
        previous: Quat,
    },
    RestoreNodeBoneBinding {
        node_id: NodeId,
        previous: Option<BoneId>,
    },
    PopAnimationClip {
        previous_active: usize,
        previous_time: f32,
    },
    RestoreAnimationClip {
        clip_index: usize,
        previous: AnimationClip,
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
                    node: Box::new(node),
                    pixel_canvas: pixel_canvas.map(Box::new),
                    voxel_volume: voxel_volume.map(Box::new),
                    lowpoly_mesh: lowpoly_mesh.map(Box::new),
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
            Self::SetNodeTransform { node_id, transform } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous = node.transform;
                node.transform = *transform;
                UndoDelta::RestoreNodeTransform {
                    node_id: *node_id,
                    previous,
                }
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
            Self::ImportLowPolyMesh { name, mesh } => {
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
                scene.lowpoly_meshes.insert(id, mesh.clone());
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
            Self::InsetMeshFace {
                node_id,
                face_id,
                factor,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::inset_face(&current, *face_id, *factor)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SmoothSubdivideMesh { node_id } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::smooth_subdivide_mesh(&current)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshSmoothShading { node_id, smooth } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = mesh.smooth_shading;
                mesh.smooth_shading = *smooth;
                UndoDelta::RestoreMeshSmoothShading {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetMeshMaterial { node_id, material } => {
                if !material.metallic.is_finite()
                    || !material.roughness.is_finite()
                    || material.emissive.iter().any(|c| !c.is_finite())
                {
                    return Err(CoreError::OutOfBounds);
                }
                let sanitized = MeshMaterial {
                    metallic: material.metallic.clamp(0.0, 1.0),
                    roughness: material.roughness.clamp(0.05, 1.0),
                    emissive: [
                        material.emissive[0].clamp(0.0, 1.0),
                        material.emissive[1].clamp(0.0, 1.0),
                        material.emissive[2].clamp(0.0, 1.0),
                    ],
                };
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = mesh.material;
                mesh.material = sanitized;
                UndoDelta::RestoreMeshMaterial {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::PaintMeshTexture {
                node_id,
                uv,
                radius_px,
                color,
            } => {
                let base_color = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .and_then(|m| m.faces.first())
                    .and_then(|f| scene.palette.get(f.material))
                    .map(|e| e.color)
                    .unwrap_or(Rgba8::rgb(225, 226, 235));
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let was_none = mesh.texture.is_none();
                let tex = mesh.texture.get_or_insert_with(|| {
                    MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                });
                let previous_pixels = tex.paint_disc(*uv, *radius_px, *color);
                UndoDelta::RestoreMeshTexturePixels {
                    node_id: *node_id,
                    was_none,
                    previous_pixels,
                }
            }
            Self::FillMeshTexture { node_id, color } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = mesh.texture.clone();
                match &mut mesh.texture {
                    Some(tex) => tex.fill(*color),
                    None => {
                        mesh.texture = Some(MeshTexture::solid(
                            DEFAULT_MESH_TEX_SIZE,
                            DEFAULT_MESH_TEX_SIZE,
                            *color,
                        ));
                    }
                }
                UndoDelta::RestoreMeshTextureFull {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetMeshTexture { node_id, texture } => {
                if let Some(tex) = texture {
                    let expected = (tex.width as usize)
                        .checked_mul(tex.height as usize)
                        .and_then(|px| px.checked_mul(4));
                    if tex.width == 0
                        || tex.height == 0
                        || tex.width > 1024
                        || tex.height > 1024
                        || expected != Some(tex.rgba.len())
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = mesh.texture.clone();
                mesh.texture = texture.clone();
                UndoDelta::RestoreMeshTextureFull {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetMeshNormalMap {
                node_id,
                normal_map,
            } => {
                if let Some(tex) = normal_map {
                    let expected = (tex.width as usize)
                        .checked_mul(tex.height as usize)
                        .and_then(|px| px.checked_mul(4));
                    if tex.width == 0
                        || tex.height == 0
                        || tex.width > 1024
                        || tex.height > 1024
                        || expected != Some(tex.rgba.len())
                    {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = mesh.normal_map.clone();
                mesh.normal_map = normal_map.clone();
                UndoDelta::RestoreMeshNormalMap {
                    node_id: *node_id,
                    previous,
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
            Self::BindNodeToBone { node_id, bone_id } => {
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if let Some(bid) = bone_id {
                    if rig.bone(*bid).is_none() {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let previous = rig
                    .bindings
                    .iter()
                    .find(|b| b.node_id == *node_id)
                    .map(|b| b.bone_id);
                rig.bindings.retain(|b| b.node_id != *node_id);
                if let Some(bid) = bone_id {
                    rig.bindings.push(SkinBinding {
                        node_id: *node_id,
                        bone_id: *bid,
                    });
                }
                UndoDelta::RestoreNodeBoneBinding {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::AddAnimationClip {
                name,
                duration_secs,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let previous_active = rig.active_animation;
                let previous_time = rig.playback_time;
                rig.animations
                    .push(AnimationClip::new_empty(name.clone(), *duration_secs));
                rig.active_animation = rig.animations.len().saturating_sub(1);
                rig.playback_time = 0.0;
                rig.playing = false;
                UndoDelta::PopAnimationClip {
                    previous_active,
                    previous_time,
                }
            }
            Self::SetAnimationKeyframe {
                clip_index,
                bone_id,
                keyframe,
            } => {
                if !keyframe.time_secs.is_finite() || keyframe.time_secs < 0.0 {
                    return Err(CoreError::OutOfBounds);
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if rig.bone(*bone_id).is_none() {
                    return Err(CoreError::OutOfBounds);
                }
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                clip.upsert_keyframe(*bone_id, *keyframe);
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::RemoveAnimationKeyframe {
                clip_index,
                bone_id,
                time_secs,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                clip.remove_keyframe(*bone_id, *time_secs);
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
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
                scene.rollback_node_id(*id);
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
                    scene.pixel_canvases.insert(node.id, (**canvas).clone());
                }
                if let Some(volume) = voxel_volume {
                    scene.voxel_volumes.insert(node.id, (**volume).clone());
                }
                if let Some(mesh) = lowpoly_mesh {
                    scene.lowpoly_meshes.insert(node.id, (**mesh).clone());
                }
                scene.nodes.insert(node.id, (**node).clone());
                scene.selected_node = *previous_selection;
            }
            Self::RestoreSelection(prev) => {
                scene.selected_node = *prev;
            }
            Self::RestoreNodeTransform { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.transform = *previous;
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
                canvas.rollback_layer_id(*layer_id);
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
                if let Some(popped) = canvas.frames.pop() {
                    if let Some(first) = popped.layers.first() {
                        canvas.rollback_layer_id(first.id);
                    }
                }
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
            Self::RestoreMeshTexturePixels {
                node_id,
                was_none,
                previous_pixels,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                if *was_none {
                    mesh.texture = None;
                } else if let Some(tex) = &mut mesh.texture {
                    tex.restore_pixels(previous_pixels);
                }
            }
            Self::RestoreMeshTextureFull { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.texture = previous.clone();
            }
            Self::RestoreMeshNormalMap { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.normal_map = previous.clone();
            }
            Self::RestoreMeshSmoothShading { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.smooth_shading = *previous;
            }
            Self::RestoreMeshMaterial { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.material = *previous;
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
            Self::RestoreNodeBoneBinding { node_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.bindings.retain(|b| b.node_id != *node_id);
                if let Some(bid) = previous {
                    rig.bindings.push(SkinBinding {
                        node_id: *node_id,
                        bone_id: *bid,
                    });
                }
            }
            Self::PopAnimationClip {
                previous_active,
                previous_time,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.animations.pop();
                rig.active_animation =
                    (*previous_active).min(rig.animations.len().saturating_sub(1));
                rig.playback_time = *previous_time;
                rig.apply_active_animation();
            }
            Self::RestoreAnimationClip {
                clip_index,
                previous,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                *clip = previous.clone();
                if rig.active_animation == *clip_index {
                    rig.apply_active_animation();
                }
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
