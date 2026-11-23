use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use tessel_core::{AppMode, SceneState, VoxelCoord};
use wgpu::util::DeviceExt;

use crate::vertex::CameraUniform;

const OVERLAY_SHADER_WGSL: &str = include_str!("../../../assets/shaders/overlay.wgsl");

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct OverlayVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Renders 3D line overlays: voxel hover ghost box, low-poly wireframes, and character bones.
pub struct OverlayPass {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_count: u32,
}

impl OverlayPass {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER_WGSL.into()),
        });

        let initial_cam = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            inv_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 5.0, 5.0, 1.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-overlay-camera-uniform"),
            contents: bytemuck::bytes_of(&initial_cam),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-overlay-bgl"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tessel-overlay-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-overlay-pipeline-layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<OverlayVertex>() as wgpu::BufferAddress,
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
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-overlay-pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
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
            bind_group,
            vertex_buffer: None,
            vertex_count: 0,
        }
    }

    pub fn update_lines(
        &mut self,
        device: &wgpu::Device,
        scene: &SceneState,
        hover_voxel: Option<VoxelCoord>,
    ) {
        let mut lines = Vec::new();
        let amber = [0.90, 0.54, 0.24, 0.95];
        let bone_color = [1.0, 0.72, 0.50, 0.95];
        let wire_color = [0.85, 0.76, 0.70, 0.45];

        match scene.active_mode {
            AppMode::Voxel => {
                if let Some(c) = hover_voxel {
                    let s = scene.unit_scale;
                    let min = Vec3::new(c.x as f32 * s, c.y as f32 * s, c.z as f32 * s);
                    let max = min + Vec3::splat(s);
                    push_wire_box(&mut lines, min, max, amber);
                }
            }
            AppMode::LowPoly => {
                for mesh in scene.lowpoly_meshes.values() {
                    for (fi, face) in mesh.faces.iter().enumerate() {
                        let color = if scene.selected_face == Some(tessel_core::FaceId(fi)) {
                            amber
                        } else {
                            wire_color
                        };
                        let n = face.vertices.len();
                        for i in 0..n {
                            let a = mesh.vertices.get(face.vertices[i]);
                            let b = mesh.vertices.get(face.vertices[(i + 1) % n]);
                            if let (Some(&pa), Some(&pb)) = (a, b) {
                                push_line(&mut lines, pa, pb, color);
                            }
                        }
                    }
                }
            }
            AppMode::Character => {
                if let Some(rig) = &scene.character_rig {
                    let mats = rig.world_pose_matrices();
                    for bone in &rig.bones {
                        let Some(&mat) = mats.get(&bone.id) else {
                            continue;
                        };
                        let joint = mat.transform_point3(Vec3::ZERO);
                        let is_selected = scene.selected_bone == Some(bone.id);
                        let r = if is_selected { 0.08 } else { 0.04 };
                        let joint_color = if is_selected {
                            [1.0, 0.95, 0.70, 1.0]
                        } else {
                            amber
                        };
                        push_line(
                            &mut lines,
                            joint - Vec3::X * r,
                            joint + Vec3::X * r,
                            joint_color,
                        );
                        push_line(
                            &mut lines,
                            joint - Vec3::Y * r,
                            joint + Vec3::Y * r,
                            joint_color,
                        );
                        push_line(
                            &mut lines,
                            joint - Vec3::Z * r,
                            joint + Vec3::Z * r,
                            joint_color,
                        );
                        if let Some(pid) = bone.parent {
                            if let Some(&pmat) = mats.get(&pid) {
                                let parent_pos = pmat.transform_point3(Vec3::ZERO);
                                push_line(&mut lines, parent_pos, joint, bone_color);
                            }
                        }
                    }
                }
            }
            AppMode::Pixel => {}
        }

        if lines.is_empty() {
            self.vertex_buffer = None;
            self.vertex_count = 0;
            return;
        }
        self.vertex_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tessel-overlay-vb"),
                contents: bytemuck::cast_slice(&lines),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.vertex_count = lines.len() as u32;
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, uniform: &CameraUniform) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(uniform));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some(vb) = &self.vertex_buffer else {
            return;
        };
        if self.vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}

fn push_line(out: &mut Vec<OverlayVertex>, a: Vec3, b: Vec3, color: [f32; 4]) {
    out.push(OverlayVertex {
        position: a.to_array(),
        color,
    });
    out.push(OverlayVertex {
        position: b.to_array(),
        color,
    });
}

fn push_wire_box(out: &mut Vec<OverlayVertex>, min: Vec3, max: Vec3, color: [f32; 4]) {
    let c = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (i, j) in edges {
        push_line(out, c[i], c[j], color);
    }
}
