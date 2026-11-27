use tessel_core::{
    AlphaMode, LookDevPreset, MeshMaterial, MeshTexture, Palette, PaletteIndex,
    PostProcessSettings, ViewportShadingMode,
};
use wgpu::util::DeviceExt;

use crate::vertex::{CameraUniform, MaterialUniform, ShadowMapUniform, StudioVertex3D};

const MESH_SHADER_WGSL: &str = include_str!("../../../assets/shaders/mesh_3d.wgsl");
const GLASS_SHADER_WGSL: &str = include_str!("../../../assets/shaders/glass_3d.wgsl");
const SHADOW_DEPTH_SHADER_WGSL: &str = include_str!("../../../assets/shaders/shadow_depth.wgsl");
const OUTLINE_HULL_SHADER_WGSL: &str = include_str!("../../../assets/shaders/outline_hull.wgsl");
const SHADOW_MAP_SIZE: u32 = 2048;

/// Computes an orthographic directional light view-projection matrix framing `[min, max]`.
pub fn compute_light_view_proj(light_dir: glam::Vec3, min: [f32; 3], max: [f32; 3]) -> glam::Mat4 {
    let dir = light_dir.normalize_or_zero();
    let key_dir = if dir.length_squared() > 1e-6 {
        dir
    } else {
        glam::Vec3::new(0.55, 0.82, 0.45).normalize()
    };
    let (center, radius) = if min.iter().all(|v| v.is_finite())
        && max.iter().all(|v| v.is_finite())
        && min[0] <= max[0]
        && min[1] <= max[1]
        && min[2] <= max[2]
    {
        let mn = glam::Vec3::from_array(min);
        let mx = glam::Vec3::from_array(max);
        ((mn + mx) * 0.5, ((mx - mn).length() * 0.5).max(1.5))
    } else {
        (glam::Vec3::ZERO, 4.0)
    };
    let eye = center + key_dir * (radius * 2.5);
    let up = if key_dir.y.abs() > 0.98 {
        glam::Vec3::Z
    } else {
        glam::Vec3::Y
    };
    let view = glam::Mat4::look_at_rh(eye, center, up);
    let extent = radius * 1.25;
    let proj = glam::Mat4::orthographic_rh(-extent, extent, -extent, extent, 0.05, radius * 5.5);
    proj * view
}

pub struct CustomMeshUpload<'a> {
    pub texture: Option<&'a MeshTexture>,
    pub normal_map: Option<&'a MeshTexture>,
    pub matcap_map: Option<&'a MeshTexture>,
    pub face_sdf_map: Option<&'a MeshTexture>,
    pub hatching_ramp_map: Option<&'a MeshTexture>,
    pub material: MeshMaterial,
    /// When `true` (translucent voxel submeshes), forces Palette Material LUT lookup and Pass 2.5 glass routing.
    pub force_palette_lut: bool,
    pub vertices: Vec<StudioVertex3D>,
    pub indices: Vec<u32>,
}

struct TexturedMeshDraw {
    _texture: Option<wgpu::Texture>,
    _normal_texture: Option<wgpu::Texture>,
    _matcap_texture: Option<wgpu::Texture>,
    _face_sdf_texture: Option<wgpu::Texture>,
    _hatching_texture: Option<wgpu::Texture>,
    _material_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    cast_shadow: bool,
    is_glass: bool,
    alpha_blend: bool,
    centroid: [f32; 3],
}

