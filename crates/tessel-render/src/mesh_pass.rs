use tessel_core::{MeshMaterial, MeshTexture, Palette, PaletteIndex};
use wgpu::util::DeviceExt;

use crate::vertex::{CameraUniform, MaterialUniform, StudioVertex3D};

const MESH_SHADER_WGSL: &str = include_str!("../../../assets/shaders/mesh_3d.wgsl");

pub struct CustomMeshUpload<'a> {
    pub texture: Option<&'a MeshTexture>,
    pub normal_map: Option<&'a MeshTexture>,
    pub material: MeshMaterial,
    pub vertices: Vec<StudioVertex3D>,
    pub indices: Vec<u32>,
}

struct TexturedMeshDraw {
    _texture: Option<wgpu::Texture>,
    _normal_texture: Option<wgpu::Texture>,
    _material_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

/// Renders shaded 3D Voxel & Low-Poly meshes using a 256x1 palette texture (or per-mesh RGBA texture)
/// with per-mesh PBR material properties, tangent-space normal mapping, and depth testing.
pub struct MeshPass {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    _default_material_buffer: wgpu::Buffer,
    palette_texture: wgpu::Texture,
    palette_view: wgpu::TextureView,
    _default_normal_texture: wgpu::Texture,
    default_normal_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    index_count: u32,
    textured_draws: Vec<TexturedMeshDraw>,
}

impl MeshPass {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-mesh-pipeline-layout"),
            bind_group_layouts: &[&bgl],
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bgl,
            camera_buffer,
            _default_material_buffer: default_material_buffer,
            palette_texture,
            palette_view,
            _default_normal_texture: default_normal_texture,
            default_normal_view,
            sampler,
            bind_group,
            vertex_buffer: None,
            index_buffer: None,
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
        let mut rgba = vec![255u8; 256 * 4];
        for raw in 1u8..=255 {
            if let Ok(idx) = PaletteIndex::new(raw) {
                if let Some(entry) = palette.get(idx) {
                    let base = usize::from(raw) * 4;
                    rgba[base] = entry.color.r;
                    rgba[base + 1] = entry.color.g;
                    rgba[base + 2] = entry.color.b;
                    rgba[base + 3] = entry.color.a;
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
    }

    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        vertices: &[StudioVertex3D],
        indices: &[u32],
    ) {
        if vertices.is_empty() || indices.is_empty() {
            self.vertex_buffer = None;
            self.index_buffer = None;
            self.index_count = 0;
            return;
        }
        self.vertex_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-mesh-vb"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.index_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-mesh-ib"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );
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
            let mat_uniform = MaterialUniform {
                pbr: [mat.metallic, mat.roughness, 0.0, 0.0],
                emissive: [mat.emissive[0], mat.emissive[1], mat.emissive[2], 0.0],
            };
            let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-custom-material-uniform"),
                contents: bytemuck::bytes_of(&mat_uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let (gpu_tex, custom_view) = if let Some(tex) = opt_tex {
                let expected = (tex.width as usize)
                    .saturating_mul(tex.height as usize)
                    .saturating_mul(4);
                if tex.width == 0 || tex.height == 0 || tex.rgba.len() < expected {
                    (None, None)
                } else {
                    let t = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("tessel-lowpoly-rgba-tex"),
                        size: wgpu::Extent3d {
                            width: tex.width,
                            height: tex.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            } else {
                (None, None)
            };

            let (gpu_norm_tex, custom_norm_view) = if let Some(nmap) = item.normal_map {
                let expected = (nmap.width as usize)
                    .saturating_mul(nmap.height as usize)
                    .saturating_mul(4);
                if nmap.width == 0 || nmap.height == 0 || nmap.rgba.len() < expected {
                    (None, None)
                } else {
                    let t = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("tessel-lowpoly-normal-tex"),
                        size: wgpu::Extent3d {
                            width: nmap.width,
                            height: nmap.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
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
                        &nmap.rgba[..expected],
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(nmap.width * 4),
                            rows_per_image: Some(nmap.height),
                        },
                        wgpu::Extent3d {
                            width: nmap.width,
                            height: nmap.height,
                            depth_or_array_layers: 1,
                        },
                    );
                    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
                    (Some(t), Some(v))
                }
            } else {
                (None, None)
            };

            let tex_view_ref = custom_view.as_ref().unwrap_or(&self.palette_view);
            let norm_view_ref = custom_norm_view
                .as_ref()
                .unwrap_or(&self.default_normal_view);
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
            self.textured_draws.push(TexturedMeshDraw {
                _texture: gpu_tex,
                _normal_texture: gpu_norm_tex,
                _material_buffer: material_buffer,
                bind_group,
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
            });
        }
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, uniform: &CameraUniform) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(uniform));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        if let (Some(vb), Some(ib)) = (&self.vertex_buffer, &self.index_buffer) {
            if self.index_count > 0 {
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
        for draw in &self.textured_draws {
            pass.set_bind_group(0, &draw.bind_group, &[]);
            pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
            pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw.index_count, 0, 0..1);
        }
    }
}
