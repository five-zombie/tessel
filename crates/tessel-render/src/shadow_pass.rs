use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::vertex::CameraUniform;

const GROUND_SHADOW_SHADER_WGSL: &str = include_str!("../../../assets/shaders/ground_shadow.wgsl");

/// GPU uniform describing the asset's world-space footprint for soft ground shadow rendering.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ShadowUniform {
    pub center_enabled: [f32; 4],
    pub extents_height_opacity: [f32; 4],
}

impl ShadowUniform {
    pub const DISABLED: Self = Self {
        center_enabled: [0.0, 0.0, 0.0, 0.0],
        extents_height_opacity: [0.5, 0.5, 1.0, 0.85],
    };

    /// Computes a soft ground shadow footprint from an axis-aligned world bounding box.
    pub fn from_bounds(min: [f32; 3], max: [f32; 3]) -> Self {
        if !min.iter().all(|v| v.is_finite())
            || !max.iter().all(|v| v.is_finite())
            || min[0] > max[0]
            || min[1] > max[1]
            || min[2] > max[2]
        {
            return Self::DISABLED;
        }
        let cx = (min[0] + max[0]) * 0.5;
        let cz = (min[2] + max[2]) * 0.5;
        let ground_y = min[1];
        let rx = ((max[0] - min[0]) * 0.5).max(0.25);
        let rz = ((max[2] - min[2]) * 0.5).max(0.25);
        let height = (max[1] - min[1]).max(0.25);
        Self {
            center_enabled: [cx, ground_y, cz, 1.0],
            extents_height_opacity: [rx, rz, height, 0.88],
        }
    }
}

/// Renders a soft studio floor pool and directional contact shadow in Render Preview mode.
pub struct ShadowPass {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    shadow_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-ground-shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(GROUND_SHADOW_SHADER_WGSL.into()),
        });

        let initial_cam = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            inv_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 5.0, 5.0, 1.0],
            light_dir: [0.55, 0.82, 0.45, 0.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-shadow-camera-uniform"),
            contents: bytemuck::bytes_of(&initial_cam),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shadow_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-shadow-bounds-uniform"),
            contents: bytemuck::bytes_of(&ShadowUniform::DISABLED),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-ground-shadow-bgl"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tessel-ground-shadow-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadow_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-ground-shadow-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-ground-shadow-pipeline"),
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
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_buffer,
            shadow_buffer,
            bind_group,
        }
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, uniform: &CameraUniform) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(uniform));
    }

    pub fn update_bounds(&self, queue: &wgpu::Queue, uniform: &ShadowUniform) {
        queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(uniform));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_uniform_from_bounds_computes_footprint_and_handles_empty() {
        let disabled = ShadowUniform::from_bounds([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        assert_eq!(disabled.center_enabled[3], 0.0);

        let active = ShadowUniform::from_bounds([-1.0, 0.0, -0.5], [1.0, 2.0, 0.5]);
        assert_eq!(active.center_enabled, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(active.extents_height_opacity[0], 1.0);
        assert_eq!(active.extents_height_opacity[1], 0.5);
        assert_eq!(active.extents_height_opacity[2], 2.0);
    }
}