/// Renders shaded 3D Voxel & Low-Poly meshes using a 256x1 palette texture (or per-mesh RGBA texture)
/// with per-mesh PBR material properties, tangent-space normal mapping, real directional shadow map,
/// NPR / Sumi-e preview + inverted-hull outlines, Pass 2.5 glass refraction + back-to-front sorting, and depth testing.
pub struct MeshPass {
    pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    shadow_depth_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    glass_opaque_bgl: wgpu::BindGroupLayout,
    glass_opaque_sampler: wgpu::Sampler,
    glass_opaque_bg: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    _default_material_buffer: wgpu::Buffer,
    palette_texture: wgpu::Texture,
    palette_view: wgpu::TextureView,
    palette_mat_texture: wgpu::Texture,
    palette_mat_view: wgpu::TextureView,
    _default_normal_texture: wgpu::Texture,
    default_normal_view: wgpu::TextureView,
    _default_white_texture: wgpu::Texture,
    default_white_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    _shadow_depth_texture: wgpu::Texture,
    shadow_depth_view: wgpu::TextureView,
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_depth_bg: wgpu::BindGroup,
    shadow_sample_bg: wgpu::BindGroup,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_capacity: usize,
    index_buffer: Option<wgpu::Buffer>,
    index_capacity: usize,
    index_count: u32,
    textured_draws: Vec<TexturedMeshDraw>,
}

