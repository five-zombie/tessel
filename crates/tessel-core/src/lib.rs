pub mod character;
pub mod command;
pub mod error;
pub mod mesh;
pub mod palette;
pub mod pixel;
pub mod scene;
pub mod voxel;

pub use character::{
    solve_chain_ik, solve_for_rig, solve_two_bone_ik, AnimationClip, AutoSkinSolver, Bone,
    BoneChannel, BoneId, CharacterRig, ClipLayer, ContactChain, GroundingOptions, Keyframe,
    KeyframeInterpolation, MorphWeightKey, MorphWeightTrack, NodeTransformTrack, ParentSwitchKey,
    ParentSwitchTrack, SampledBonePose, SkinBinding, SkinnedBoundsCache, SurfaceHit, SurfaceSource,
    SurfaceTarget, VertexSkinWeights, VisibilityKey, VisibilityTrack,
};
pub use command::{Command, HistoryStack, UndoDelta};
pub use error::CoreError;
pub use mesh::{
    bake_mesh_maps, box_atlas_uv, composite_paint_layers, compute_smooth_outline_normals,
    dilate_rgba_texture, generate_smart_material_texture, paint_mesh_spot_3d,
    validate_mesh_topology, AlphaMode, BakeMapKind, BakeParams, BakedMaps, Face, FaceId,
    LowPolyMesh, MeshMaterial, MeshPaintLayer, MeshTexture, MorphTarget, PaintSpot3DParams,
    PrimitiveKind, SmartMaterialOutput, SmartMaterialParams, TerrainParams, TopologyReport,
    TriVertex, TriangulatedMesh, VertexId, DEFAULT_MESH_TEX_SIZE,
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
