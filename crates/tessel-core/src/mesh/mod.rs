mod face;
mod lowpoly;
mod nano;
pub mod ops;
pub mod texture;

pub use face::Face;
pub use lowpoly::{LowPolyMesh, TriVertex, TriangulatedMesh};
pub use nano::{FaceId, PrimitiveKind, VertexId};
pub use texture::{box_atlas_uv, MeshTexture, DEFAULT_MESH_TEX_SIZE};
