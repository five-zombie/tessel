pub mod character;
pub mod command;
pub mod error;
pub mod mesh;
pub mod palette;
pub mod pixel;
pub mod scene;
pub mod voxel;

pub use character::{
    apply_stick_gizmo_pose, assign_bone_grid_mask, bake_cloth_grid_animation,
    bone_longitudinal_axis, create_bone_grid, find_mirror_bone, snap_angle_deg, solve_chain_ik,
    solve_for_rig, solve_two_bone_ik, AnimationClip, AutoSkinSolver, Bone, BoneChannel,
    BoneGridResult, BoneGridSpec, BoneId, CharacterRig, ClipLayer, ClothWindParams, ContactChain,
    GroundingOptions, Keyframe, KeyframeInterpolation, MorphWeightKey, MorphWeightTrack,
    NodeTransformTrack, ParentSwitchKey, ParentSwitchTrack, SampledBonePose, SkinBinding,
    SkinnedBoundsCache, StickGizmoMode, StickGizmoParams, StickGizmoSettings, SurfaceHit,
    SurfaceSource, SurfaceTarget, VertexSkinWeights, VisibilityKey, VisibilityTrack,
};
pub use command::{Command, HistoryStack, UndoDelta};
pub use error::CoreError;
pub use mesh::{
    apply_primitive_anchor, bake_mesh_maps, bake_vertex_animation_texture, box_atlas_uv,
    build_ribbon_visibility_track, composite_paint_layers, compute_smooth_outline_normals,
    dilate_rgba_texture, generate_ribbon_mesh, generate_smart_material_texture, paint_mesh_spot_3d,
    sample_trajectory_points, sample_vat_position, validate_mesh_topology, AlphaMode, BakeMapKind,
    BakeParams, BakedMaps, Face, FaceId, LowPolyMesh, MeshMaterial, MeshPaintLayer, MeshTexture,
    MorphTarget, PaintSpot3DParams, PrimitiveAnchor, PrimitiveKind, ProxyCage, RibbonParams,
    SmartMaterialOutput, SmartMaterialParams, TerrainParams, TopologyReport, TriVertex,
    TriangulatedMesh, VatBakeParams, VatBakeResult, VatMetadata, VertexId, DEFAULT_MESH_TEX_SIZE,
};
pub use palette::{Palette, PaletteEntry, PaletteIndex, Rgba8};
pub use pixel::{
    FrameIndex, LayerId, PixelCanvas, PixelCoord, PixelEdit, PixelFrame, PixelLayer, PixelTool,
    Symmetry2D,
};
pub use scene::{AppMode, NodeId, NodeKind, SceneNode, SceneState, Transform, ViewportShadingMode};
pub use voxel::{
    ChunkCoord, Symmetry3D, VoxelChunk, VoxelCoord, VoxelEdit, VoxelHit, VoxelMeshOutput,
    VoxelTool, VoxelVertex, VoxelVolume,
};
