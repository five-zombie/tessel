use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use tessel_core::{AppMode, SceneState, VoxelCoord};
use wgpu::util::DeviceExt;

use crate::camera::{GizmoAxis, TransformGizmoMode};
use crate::vertex::CameraUniform;

pub const GIZMO_AXIS_LEN: f32 = 0.85;

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
    vertex_capacity: usize,
    vertex_count: u32,
}

/// Wireframe ghost preview for interactive object placement on a grid or surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementGhost {
    pub position: [f32; 3],
    pub half_extents: [f32; 3],
}

impl OverlayPass {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER_WGSL.into()),
        });

        let initial_cam = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            inv_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0, 5.0, 5.0, 1.0],
            light_dir: [0.55, 0.82, 0.45, 0.0],
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
            camera_buffer,
            bind_group,
            vertex_buffer: None,
            vertex_capacity: 0,
            vertex_count: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_lines(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &SceneState,
        hover_voxel: Option<VoxelCoord>,
        active_gizmo_axis: Option<GizmoAxis>,
        gizmo_mode: TransformGizmoMode,
        show_gizmo: bool,
        placement_ghost: Option<PlacementGhost>,
    ) {
        let mut lines = Vec::new();
        let amber = [0.90, 0.54, 0.24, 0.95];
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
                let skin_solver = scene
                    .character_rig
                    .as_ref()
                    .and_then(|r| tessel_core::AutoSkinSolver::from_rig(r, 1.0));
                let skin_mats = skin_solver
                    .as_ref()
                    .zip(scene.character_rig.as_ref())
                    .map(|(solver, rig)| solver.skin_matrices(rig));
                let rig_pose_mats = scene
                    .character_rig
                    .as_ref()
                    .filter(|r| !r.node_parent_overrides.is_empty())
                    .map(|r| (r.world_pose_matrices(), r.world_rest_matrices()));

                for (node_id, mesh) in &scene.lowpoly_meshes {
                    let visible = scene
                        .character_rig
                        .as_ref()
                        .and_then(|r| r.node_visibility.get(node_id).copied())
                        .unwrap_or_else(|| {
                            scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true)
                        });
                    if !visible || mesh.faces.is_empty() {
                        continue;
                    }
                    let base_node_mat = scene.node_world_matrix(*node_id);
                    let parent_override = scene
                        .character_rig
                        .as_ref()
                        .and_then(|r| r.node_parent_overrides.get(node_id).copied());
                    let node_mat = match (parent_override, &rig_pose_mats) {
                        (Some(ps_key), Some((world_mats, rest_mats))) => {
                            tessel_core::CharacterRig::compute_node_override_matrix(
                                ps_key,
                                base_node_mat,
                                world_mats,
                                rest_mats,
                            )
                        }
                        _ => base_node_mat,
                    };
                    let skip_skin = parent_override.is_some()
                        || !scene
                            .character_rig
                            .as_ref()
                            .is_some_and(|r| r.should_skin_node(*node_id));
                    let rest_node_mat = scene.node_rest_world_matrix(*node_id);

                    let mut world_verts: Vec<Vec3> = Vec::with_capacity(mesh.vertices.len());
                    for (vi, &v) in mesh.vertices.iter().enumerate() {
                        let mut lv = v;
                        for morph in &mesh.morphs {
                            let w = scene
                                .character_rig
                                .as_ref()
                                .and_then(|rig| {
                                    rig.morph_weights
                                        .get(&(*node_id, morph.name.clone()))
                                        .copied()
                                })
                                .unwrap_or(morph.default_weight);
                            if w.abs() > 1e-5 {
                                if let Some(&delta) = morph.deltas.get(&vi) {
                                    lv += delta * w;
                                }
                            }
                        }
                        let mut wp = node_mat.transform_point3(lv);
                        if !skip_skin {
                            if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                                let w = mesh.skin_weights.get(&vi).copied().unwrap_or_else(|| {
                                    let rest_wp = rest_node_mat.transform_point3(v);
                                    solver.weights_for_vertex(Some(*node_id), rest_wp)
                                });
                                let (sp, _) = solver.deform_point_normal(mats, w, wp, Vec3::Y);
                                wp = sp;
                            }
                        }
                        world_verts.push(wp);
                    }

                    for (fi, face) in mesh.faces.iter().enumerate() {
                        let color = if scene.selected_node == Some(*node_id)
                            && scene.selected_face == Some(tessel_core::FaceId(fi))
                        {
                            amber
                        } else {
                            wire_color
                        };
                        let n = face.vertices.len();
                        for i in 0..n {
                            let a = world_verts.get(face.vertices[i]);
                            let b = world_verts.get(face.vertices[(i + 1) % n]);
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
                        let side_color = if bone.name.starts_with("Left")
                            || bone.name.contains("_Left")
                            || bone.name.ends_with("_L")
                        {
                            [0.35, 0.72, 1.0, 1.0]
                        } else if bone.name.starts_with("Right")
                            || bone.name.contains("_Right")
                            || bone.name.ends_with("_R")
                        {
                            [1.0, 0.42, 0.42, 1.0]
                        } else {
                            amber
                        };
                        let r = if is_selected { 0.08 } else { 0.04 };
                        let joint_color = if is_selected {
                            [1.0, 0.95, 0.70, 1.0]
                        } else {
                            side_color
                        };
                        if is_selected {
                            let axis_len = 0.16;
                            let rx = mat.transform_vector3(Vec3::X).normalize_or_zero() * axis_len;
                            let ry = mat.transform_vector3(Vec3::Y).normalize_or_zero() * axis_len;
                            let rz = mat.transform_vector3(Vec3::Z).normalize_or_zero() * axis_len;
                            push_line(&mut lines, joint, joint + rx, [1.0, 0.25, 0.25, 1.0]);
                            push_line(&mut lines, joint, joint + ry, [0.25, 1.0, 0.35, 1.0]);
                            push_line(&mut lines, joint, joint + rz, [0.30, 0.60, 1.0, 1.0]);
                        }
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
                                if scene.stick_gizmo.enabled {
                                    let stick_color = if is_selected {
                                        [1.0, 0.92, 0.45, 1.0]
                                    } else {
                                        side_color
                                    };
                                    push_octahedral_stick(
                                        &mut lines,
                                        parent_pos,
                                        joint,
                                        stick_color,
                                        is_selected,
                                    );
                                } else {
                                    push_line(&mut lines, parent_pos, joint, side_color);
                                }
                            }
                        }
                    }
                }
            }
            AppMode::Pixel => {}
        }

        if scene.active_mode != AppMode::Pixel {
            if let Some(ghost) = placement_ghost {
                let p = Vec3::from_array(ghost.position);
                let he = Vec3::from_array(ghost.half_extents).max(Vec3::splat(0.05));
                let ghost_color = [0.28, 0.90, 0.96, 0.95];
                let min = Vec3::new(p.x - he.x, p.y, p.z - he.z);
                let max = Vec3::new(p.x + he.x, p.y + he.y * 2.0, p.z + he.z);
                push_wire_box(&mut lines, min, max, ghost_color);
                let cross = he.x.max(he.z) * 0.6;
                push_line(
                    &mut lines,
                    p - Vec3::X * cross,
                    p + Vec3::X * cross,
                    ghost_color,
                );
                push_line(
                    &mut lines,
                    p - Vec3::Z * cross,
                    p + Vec3::Z * cross,
                    ghost_color,
                );
            }
            if show_gizmo {
                if let Some(selected_id) = scene.selected_node {
                    let rig_visible = scene
                        .character_rig
                        .as_ref()
                        .and_then(|r| r.node_visibility.get(&selected_id).copied())
                        .unwrap_or(true);
                    if rig_visible && scene.nodes.get(&selected_id).is_some_and(|n| n.visible) {
                        let world_origin = scene
                            .node_world_matrix(selected_id)
                            .transform_point3(Vec3::ZERO);
                        match gizmo_mode {
                            TransformGizmoMode::Move => {
                                push_move_gizmo(&mut lines, world_origin, active_gizmo_axis);
                            }
                            TransformGizmoMode::Rotate => {
                                // Rendered in screen-space as a floating circular trackball disc in tessel-app.
                            }
                            TransformGizmoMode::Scale => {
                                let world_rot = node_world_rotation(scene, selected_id);
                                push_scale_gizmo(
                                    &mut lines,
                                    world_origin,
                                    world_rot,
                                    active_gizmo_axis,
                                );
                            }
                        }
                    }
                }
            }
        }

        if lines.is_empty() {
            self.vertex_count = 0;
            return;
        }
        if lines.len() > self.vertex_capacity || self.vertex_buffer.is_none() {
            let new_cap = lines.len().next_power_of_two().max(256);
            let byte_size = (new_cap * std::mem::size_of::<OverlayVertex>()) as wgpu::BufferAddress;
            self.vertex_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tessel-overlay-vb"),
                size: byte_size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.vertex_capacity = new_cap;
        }
        if let Some(vb) = &self.vertex_buffer {
            queue.write_buffer(vb, 0, bytemuck::cast_slice(&lines));
        }
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

