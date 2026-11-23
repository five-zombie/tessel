mod face;
mod lowpoly;
mod nano;
pub mod ops;

pub use face::Face;
pub use lowpoly::{LowPolyMesh, TriVertex, TriangulatedMesh};
pub use nano::{FaceId, PrimitiveKind, VertexId};
