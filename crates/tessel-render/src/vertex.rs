use bytemuck::{Pod, Zeroable};

/// GPU vertex layout for static 3D Voxel & Low-Poly geometry (36 bytes, Right-Handed Y-Up).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct StudioVertex3D {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub palette_uv: [f32; 2],
    pub ao: f32,
}

/// Shared per-frame camera uniform matching `assets/shaders/grid.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
}
