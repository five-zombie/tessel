pub mod character;
pub mod command;
pub mod error;
pub mod mesh;
pub mod palette;
pub mod pixel;
pub mod scene;
pub mod voxel;

pub use character::{
    apply_foot_roll, apply_stick_gizmo_pose, assign_bone_grid_mask, bake_cloth_grid_animation,
    bake_spring_bones, bone_longitudinal_axis, create_bone_grid, evaluate_rig_constraints,
    find_mirror_bone, mirror_rig_pose, prune_and_normalize_weights, retarget_clip_from_mixamo,
    snap_angle_deg, solve_chain_ik, solve_for_rig, solve_two_bone_ik, swap_rig_pose_lr,
    AnimationClip, AnimationEventKey, AutoSkinSolver, Bone, BoneChannel, BoneColliderMeta,
    BoneConstraint, BoneGridResult, BoneGridSpec, BoneGroup, BoneId, CharacterRig,
    ClipGroundingRequest, ClipLayer, ClothWindParams, ContactChain, GroundingOptions, Keyframe,
    KeyframeInterpolation, MorphWeightKey, MorphWeightTrack, NodeTransformTrack, ParentSwitchKey,
    ParentSwitchTrack, RigSocket, SampledBonePose, SkinBinding, SkinnedBoundsCache, StickGizmoMode,
    StickGizmoParams, StickGizmoSettings, SurfaceHit, SurfaceSource, SurfaceTarget,
    VertexSkinWeights, VisibilityKey, VisibilityTrack, MIXAMO_TO_TESSEL_MAP,
};
pub use command::{Command, HistoryStack, UndoDelta};
pub use error::CoreError;
pub use mesh::{
    apply_primitive_anchor, apply_sculpt_stroke, bake_mesh_maps, bake_vertex_animation_texture,
    box_atlas_uv, build_ribbon_visibility_track, canonical_edge, cleanup_mesh,
    composite_paint_layers, compute_smooth_outline_normals, dilate_rgba_texture,
    evaluate_modifier_stack, extract_rgba_silhouette_and_palette, generate_ribbon_mesh,
    generate_smart_material_texture, normalize_texel_density, pack_orm_texture, paint_mesh_spot_3d,
    preflight_check_scene, sample_trajectory_points, sample_vat_position, unwrap_with_uv_seams,
    validate_mesh_topology, AlphaMode, BakeMapKind, BakeParams, BakedMaps, CollisionHull, Face,
    FaceId, ImageSilhouettePalette, LowPolyMesh, MeshBvh, MeshMaterial, MeshModifier,
    MeshPaintLayer, MeshTexture, MorphTarget, NprTextureSlot, PaintSpot3DParams, PreflightIssue,
    PreflightReport, PreflightSeverity, PrimitiveAnchor, PrimitiveKind, ProxyCage, RibbonParams,
    SculptBrushKind, SculptStrokeParams, SmartMaterialOutput, SmartMaterialParams, TerrainParams,
    TexelDensityReport, TopologyReport, TriVertex, TriangulatedMesh, VatBakeParams, VatBakeResult,
    VatMetadata, VertexId, DEFAULT_MESH_TEX_SIZE,
};
pub use palette::{Palette, PaletteEntry, PaletteIndex, Rgba8};
pub use pixel::{
    FrameIndex, LayerId, PixelCanvas, PixelCoord, PixelEdit, PixelFrame, PixelLayer, PixelTool,
    Symmetry2D,
};
pub use scene::{
    validate_node_name, AppMode, CameraBookmark, LookDevPreset, NodeId, NodeKind, NprLookParams,
    PostProcessSettings, ReferenceImagePlane, SceneNode, SceneState, Transform,
    ViewportShadingMode, MAX_NODE_NAME_LEN,
};
pub use voxel::{
    ChunkCoord, Symmetry3D, VoxelChunk, VoxelCoord, VoxelEdit, VoxelHit, VoxelMeshOutput,
    VoxelTool, VoxelVertex, VoxelVolume,
};
