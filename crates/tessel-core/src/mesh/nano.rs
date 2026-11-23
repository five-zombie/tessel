use serde::{Deserialize, Serialize};

/// Nano-Unit: Index into `LowPolyMesh::vertices`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VertexId(pub usize);

/// Nano-Unit: Index into `LowPolyMesh::faces`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FaceId(pub usize);

/// Nano-Unit: Built-in starter low-poly primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveKind {
    Cube,
    Plane,
    Cylinder,
    Cone,
}

impl PrimitiveKind {
    pub const ALL: [Self; 4] = [Self::Cube, Self::Plane, Self::Cylinder, Self::Cone];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Plane => "Plane",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
        }
    }
}
