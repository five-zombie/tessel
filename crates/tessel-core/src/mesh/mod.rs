pub mod bake;
pub mod bvh;
pub mod cage;
mod face;
pub mod layer;
mod lowpoly;
mod nano;
pub mod npr;
pub mod ops;
pub mod paint;
pub mod pipeline;
pub mod ribbon;
pub mod sculpt;
pub mod smart_material;
pub mod texture;
pub mod uv_seam;
pub mod vat;

pub use bake::{bake_mesh_maps, BakeMapKind, BakeParams, BakedMaps};
pub use bvh::MeshBvh;
pub use cage::ProxyCage;
pub use face::Face;
pub use layer::{composite_paint_layers, MeshPaintLayer};
pub use lowpoly::{
    LowPolyMesh, MeshMaterial, MorphTarget, NprTextureSlot, TriVertex, TriangulatedMesh,
};
pub use nano::{FaceId, PrimitiveAnchor, PrimitiveKind, VertexId};
pub use npr::{
    compute_smooth_outline_normals, dilate_rgba_texture, validate_mesh_topology, AlphaMode,
    TopologyReport,
};
pub use ops::{apply_primitive_anchor, TerrainParams};
pub use paint::{paint_mesh_spot_3d, PaintSpot3DParams};
pub use pipeline::{
    cleanup_mesh, evaluate_modifier_stack, extract_rgba_silhouette_and_palette, pack_orm_texture,
    preflight_check_scene, CollisionHull, ImageSilhouettePalette, MeshModifier, PreflightIssue,
    PreflightReport, PreflightSeverity,
};
pub use ribbon::{
    build_ribbon_visibility_track, generate_ribbon_mesh, sample_trajectory_points, RibbonParams,
};
pub use sculpt::{apply_sculpt_stroke, SculptBrushKind, SculptStrokeParams};
pub use smart_material::{
    generate_smart_material_texture, SmartMaterialOutput, SmartMaterialParams,
};
pub use texture::{box_atlas_uv, MeshTexture, UvProjection, DEFAULT_MESH_TEX_SIZE};
pub use uv_seam::{
    canonical_edge, normalize_texel_density, unwrap_with_uv_seams, TexelDensityReport,
};
pub use vat::{
    bake_vertex_animation_texture, sample_vat_position, VatBakeParams, VatBakeResult, VatMetadata,
};
