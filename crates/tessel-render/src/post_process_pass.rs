use bytemuck::{Pod, Zeroable};
use tessel_core::{PostProcessSettings, ViewportShadingMode};
use wgpu::util::DeviceExt;

const POST_COMPOSITE_WGSL: &str = include_str!("../../../assets/shaders/post_composite.wgsl");

/// GPU uniform for Pass 4 HDR composite (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PostProcessUniform {
    /// `[exposure, bloom_intensity, aces_enabled, bypass_flag]`
    pub tone: [f32; 4],
}

impl Default for PostProcessUniform {
    fn default() -> Self {
        Self {
            tone: [1.0, 0.0, 1.0, 0.0],
        }
    }
}

/// Pass 4: Composites the linear `Rgba16Float` HDR viewport target (plus optional bloom)
/// with exposure and ACES Filmic tone mapping into the final `Rgba8UnormSrgb` viewport target.
pub struct PostProcessPass {
    composite_pipeline: wgpu::RenderPipeline,
    composite_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    linear_sampler: wgpu::Sampler,
    _black_bloom_texture: wgpu::Texture,
    black_bloom_view: wgpu::TextureView,
    composite_bg: wgpu::BindGroup,
}

impl PostProcessPass {
    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        hdr_view: &wgpu::TextureView,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-post-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(POST_COMPOSITE_WGSL.into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-post-composite-uniform"),
            contents: bytemuck::bytes_of(&PostProcessUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tessel-post-linear-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Newly created WebGPU textures are guaranteed zero-initialized (0, 0, 0, 0).
        let black_bloom_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tessel-post-black-bloom-1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let black_bloom_view =
            black_bloom_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-post-composite-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let composite_bg = create_composite_bind_group(
            device,
            &composite_bgl,
            &uniform_buffer,
            hdr_view,
            &black_bloom_view,
            &linear_sampler,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-post-composite-pipeline-layout"),
            bind_group_layouts: &[&composite_bgl],
            push_constant_ranges: &[],
        });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-post-composite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            composite_pipeline,
            composite_bgl,
            uniform_buffer,
            linear_sampler,
            _black_bloom_texture: black_bloom_texture,
            black_bloom_view,
            composite_bg,
        }
    }

    /// Rebinds the composite pass whenever the HDR offscreen texture is resized.
    pub fn rebind_hdr(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.composite_bg = create_composite_bind_group(
            device,
            &self.composite_bgl,
            &self.uniform_buffer,
            hdr_view,
            &self.black_bloom_view,
            &self.linear_sampler,
        );
    }

    pub fn update_params(
        &self,
        queue: &wgpu::Queue,
        settings: &PostProcessSettings,
        shading_mode: ViewportShadingMode,
    ) {
        let bypass = shading_mode != ViewportShadingMode::Pbr;
        let bloom_intensity = if !bypass && settings.bloom_enabled {
            settings.bloom_intensity.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let uniform = PostProcessUniform {
            tone: [
                settings.exposure.clamp(0.05, 8.0),
                bloom_intensity,
                if settings.aces_enabled { 1.0 } else { 0.0 },
                if bypass { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw_composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output_view: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tessel-post-composite-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &self.composite_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    hdr_view: &wgpu::TextureView,
    bloom_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tessel-post-composite-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(hdr_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(bloom_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
