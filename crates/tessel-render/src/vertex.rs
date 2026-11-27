use bytemuck::{Pod, Zeroable};

/// GPU vertex layout for static 3D Voxel & Low-Poly geometry (64 bytes, Right-Handed Y-Up).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct StudioVertex3D {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub palette_uv: [f32; 2],
    pub ao: f32,
    pub tangent: [f32; 4],
    pub outline_normal: [f32; 3],
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

/// Per-mesh PBR & NPR material uniform (80 bytes) matching `mesh_3d.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialUniform {
    /// `[metallic, roughness, receive_shadow (1.0/0.0), alpha_cutoff]`
    pub pbr: [f32; 4],
    /// `[emissive_r, emissive_g, emissive_b, unlit_flag]`
    pub emissive: [f32; 4],
    /// `[matcap_strength, has_face_sdf, has_hatching, hatching_scale]`
    pub npr_slots: [f32; 4],
    /// `[light_x, light_y, light_z, has_face_sdf_light_override]`
    pub face_sdf_light: [f32; 4],
    /// `[emissive_intensity, use_palette_mat_lut (1.0/0.0), transmission, ior]`
    pub hdr_flags: [f32; 4],
}

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            pbr: [0.0, 0.85, 1.0, 0.0],
            emissive: [0.0, 0.0, 0.0, 0.0],
            npr_slots: [0.0, 0.0, 0.0, 1.0],
            face_sdf_light: [0.0, 0.0, 1.0, 0.0],
            hdr_flags: [1.0, 1.0, 0.0, 1.5],
        }
    }
}

/// Directional shadow-map light projection, NPR viewport shading, look-dev tint, and forward fog params (144 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShadowMapUniform {
    pub light_view_proj: [[f32; 4]; 4],
    /// `[normal_bias, depth_bias, shadow_strength, enabled]`
    pub bias_params: [f32; 4],
    /// `[shading_mode (0=PBR,1=NPR,2=Silhouette,3=Luminance), outline_width, fog_dist, paper_grain]`
    pub npr_params: [f32; 4],
    /// `[fog_r, fog_g, fog_b, fog_density]`
    pub fog_color_density: [f32; 4],
    /// `[fog_enabled (1.0/0.0), fog_start, fog_height_falloff, 0.0]`
    pub fog_params: [f32; 4],
    /// `[key_tint_r, key_tint_g, key_tint_b, ambient_scale]`
    pub lookdev_tint: [f32; 4],
}

impl Default for ShadowMapUniform {
    fn default() -> Self {
        Self {
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            bias_params: [0.015, 0.0015, 0.55, 0.0],
            npr_params: [0.0, 0.014, 40.0, 0.06],
            fog_color_density: [0.08, 0.10, 0.14, 0.02],
            fog_params: [0.0, 8.0, 0.25, 0.0],
            lookdev_tint: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
