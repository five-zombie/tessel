#![allow(unused_imports)]
mod character_cmd;
mod mesh_cmd;
mod pixel_cmd;
mod revert;
mod scene_cmd;
#[cfg(test)]
mod tests;
mod voxel_cmd;

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
        mesh: Box<LowPolyMesh>,
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
    PaintMeshWeights {
        node_id: NodeId,
        hit_point: [f32; 3],
        radius: f32,
        bone_id: BoneId,
        strength: f32,
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
    SetMeshNprSlot {
        node_id: NodeId,
        slot: crate::mesh::NprTextureSlot,
        texture: Option<MeshTexture>,
    },
    SetMeshUvSeams {
        node_id: NodeId,
        edges: Vec<(usize, usize)>,
        replace: bool,
        unwrap: bool,
    },
    NormalizeMeshTexelDensity {
        node_id: NodeId,
        texture_size: u32,
        target_px_per_unit: Option<f32>,
        padding_uv: f32,
    },
    SculptMeshStroke {
        node_id: NodeId,
        params: crate::mesh::SculptStrokeParams,
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
    SetCharacterRig {
        rig: Option<CharacterRig>,
    },
    AddBone {
        name: String,
        parent: Option<BoneId>,
        position: glam::Vec3,
    },
    RenameBone {
        bone_id: BoneId,
        name: String,
    },
    ReparentBone {
        bone_id: BoneId,
        parent: Option<BoneId>,
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
    SetNodeRigSkinned {
        node_id: NodeId,
        skinned: bool,
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
    /// Batches multiple commands into one atomic undoable transaction.
    CreateBoneGrid {
        spec: crate::character::BoneGridSpec,
        bind_node: Option<NodeId>,
    },
    BakeClothGridAnimation {
        bone_prefix: String,
        params: crate::character::ClothWindParams,
    },
    SetMorphTargetDeltas {
        node_id: NodeId,
        name: String,
        position_deltas: Vec<(usize, glam::Vec3)>,
        normal_deltas: Vec<(usize, glam::Vec3)>,
        replace: bool,
    },
    SetMorphDefaultWeight {
        node_id: NodeId,
        name: String,
        weight: f32,
    },
    ApplyProxyCageDeform {
        node_id: NodeId,
        divisions: [usize; 3],
        control_deltas: Vec<(usize, glam::Vec3)>,
        floor_y: Option<f32>,
        bake_as_morph: Option<String>,
    },
    BakeProceduralRibbon {
        params: crate::mesh::RibbonParams,
    },
    ApplyStickGizmoPose {
        params: crate::character::StickGizmoParams,
    },
    SetBoneScale {
        bone_id: BoneId,
        scale: glam::Vec3,
    },
    SetBoneProperties {
        bone_id: BoneId,
        rotation_limits_deg: Option<Option<([f32; 3], [f32; 3])>>,
        inherit_rotation: Option<bool>,
        inherit_scale: Option<bool>,
        connected: Option<bool>,
        deform: Option<bool>,
        spring_stiffness: Option<f32>,
        spring_damping: Option<f32>,
        lod_level: Option<u8>,
        physics_collider: Option<Option<crate::character::BoneColliderMeta>>,
    },
    SetChainPole {
        end_bone_id: BoneId,
        pole: Option<glam::Vec3>,
    },
    UpsertRigConstraint {
        constraint: crate::character::BoneConstraint,
    },
    UpsertRigSocket {
        socket: crate::character::RigSocket,
    },
    UpsertBoneGroup {
        group: crate::character::BoneGroup,
    },
    SavePosePreset {
        name: String,
    },
    ApplyPosePreset {
        name: String,
        flipped: bool,
    },
    MirrorRigPose {
        left_to_right: bool,
    },
    AddAnimationEvent {
        clip_index: usize,
        event: crate::character::AnimationEventKey,
    },
    ApplyFootRoll {
        heel_id: BoneId,
        ball_id: BoneId,
        toe_id: BoneId,
        roll_deg: f32,
    },
    CleanupMesh {
        node_id: NodeId,
        weld_epsilon: f32,
    },
    AddMeshModifier {
        node_id: NodeId,
        modifier: crate::mesh::MeshModifier,
    },
    SetNodeExtras {
        node_id: NodeId,
        extras: std::collections::BTreeMap<String, String>,
    },
    SetCollisionHull {
        node_id: NodeId,
        hull: Option<crate::mesh::CollisionHull>,
    },
    SetMeshInstances {
        node_id: NodeId,
        transforms: Vec<Transform>,
    },
    SetLookDevPreset {
        preset: crate::scene::LookDevPreset,
    },
    SaveCameraBookmark {
        bookmark: crate::scene::CameraBookmark,
    },
    AddReferenceImagePlane {
        plane: crate::scene::ReferenceImagePlane,
    },
    SetNprLookParams {
        params: crate::scene::NprLookParams,
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
        previous_selected_face: Option<FaceId>,
        orphaned_children: Vec<NodeId>,
        previous_rig: Option<Box<CharacterRig>>,
        previous_extras: Option<std::collections::BTreeMap<String, String>>,
        previous_collision_hull: Option<crate::mesh::CollisionHull>,
        previous_mesh_instances: Option<Vec<Transform>>,
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
    RestoreMeshNprSlot {
        node_id: NodeId,
        slot: crate::mesh::NprTextureSlot,
        previous: Option<MeshTexture>,
    },
    RestoreMeshVertexPositions {
        node_id: NodeId,
        previous: Vec<(usize, glam::Vec3)>,
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
    RestoreBoneName {
        bone_id: BoneId,
        previous: String,
    },
    RestoreBoneRotation {
        bone_id: BoneId,
        previous: Quat,
    },
    RestoreBoneRotations(Vec<(BoneId, Quat)>),
    RestoreMorphDefaultWeight {
        node_id: NodeId,
        name: String,
        previous_mesh: LowPolyMesh,
        previous_rig_weight: Option<Option<f32>>,
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
    RestoreNodeRigSkinned {
        node_id: NodeId,
        previous_unrigged: bool,
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
    RestoreNodeExtras {
        node_id: NodeId,
        previous: Option<std::collections::BTreeMap<String, String>>,
    },
    RestoreCollisionHull {
        node_id: NodeId,
        previous: Option<crate::mesh::CollisionHull>,
    },
    RestoreMeshInstances {
        node_id: NodeId,
        previous: Option<Vec<Transform>>,
    },
    RestoreLookDevPreset(crate::scene::LookDevPreset),
    RestoreCameraBookmarks(Vec<crate::scene::CameraBookmark>),
    RestoreReferencePlanes(Vec<crate::scene::ReferenceImagePlane>),
    RestoreNprLookParams(crate::scene::NprLookParams),
    Transaction(Vec<UndoDelta>),
}

pub(super) fn insert_validated_scene_node(
    scene: &mut SceneState,
    raw_name: &str,
    kind: NodeKind,
    parent: Option<NodeId>,
    transform: Transform,
) -> Result<(NodeId, Option<NodeId>), CoreError> {
    let name = crate::scene::validate_node_name(raw_name)?;
    let id = scene.allocate_node_id();
    let previous_selection = scene.selected_node;
    scene.nodes.insert(
        id,
        SceneNode {
            id,
            name,
            kind,
            visible: true,
            parent,
            transform,
        },
    );
    scene.selected_node = Some(id);
    Ok((id, previous_selection))
}

impl Command {
    /// Applies this command to `scene`, returning an `UndoDelta` for deterministic undo.
    pub fn apply(&self, scene: &mut SceneState) -> Result<UndoDelta, CoreError> {
        match self {
            Self::SetMode(..)
            | Self::CreateNode { .. }
            | Self::DeleteNode(..)
            | Self::SelectNode(..)
            | Self::SetNodeTransform { .. }
            | Self::SetNodeParent { .. }
            | Self::SetLightDirection { .. }
            | Self::SetShadingMode(..)
            | Self::SetPaletteEntry { .. }
            | Self::CreateGroupNode { .. }
            | Self::DuplicateNode { .. }
            | Self::RenameNode { .. }
            | Self::ClearScene
            | Self::ResetCharacterPose
            | Self::SetNodeExtras { .. }
            | Self::SetCollisionHull { .. }
            | Self::SetMeshInstances { .. }
            | Self::SetLookDevPreset { .. }
            | Self::SaveCameraBookmark { .. }
            | Self::AddReferenceImagePlane { .. }
            | Self::SetNprLookParams { .. } => scene_cmd::apply(self, scene),
            Self::CreateVoxelVolume { .. }
            | Self::SetVoxels { .. }
            | Self::FillVoxelBox { .. }
            | Self::ConvertPixelToVoxel { .. }
            | Self::ConvertVoxelToLowPoly { .. } => voxel_cmd::apply(self, scene),
            Self::CreatePixelCanvas { .. }
            | Self::DrawPixels { .. }
            | Self::AddPixelLayer { .. }
            | Self::SetPixelLayerVisible { .. }
            | Self::AddPixelFrame { .. }
            | Self::SetActivePixelFrame { .. }
            | Self::SetPixelFps { .. }
            | Self::SetPixelLayerOpacity { .. }
            | Self::SetPixelOnionSkin { .. } => pixel_cmd::apply(self, scene),
            Self::CreatePrimitiveMesh { .. }
            | Self::CreateTerrainMesh { .. }
            | Self::CreateCurveTubeMesh { .. }
            | Self::ImportLowPolyMesh { .. }
            | Self::ExtrudeMeshFace { .. }
            | Self::InsetMeshFace { .. }
            | Self::TranslateMeshFace { .. }
            | Self::MoveMeshVertices { .. }
            | Self::SetMeshVertex { .. }
            | Self::SetMeshVerticesBatch { .. }
            | Self::SetMeshFaceMaterials { .. }
            | Self::MirrorLowPolyMesh { .. }
            | Self::TaperLowPolyMesh { .. }
            | Self::BendLowPolyMesh { .. }
            | Self::SetMeshOrigin { .. }
            | Self::ApplyMeshTransform { .. }
            | Self::WeldMeshVertices { .. }
            | Self::FlipMeshNormals { .. }
            | Self::JoinLowPolyMeshes { .. }
            | Self::SetMeshFaceUvs { .. }
            | Self::SetMeshVertexWeights { .. }
            | Self::PaintMeshWeights { .. }
            | Self::ClearMeshVertexWeights { .. }
            | Self::AddMorphDelta { .. }
            | Self::ClearMorphs { .. }
            | Self::SmoothSubdivideMesh { .. }
            | Self::BevelMesh { .. }
            | Self::DecimateMesh { .. }
            | Self::SetMeshSmoothShading { .. }
            | Self::SetMeshMaterial { .. }
            | Self::PaintMeshTexture { .. }
            | Self::PaintMeshSpot3D { .. }
            | Self::PaintMeshLine { .. }
            | Self::PaintMeshRect { .. }
            | Self::ProjectMeshUvs { .. }
            | Self::SubdivideMeshFace { .. }
            | Self::FillMeshTexture { .. }
            | Self::SetMeshTexture { .. }
            | Self::SetMeshNormalMap { .. }
            | Self::SetMeshAoMap { .. }
            | Self::SetMeshNprSlot { .. }
            | Self::SetMeshUvSeams { .. }
            | Self::NormalizeMeshTexelDensity { .. }
            | Self::SculptMeshStroke { .. }
            | Self::AddMeshPaintLayer { .. }
            | Self::SetMeshPaintLayer { .. }
            | Self::DeleteMeshPaintLayer { .. }
            | Self::SetMeshVertexColors { .. }
            | Self::ClearMeshVertexColors { .. }
            | Self::SetMeshProxyNormals { .. }
            | Self::SetMeshCreaseEdges { .. }
            | Self::ClearMeshTexture { .. }
            | Self::SetMorphTargetDeltas { .. }
            | Self::SetMorphDefaultWeight { .. }
            | Self::ApplyProxyCageDeform { .. }
            | Self::BakeProceduralRibbon { .. }
            | Self::CleanupMesh { .. }
            | Self::AddMeshModifier { .. } => mesh_cmd::apply(self, scene),
            Self::UpdateAnimationClipMeta { .. }
            | Self::ApplyBipedRig
            | Self::ApplyHumanoidRig
            | Self::SetCharacterRig { .. }
            | Self::AddBone { .. }
            | Self::RenameBone { .. }
            | Self::ReparentBone { .. }
            | Self::SetBoneRotation { .. }
            | Self::SetBoneTranslation { .. }
            | Self::SetBoneRestPosition { .. }
            | Self::SetBoneRestRotation { .. }
            | Self::SolveTwoBoneIk { .. }
            | Self::BindNodeToBone { .. }
            | Self::SetNodeBoneMask { .. }
            | Self::SetNodeRigSkinned { .. }
            | Self::AddAnimationClip { .. }
            | Self::SetAnimationKeyframe { .. }
            | Self::RemoveAnimationKeyframe { .. }
            | Self::SetMorphWeightKeyframe { .. }
            | Self::SetVisibilityKeyframe { .. }
            | Self::SetParentSwitchKeyframe { .. }
            | Self::SetClipLayers { .. }
            | Self::UpsertAnimationClip { .. }
            | Self::RemoveMorphWeightKeyframe { .. }
            | Self::SetActiveAnimationClip { .. }
            | Self::RemoveAnimationClip { .. }
            | Self::RemoveBone { .. }
            | Self::SetNodeTransformKeyframe { .. }
            | Self::SetNodeVisibility { .. }
            | Self::SetAnimationPlaybackTime { .. }
            | Self::CreateBoneGrid { .. }
            | Self::BakeClothGridAnimation { .. }
            | Self::ApplyStickGizmoPose { .. }
            | Self::SetBoneScale { .. }
            | Self::SetBoneProperties { .. }
            | Self::SetChainPole { .. }
            | Self::UpsertRigConstraint { .. }
            | Self::UpsertRigSocket { .. }
            | Self::UpsertBoneGroup { .. }
            | Self::SavePosePreset { .. }
            | Self::ApplyPosePreset { .. }
            | Self::MirrorRigPose { .. }
            | Self::AddAnimationEvent { .. }
            | Self::ApplyFootRoll { .. } => character_cmd::apply(self, scene),
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
                Ok(UndoDelta::Transaction(applied))
            }
        }
    }
}
