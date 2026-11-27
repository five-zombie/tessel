use bytemuck::{Pod, Zeroable};
use tessel_core::{PostProcessSettings, ViewportShadingMode};
use wgpu::util::DeviceExt;

const POST_COMPOSITE_WGSL: &str = include_str!("../../../assets/shaders/post_composite.wgsl");
const BLOOM_DOWNSAMPLE_WGSL: &str = include_str!("../../../assets/shaders/bloom_downsample.wgsl");
const BLOOM_UPSAMPLE_WGSL: &str = include_str!("../../../assets/shaders/bloom_upsample.wgsl");

const BLOOM_MIP_COUNT: usize = 5;
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

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

/// GPU uniform for each Dual-Kawase bloom downsample/upsample step (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct BloomStepUniform {
    params: [f32; 4],
}

struct BloomMip {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// Pass 3 & 4: Dual-Kawase 5-level HDR Bloom + Exposure & ACES Filmic Composite into `Rgba8UnormSrgb`.
pub struct PostProcessPass {
    hdr_size: (u32, u32),
    composite_pipeline: wgpu::RenderPipeline,
    composite_bgl: wgpu::BindGroupLayout,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    bloom_step_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    downsample_uniforms: Vec<wgpu::Buffer>,
    upsample_uniforms: Vec<wgpu::Buffer>,
    linear_sampler: wgpu::Sampler,
    bloom_mips: Vec<BloomMip>,
    downsample_bgs: Vec<wgpu::BindGroup>,
    upsample_bgs: Vec<wgpu::BindGroup>,
    composite_bg: wgpu::BindGroup,
}

impl PostProcessPass {
    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        hdr_size: (u32, u32),
        hdr_view: &wgpu::TextureView,
    ) -> Self {
        let safe_size = (hdr_size.0.max(1), hdr_size.1.max(1));

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-post-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(POST_COMPOSITE_WGSL.into()),
        });
        let downsample_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-bloom-downsample-shader"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_DOWNSAMPLE_WGSL.into()),
        });
        let upsample_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-bloom-upsample-shader"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_UPSAMPLE_WGSL.into()),
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

        let bloom_step_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-bloom-step-bgl"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tessel-bloom-pipeline-layout"),
                bind_group_layouts: &[&bloom_step_bgl],
                push_constant_ranges: &[],
            });

        let downsample_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-bloom-downsample-pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &downsample_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &downsample_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: BLOOM_FORMAT,
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

        let additive_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let upsample_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-bloom-upsample-pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &upsample_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &upsample_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: BLOOM_FORMAT,
                    blend: Some(additive_blend),
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

        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tessel-post-composite-pipeline-layout"),
                bind_group_layouts: &[&composite_bgl],
                push_constant_ranges: &[],
            });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-post-composite-pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
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

        let zero_step = BloomStepUniform { params: [0.0; 4] };
        let downsample_uniforms: Vec<wgpu::Buffer> = (0..BLOOM_MIP_COUNT)
            .map(|i| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("tessel-bloom-down-uniform-{i}")),
                    contents: bytemuck::bytes_of(&zero_step),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            })
            .collect();
        let upsample_uniforms: Vec<wgpu::Buffer> = (0..(BLOOM_MIP_COUNT - 1))
            .map(|i| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("tessel-bloom-up-uniform-{i}")),
                    contents: bytemuck::bytes_of(&zero_step),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            })
            .collect();

        let bloom_mips = create_bloom_mip_chain(device, safe_size.0, safe_size.1);
        let (downsample_bgs, upsample_bgs, composite_bg) = build_post_bind_groups(
            device,
            &bloom_step_bgl,
            &composite_bgl,
            &uniform_buffer,
            &downsample_uniforms,
            &upsample_uniforms,
            hdr_view,
            &bloom_mips,
            &linear_sampler,
        );

        Self {
            hdr_size: safe_size,
            composite_pipeline,
            composite_bgl,
            downsample_pipeline,
            upsample_pipeline,
            bloom_step_bgl,
            uniform_buffer,
            downsample_uniforms,
            upsample_uniforms,
            linear_sampler,
            bloom_mips,
            downsample_bgs,
            upsample_bgs,
            composite_bg,
        }
    }

    /// Reallocates the 5-level Dual-Kawase bloom mip chain and rebinds HDR/composite passes on resize.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        hdr_size: (u32, u32),
        hdr_view: &wgpu::TextureView,
    ) {
        let safe_size = (hdr_size.0.max(1), hdr_size.1.max(1));
        self.hdr_size = safe_size;
        self.bloom_mips = create_bloom_mip_chain(device, safe_size.0, safe_size.1);
        let (down_bgs, up_bgs, comp_bg) = build_post_bind_groups(
            device,
            &self.bloom_step_bgl,
            &self.composite_bgl,
            &self.uniform_buffer,
            &self.downsample_uniforms,
            &self.upsample_uniforms,
            hdr_view,
            &self.bloom_mips,
            &self.linear_sampler,
        );
        self.downsample_bgs = down_bgs;
        self.upsample_bgs = up_bgs;
        self.composite_bg = comp_bg;
    }

    /// Executes Pass 3 (Dual-Kawase Bloom when active in PBR mode) and Pass 4 (HDR -> LDR Composite).
    pub fn execute(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        settings: &PostProcessSettings,
        shading_mode: ViewportShadingMode,
        output_view: &wgpu::TextureView,
    ) {
        let bypass = shading_mode != ViewportShadingMode::Pbr;
        let run_bloom = !bypass && settings.bloom_enabled && settings.bloom_intensity > 0.0;
        let bloom_intensity = if run_bloom {
            settings.bloom_intensity.clamp(0.0, 4.0)
        } else {
            0.0
        };

        if run_bloom {
            let threshold = settings.bloom_threshold.clamp(0.0, 10.0);
            // Pass 3a: Prefilter + 5-level downsample chain (HDR -> mip 0 -> ... -> mip 4)
            for i in 0..BLOOM_MIP_COUNT {
                let src_size = if i == 0 {
                    self.hdr_size
                } else {
                    self.bloom_mips[i - 1].size
                };
                let step = BloomStepUniform {
                    params: [
                        1.0 / src_size.0.max(1) as f32,
                        1.0 / src_size.1.max(1) as f32,
                        threshold,
                        if i == 0 { 1.0 } else { 0.0 },
                    ],
                };
                queue.write_buffer(&self.downsample_uniforms[i], 0, bytemuck::bytes_of(&step));

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tessel-bloom-downsample-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_mips[i].view,
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
                pass.set_pipeline(&self.downsample_pipeline);
                pass.set_bind_group(0, &self.downsample_bgs[i], &[]);
                pass.draw(0..3, 0..1);
            }

            // Pass 3b: 4-level additive 9-tap tent upsample (mip 4 -> mip 3 -> ... -> mip 0)
            for target_idx in (0..(BLOOM_MIP_COUNT - 1)).rev() {
                let src_size = self.bloom_mips[target_idx + 1].size;
                let step = BloomStepUniform {
                    params: [
                        1.0 / src_size.0.max(1) as f32,
                        1.0 / src_size.1.max(1) as f32,
                        1.0,
                        0.0,
                    ],
                };
                queue.write_buffer(
                    &self.upsample_uniforms[target_idx],
                    0,
                    bytemuck::bytes_of(&step),
                );

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tessel-bloom-upsample-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_mips[target_idx].view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.upsample_pipeline);
                pass.set_bind_group(0, &self.upsample_bgs[target_idx], &[]);
                pass.draw(0..3, 0..1);
            }
        }

        // Pass 4: Final HDR + Bloom -> Exposure + ACES -> LDR output_view
        let uniform = PostProcessUniform {
            tone: [
                settings.exposure.clamp(0.05, 8.0),
                bloom_intensity,
                if settings.aces_enabled { 1.0 } else { 0.0 },
                if bypass { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

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

fn create_bloom_mip_chain(device: &wgpu::Device, width: u32, height: u32) -> Vec<BloomMip> {
    (0..BLOOM_MIP_COUNT)
        .map(|i| {
            let w = (width >> (i + 1)).max(1);
            let h = (height >> (i + 1)).max(1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("tessel-bloom-mip-{i}")),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BLOOM_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            BloomMip {
                _texture: texture,
                view,
                size: (w, h),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_post_bind_groups(
    device: &wgpu::Device,
    bloom_step_bgl: &wgpu::BindGroupLayout,
    composite_bgl: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    downsample_uniforms: &[wgpu::Buffer],
    upsample_uniforms: &[wgpu::Buffer],
    hdr_view: &wgpu::TextureView,
    bloom_mips: &[BloomMip],
    sampler: &wgpu::Sampler,
) -> (Vec<wgpu::BindGroup>, Vec<wgpu::BindGroup>, wgpu::BindGroup) {
    let downsample_bgs = (0..BLOOM_MIP_COUNT)
        .map(|i| {
            let src_view = if i == 0 {
                hdr_view
            } else {
                &bloom_mips[i - 1].view
            };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("tessel-bloom-down-bg-{i}")),
                layout: bloom_step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: downsample_uniforms[i].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        })
        .collect();

    let upsample_bgs = (0..(BLOOM_MIP_COUNT - 1))
        .map(|target_idx| {
            let src_view = &bloom_mips[target_idx + 1].view;
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("tessel-bloom-up-bg-{target_idx}")),
                layout: bloom_step_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: upsample_uniforms[target_idx].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        })
        .collect();

    let composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tessel-post-composite-bg"),
        layout: composite_bgl,
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
                resource: wgpu::BindingResource::TextureView(&bloom_mips[0].view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    (downsample_bgs, upsample_bgs, composite_bg)
}