fn push_octahedral_stick(
    out: &mut Vec<OverlayVertex>,
    base: Vec3,
    tip: Vec3,
    color: [f32; 4],
    selected: bool,
) {
    let dir = tip - base;
    let len = dir.length();
    if len < 1e-5 {
        return;
    }
    let axis = dir / len;
    let ref_up = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = axis.cross(ref_up).normalize_or_zero();
    let v = axis.cross(u).normalize_or_zero();
    let shoulder = base + axis * (len * 0.2);
    let radius = (len * 0.14).clamp(0.015, 0.08);
    let corners = [
        shoulder + u * radius,
        shoulder + v * radius,
        shoulder - u * radius,
        shoulder - v * radius,
    ];
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        push_line(out, base, a, color);
        push_line(out, a, tip, color);
        push_line(out, a, b, color);
    }
    if selected {
        // Explicit orientation roll/twist fin ring at shoulder for active stick handle
        let ring_color = [0.35, 0.92, 1.0, 0.95];
        let fin_r = radius * 1.65;
        let segs = 12usize;
        for i in 0..segs {
            let a0 = (i as f32 / segs as f32) * std::f32::consts::TAU;
            let a1 = ((i + 1) as f32 / segs as f32) * std::f32::consts::TAU;
            let p0 = shoulder + (u * a0.cos() + v * a0.sin()) * fin_r;
            let p1 = shoulder + (u * a1.cos() + v * a1.sin()) * fin_r;
            push_line(out, p0, p1, ring_color);
        }
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

fn push_move_gizmo(out: &mut Vec<OverlayVertex>, origin: Vec3, active: Option<GizmoAxis>) {
    let axes = [
        (GizmoAxis::X, Vec3::X, [0.95, 0.30, 0.30, 1.0]),
        (GizmoAxis::Y, Vec3::Y, [0.35, 0.88, 0.42, 1.0]),
        (GizmoAxis::Z, Vec3::Z, [0.32, 0.58, 0.96, 1.0]),
    ];
    let highlight = [1.0, 0.92, 0.40, 1.0];

    for (axis, dir, base_color) in axes {
        let color = if active == Some(axis) {
            highlight
        } else {
            base_color
        };
        let tip = origin + dir * GIZMO_AXIS_LEN;
        push_line(out, origin, tip, color);

        // Simple 3D arrowhead fins
        let ortho_a = if dir.y.abs() < 0.9 {
            dir.cross(Vec3::Y).normalize()
        } else {
            dir.cross(Vec3::X).normalize()
        };
        let ortho_b = dir.cross(ortho_a).normalize();
        let back = tip - dir * 0.12;
        let spread = 0.045;
        push_line(out, tip, back + ortho_a * spread, color);
        push_line(out, tip, back - ortho_a * spread, color);
        push_line(out, tip, back + ortho_b * spread, color);
        push_line(out, tip, back - ortho_b * spread, color);
    }
}

fn node_world_rotation(scene: &SceneState, node_id: tessel_core::NodeId) -> glam::Quat {
    let mut acc = glam::Quat::IDENTITY;
    let mut cur = Some(node_id);
    let mut depth = 0usize;
    while let Some(id) = cur {
        if depth >= 64 {
            break;
        }
        let Some(node) = scene.nodes.get(&id) else {
            break;
        };
        let q = if node.transform.rotation.length_squared().is_finite()
            && node.transform.rotation.length_squared() > 1e-8
        {
            node.transform.rotation.normalize()
        } else {
            glam::Quat::IDENTITY
        };
        acc = (q * acc).normalize();
        cur = node.parent;
        depth += 1;
    }
    acc
}

fn push_scale_gizmo(
    out: &mut Vec<OverlayVertex>,
    origin: Vec3,
    world_rot: glam::Quat,
    active: Option<GizmoAxis>,
) {
    let axes = [
        (GizmoAxis::X, world_rot * Vec3::X, [0.95, 0.30, 0.30, 1.0]),
        (GizmoAxis::Y, world_rot * Vec3::Y, [0.35, 0.88, 0.42, 1.0]),
        (GizmoAxis::Z, world_rot * Vec3::Z, [0.32, 0.58, 0.96, 1.0]),
    ];
    let highlight = [1.0, 0.92, 0.40, 1.0];
    let half = Vec3::splat(0.055);

    for (axis, dir, base_color) in axes {
        let color = if active == Some(axis) {
            highlight
        } else {
            base_color
        };
        let tip = origin + dir * GIZMO_AXIS_LEN;
        push_line(out, origin, tip, color);
        push_wire_box(out, tip - half, tip + half, color);
    }
}
