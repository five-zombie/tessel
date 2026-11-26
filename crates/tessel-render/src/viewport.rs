use std::io::Cursor;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use image::{ImageFormat, RgbaImage};
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AppMode, SceneState, VoxelCoord};

use crate::camera::{GizmoAxis, OrbitCamera};
use crate::error::RenderError;
use crate::grid_pass::GridPass;
use crate::mesh_pass::MeshPass;
use crate::overlay_pass::OverlayPass;
use crate::pixel_pass::{PixelCamera2D, PixelPass};
use crate::shadow_pass::{ShadowPass, ShadowUniform};
use crate::vertex::StudioVertex3D;

/// Clear color matching `surface-container-lowest` (`#0B0E14` in linear sRGB).
const VIEWPORT_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0033,
    g: 0.0044,
    b: 0.0070,
    a: 1.0,
};

pub const VIEWPORT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Default upper bound for synchronous GPU-to-CPU viewport readback before timing out.
pub const DEFAULT_READBACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns the offscreen viewport render target and orchestrates 2D/3D viewport passes.
pub struct ViewportRenderer {
    size: (u32, u32),
    color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    grid_pass: GridPass,
    shadow_pass: ShadowPass,
    mesh_pass: MeshPass,
    overlay_pass: OverlayPass,
    pixel_pass: PixelPass,
    last_uploaded_revision: u64,
    last_playback_time_bits: Option<u32>,
    last_hover_voxel: Option<VoxelCoord>,
    last_gizmo_axis: Option<GizmoAxis>,
    last_light_angles: (f32, f32),
    scene_bounds: ([f32; 3], [f32; 3]),
    scratch_vertices: Vec<StudioVertex3D>,
    scratch_indices: Vec<u32>,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let safe_size = (width.max(1), height.max(1));
        let (color_texture, color_view) = create_color_target(device, safe_size.0, safe_size.1);
        let (depth_texture, depth_view) = create_depth_target(device, safe_size.0, safe_size.1);
        let grid_pass = GridPass::new(device, VIEWPORT_FORMAT, DEPTH_FORMAT);
        let shadow_pass = ShadowPass::new(device, VIEWPORT_FORMAT, DEPTH_FORMAT);
        let mesh_pass = MeshPass::new(device, VIEWPORT_FORMAT, DEPTH_FORMAT);
        let overlay_pass = OverlayPass::new(device, VIEWPORT_FORMAT, DEPTH_FORMAT);
        let pixel_pass = PixelPass::new(device, VIEWPORT_FORMAT);
        Self {
            size: safe_size,
            color_texture,
            color_view,
            depth_texture,
            depth_view,
            grid_pass,
            shadow_pass,
            mesh_pass,
            overlay_pass,
            pixel_pass,
            last_uploaded_revision: u64::MAX,
            last_playback_time_bits: None,
            last_hover_voxel: None,
            last_gizmo_axis: None,
            last_light_angles: (f32::NAN, f32::NAN),
            scene_bounds: ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            scratch_vertices: Vec::new(),
            scratch_indices: Vec::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    /// Resizes offscreen color & depth targets only when physical pixel dimensions change.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let next = (width.max(1), height.max(1));
        if next == self.size {
            return false;
        }
        self.size = next;
        let (tex, view) = create_color_target(device, next.0, next.1);
        let (dtex, dview) = create_depth_target(device, next.0, next.1);
        self.color_texture = tex;
        self.color_view = view;
        self.depth_texture = dtex;
        self.depth_view = dview;
        true
    }

