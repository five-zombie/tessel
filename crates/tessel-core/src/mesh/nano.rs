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
    Sphere,
    Torus,
    Capsule,
}

impl PrimitiveKind {
    pub const ALL: [Self; 7] = [
        Self::Cube,
        Self::Plane,
        Self::Cylinder,
        Self::Cone,
        Self::Sphere,
        Self::Torus,
        Self::Capsule,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Plane => "Plane",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Sphere => "Sphere",
            Self::Torus => "Torus",
            Self::Capsule => "Capsule",
        }
    }
}
