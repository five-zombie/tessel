use bytemuck::{Pod, Zeroable};

/// GPU vertex layout for static 3D Voxel & Low-Poly geometry (52 bytes, Right-Handed Y-Up).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct StudioVertex3D {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub palette_uv: [f32; 2],
    pub ao: f32,
    pub tangent: [f32; 4],
}

/// Shared per-frame camera & lighting uniform matching 3D WGSL shaders.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub light_dir: [f32; 4],
}

/// Per-mesh PBR material uniform (32 bytes) matching `mesh_3d.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialUniform {
    /// `[metallic, roughness, 0.0, 0.0]`
    pub pbr: [f32; 4],
    /// `[emissive_r, emissive_g, emissive_b, 0.0]`
    pub emissive: [f32; 4],
}

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            pbr: [0.0, 0.85, 0.0, 0.0],
            emissive: [0.0, 0.0, 0.0, 0.0],
        }
    }
}
