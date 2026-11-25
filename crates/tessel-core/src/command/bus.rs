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
    SetNodeParent {
        node_id: NodeId,
        parent: Option<NodeId>,
    },
    SetLightDirection {
        azimuth: f32,
        elevation: f32,
    },
    SetShadingMode(crate::scene::ViewportShadingMode),
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
    SetPixelFps {
        node_id: NodeId,
        fps: u16,
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
    CreateTerrainMesh {
        name: String,
        params: crate::mesh::TerrainParams,
        transform: Transform,
    },
    CreateCurveTubeMesh {
        name: String,
        points: Vec<glam::Vec3>,
        radius: f32,
        flatness: f32,
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
    TranslateMeshFace {
        node_id: NodeId,
        face_id: FaceId,
        delta: glam::Vec3,
    },
    MoveMeshVertices {
        node_id: NodeId,
        vertex_ids: Vec<usize>,
        delta: glam::Vec3,
    },
    SetMeshVertex {
        node_id: NodeId,
        vertex_id: usize,
        position: glam::Vec3,
    },
    SetMeshVerticesBatch {
        node_id: NodeId,
        updates: Vec<(usize, glam::Vec3)>,
    },
    SetMeshFaceMaterials {
        node_id: NodeId,
        face_ids: Vec<FaceId>,
        material: PaletteIndex,
    },
    MirrorLowPolyMesh {
        node_id: NodeId,
        axis: u8,
        merge_original: bool,
    },
    TaperLowPolyMesh {
        node_id: NodeId,
        axis: u8,
        start_scale: f32,
        end_scale: f32,
    },
    BendLowPolyMesh {
        node_id: NodeId,
        bend_axis: u8,
        curve_axis: u8,
        angle_rad: f32,
    },
    SetMeshOrigin {
        node_id: NodeId,
        origin_local: glam::Vec3,
    },
    ApplyMeshTransform {
        node_id: NodeId,
    },
    WeldMeshVertices {
        node_id: NodeId,
        tolerance: f32,
    },
    FlipMeshNormals {
        node_id: NodeId,
        face_ids: Option<Vec<FaceId>>,
    },
    JoinLowPolyMeshes {
        target_node_id: NodeId,
        source_node_ids: Vec<NodeId>,
    },
    SetMeshFaceUvs {
        node_id: NodeId,
        face_id: FaceId,
        uvs: Vec<[f32; 2]>,
    },
    SetMeshVertexWeights {
        node_id: NodeId,
        vertex_ids: Vec<usize>,
        joints: [u16; 4],
        weights: [f32; 4],
    },
    ClearMeshVertexWeights {
        node_id: NodeId,
        vertex_ids: Option<Vec<usize>>,
    },
    AddMorphDelta {
        node_id: NodeId,
        name: String,
        vertex_ids: Vec<usize>,
        delta: glam::Vec3,
    },
    ClearMorphs {
        node_id: NodeId,
        name: Option<String>,
    },
    SmoothSubdivideMesh {
        node_id: NodeId,
    },
    BevelMesh {
        node_id: NodeId,
        amount: f32,
    },
    DecimateMesh {
        node_id: NodeId,
        ratio: f32,
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
    PaintMeshSpot3D {
        node_id: NodeId,
        center: glam::Vec3,
        radius: f32,
        color: Rgba8,
        brush_normal: Option<glam::Vec3>,
        camera_pos: Option<glam::Vec3>,
    },
    PaintMeshLine {
        node_id: NodeId,
        uv_start: [f32; 2],
        uv_end: [f32; 2],
        width_px: f32,
        color: Rgba8,
    },
    PaintMeshRect {
        node_id: NodeId,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        color: Rgba8,
    },
    ProjectMeshUvs {
        node_id: NodeId,
        mode: UvProjection,
    },
    SubdivideMeshFace {
        node_id: NodeId,
        face_id: FaceId,
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
    SetMeshAoMap {
        node_id: NodeId,
        ao_map: Option<MeshTexture>,
    },
    AddMeshPaintLayer {
        node_id: NodeId,
        name: String,
        opacity: f32,
        fill_color: Option<Rgba8>,
    },
    SetMeshPaintLayer {
        node_id: NodeId,
        layer_id: u32,
        name: Option<String>,
        visible: Option<bool>,
        opacity: Option<f32>,
        set_active: bool,
    },
    DeleteMeshPaintLayer {
        node_id: NodeId,
        layer_id: u32,
    },
    SetMeshVertexColors {
        node_id: NodeId,
        updates: Vec<(usize, [f32; 4])>,
    },
    ClearMeshVertexColors {
        node_id: NodeId,
    },
    SetMeshCreaseEdges {
        node_id: NodeId,
        edges: Vec<(usize, usize)>,
    },
    SetMeshProxyNormals {
        node_id: NodeId,
        normals: Vec<(usize, glam::Vec3)>,
        clear: bool,
    },
    SetPixelLayerOpacity {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        opacity: f32,
    },
    SetPixelOnionSkin {
        node_id: NodeId,
        enabled: bool,
    },
    UpdateAnimationClipMeta {
        clip_index: usize,
        name: Option<String>,
        duration_secs: Option<f32>,
    },
    ApplyBipedRig,
    ApplyHumanoidRig,
    AddBone {
        name: String,
        parent: Option<BoneId>,
        position: glam::Vec3,
    },
    SetBoneRestPosition {
        bone_id: BoneId,
        position: glam::Vec3,
    },
    SetBoneRestRotation {
        bone_id: BoneId,
        rotation: Quat,
    },
    SetBoneRotation {
        bone_id: BoneId,
        rotation: Quat,
    },
    SetBoneTranslation {
        bone_id: BoneId,
        translation: glam::Vec3,
    },
    SolveTwoBoneIk {
        root_bone: BoneId,
        mid_bone: BoneId,
        end_bone: BoneId,
        target: glam::Vec3,
        pole: glam::Vec3,
    },
    BindNodeToBone {
        node_id: NodeId,
        bone_id: Option<BoneId>,
    },
    SetNodeBoneMask {
        node_id: NodeId,
        bone_ids: Option<Vec<BoneId>>,
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
    SetMorphWeightKeyframe {
        clip_index: usize,
        node_id: NodeId,
        target: String,
        time_secs: f32,
        weight: f32,
    },
    RemoveMorphWeightKeyframe {
        clip_index: usize,
        node_id: NodeId,
        target: String,
        time_secs: f32,
    },
    SetVisibilityKeyframe {
        clip_index: usize,
        node_id: NodeId,
        time_secs: f32,
        visible: bool,
    },
    SetParentSwitchKeyframe {
        clip_index: usize,
        node_id: NodeId,
        key: crate::ParentSwitchKey,
    },
    SetClipLayers {
        layers: Vec<ClipLayer>,
    },
    UpsertAnimationClip {
        clip: AnimationClip,
        activate: bool,
    },
    SetActiveAnimationClip {
        clip_index: usize,
    },
    RemoveAnimationClip {
        clip_index: usize,
    },
    RemoveBone {
        bone_id: BoneId,
    },
    CreateGroupNode {
        name: String,
        transform: Transform,
        parent: Option<NodeId>,
    },
    SetNodeTransformKeyframe {
        clip_index: usize,
        node_id: NodeId,
        keyframe: Keyframe,
    },
    DuplicateNode {
        node_id: NodeId,
        name: Option<String>,
    },
    SetNodeVisibility {
        node_id: NodeId,
        visible: bool,
    },
    RenameNode {
        node_id: NodeId,
        name: String,
    },
    ClearScene,
    SetAnimationPlaybackTime {
        clip_index: Option<usize>,
        time_secs: f32,
        playing: Option<bool>,
    },
    ResetCharacterPose,
    ClearMeshTexture {
        node_id: NodeId,
        clear_color: bool,
        clear_normal: bool,
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
    RestoreNodeParent {
        node_id: NodeId,
        previous: Option<NodeId>,
    },
    RestoreLightDirection {
        azimuth: f32,
        elevation: f32,
    },
    RestoreShadingMode(crate::scene::ViewportShadingMode),
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
    RestorePixelFps {
        node_id: NodeId,
        previous: u16,
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
    RestoreMeshAoMap {
        node_id: NodeId,
        previous: Option<MeshTexture>,
    },
    RestoreMeshPaintLayers {
        node_id: NodeId,
        previous_layers: Vec<MeshPaintLayer>,
        previous_active: Option<u32>,
        previous_texture: Option<MeshTexture>,
    },
    RestoreMeshPaintLayerPixels {
        node_id: NodeId,
        layer_id: u32,
        previous_pixels: Vec<(usize, [u8; 4])>,
    },
    RestoreMeshVertexColors {
        node_id: NodeId,
        previous: std::collections::BTreeMap<usize, [f32; 4]>,
    },
    RestoreMeshCreaseEdges {
        node_id: NodeId,
        previous: Vec<(usize, usize)>,
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
    RestoreBoneTranslation {
        bone_id: BoneId,
        previous: glam::Vec3,
    },
    RestoreBoneRestPosition {
        bone_id: BoneId,
        previous_rest: glam::Vec3,
        previous_pose: glam::Vec3,
    },
    RestoreBoneRestRotation {
        bone_id: BoneId,
        previous_rest: Quat,
        previous_pose: Quat,
    },
    RestoreClipLayers(Vec<ClipLayer>),
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
    RestoreNodeVisibility {
        node_id: NodeId,
        previous: bool,
    },
    RestoreNodeName {
        node_id: NodeId,
        previous: String,
    },
    RestorePixelLayerOpacity {
        node_id: NodeId,
        frame: FrameIndex,
        layer_id: LayerId,
        previous: f32,
    },
    RestorePixelOnionSkin {
        node_id: NodeId,
        previous: bool,
    },
    RestoreMeshProxyNormals {
        node_id: NodeId,
        previous: std::collections::BTreeMap<usize, glam::Vec3>,
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
                        parent: None,
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
                        parent: None,
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
            Self::SetNodeParent { node_id, parent } => {
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                if let Some(pid) = parent {
                    if *pid == *node_id || !scene.nodes.contains_key(pid) {
                        return Err(CoreError::OutOfBounds);
                    }
                    // Prevent cycle: walk ancestors of pid
                    let mut curr = Some(*pid);
                    let mut hops = 0usize;
                    while let Some(anc) = curr {
                        if anc == *node_id || hops >= 16 {
                            return Err(CoreError::OutOfBounds);
                        }
                        curr = scene.nodes.get(&anc).and_then(|n| n.parent);
                        hops += 1;
                    }
                }
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous = node.parent;
                node.parent = *parent;
                UndoDelta::RestoreNodeParent {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetLightDirection { azimuth, elevation } => {
                if !azimuth.is_finite() || !elevation.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                let prev_az = scene.light_azimuth;
                let prev_el = scene.light_elevation;
                scene.light_azimuth = *azimuth;
                scene.light_elevation = elevation.clamp(0.05, 1.52);
                UndoDelta::RestoreLightDirection {
                    azimuth: prev_az,
                    elevation: prev_el,
                }
            }
            Self::SetShadingMode(mode) => {
                let prev = scene.shading_mode;
                scene.shading_mode = *mode;
                UndoDelta::RestoreShadingMode(prev)
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
                if canvas.frames.len() >= 128 {
                    return Err(CoreError::OutOfBounds);
                }
                let (w, h) = (canvas.width, canvas.height);
                let new_id = canvas.allocate_layer_id();
                let frame_ref = canvas
                    .frame_mut(*frame)
                    .ok_or(CoreError::PixelFrameNotFound(*frame))?;
                if frame_ref.layers.len() >= 32 {
                    return Err(CoreError::OutOfBounds);
                }
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
            Self::SetPixelFps { node_id, fps } => {
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
                        parent: None,
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
                        parent: None,
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
                        parent: None,
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
            Self::CreateTerrainMesh {
                name,
                params,
                transform,
            } => {
                let mesh = mesh_ops::create_terrain_mesh(params)?;
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::LowPolyMesh,
                        visible: true,
                        parent: None,
                        transform: *transform,
                    },
                );
                scene.lowpoly_meshes.insert(id, mesh);
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::CreateCurveTubeMesh {
                name,
                points,
                radius,
                flatness,
                material,
            } => {
                let mesh = mesh_ops::create_curve_tube(points, *radius, *flatness, *material)?;
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::LowPolyMesh,
                        visible: true,
                        parent: None,
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
                        parent: None,
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
                        parent: None,
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
            Self::TranslateMeshFace {
                node_id,
                face_id,
                delta,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::translate_face(&current, *face_id, *delta)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::MoveMeshVertices {
                node_id,
                vertex_ids,
                delta,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::move_mesh_vertices(&current, vertex_ids, *delta)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshVertex {
                node_id,
                vertex_id,
                position,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_mesh_vertex(&current, *vertex_id, *position)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshVerticesBatch { node_id, updates } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_vertices_batch(&current, updates)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshFaceMaterials {
                node_id,
                face_ids,
                material,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_face_materials(&current, face_ids, *material)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::MirrorLowPolyMesh {
                node_id,
                axis,
                merge_original,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::mirror_mesh(&current, *axis, *merge_original)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::TaperLowPolyMesh {
                node_id,
                axis,
                start_scale,
                end_scale,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::taper_mesh(&current, *axis, *start_scale, *end_scale)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::BendLowPolyMesh {
                node_id,
                bend_axis,
                curve_axis,
                angle_rad,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::bend_mesh(&current, *bend_axis, *curve_axis, *angle_rad)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshOrigin {
                node_id,
                origin_local,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_mesh_origin(&current, *origin_local)?;
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous_transform = node.transform;
                let world_shift = node.transform.rotation * (node.transform.scale * *origin_local);
                node.transform.translation += world_shift;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::Transaction(vec![
                    UndoDelta::RestoreLowPolyMesh {
                        node_id: *node_id,
                        previous_mesh: current,
                    },
                    UndoDelta::RestoreNodeTransform {
                        node_id: *node_id,
                        previous: previous_transform,
                    },
                ])
            }
            Self::ApplyMeshTransform { node_id } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous_transform = node.transform;
                let baked = mesh_ops::apply_transform(&current, &previous_transform)?;
                node.transform = Transform::default();
                scene.lowpoly_meshes.insert(*node_id, baked);
                UndoDelta::Transaction(vec![
                    UndoDelta::RestoreLowPolyMesh {
                        node_id: *node_id,
                        previous_mesh: current,
                    },
                    UndoDelta::RestoreNodeTransform {
                        node_id: *node_id,
                        previous: previous_transform,
                    },
                ])
            }
            Self::WeldMeshVertices { node_id, tolerance } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let (next, _) = mesh_ops::weld_vertices(&current, *tolerance)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::FlipMeshNormals { node_id, face_ids } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::flip_normals(&current, face_ids.as_deref())?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::JoinLowPolyMeshes {
                target_node_id,
                source_node_ids,
            } => {
                if source_node_ids.is_empty() {
                    return Err(CoreError::OutOfBounds);
                }
                let target_mesh = scene
                    .lowpoly_meshes
                    .get(target_node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*target_node_id))?
                    .clone();
                let target_world = scene.node_world_matrix(*target_node_id);
                let target_inv = target_world.inverse();
                let mut owned_sources: Vec<(NodeId, LowPolyMesh, glam::Mat4)> =
                    Vec::with_capacity(source_node_ids.len());
                for &src_id in source_node_ids {
                    if src_id == *target_node_id {
                        continue;
                    }
                    let src_mesh = scene
                        .lowpoly_meshes
                        .get(&src_id)
                        .ok_or(CoreError::LowPolyMeshNotFound(src_id))?
                        .clone();
                    let src_world = scene.node_world_matrix(src_id);
                    owned_sources.push((src_id, src_mesh, src_world));
                }
                if owned_sources.is_empty() {
                    return Err(CoreError::OutOfBounds);
                }
                let mut parts: Vec<(&LowPolyMesh, glam::Mat4)> =
                    Vec::with_capacity(1 + owned_sources.len());
                parts.push((&target_mesh, target_world));
                for (_, m, w) in &owned_sources {
                    parts.push((m, *w));
                }
                let merged = mesh_ops::join_meshes(&parts, target_inv)?;
                scene.lowpoly_meshes.insert(*target_node_id, merged);
                let mut deltas = Vec::with_capacity(1 + owned_sources.len());
                deltas.push(UndoDelta::RestoreLowPolyMesh {
                    node_id: *target_node_id,
                    previous_mesh: target_mesh,
                });
                for (src_id, _, _) in owned_sources {
                    let del = Self::DeleteNode(src_id).apply(scene)?;
                    deltas.push(del);
                }
                scene.selected_node = Some(*target_node_id);
                UndoDelta::Transaction(deltas)
            }
            Self::SetMeshFaceUvs {
                node_id,
                face_id,
                uvs,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_face_uvs(&current, *face_id, uvs)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SetMeshVertexWeights {
                node_id,
                vertex_ids,
                joints,
                weights,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::set_vertex_weights(&current, vertex_ids, *joints, *weights)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::ClearMeshVertexWeights {
                node_id,
                vertex_ids,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::clear_vertex_weights(&current, vertex_ids.as_deref())?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::AddMorphDelta {
                node_id,
                name,
                vertex_ids,
                delta,
            } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::add_morph_delta(&current, name, vertex_ids, *delta)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::ClearMorphs { node_id, name } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::clear_morphs(&current, name.as_deref())?;
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
            Self::BevelMesh { node_id, amount } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::bevel_mesh(&current, *amount)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::DecimateMesh { node_id, ratio } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::decimate_mesh(&current, *ratio)?;
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
                    double_sided: material.double_sided,
                    cast_shadow: material.cast_shadow,
                    receive_shadow: material.receive_shadow,
                    alpha_mode: match material.alpha_mode {
                        crate::mesh::AlphaMode::Mask { cutoff } if cutoff.is_finite() => {
                            crate::mesh::AlphaMode::Mask {
                                cutoff: cutoff.clamp(0.0, 1.0),
                            }
                        }
                        crate::mesh::AlphaMode::Mask { .. } => {
                            crate::mesh::AlphaMode::Mask { cutoff: 0.5 }
                        }
                        other => other,
                    },
                    unlit: material.unlit,
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
                if let Some(layer_idx) = mesh.active_paint_layer_index() {
                    let layer_id = mesh.paint_layers[layer_idx].id;
                    let previous_pixels = mesh.paint_layers[layer_idx]
                        .texture
                        .paint_disc(*uv, *radius_px, *color);
                    mesh.recomposite_paint_layers();
                    UndoDelta::RestoreMeshPaintLayerPixels {
                        node_id: *node_id,
                        layer_id,
                        previous_pixels,
                    }
                } else {
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
            }
            Self::PaintMeshSpot3D {
                node_id,
                center,
                radius,
                color,
                brush_normal,
                camera_pos,
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
                let params = crate::mesh::PaintSpot3DParams {
                    center: *center,
                    radius: *radius,
                    color: *color,
                    brush_normal: *brush_normal,
                    camera_pos: *camera_pos,
                };
                if let Some(layer_idx) = mesh.active_paint_layer_index() {
                    let layer_id = mesh.paint_layers[layer_idx].id;
                    let mut tex = std::mem::replace(
                        &mut mesh.paint_layers[layer_idx].texture,
                        MeshTexture {
                            width: 0,
                            height: 0,
                            rgba: Vec::new(),
                        },
                    );
                    let previous_pixels = crate::mesh::paint_mesh_spot_3d(mesh, &mut tex, &params);
                    mesh.paint_layers[layer_idx].texture = tex;
                    mesh.recomposite_paint_layers();
                    UndoDelta::RestoreMeshPaintLayerPixels {
                        node_id: *node_id,
                        layer_id,
                        previous_pixels,
                    }
                } else {
                    let was_none = mesh.texture.is_none();
                    let mut tex = mesh.texture.take().unwrap_or_else(|| {
                        MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                    });
                    let previous_pixels = crate::mesh::paint_mesh_spot_3d(mesh, &mut tex, &params);
                    mesh.texture = Some(tex);
                    UndoDelta::RestoreMeshTexturePixels {
                        node_id: *node_id,
                        was_none,
                        previous_pixels,
                    }
                }
            }
            Self::PaintMeshLine {
                node_id,
                uv_start,
                uv_end,
                width_px,
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
                if let Some(layer_idx) = mesh.active_paint_layer_index() {
                    let layer_id = mesh.paint_layers[layer_idx].id;
                    let previous_pixels = mesh.paint_layers[layer_idx]
                        .texture
                        .paint_line(*uv_start, *uv_end, *width_px, *color);
                    mesh.recomposite_paint_layers();
                    UndoDelta::RestoreMeshPaintLayerPixels {
                        node_id: *node_id,
                        layer_id,
                        previous_pixels,
                    }
                } else {
                    let was_none = mesh.texture.is_none();
                    let tex = mesh.texture.get_or_insert_with(|| {
                        MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                    });
                    let previous_pixels = tex.paint_line(*uv_start, *uv_end, *width_px, *color);
                    UndoDelta::RestoreMeshTexturePixels {
                        node_id: *node_id,
                        was_none,
                        previous_pixels,
                    }
                }
            }
            Self::PaintMeshRect {
                node_id,
                uv_min,
                uv_max,
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
                if let Some(layer_idx) = mesh.active_paint_layer_index() {
                    let layer_id = mesh.paint_layers[layer_idx].id;
                    let previous_pixels = mesh.paint_layers[layer_idx]
                        .texture
                        .paint_rect(*uv_min, *uv_max, *color);
                    mesh.recomposite_paint_layers();
                    UndoDelta::RestoreMeshPaintLayerPixels {
                        node_id: *node_id,
                        layer_id,
                        previous_pixels,
                    }
                } else {
                    let was_none = mesh.texture.is_none();
                    let tex = mesh.texture.get_or_insert_with(|| {
                        MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                    });
                    let previous_pixels = tex.paint_rect(*uv_min, *uv_max, *color);
                    UndoDelta::RestoreMeshTexturePixels {
                        node_id: *node_id,
                        was_none,
                        previous_pixels,
                    }
                }
            }
            Self::ProjectMeshUvs { node_id, mode } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::project_uvs(&current, *mode)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::SubdivideMeshFace { node_id, face_id } => {
                let current = scene
                    .lowpoly_meshes
                    .get(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                    .clone();
                let next = mesh_ops::subdivide_face(&current, *face_id)?;
                scene.lowpoly_meshes.insert(*node_id, next);
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                }
            }
            Self::FillMeshTexture { node_id, color } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                if let Some(layer_idx) = mesh.active_paint_layer_index() {
                    let previous_layers = mesh.paint_layers.clone();
                    let previous_active = mesh.active_paint_layer;
                    let previous_texture = mesh.texture.clone();
                    mesh.paint_layers[layer_idx].texture.fill(*color);
                    mesh.recomposite_paint_layers();
                    UndoDelta::RestoreMeshPaintLayers {
                        node_id: *node_id,
                        previous_layers,
                        previous_active,
                        previous_texture,
                    }
                } else {
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
                if !mesh.paint_layers.is_empty() {
                    let previous_layers = std::mem::take(&mut mesh.paint_layers);
                    let previous_active = mesh.active_paint_layer.take();
                    let previous_texture = mesh.texture.clone();
                    mesh.texture = texture.clone();
                    UndoDelta::RestoreMeshPaintLayers {
                        node_id: *node_id,
                        previous_layers,
                        previous_active,
                        previous_texture,
                    }
                } else {
                    let previous = mesh.texture.clone();
                    mesh.texture = texture.clone();
                    UndoDelta::RestoreMeshTextureFull {
                        node_id: *node_id,
                        previous,
                    }
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
            Self::SetMeshAoMap { node_id, ao_map } => {
                if let Some(tex) = ao_map {
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
                let previous = mesh.ao_map.clone();
                mesh.ao_map = ao_map.clone();
                UndoDelta::RestoreMeshAoMap {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::AddMeshPaintLayer {
                node_id,
                name,
                opacity,
                fill_color,
            } => {
                if !opacity.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
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
                let added_slots = if mesh.paint_layers.is_empty() { 2 } else { 1 };
                if mesh.paint_layers.len().saturating_add(added_slots)
                    > crate::mesh::layer::MAX_PAINT_LAYERS
                {
                    return Err(CoreError::OutOfBounds);
                }
                let previous_layers = mesh.paint_layers.clone();
                let previous_active = mesh.active_paint_layer;
                let previous_texture = mesh.texture.clone();

                let (w, h) = mesh
                    .paint_layers
                    .first()
                    .map(|l| (l.texture.width, l.texture.height))
                    .or_else(|| mesh.texture.as_ref().map(|t| (t.width, t.height)))
                    .unwrap_or((DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE));

                if mesh.paint_layers.is_empty() {
                    let base_tex = mesh
                        .texture
                        .clone()
                        .unwrap_or_else(|| MeshTexture::solid(w, h, base_color));
                    mesh.paint_layers
                        .push(MeshPaintLayer::new(1, "Base", 1.0, base_tex));
                }

                let new_id = mesh.next_paint_layer_id();
                let fill = fill_color.unwrap_or(Rgba8::rgba(0, 0, 0, 0));
                let layer_tex = MeshTexture::solid(w, h, fill);
                let label = if name.trim().is_empty() {
                    format!("Layer {new_id}")
                } else {
                    name.trim().to_string()
                };
                mesh.paint_layers
                    .push(MeshPaintLayer::new(new_id, label, *opacity, layer_tex));
                mesh.active_paint_layer = Some(new_id);
                mesh.recomposite_paint_layers();

                UndoDelta::RestoreMeshPaintLayers {
                    node_id: *node_id,
                    previous_layers,
                    previous_active,
                    previous_texture,
                }
            }
            Self::SetMeshPaintLayer {
                node_id,
                layer_id,
                name,
                visible,
                opacity,
                set_active,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let Some(pos) = mesh.paint_layers.iter().position(|l| l.id == *layer_id) else {
                    return Err(CoreError::OutOfBounds);
                };
                if let Some(op) = opacity {
                    if !op.is_finite() {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let previous_layers = mesh.paint_layers.clone();
                let previous_active = mesh.active_paint_layer;
                let previous_texture = mesh.texture.clone();

                if let Some(n) = name {
                    if !n.trim().is_empty() {
                        mesh.paint_layers[pos].name = n.trim().to_string();
                    }
                }
                if let Some(v) = visible {
                    mesh.paint_layers[pos].visible = *v;
                }
                if let Some(op) = opacity {
                    mesh.paint_layers[pos].opacity = op.clamp(0.0, 1.0);
                }
                if *set_active {
                    mesh.active_paint_layer = Some(*layer_id);
                }
                mesh.recomposite_paint_layers();

                UndoDelta::RestoreMeshPaintLayers {
                    node_id: *node_id,
                    previous_layers,
                    previous_active,
                    previous_texture,
                }
            }
            Self::DeleteMeshPaintLayer { node_id, layer_id } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let Some(pos) = mesh.paint_layers.iter().position(|l| l.id == *layer_id) else {
                    return Err(CoreError::OutOfBounds);
                };
                let previous_layers = mesh.paint_layers.clone();
                let previous_active = mesh.active_paint_layer;
                let previous_texture = mesh.texture.clone();

                mesh.paint_layers.remove(pos);
                if mesh.active_paint_layer == Some(*layer_id) {
                    mesh.active_paint_layer = mesh.paint_layers.last().map(|l| l.id);
                }
                if mesh.paint_layers.is_empty() {
                    mesh.texture = None;
                } else {
                    mesh.recomposite_paint_layers();
                }

                UndoDelta::RestoreMeshPaintLayers {
                    node_id: *node_id,
                    previous_layers,
                    previous_active,
                    previous_texture,
                }
            }
            Self::SetMeshVertexColors { node_id, updates } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                for &(vid, rgba) in updates {
                    if vid >= mesh.vertices.len() || rgba.iter().any(|c| !c.is_finite()) {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let previous = mesh.vertex_colors.clone();
                for &(vid, rgba) in updates {
                    mesh.vertex_colors.insert(
                        vid,
                        [
                            rgba[0].clamp(0.0, 1.0),
                            rgba[1].clamp(0.0, 1.0),
                            rgba[2].clamp(0.0, 1.0),
                            rgba[3].clamp(0.0, 1.0),
                        ],
                    );
                }
                UndoDelta::RestoreMeshVertexColors {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::ClearMeshVertexColors { node_id } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let previous = std::mem::take(&mut mesh.vertex_colors);
                UndoDelta::RestoreMeshVertexColors {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetMeshProxyNormals {
                node_id,
                normals,
                clear,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                for &(vid, n) in normals {
                    if vid >= mesh.vertices.len() || !n.is_finite() || n.length_squared() < 1e-8 {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let previous = mesh.proxy_normals.clone();
                if *clear {
                    mesh.proxy_normals.clear();
                }
                for &(vid, n) in normals {
                    mesh.proxy_normals.insert(vid, n.normalize());
                }
                UndoDelta::RestoreMeshProxyNormals {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::SetPixelLayerOpacity {
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
            Self::SetPixelOnionSkin { node_id, enabled } => {
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
            Self::UpdateAnimationClipMeta {
                clip_index,
                name,
                duration_secs,
            } => {
                if let Some(d) = duration_secs {
                    if !d.is_finite() || *d < 0.05 || *d > 600.0 {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                if let Some(n) = name {
                    let trimmed = n.trim();
                    if !trimmed.is_empty() {
                        clip.name = trimmed.to_string();
                    }
                }
                if let Some(d) = duration_secs {
                    clip.duration_secs = *d;
                }
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::SetMeshCreaseEdges { node_id, edges } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                for &(a, b) in edges {
                    if a >= mesh.vertices.len() || b >= mesh.vertices.len() || a == b {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let previous = mesh.crease_edges.clone();
                mesh.crease_edges = edges.clone();
                UndoDelta::RestoreMeshCreaseEdges {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::ApplyBipedRig => {
                let prev = scene.character_rig.replace(CharacterRig::biped_template());
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::ApplyHumanoidRig => {
                let prev = scene
                    .character_rig
                    .replace(CharacterRig::humanoid_template());
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::AddBone {
                name,
                parent,
                position,
            } => {
                if !position.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                let prev = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .get_or_insert_with(CharacterRig::default);
                if let Some(pid) = parent {
                    if rig.bone(*pid).is_none() {
                        scene.character_rig = prev;
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let next_id = rig
                    .bones
                    .iter()
                    .map(|b| b.id.0.saturating_add(1))
                    .max()
                    .unwrap_or(0);
                rig.bones.push(Bone::new(
                    BoneId(next_id),
                    name.clone(),
                    *parent,
                    Transform {
                        translation: *position,
                        rotation: Quat::IDENTITY,
                        scale: glam::Vec3::ONE,
                    },
                ));
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
            Self::SetBoneTranslation {
                bone_id,
                translation,
            } => {
                if !translation.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                let previous = bone.pose_local.translation;
                bone.pose_local.translation = *translation;
                UndoDelta::RestoreBoneTranslation {
                    bone_id: *bone_id,
                    previous,
                }
            }
            Self::SetBoneRestPosition { bone_id, position } => {
                if !position.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                let previous_rest = bone.rest_local.translation;
                let previous_pose = bone.pose_local.translation;
                bone.rest_local.translation = *position;
                bone.pose_local.translation = *position;
                UndoDelta::RestoreBoneRestPosition {
                    bone_id: *bone_id,
                    previous_rest,
                    previous_pose,
                }
            }
            Self::SetBoneRestRotation { bone_id, rotation } => {
                if !rotation.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                let previous_rest = bone.rest_local.rotation;
                let previous_pose = bone.pose_local.rotation;
                bone.rest_local.rotation = *rotation;
                bone.pose_local.rotation = *rotation;
                UndoDelta::RestoreBoneRestRotation {
                    bone_id: *bone_id,
                    previous_rest,
                    previous_pose,
                }
            }
            Self::SolveTwoBoneIk {
                root_bone,
                mid_bone,
                end_bone,
                target,
                pole,
            } => {
                let rig = scene
                    .character_rig
                    .as_ref()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let (root_local, mid_local) = crate::character::ik::solve_for_rig(
                    rig,
                    *root_bone,
                    *mid_bone,
                    *end_bone,
                    *target,
                    Some(*pole),
                )?;
                let mut applied = Vec::with_capacity(2);
                for (id, rot) in [(*root_bone, root_local), (*mid_bone, mid_local)] {
                    let delta = Self::SetBoneRotation {
                        bone_id: id,
                        rotation: rot,
                    }
                    .apply(scene)?;
                    applied.push(delta);
                }
                UndoDelta::Transaction(applied)
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
            Self::SetNodeBoneMask { node_id, bone_ids } => {
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                let prev = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                match bone_ids {
                    Some(ids) if !ids.is_empty() => {
                        for bid in ids {
                            if rig.bone(*bid).is_none() {
                                scene.character_rig = prev;
                                return Err(CoreError::OutOfBounds);
                            }
                        }
                        rig.node_bone_masks.insert(*node_id, ids.clone());
                    }
                    _ => {
                        rig.node_bone_masks.remove(node_id);
                    }
                }
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::AddAnimationClip {
                name,
                duration_secs,
            } => {
                let prev_rig = scene.character_rig.clone();
                let had_rig = scene.character_rig.is_some();
                let rig = scene
                    .character_rig
                    .get_or_insert_with(CharacterRig::default);
                let previous_active = rig.active_animation;
                let previous_time = rig.playback_time;
                rig.animations
                    .push(AnimationClip::new_empty(name.clone(), *duration_secs));
                rig.active_animation = rig.animations.len().saturating_sub(1);
                rig.playback_time = 0.0;
                rig.playing = false;
                if had_rig {
                    UndoDelta::PopAnimationClip {
                        previous_active,
                        previous_time,
                    }
                } else {
                    UndoDelta::RestoreCharacterRig(prev_rig)
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
                if !keyframe.translation.is_finite()
                    || !keyframe.scale.is_finite()
                    || !keyframe.rotation.is_finite()
                {
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
            Self::SetMorphWeightKeyframe {
                clip_index,
                node_id,
                target,
                time_secs,
                weight,
            } => {
                if !time_secs.is_finite() || *time_secs < 0.0 || !weight.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
                if target.is_empty() {
                    return Err(CoreError::OutOfBounds);
                }
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                clip.upsert_morph_weight(*node_id, target, *time_secs, *weight);
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::SetVisibilityKeyframe {
                clip_index,
                node_id,
                time_secs,
                visible,
            } => {
                if !time_secs.is_finite() || *time_secs < 0.0 {
                    return Err(CoreError::OutOfBounds);
                }
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                clip.upsert_visibility_key(*node_id, *time_secs, *visible);
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::SetParentSwitchKeyframe {
                clip_index,
                node_id,
                key,
            } => {
                if !key.time_secs.is_finite() || key.time_secs < 0.0 {
                    return Err(CoreError::OutOfBounds);
                }
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if let Some(bid) = key.bone_id {
                    if rig.bone(bid).is_none() {
                        return Err(CoreError::OutOfBounds);
                    }
                }
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                let previous = clip.clone();
                clip.upsert_parent_switch_key(*node_id, *key);
                rig.apply_active_animation();
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::SetClipLayers { layers } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let previous = std::mem::replace(&mut rig.clip_layers, layers.clone());
                rig.apply_active_animation();
                UndoDelta::RestoreClipLayers(previous)
            }
            Self::UpsertAnimationClip { clip, activate } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if let Some(idx) = rig.animations.iter().position(|c| c.name == clip.name) {
                    let previous = std::mem::replace(&mut rig.animations[idx], clip.clone());
                    if *activate {
                        rig.active_animation = idx;
                    }
                    rig.apply_active_animation();
                    UndoDelta::RestoreAnimationClip {
                        clip_index: idx,
                        previous,
                    }
                } else {
                    let previous_active = rig.active_animation;
                    let previous_time = rig.playback_time;
                    rig.animations.push(clip.clone());
                    if *activate {
                        rig.active_animation = rig.animations.len().saturating_sub(1);
                    }
                    rig.apply_active_animation();
                    UndoDelta::PopAnimationClip {
                        previous_active,
                        previous_time,
                    }
                }
            }
            Self::RemoveMorphWeightKeyframe {
                clip_index,
                node_id,
                target,
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
                clip.remove_morph_weight(*node_id, target, *time_secs);
                UndoDelta::RestoreAnimationClip {
                    clip_index: *clip_index,
                    previous,
                }
            }
            Self::SetActiveAnimationClip { clip_index } => {
                let previous = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if *clip_index >= rig.animations.len() {
                    return Err(CoreError::OutOfBounds);
                }
                rig.active_animation = *clip_index;
                rig.playback_time = 0.0;
                rig.apply_active_animation();
                UndoDelta::RestoreCharacterRig(previous)
            }
            Self::RemoveAnimationClip { clip_index } => {
                let previous = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if *clip_index >= rig.animations.len() {
                    return Err(CoreError::OutOfBounds);
                }
                rig.animations.remove(*clip_index);
                rig.clip_layers.retain_mut(|layer| {
                    if layer.clip_index == *clip_index {
                        false
                    } else {
                        if layer.clip_index > *clip_index {
                            layer.clip_index -= 1;
                        }
                        true
                    }
                });
                if rig.animations.is_empty() {
                    rig.active_animation = 0;
                } else if *clip_index < rig.active_animation {
                    rig.active_animation -= 1;
                } else if rig.active_animation >= rig.animations.len() {
                    rig.active_animation = rig.animations.len() - 1;
                }
                UndoDelta::RestoreCharacterRig(previous)
            }
            Self::RemoveBone { bone_id } => {
                let previous = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if rig.bone(*bone_id).is_none() {
                    return Err(CoreError::OutOfBounds);
                }
                // Collect the whole subtree: any bone whose parent chain
                // reaches `bone_id` goes too.
                let mut removed = vec![*bone_id];
                loop {
                    let mut added = false;
                    for bone in rig.bones.iter() {
                        if !removed.contains(&bone.id)
                            && bone.parent.is_some_and(|p| removed.contains(&p))
                        {
                            removed.push(bone.id);
                            added = true;
                        }
                    }
                    if !added {
                        break;
                    }
                }
                rig.bones.retain(|b| !removed.contains(&b.id));
                rig.bindings.retain(|b| !removed.contains(&b.bone_id));
                for mask in rig.node_bone_masks.values_mut() {
                    mask.retain(|bid| !removed.contains(bid));
                }
                rig.node_bone_masks.retain(|_, mask| !mask.is_empty());
                for clip in rig.animations.iter_mut() {
                    clip.channels.retain(|c| !removed.contains(&c.bone_id));
                    for track in clip.parent_switch_tracks.iter_mut() {
                        for k in track.keys.iter_mut() {
                            if k.bone_id.is_some_and(|bid| removed.contains(&bid)) {
                                k.bone_id = None;
                            }
                        }
                    }
                }
                UndoDelta::RestoreCharacterRig(previous)
            }
            Self::CreateGroupNode {
                name,
                transform,
                parent,
            } => {
                if let Some(pid) = *parent {
                    if !scene.nodes.contains_key(&pid) {
                        return Err(CoreError::NodeNotFound(pid));
                    }
                }
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: name.clone(),
                        kind: NodeKind::Group,
                        visible: true,
                        parent: *parent,
                        transform: *transform,
                    },
                );
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::SetNodeTransformKeyframe {
                clip_index,
                node_id,
                keyframe,
            } => {
                if !scene.nodes.contains_key(node_id) {
                    return Err(CoreError::NodeNotFound(*node_id));
                }
                if !keyframe.time_secs.is_finite()
                    || keyframe.time_secs < 0.0
                    || !keyframe.translation.is_finite()
                    || !keyframe.rotation.is_finite()
                    || !keyframe.scale.is_finite()
                {
                    return Err(CoreError::OutOfBounds);
                }
                let prev = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .get_or_insert_with(CharacterRig::default);
                if rig.animations.is_empty() && *clip_index == 0 {
                    rig.animations.push(AnimationClip::new_empty(
                        "SceneAnim",
                        keyframe.time_secs.max(1.0),
                    ));
                }
                let clip = rig
                    .animations
                    .get_mut(*clip_index)
                    .ok_or(CoreError::OutOfBounds)?;
                clip.upsert_node_transform_key(*node_id, *keyframe);
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::DuplicateNode { node_id, name } => {
                let source = scene
                    .nodes
                    .get(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?
                    .clone();
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                let mut node = source.clone();
                node.id = id;
                node.name = name
                    .clone()
                    .unwrap_or_else(|| format!("{} Copy", source.name));
                node.transform.translation += glam::Vec3::new(0.5, 0.0, 0.0);
                scene.nodes.insert(id, node);
                if let Some(mesh) = scene.lowpoly_meshes.get(node_id).cloned() {
                    scene.lowpoly_meshes.insert(id, mesh);
                }
                if let Some(volume) = scene.voxel_volumes.get(node_id).cloned() {
                    scene.voxel_volumes.insert(id, volume);
                }
                if let Some(canvas) = scene.pixel_canvases.get(node_id).cloned() {
                    scene.pixel_canvases.insert(id, canvas);
                }
                if let Some(rig) = scene.character_rig.as_mut() {
                    let inherited: Vec<BoneId> = rig
                        .bindings
                        .iter()
                        .filter(|b| b.node_id == *node_id)
                        .map(|b| b.bone_id)
                        .collect();
                    for bone_id in inherited {
                        rig.bindings.push(SkinBinding {
                            node_id: id,
                            bone_id,
                        });
                    }
                }
                scene.selected_node = Some(id);
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
            Self::SetNodeVisibility { node_id, visible } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous = node.visible;
                node.visible = *visible;
                UndoDelta::RestoreNodeVisibility {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::RenameNode { node_id, name } => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    return Err(CoreError::OutOfBounds);
                }
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                let previous = std::mem::replace(&mut node.name, trimmed.to_string());
                UndoDelta::RestoreNodeName {
                    node_id: *node_id,
                    previous,
                }
            }
            Self::ClearScene => {
                let node_ids: Vec<NodeId> = scene.nodes.keys().copied().collect();
                let mut deltas = Vec::with_capacity(node_ids.len() + 1);
                for id in node_ids {
                    deltas.push(Self::DeleteNode(id).apply(scene)?);
                }
                let prev_rig = scene.character_rig.take();
                if prev_rig.is_some() {
                    deltas.push(UndoDelta::RestoreCharacterRig(prev_rig));
                }
                scene.selected_node = None;
                scene.selected_bone = None;
                scene.selected_face = None;
                UndoDelta::Transaction(deltas)
            }
            Self::SetAnimationPlaybackTime {
                clip_index,
                time_secs,
                playing,
            } => {
                if !time_secs.is_finite() || *time_secs < 0.0 {
                    return Err(CoreError::OutOfBounds);
                }
                let prev = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                if let Some(idx) = *clip_index {
                    if idx >= rig.animations.len() {
                        scene.character_rig = prev;
                        return Err(CoreError::OutOfBounds);
                    }
                    rig.active_animation = idx;
                }
                rig.playback_time = *time_secs;
                rig.playing = playing.unwrap_or(false);
                rig.apply_active_animation();
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::ResetCharacterPose => {
                let prev = scene.character_rig.clone();
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                for bone in &mut rig.bones {
                    bone.pose_local = bone.rest_local;
                }
                rig.morph_weights.clear();
                rig.node_transform_overrides.clear();
                rig.playback_time = 0.0;
                rig.playing = false;
                UndoDelta::RestoreCharacterRig(prev)
            }
            Self::ClearMeshTexture {
                node_id,
                clear_color,
                clear_normal,
            } => {
                if !clear_color && !clear_normal {
                    return Err(CoreError::OutOfBounds);
                }
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                let mut deltas = Vec::with_capacity(2);
                if *clear_color {
                    if !mesh.paint_layers.is_empty() {
                        let previous_layers = std::mem::take(&mut mesh.paint_layers);
                        let previous_active = mesh.active_paint_layer.take();
                        let previous_texture = mesh.texture.take();
                        deltas.push(UndoDelta::RestoreMeshPaintLayers {
                            node_id: *node_id,
                            previous_layers,
                            previous_active,
                            previous_texture,
                        });
                    } else {
                        let previous = mesh.texture.take();
                        deltas.push(UndoDelta::RestoreMeshTextureFull {
                            node_id: *node_id,
                            previous,
                        });
                    }
                }
                if *clear_normal {
                    let previous = mesh.normal_map.take();
                    deltas.push(UndoDelta::RestoreMeshNormalMap {
                        node_id: *node_id,
                        previous,
                    });
                }
                let mut iter = deltas.into_iter();
                match (iter.next(), iter.next()) {
                    (Some(one), None) => one,
                    (Some(a), Some(b)) => UndoDelta::Transaction(vec![a, b]),
                    _ => return Err(CoreError::OutOfBounds),
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
            Self::RestoreNodeParent { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.parent = *previous;
            }
            Self::RestoreLightDirection { azimuth, elevation } => {
                scene.light_azimuth = *azimuth;
                scene.light_elevation = *elevation;
            }
            Self::RestoreShadingMode(prev) => {
                scene.shading_mode = *prev;
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
            Self::RestorePixelFps { node_id, previous } => {
                let canvas = scene
                    .pixel_canvases
                    .get_mut(node_id)
                    .ok_or(CoreError::PixelCanvasNotFound(*node_id))?;
                canvas.fps = *previous;
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
            Self::RestoreMeshAoMap { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.ao_map = previous.clone();
            }
            Self::RestoreMeshPaintLayers {
                node_id,
                previous_layers,
                previous_active,
                previous_texture,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.paint_layers = previous_layers.clone();
                mesh.active_paint_layer = *previous_active;
                mesh.texture = previous_texture.clone();
            }
            Self::RestoreMeshPaintLayerPixels {
                node_id,
                layer_id,
                previous_pixels,
            } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                if let Some(layer) = mesh.paint_layers.iter_mut().find(|l| l.id == *layer_id) {
                    layer.texture.restore_pixels(previous_pixels);
                }
                mesh.recomposite_paint_layers();
            }
            Self::RestoreMeshVertexColors { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.vertex_colors = previous.clone();
            }
            Self::RestoreMeshCreaseEdges { node_id, previous } => {
                let mesh = scene
                    .lowpoly_meshes
                    .get_mut(node_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
                mesh.crease_edges = previous.clone();
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
            Self::RestoreBoneTranslation { bone_id, previous } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.pose_local.translation = *previous;
            }
            Self::RestoreBoneRestPosition {
                bone_id,
                previous_rest,
                previous_pose,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.rest_local.translation = *previous_rest;
                bone.pose_local.translation = *previous_pose;
            }
            Self::RestoreBoneRestRotation {
                bone_id,
                previous_rest,
                previous_pose,
            } => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                let bone = rig.bone_mut(*bone_id).ok_or(CoreError::OutOfBounds)?;
                bone.rest_local.rotation = *previous_rest;
                bone.pose_local.rotation = *previous_pose;
            }
            Self::RestoreClipLayers(previous) => {
                let rig = scene
                    .character_rig
                    .as_mut()
                    .ok_or(CoreError::CharacterRigNotFound)?;
                rig.clip_layers = previous.clone();
                rig.apply_active_animation();
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
            Self::RestorePixelLayerOpacity {
                node_id,
                frame,
                layer_id,
                previous,
            } => {
                if let Some(canvas) = scene.pixel_canvases.get_mut(node_id) {
                    if let Some(frame_ref) = canvas.frame_mut(*frame) {
                        if let Some(layer) = frame_ref.layer_mut(*layer_id) {
                            layer.opacity = *previous;
                        }
                    }
                }
            }
            Self::RestorePixelOnionSkin { node_id, previous } => {
                if let Some(canvas) = scene.pixel_canvases.get_mut(node_id) {
                    canvas.onion_skin = *previous;
                }
            }
            Self::RestoreMeshProxyNormals { node_id, previous } => {
                if let Some(mesh) = scene.lowpoly_meshes.get_mut(node_id) {
                    mesh.proxy_normals = previous.clone();
                }
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
            Self::RestoreNodeVisibility { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.visible = *previous;
            }
            Self::RestoreNodeName { node_id, previous } => {
                let node = scene
                    .nodes
                    .get_mut(node_id)
                    .ok_or(CoreError::NodeNotFound(*node_id))?;
                node.name = previous.clone();
            }
            Self::Transaction(deltas) => {
                for delta in deltas.iter().rev() {
                    delta.revert(scene)?;
                }
            }
        }
        scene.mark_modified();
        Ok(())
    }
}