impl MeshPass {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        sample_count: u32,
        opaque_copy_view: &wgpu::TextureView,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-mesh-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER_WGSL.into()),
        });

        let initial_cam = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            inv_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 5.0, 5.0, 1.0],
            light_dir: [0.55, 0.82, 0.45, 0.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-mesh-camera-uniform"),
            contents: bytemuck::bytes_of(&initial_cam),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let palette_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-palette-256x1"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_view = palette_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // 256x2 Rgba16Float Palette Material LUT:
        // Row 0 (v=0.25): [roughness, metallic, emissive_hdr, transmission]
        // Row 1 (v=0.75): [ior, 0.0, 0.0, 0.0]
        let palette_mat_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-palette-mat-lut-256x2"),
            size: wgpu::Extent3d {
                width: 256,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_mat_view =
            palette_mat_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tessel-palette-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let default_normal_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-default-flat-normal-1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let default_normal_view =
            default_normal_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let default_white_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-default-white-1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let default_white_view =
            default_white_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let default_material = MaterialUniform::default();
        let default_material_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-default-material-uniform"),
                contents: bytemuck::bytes_of(&default_material),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-mesh-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tessel-mesh-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: default_material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&default_normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&default_white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&default_white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&default_white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&palette_mat_view),
                },
            ],
        });

        let shadow_depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-dir-shadow-depth-2048"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_depth_view =
            shadow_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tessel-dir-shadow-cmp-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-dir-shadow-uniform"),
            contents: bytemuck::bytes_of(&ShadowMapUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shadow_depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-shadow-depth-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shadow_depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tessel-shadow-depth-bg"),
            layout: &shadow_depth_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_uniform_buffer.as_entire_binding(),
            }],
        });

        let shadow_sample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-shadow-sample-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_sample_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tessel-shadow-sample-bg"),
            layout: &shadow_sample_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-mesh-pipeline-layout"),
            bind_group_layouts: &[&bgl, &shadow_sample_bgl],
            push_constant_ranges: &[],
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<StudioVertex3D>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: 36,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 52,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-mesh-3d-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-shadow-depth-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_DEPTH_SHADER_WGSL.into()),
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tessel-shadow-depth-pipeline-layout"),
                bind_group_layouts: &[&shadow_depth_bgl],
                push_constant_ranges: &[],
            });
        let shadow_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<StudioVertex3D>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: 36,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 52,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        };
        let shadow_depth_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tessel-shadow-depth-pipeline"),
                layout: Some(&shadow_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shadow_shader,
                    entry_point: Some("vs_main"),
                    buffers: std::slice::from_ref(&shadow_vertex_layout),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: None,
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    front_face: wgpu::FrontFace::Ccw,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let blend_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-mesh-3d-blend-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: std::slice::from_ref(&shadow_vertex_layout),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let glass_opaque_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-glass-opaque-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let glass_opaque_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tessel-glass-opaque-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let glass_opaque_bg = create_glass_opaque_bind_group(
            device,
            &glass_opaque_bgl,
            opaque_copy_view,
            &glass_opaque_sampler,
        );

        let glass_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-glass-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(GLASS_SHADER_WGSL.into()),
        });
        let glass_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tessel-glass-pipeline-layout"),
                bind_group_layouts: &[&bgl, &shadow_sample_bgl, &glass_opaque_bgl],
                push_constant_ranges: &[],
            });
        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-glass-3d-pipeline"),
            layout: Some(&glass_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &glass_shader,
                entry_point: Some("vs_main"),
                buffers: std::slice::from_ref(&shadow_vertex_layout),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &glass_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-outline-hull-shader"),
            source: wgpu::ShaderSource::Wgsl(OUTLINE_HULL_SHADER_WGSL.into()),
        });
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-outline-hull-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_shader,
                entry_point: Some("vs_main"),
                buffers: std::slice::from_ref(&shadow_vertex_layout),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Front),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            blend_pipeline,
            glass_pipeline,
            outline_pipeline,
            shadow_depth_pipeline,
            bgl,
            glass_opaque_bgl,
            glass_opaque_sampler,
            glass_opaque_bg,
            camera_buffer,
            _default_material_buffer: default_material_buffer,
            palette_texture,
            palette_view,
            palette_mat_texture,
            palette_mat_view,
            _default_normal_texture: default_normal_texture,
            default_normal_view,
            _default_white_texture: default_white_texture,
            default_white_view,
            sampler,
            bind_group,
            _shadow_depth_texture: shadow_depth_texture,
            shadow_depth_view,
            shadow_uniform_buffer,
            shadow_depth_bg,
            shadow_sample_bg,
            vertex_buffer: None,
            vertex_capacity: 0,
            index_buffer: None,
            index_capacity: 0,
            index_count: 0,
            textured_draws: Vec::new(),
        }
    }

    pub fn upload_palette(&self, queue: &wgpu::Queue, palette: &Palette) {
        // Initialize 1x1 flat tangent-space normal map (0, 0, 1) -> [128, 128, 255, 255].
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._default_normal_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[128u8, 128, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._default_white_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let mut rgba = vec![255u8; 256 * 4];
        let mut lut_f16 = vec![0u16; 256 * 2 * 4];
        // Initialize default matte material values across all 256 slots.
        let def_rough = f32_to_f16_bits(0.85);
        let def_ior = f32_to_f16_bits(1.5);
        for slot in 0..256usize {
            lut_f16[slot * 4] = def_rough;
            lut_f16[(256 + slot) * 4] = def_ior;
        }
        for raw in 1u8..=255 {
            if let Ok(idx) = PaletteIndex::new(raw) {
                if let Some(entry) = palette.get(idx) {
                    let slot = usize::from(raw);
                    let base = slot * 4;
                    rgba[base] = entry.color.r;
                    rgba[base + 1] = entry.color.g;
                    rgba[base + 2] = entry.color.b;
                    rgba[base + 3] = entry.color.a;
                    lut_f16[base] = f32_to_f16_bits(entry.roughness.clamp(0.05, 1.0));
                    lut_f16[base + 1] = f32_to_f16_bits(entry.metallic.clamp(0.0, 1.0));
                    lut_f16[base + 2] = f32_to_f16_bits(entry.emissive.clamp(0.0, 20.0));
                    lut_f16[base + 3] = f32_to_f16_bits(entry.transmission.clamp(0.0, 1.0));
                    let row1 = (256 + slot) * 4;
                    lut_f16[row1] = f32_to_f16_bits(entry.ior.clamp(1.0, 2.5));
                }
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.palette_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.palette_mat_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut_f16),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 8),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 256,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[StudioVertex3D],
        indices: &[u32],
    ) {
        if vertices.is_empty() || indices.is_empty() {
            self.index_count = 0;
            return;
        }
        if vertices.len() > self.vertex_capacity || self.vertex_buffer.is_none() {
            let new_cap = vertices.len().next_power_of_two().max(256);
            let byte_size =
                (new_cap * std::mem::size_of::<StudioVertex3D>()) as wgpu::BufferAddress;
            self.vertex_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tessel-mesh-vb"),
                size: byte_size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.vertex_capacity = new_cap;
        }
        if indices.len() > self.index_capacity || self.index_buffer.is_none() {
            let new_cap = indices.len().next_power_of_two().max(512);
            let byte_size = (new_cap * std::mem::size_of::<u32>()) as wgpu::BufferAddress;
            self.index_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tessel-mesh-ib"),
                size: byte_size,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.index_capacity = new_cap;
        }
        if let Some(vb) = &self.vertex_buffer {
            queue.write_buffer(vb, 0, bytemuck::cast_slice(vertices));
        }
        if let Some(ib) = &self.index_buffer {
            queue.write_buffer(ib, 0, bytemuck::cast_slice(indices));
        }
        self.index_count = indices.len() as u32;
    }

    pub fn upload_custom_meshes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[CustomMeshUpload<'_>],
    ) {
        self.textured_draws.clear();
        for item in items {
            let (opt_tex, mat, vertices, indices) =
                (item.texture, item.material, &item.vertices, &item.indices);
            if vertices.is_empty() || indices.is_empty() {
                continue;
            }
            let alpha_cutoff = match mat.alpha_mode {
                AlphaMode::Mask { cutoff } => cutoff.clamp(1e-4, 1.0),
                _ => 0.0,
            };
            let (gpu_matcap_tex, custom_matcap_view) = upload_optional_rgba_texture(
                device,
                queue,
                "tessel-lowpoly-matcap-tex",
                wgpu::TextureFormat::Rgba8UnormSrgb,
                item.matcap_map,
            );
            let (gpu_sdf_tex, custom_sdf_view) = upload_optional_rgba_texture(
                device,
                queue,
                "tessel-lowpoly-facesdf-tex",
                wgpu::TextureFormat::Rgba8Unorm,
                item.face_sdf_map,
            );
            let (gpu_hatch_tex, custom_hatch_view) = upload_optional_rgba_texture(
                device,
                queue,
                "tessel-lowpoly-hatching-tex",
                wgpu::TextureFormat::Rgba8Unorm,
                item.hatching_ramp_map,
            );

            let (sdf_lx, sdf_ly, sdf_lz, has_sdf_override) = match mat.face_sdf_light_override {
                Some([x, y, z]) => (x, y, z, 1.0),
                None => (0.0, 0.0, 1.0, 0.0),
            };
            let use_palette_mat_lut = item.force_palette_lut
                || (item.texture.is_none()
                    && mat.metallic == 0.0
                    && (mat.roughness - 0.85).abs() < 1e-4
                    && mat.emissive == [0.0, 0.0, 0.0]
                    && mat.transmission == 0.0
                    && (mat.ior - 1.5).abs() < 1e-4);
            let mat_uniform = MaterialUniform {
                pbr: [
                    mat.metallic,
                    mat.roughness,
                    if mat.receive_shadow { 1.0 } else { 0.0 },
                    alpha_cutoff,
                ],
                emissive: [
                    mat.emissive[0],
                    mat.emissive[1],
                    mat.emissive[2],
                    if mat.unlit { 1.0 } else { 0.0 },
                ],
                npr_slots: [
                    if custom_matcap_view.is_some() {
                        mat.matcap_strength.clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                    if custom_sdf_view.is_some() { 1.0 } else { 0.0 },
                    if custom_hatch_view.is_some() {
                        1.0
                    } else {
                        0.0
                    },
                    mat.hatching_scale.clamp(0.1, 64.0),
                ],
                face_sdf_light: [sdf_lx, sdf_ly, sdf_lz, has_sdf_override],
                hdr_flags: [
                    mat.emissive_intensity.clamp(0.0, 20.0),
                    if use_palette_mat_lut { 1.0 } else { 0.0 },
                    mat.transmission.clamp(0.0, 1.0),
                    mat.ior.clamp(1.0, 2.5),
                ],
            };
            let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-custom-material-uniform"),
                contents: bytemuck::bytes_of(&mat_uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let (gpu_tex, custom_view) = upload_optional_rgba_texture(
                device,
                queue,
                "tessel-lowpoly-rgba-tex",
                wgpu::TextureFormat::Rgba8UnormSrgb,
                opt_tex,
            );
            let (gpu_norm_tex, custom_norm_view) = upload_optional_rgba_texture(
                device,
                queue,
                "tessel-lowpoly-normal-tex",
                wgpu::TextureFormat::Rgba8Unorm,
                item.normal_map,
            );

            let tex_view_ref = custom_view.as_ref().unwrap_or(&self.palette_view);
            let norm_view_ref = custom_norm_view
                .as_ref()
                .unwrap_or(&self.default_normal_view);
            let matcap_view_ref = custom_matcap_view
                .as_ref()
                .unwrap_or(&self.default_white_view);
            let sdf_view_ref = custom_sdf_view.as_ref().unwrap_or(&self.default_white_view);
            let hatch_view_ref = custom_hatch_view
                .as_ref()
                .unwrap_or(&self.default_white_view);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tessel-lowpoly-custom-bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.camera_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(tex_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: material_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(norm_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(matcap_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(sdf_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(hatch_view_ref),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&self.palette_mat_view),
                    },
                ],
            });
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-custom-mesh-vb"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-custom-mesh-ib"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let is_glass = item.force_palette_lut || mat.transmission > 0.0;
            let centroid = compute_vertices_centroid(vertices);
            self.textured_draws.push(TexturedMeshDraw {
                _texture: gpu_tex,
                _normal_texture: gpu_norm_tex,
                _matcap_texture: gpu_matcap_tex,
                _face_sdf_texture: gpu_sdf_tex,
                _hatching_texture: gpu_hatch_tex,
                _material_buffer: material_buffer,
                bind_group,
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
                cast_shadow: mat.cast_shadow && !is_glass,
                is_glass,
                alpha_blend: !is_glass && mat.alpha_mode == AlphaMode::Blend,
                centroid,
            });
        }
    }

    /// Rebinds `opaque_copy_view` for Pass 2.5 screen-space glass refraction after viewport resize.
    pub fn rebind_opaque_copy(
        &mut self,
        device: &wgpu::Device,
        opaque_copy_view: &wgpu::TextureView,
    ) {
        self.glass_opaque_bg = create_glass_opaque_bind_group(
            device,
            &self.glass_opaque_bgl,
            opaque_copy_view,
            &self.glass_opaque_sampler,
        );
    }

    pub fn has_translucent_draws(&self) -> bool {
        self.textured_draws
            .iter()
            .any(|d| d.is_glass || d.alpha_blend)
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, uniform: &CameraUniform) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(uniform));
    }

    /// Renders all shadow-casting 3D meshes into the 2048x2048 directional depth shadow map
    /// and updates the active NPR/PBR shading mode, forward fog, and look-dev lighting uniform.
    #[allow(clippy::too_many_arguments)]
    pub fn render_shadow_map(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        light_dir: glam::Vec3,
        min: [f32; 3],
        max: [f32; 3],
        shading_mode: ViewportShadingMode,
        post_process: &PostProcessSettings,
        lookdev_preset: LookDevPreset,
    ) {
        let has_geo = self.index_count > 0 || self.textured_draws.iter().any(|d| d.cast_shadow);
        let light_vp = compute_light_view_proj(light_dir, min, max);
        let mode_id = match shading_mode {
            ViewportShadingMode::Pbr => 0.0,
            ViewportShadingMode::NprSumie => 1.0,
            ViewportShadingMode::SilhouetteCheck => 2.0,
            ViewportShadingMode::LuminanceValueCheck => 3.0,
        };
        let lookdev_tint = match lookdev_preset {
            LookDevPreset::Daylight => [1.0, 1.0, 1.0, 1.0],
            LookDevPreset::Night => [0.55, 0.70, 1.05, 0.45],
            LookDevPreset::Indoor => [1.05, 0.96, 0.86, 0.90],
            LookDevPreset::Sunset => [1.18, 0.78, 0.56, 0.82],
        };
        let uniform = ShadowMapUniform {
            light_view_proj: light_vp.to_cols_array_2d(),
            bias_params: [0.015, 0.0015, 0.55, if has_geo { 1.0 } else { 0.0 }],
            npr_params: [mode_id, 0.014, 40.0, 0.06],
            fog_color_density: [
                post_process.fog_color[0].clamp(0.0, 1.0),
                post_process.fog_color[1].clamp(0.0, 1.0),
                post_process.fog_color[2].clamp(0.0, 1.0),
                post_process.fog_density.clamp(0.0, 1.0),
            ],
            fog_params: [
                if post_process.fog_enabled { 1.0 } else { 0.0 },
                post_process.fog_start.clamp(0.0, 500.0),
                post_process.fog_height_falloff.clamp(0.0, 5.0),
                0.0,
            ],
            lookdev_tint,
        };
        queue.write_buffer(&self.shadow_uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tessel-dir-shadow-depth-pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        if !has_geo {
            return;
        }
        pass.set_pipeline(&self.shadow_depth_pipeline);
        pass.set_bind_group(0, &self.shadow_depth_bg, &[]);
        if let (Some(vb), Some(ib)) = (&self.vertex_buffer, &self.index_buffer) {
            if self.index_count > 0 {
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
        for draw in &self.textured_draws {
            if !draw.cast_shadow {
                continue;
            }
            pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
            pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw.index_count, 0, 0..1);
        }
    }

    /// Pass 2: Draws all opaque and alpha-mask 3D geometry plus NPR inverted-hull outlines.
    pub fn draw_opaque<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shading_mode: ViewportShadingMode,
    ) {
        // 1. Opaque & Alpha-Mask geometry pass
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.shadow_sample_bg, &[]);
        if let (Some(vb), Some(ib)) = (&self.vertex_buffer, &self.index_buffer) {
            if self.index_count > 0 {
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
        for draw in &self.textured_draws {
            if draw.is_glass || draw.alpha_blend {
                continue;
            }
            pass.set_bind_group(0, &draw.bind_group, &[]);
            pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
            pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw.index_count, 0, 0..1);
        }

        // 2. Inverted-hull ink outline pass when NPR Sumi-e mode is active
        if shading_mode == ViewportShadingMode::NprSumie {
            pass.set_pipeline(&self.outline_pipeline);
            pass.set_bind_group(1, &self.shadow_sample_bg, &[]);
            if let (Some(vb), Some(ib)) = (&self.vertex_buffer, &self.index_buffer) {
                if self.index_count > 0 {
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..self.index_count, 0, 0..1);
                }
            }
            for draw in &self.textured_draws {
                if draw.is_glass || draw.alpha_blend {
                    continue;
                }
                pass.set_bind_group(0, &draw.bind_group, &[]);
                pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
                pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..draw.index_count, 0, 0..1);
            }
        }
    }

    /// Pass 2.5: Draws all refractive glass (voxel & low-poly) and `AlphaMode::Blend` meshes
    /// sorted strictly back-to-front against `camera_pos`.
    pub fn draw_translucent<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shading_mode: ViewportShadingMode,
        camera_pos: glam::Vec3,
    ) {
        let mut items: Vec<(usize, [f32; 3])> = self
            .textured_draws
            .iter()
            .enumerate()
            .filter(|(_, d)| d.is_glass || d.alpha_blend)
            .map(|(i, d)| (i, d.centroid))
            .collect();
        if items.is_empty() {
            return;
        }
        let centroids: Vec<[f32; 3]> = items.iter().map(|(_, c)| *c).collect();
        let order = sort_back_to_front_indices(&centroids, camera_pos);
        for ord in order {
            let (draw_idx, _) = items[ord];
            let draw = &self.textured_draws[draw_idx];
            if draw.is_glass && shading_mode == ViewportShadingMode::Pbr {
                pass.set_pipeline(&self.glass_pipeline);
                pass.set_bind_group(1, &self.shadow_sample_bg, &[]);
                pass.set_bind_group(2, &self.glass_opaque_bg, &[]);
            } else {
                pass.set_pipeline(&self.blend_pipeline);
                pass.set_bind_group(1, &self.shadow_sample_bg, &[]);
            }
            pass.set_bind_group(0, &draw.bind_group, &[]);
            pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
            pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw.index_count, 0, 0..1);
        }
        items.clear();
    }
}