    pub fn sync_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &SceneState,
        hover_voxel: Option<VoxelCoord>,
        active_gizmo_axis: Option<GizmoAxis>,
    ) {
        let playback_time_bits = scene
            .character_rig
            .as_ref()
            .map(|r| r.playback_time.to_bits());
        let scene_changed = self.last_uploaded_revision != scene.revision
            || self.last_playback_time_bits != playback_time_bits
            || scene.dirty_viewport;
        let hover_changed = self.last_hover_voxel != hover_voxel;
        let gizmo_changed = self.last_gizmo_axis != active_gizmo_axis;
        let light_angles = (scene.light_azimuth, scene.light_elevation);
        let light_changed = self.last_light_angles != light_angles;
        if scene_changed || hover_changed || gizmo_changed || light_changed {
            self.overlay_pass
                .update_lines(device, queue, scene, hover_voxel, active_gizmo_axis);
            self.last_hover_voxel = hover_voxel;
            self.last_gizmo_axis = active_gizmo_axis;
            self.last_light_angles = light_angles;
        }
        if !scene_changed {
            return;
        }
        self.last_playback_time_bits = playback_time_bits;
        self.mesh_pass.upload_palette(queue, &scene.palette);

        if let Some(node_id) = scene.active_pixel_node() {
            if let Some(canvas) = scene.pixel_canvases.get(&node_id) {
                let rgba = canvas.composite_viewport_rgba(&scene.palette);
                if let Err(err) =
                    self.pixel_pass
                        .upload_canvas(device, queue, canvas.width, canvas.height, &rgba)
                {
                    tracing::warn!(error = %err, "Skipping pixel canvas GPU upload");
                }
            }
        }

        let skin_solver = scene
            .character_rig
            .as_ref()
            .and_then(|r| tessel_core::AutoSkinSolver::from_rig(r, 1.0));
        let skin_mats = skin_solver
            .as_ref()
            .zip(scene.character_rig.as_ref())
            .map(|(solver, rig)| solver.skin_matrices(rig));

        let is_visible = |node_id: tessel_core::NodeId| -> bool {
            scene
                .character_rig
                .as_ref()
                .and_then(|r| r.node_visibility.get(&node_id).copied())
                .unwrap_or_else(|| scene.nodes.get(&node_id).map(|n| n.visible).unwrap_or(true))
        };
        let rig_pose_mats = scene
            .character_rig
            .as_ref()
            .filter(|r| !r.node_parent_overrides.is_empty())
            .map(|r| (r.world_pose_matrices(), r.world_rest_matrices()));

        self.scratch_vertices.clear();
        self.scratch_indices.clear();
        let vertices = &mut self.scratch_vertices;
        let indices = &mut self.scratch_indices;
        for (node_id, vol) in &scene.voxel_volumes {
            let visible = is_visible(*node_id);
            if !visible || vol.is_empty() {
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
            let skip_skin = parent_override.is_some();
            let normal_mat = if node_mat.determinant().abs() > 1e-8 {
                node_mat.inverse().transpose()
            } else {
                glam::Mat4::IDENTITY
            };
            let mesh = greedy_mesh_volume(vol, scene.unit_scale);
            let base = vertices.len() as u32;
            for v in mesh.vertices {
                let u = (f32::from(v.palette_index.get()) + 0.5) / 256.0;
                let mut wp = node_mat.transform_point3(glam::Vec3::from_array(v.position));
                let mut wn = normal_mat
                    .transform_vector3(glam::Vec3::from_array(v.normal))
                    .normalize_or_zero();
                if !skip_skin {
                    if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                        let rest_wp = scene
                            .node_rest_world_matrix(*node_id)
                            .transform_point3(glam::Vec3::from_array(v.position));
                        let w = solver.weights_for_vertex(Some(*node_id), rest_wp);
                        let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                        wp = sp;
                        wn = sn;
                    }
                }
                vertices.push(StudioVertex3D {
                    position: wp.to_array(),
                    normal: wn.to_array(),
                    palette_uv: [u, 0.5],
                    ao: v.ao,
                    tangent: [1.0, 0.0, 0.0, 1.0],
                    outline_normal: wn.to_array(),
                });
            }
            indices.extend(mesh.indices.into_iter().map(|idx| idx + base));
        }

        let mut custom_items = Vec::new();
        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = is_visible(*node_id);
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
            let skip_skin = parent_override.is_some();
            let normal_mat = if node_mat.determinant().abs() > 1e-8 {
                node_mat.inverse().transpose()
            } else {
                glam::Mat4::IDENTITY
            };

            // Apply live morph-target deformations (position + normal deltas) using active rig
            // weight or each morph target's `default_weight`.
            let morphed_mesh = {
                let has_active_morph = mesh.morphs.iter().any(|m| {
                    let w = scene
                        .character_rig
                        .as_ref()
                        .and_then(|rig| rig.morph_weights.get(&(*node_id, m.name.clone())).copied())
                        .unwrap_or(m.default_weight);
                    w.abs() > 1e-5
                });
                if !has_active_morph {
                    None
                } else {
                    let mut deformed = mesh.clone();
                    let has_normal_deltas = mesh.morphs.iter().any(|m| !m.normal_deltas.is_empty());
                    let computed_base_normals = if has_normal_deltas {
                        let mut acc = vec![glam::Vec3::ZERO; deformed.vertices.len()];
                        for face in &deformed.faces {
                            if face.vertices.len() < 3 {
                                continue;
                            }
                            let Some(&i0) = face.vertices.first() else {
                                continue;
                            };
                            let Some(&p0) = deformed.vertices.get(i0) else {
                                continue;
                            };
                            for fan in 1..(face.vertices.len() - 1) {
                                let (Some(&i1), Some(&i2)) =
                                    (face.vertices.get(fan), face.vertices.get(fan + 1))
                                else {
                                    continue;
                                };
                                let (Some(&p1), Some(&p2)) =
                                    (deformed.vertices.get(i1), deformed.vertices.get(i2))
                                else {
                                    continue;
                                };
                                let cross = (p1 - p0).cross(p2 - p0);
                                if cross.length_squared() > 1e-10 {
                                    acc[i0] += cross;
                                    acc[i1] += cross;
                                    acc[i2] += cross;
                                }
                            }
                        }
                        for n in &mut acc {
                            *n = n.normalize_or_zero();
                            if n.length_squared() < 1e-6 {
                                *n = glam::Vec3::Y;
                            }
                        }
                        Some(acc)
                    } else {
                        None
                    };
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
                        if w.abs() <= 1e-5 {
                            continue;
                        }
                        for (&vi, &delta) in &morph.deltas {
                            if let Some(pos) = deformed.vertices.get_mut(vi) {
                                *pos += delta * w;
                            }
                        }
                        for (&vi, &nd) in &morph.normal_deltas {
                            let base_n = deformed
                                .proxy_normals
                                .get(&vi)
                                .copied()
                                .or_else(|| {
                                    computed_base_normals
                                        .as_ref()
                                        .and_then(|bn| bn.get(vi).copied())
                                })
                                .unwrap_or(glam::Vec3::Y);
                            deformed
                                .proxy_normals
                                .insert(vi, (base_n + nd * w).normalize_or_zero());
                        }
                    }
                    Some(deformed)
                }
            };
            let rest_node_mat = scene.node_rest_world_matrix(*node_id);
            let active_mesh = morphed_mesh.as_ref().unwrap_or(mesh);
            let tri = active_mesh.to_triangulated_flat();
            let has_custom = active_mesh.texture.is_some()
                || active_mesh.normal_map.is_some()
                || active_mesh.material != tessel_core::MeshMaterial::default();
            if has_custom {
                let mut t_verts = Vec::with_capacity(tri.vertices.len());
                for (corner, v) in tri.vertices.into_iter().enumerate() {
                    let mut wp = node_mat.transform_point3(glam::Vec3::from_array(v.position));
                    let mut wn = normal_mat
                        .transform_vector3(glam::Vec3::from_array(v.normal))
                        .normalize_or_zero();
                    let mut wt = normal_mat
                        .transform_vector3(glam::Vec3::new(
                            v.tangent[0],
                            v.tangent[1],
                            v.tangent[2],
                        ))
                        .normalize_or_zero();
                    if !skip_skin {
                        if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                            let w = tri
                                .source_indices
                                .get(corner)
                                .and_then(|src| active_mesh.skin_weights.get(src).copied())
                                .unwrap_or_else(|| {
                                    let rest_local = tri
                                        .source_indices
                                        .get(corner)
                                        .and_then(|src| mesh.vertices.get(*src).copied())
                                        .unwrap_or_else(|| glam::Vec3::from_array(v.position));
                                    let rest_wp = rest_node_mat.transform_point3(rest_local);
                                    solver.weights_for_vertex(Some(*node_id), rest_wp)
                                });
                            let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                            let (_, st) = solver.deform_point_normal(mats, w, wp, wt);
                            wp = sp;
                            wn = sn;
                            wt = st;
                        }
                    }
                    let uv = if active_mesh.texture.is_some() || active_mesh.normal_map.is_some() {
                        v.uv
                    } else {
                        [(f32::from(v.palette_index.get()) + 0.5) / 256.0, 0.5]
                    };
                    t_verts.push(StudioVertex3D {
                        position: wp.to_array(),
                        normal: wn.to_array(),
                        palette_uv: uv,
                        ao: 1.0,
                        tangent: [wt.x, wt.y, wt.z, v.tangent[3]],
                        outline_normal: wn.to_array(),
                    });
                }
                fill_smooth_outline_normals(&mut t_verts);
                custom_items.push(crate::mesh_pass::CustomMeshUpload {
                    texture: mesh.texture.as_ref(),
                    normal_map: mesh.normal_map.as_ref(),
                    material: mesh.material,
                    vertices: t_verts,
                    indices: tri.indices,
                });
            } else {
                let base = vertices.len() as u32;
                for (corner, v) in tri.vertices.into_iter().enumerate() {
                    let u = (f32::from(v.palette_index.get()) + 0.5) / 256.0;
                    let mut wp = node_mat.transform_point3(glam::Vec3::from_array(v.position));
                    let mut wn = normal_mat
                        .transform_vector3(glam::Vec3::from_array(v.normal))
                        .normalize_or_zero();
                    if !skip_skin {
                        if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                            let w = tri
                                .source_indices
                                .get(corner)
                                .and_then(|src| active_mesh.skin_weights.get(src).copied())
                                .unwrap_or_else(|| {
                                    let rest_local = tri
                                        .source_indices
                                        .get(corner)
                                        .and_then(|src| mesh.vertices.get(*src).copied())
                                        .unwrap_or_else(|| glam::Vec3::from_array(v.position));
                                    let rest_wp = rest_node_mat.transform_point3(rest_local);
                                    solver.weights_for_vertex(Some(*node_id), rest_wp)
                                });
                            let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                            wp = sp;
                            wn = sn;
                        }
                    }
                    vertices.push(StudioVertex3D {
                        position: wp.to_array(),
                        normal: wn.to_array(),
                        palette_uv: [u, 0.5],
                        ao: 1.0,
                        tangent: [1.0, 0.0, 0.0, 1.0],
                        outline_normal: wn.to_array(),
                    });
                }
                indices.extend(tri.indices.into_iter().map(|idx| idx + base));
            }
        }
        fill_smooth_outline_normals(vertices);
        self.mesh_pass.upload_mesh(device, queue, vertices, indices);
        self.mesh_pass
            .upload_custom_meshes(device, queue, &custom_items);

        let mut bmin = [f32::INFINITY; 3];
        let mut bmax = [f32::NEG_INFINITY; 3];
        for v in vertices
            .iter()
            .chain(custom_items.iter().flat_map(|item| item.vertices.iter()))
        {
            for axis in 0..3 {
                bmin[axis] = bmin[axis].min(v.position[axis]);
                bmax[axis] = bmax[axis].max(v.position[axis]);
            }
        }
        self.scene_bounds = (bmin, bmax);
        self.shadow_pass
            .update_bounds(queue, &ShadowUniform::from_bounds(bmin, bmax));
        self.last_uploaded_revision = scene.revision;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneState,
        camera_3d: &OrbitCamera,
        camera_2d: &PixelCamera2D,
        pixels_per_point: f32,
        render_preview: bool,
    ) {
        self.render_with_light(
            queue,
            encoder,
            scene,
            camera_3d,
            camera_2d,
            pixels_per_point,
            render_preview,
            scene.light_azimuth,
            scene.light_elevation,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_with_light(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneState,
        camera_3d: &OrbitCamera,
        camera_2d: &PixelCamera2D,
        pixels_per_point: f32,
        render_preview: bool,
        light_azimuth: f32,
        light_elevation: f32,
    ) {
        match scene.active_mode {
            AppMode::Pixel => {
                self.pixel_pass
                    .update_view(queue, self.size, camera_2d, pixels_per_point);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tessel-pixel-viewport-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(VIEWPORT_CLEAR_COLOR),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                self.pixel_pass.draw(&mut pass);
            }
            AppMode::Voxel | AppMode::LowPoly | AppMode::Character => {
                let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
                let uniform = camera_3d.to_uniform(aspect, light_azimuth, light_elevation);
                self.grid_pass.update_camera(queue, &uniform);
                self.shadow_pass.update_camera(queue, &uniform);
                self.mesh_pass.update_camera(queue, &uniform);
                self.overlay_pass.update_camera(queue, &uniform);

                let light_dir = glam::Vec3::new(
                    uniform.light_dir[0],
                    uniform.light_dir[1],
                    uniform.light_dir[2],
                );
                self.mesh_pass.render_shadow_map(
                    queue,
                    encoder,
                    light_dir,
                    self.scene_bounds.0,
                    self.scene_bounds.1,
                    scene.shading_mode,
                );

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tessel-3d-viewport-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(VIEWPORT_CLEAR_COLOR),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                if !render_preview {
                    self.grid_pass.draw(&mut pass);
                }
                self.shadow_pass.draw(&mut pass);
                self.mesh_pass.draw(&mut pass, scene.shading_mode);
                if !render_preview {
                    self.overlay_pass.draw(&mut pass);
                }
            }
        }
    }

    /// Captures the current offscreen viewport texture as raw sRGB RGBA8 bytes `(width, height, rgba)`.
    ///
    /// Fails with [`RenderError::ReadbackTimeout`] if GPU readback does not complete within
    /// [`DEFAULT_READBACK_TIMEOUT`].
    pub fn capture_rgba_bytes(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(u32, u32, Vec<u8>), RenderError> {
        self.capture_rgba_bytes_with_timeout(device, queue, DEFAULT_READBACK_TIMEOUT)
    }

    /// Captures the current offscreen viewport texture as raw sRGB RGBA8 bytes `(width, height, rgba)`,
    /// aborting with [`RenderError::ReadbackTimeout`] if mapping does not finish within `timeout`.
    pub fn capture_rgba_bytes_with_timeout(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timeout: Duration,
    ) -> Result<(u32, u32, Vec<u8>), RenderError> {
        let (width, height) = self.size;
        let bytes_per_pixel = 4u32;
        let unpadded_bpr = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = unpadded_bpr.div_ceil(align) * align;

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tessel-viewport-readback"),
            size: u64::from(padded_bpr) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tessel-viewport-capture-encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        wait_for_map_with_timeout(&rx, timeout, || {
            device.poll(wgpu::Maintain::Poll);
        })?;

        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
        for row in 0..height as usize {
            let start = row * (padded_bpr as usize);
            rgba.extend_from_slice(&mapped[start..start + (unpadded_bpr as usize)]);
        }
        drop(mapped);
        staging.unmap();
        Ok((width, height, rgba))
    }

    /// Captures the current offscreen viewport texture as PNG bytes for MCP `tessel://viewport/preview`.
    pub fn capture_png_bytes(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<u8>, RenderError> {
        let (width, height, rgba) = self.capture_rgba_bytes(device, queue)?;
        let img = RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| RenderError::PreviewEncode("invalid image buffer size".into()))?;
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png)
            .map_err(|e| RenderError::PreviewEncode(e.to_string()))?;
        Ok(out.into_inner())
    }
}

fn quantize_outline_key(pos: [f32; 3]) -> Option<[i64; 3]> {
    if !pos.iter().all(|c| c.is_finite()) {
        return None;
    }
    const MAX_COORD: f32 = 1e10;
    Some([
        (pos[0].clamp(-MAX_COORD, MAX_COORD) * 10_000.0).round() as i64,
        (pos[1].clamp(-MAX_COORD, MAX_COORD) * 10_000.0).round() as i64,
        (pos[2].clamp(-MAX_COORD, MAX_COORD) * 10_000.0).round() as i64,
    ])
}

/// Computes position-welded smooth extrusion normals for `outline_hull.wgsl` so flat-shaded
/// low-poly and voxel meshes never split apart at sharp corners.
fn fill_smooth_outline_normals(vertices: &mut [StudioVertex3D]) {
    if vertices.is_empty() {
        return;
    }
    let mut accum: std::collections::HashMap<[i64; 3], glam::Vec3> =
        std::collections::HashMap::with_capacity(vertices.len());
    for v in vertices.iter() {
        let Some(key) = quantize_outline_key(v.position) else {
            continue;
        };
        let n = glam::Vec3::from_array(v.normal);
        if n.is_finite() {
            *accum.entry(key).or_insert(glam::Vec3::ZERO) += n;
        }
    }
    for v in vertices.iter_mut() {
        let Some(key) = quantize_outline_key(v.position) else {
            continue;
        };
        if let Some(sum) = accum.get(&key) {
            let smooth = sum.normalize_or_zero();
            if smooth.length_squared() > 1e-6 {
                v.outline_normal = smooth.to_array();
            }
        }
    }
}

fn create_color_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tessel-viewport-color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VIEWPORT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_depth_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tessel-viewport-depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn wait_for_map_with_timeout<E, F>(
    rx: &Receiver<Result<(), E>>,
    timeout: Duration,
    mut poll_device: F,
) -> Result<(), RenderError>
where
    E: std::fmt::Display,
    F: FnMut(),
{
    let deadline = Instant::now().checked_add(timeout);
    loop {
        poll_device();
        match rx.try_recv() {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(e)) => return Err(RenderError::PreviewEncode(e.to_string())),
            Err(TryRecvError::Disconnected) => {
                return Err(RenderError::PreviewEncode(
                    "GPU readback channel disconnected".into(),
                ));
            }
            Err(TryRecvError::Empty) => {
                let wait_step = match deadline {
                    Some(dl) => {
                        let Some(remaining) = dl.checked_duration_since(Instant::now()) else {
                            return Err(RenderError::ReadbackTimeout(timeout));
                        };
                        if remaining.is_zero() {
                            return Err(RenderError::ReadbackTimeout(timeout));
                        }
                        remaining.min(Duration::from_millis(1))
                    }
                    None => Duration::from_millis(1),
                };
                match rx.recv_timeout(wait_step) {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(e)) => return Err(RenderError::PreviewEncode(e.to_string())),
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(RenderError::PreviewEncode(
                            "GPU readback channel disconnected".into(),
                        ));
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_map_times_out_when_gpu_never_signals() {
        let (_tx, rx) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        let mut polls = 0usize;
        let timeout = Duration::from_millis(15);
        let err = wait_for_map_with_timeout(&rx, timeout, || {
            polls += 1;
        })
        .unwrap_err();
        assert!(matches!(err, RenderError::ReadbackTimeout(d) if d == timeout));
        assert!(polls >= 1);
    }

    #[test]
    fn wait_for_map_succeeds_after_poll_triggers_callback() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        let mut polls = 0usize;
        let res = wait_for_map_with_timeout(&rx, Duration::from_millis(100), || {
            polls += 1;
            if polls == 2 {
                let _ = tx.send(Ok(()));
            }
        });
        assert!(res.is_ok());
        assert_eq!(polls, 2);
    }

    #[test]
    fn wait_for_map_errors_on_disconnected_channel() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        drop(tx);
        let err = wait_for_map_with_timeout(&rx, Duration::from_millis(50), || {}).unwrap_err();
        assert!(matches!(err, RenderError::PreviewEncode(_)));
    }

    #[test]
    fn wait_for_map_handles_zero_max_duration_and_map_error() {
        let (_tx0, rx0) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        let err0 = wait_for_map_with_timeout(&rx0, Duration::ZERO, || {}).unwrap_err();
        assert!(matches!(err0, RenderError::ReadbackTimeout(d) if d.is_zero()));

        let (tx_max, rx_max) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        let res_max = wait_for_map_with_timeout(&rx_max, Duration::MAX, || {
            let _ = tx_max.send(Ok(()));
        });
        assert!(res_max.is_ok());

        let (tx_err, rx_err) = std::sync::mpsc::channel::<Result<(), &'static str>>();
        tx_err.send(Err("buffer map failed")).unwrap();
        let err_map =
            wait_for_map_with_timeout(&rx_err, Duration::from_millis(50), || {}).unwrap_err();
        assert!(matches!(err_map, RenderError::PreviewEncode(msg) if msg.contains("map failed")));
    }
}