fn create_glass_opaque_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    opaque_copy_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tessel-glass-opaque-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(opaque_copy_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn compute_vertices_centroid(vertices: &[StudioVertex3D]) -> [f32; 3] {
    if vertices.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    let mut sum = glam::Vec3::ZERO;
    for v in vertices {
        sum += glam::Vec3::from_array(v.position);
    }
    (sum / (vertices.len() as f32)).to_array()
}

pub(crate) fn sort_back_to_front_indices(
    centroids: &[[f32; 3]],
    camera_pos: glam::Vec3,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..centroids.len()).collect();
    order.sort_by(|&a, &b| {
        let da = glam::Vec3::from_array(centroids[a]).distance_squared(camera_pos);
        let db = glam::Vec3::from_array(centroids[b]).distance_squared(camera_pos);
        db.total_cmp(&da).then_with(|| a.cmp(&b))
    });
    order
}

fn upload_optional_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    opt_tex: Option<&MeshTexture>,
) -> (Option<wgpu::Texture>, Option<wgpu::TextureView>) {
    let Some(tex) = opt_tex else {
        return (None, None);
    };
    let expected = (tex.width as usize)
        .saturating_mul(tex.height as usize)
        .saturating_mul(4);
    if tex.width == 0 || tex.height == 0 || tex.rgba.len() < expected {
        return (None, None);
    }
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: tex.width,
            height: tex.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &t,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &tex.rgba[..expected],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(tex.width * 4),
            rows_per_image: Some(tex.height),
        },
        wgpu::Extent3d {
            width: tex.width,
            height: tex.height,
            depth_or_array_layers: 1,
        },
    );
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (Some(t), Some(v))
}

pub(crate) fn f32_to_f16_bits(val: f32) -> u16 {
    if !val.is_finite() {
        return 0;
    }
    let clamped = val.clamp(-65504.0, 65504.0);
    let bits = clamped.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0 {
        return sign;
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 31 {
        return sign | 0x7bff;
    }
    if new_exp <= 0 {
        if new_exp < -10 {
            return sign;
        }
        let m = (mant | 0x0080_0000) >> (1 - new_exp + 13);
        return sign | (m as u16);
    }
    let rounded = (mant + 0x0000_1000) >> 13;
    if rounded >= 0x0400 {
        let e = (new_exp + 1).min(30) as u16;
        return sign | (e << 10);
    }
    sign | ((new_exp as u16) << 10) | (rounded as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_to_f16_bits_encodes_material_values_accurately() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(1.5), 0x3e00);
        assert_eq!(f32_to_f16_bits(20.0), 0x4d00);
        assert_eq!(f32_to_f16_bits(f32::NAN), 0x0000);
        assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x0000);
    }

    #[test]
    fn sort_back_to_front_orders_farthest_centroid_first() {
        let cam = glam::Vec3::new(0.0, 0.0, 10.0);
        let centroids = [
            [0.0, 0.0, 5.0],  // dist 5 (nearest)
            [0.0, 0.0, -4.0], // dist 14 (farthest)
            [0.0, 0.0, 1.0],  // dist 9 (middle)
        ];
        assert_eq!(sort_back_to_front_indices(&centroids, cam), vec![1, 2, 0]);
    }

    #[test]
    fn compute_light_view_proj_frames_scene_bounds_in_clip_space() {
        let vp = compute_light_view_proj(
            glam::Vec3::new(0.5, 0.8, 0.3),
            [-2.0, 0.0, -2.0],
            [2.0, 3.0, 2.0],
        );
        for x in [-2.0_f32, 2.0] {
            for y in [0.0_f32, 3.0] {
                for z in [-2.0_f32, 2.0] {
                    let clip = vp * glam::Vec4::new(x, y, z, 1.0);
                    let ndc = clip.truncate() / clip.w;
                    assert!(
                        ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z),
                        "corner ({x},{y},{z}) projected outside shadow frustum: {ndc:?}"
                    );
                }
            }
        }
    }
}
