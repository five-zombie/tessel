use std::io::Cursor;

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
    last_hover_voxel: Option<VoxelCoord>,
    last_gizmo_axis: Option<GizmoAxis>,
    last_light_angles: (f32, f32),
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
            last_hover_voxel: None,
            last_gizmo_axis: None,
            last_light_angles: (f32::NAN, f32::NAN),
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
        let scene_changed = self.last_uploaded_revision != scene.revision;
        let hover_changed = self.last_hover_voxel != hover_voxel;
        let gizmo_changed = self.last_gizmo_axis != active_gizmo_axis;
        let light_angles = (scene.light_azimuth, scene.light_elevation);
        let light_changed = self.last_light_angles != light_angles;
        if scene_changed || hover_changed || gizmo_changed || light_changed {
            self.overlay_pass
                .update_lines(device, scene, hover_voxel, active_gizmo_axis);
            self.last_hover_voxel = hover_voxel;
            self.last_gizmo_axis = active_gizmo_axis;
            self.last_light_angles = light_angles;
        }
        if !scene_changed {
            return;
        }
        self.mesh_pass.upload_palette(queue, &scene.palette);

        if let Some(node_id) = scene.active_pixel_node() {
            if let Some(canvas) = scene.pixel_canvases.get(&node_id) {
                let rgba = canvas.composite_viewport_rgba(&scene.palette);
                self.pixel_pass
                    .upload_canvas(device, queue, canvas.width, canvas.height, &rgba);
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

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (node_id, vol) in &scene.voxel_volumes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || vol.is_empty() {
                continue;
            }
            let node_mat = scene.node_world_matrix(*node_id);
            let normal_mat = node_mat.inverse().transpose();
            let mesh = greedy_mesh_volume(vol, scene.unit_scale);
            let base = vertices.len() as u32;
            for v in mesh.vertices {
                let u = (f32::from(v.palette_index.get()) + 0.5) / 256.0;
                let mut wp = node_mat.transform_point3(glam::Vec3::from_array(v.position));
                let mut wn = normal_mat
                    .transform_vector3(glam::Vec3::from_array(v.normal))
                    .normalize_or_zero();
                if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                    let w = solver.weights_for_vertex(Some(*node_id), wp);
                    let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                    wp = sp;
                    wn = sn;
                }
                vertices.push(StudioVertex3D {
                    position: wp.to_array(),
                    normal: wn.to_array(),
                    palette_uv: [u, 0.5],
                    ao: v.ao,
                    tangent: [1.0, 0.0, 0.0, 1.0],
                });
            }
            indices.extend(mesh.indices.into_iter().map(|idx| idx + base));
        }

        let mut custom_items = Vec::new();
        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || mesh.faces.is_empty() {
                continue;
            }
            let node_mat = scene.node_world_matrix(*node_id);
            let normal_mat = node_mat.inverse().transpose();

            // Apply live morph-target deformations when any morph weight for this node is non-zero.
            let morphed_mesh = scene.character_rig.as_ref().and_then(|rig| {
                let has_active_morph = mesh.morphs.iter().any(|m| {
                    rig.morph_weights
                        .get(&(*node_id, m.name.clone()))
                        .is_some_and(|&w| w.abs() > 1e-5)
                });
                if !has_active_morph {
                    return None;
                }
                let mut deformed = mesh.clone();
                for morph in &mesh.morphs {
                    let w = rig
                        .morph_weights
                        .get(&(*node_id, morph.name.clone()))
                        .copied()
                        .unwrap_or(0.0);
                    if w.abs() <= 1e-5 {
                        continue;
                    }
                    for (&vi, &delta) in &morph.deltas {
                        if let Some(pos) = deformed.vertices.get_mut(vi) {
                            *pos += delta * w;
                        }
                    }
                }
                Some(deformed)
            });
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
                    if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                        let w = tri
                            .source_indices
                            .get(corner)
                            .and_then(|src| active_mesh.skin_weights.get(src).copied())
                            .unwrap_or_else(|| solver.weights_for_vertex(Some(*node_id), wp));
                        let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                        let (_, st) = solver.deform_point_normal(mats, w, wp, wt);
                        wp = sp;
                        wn = sn;
                        wt = st;
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
                    });
                }
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
                    if let (Some(solver), Some(mats)) = (&skin_solver, &skin_mats) {
                        let w = tri
                            .source_indices
                            .get(corner)
                            .and_then(|src| active_mesh.skin_weights.get(src).copied())
                            .unwrap_or_else(|| solver.weights_for_vertex(Some(*node_id), wp));
                        let (sp, sn) = solver.deform_point_normal(mats, w, wp, wn);
                        wp = sp;
                        wn = sn;
                    }
                    vertices.push(StudioVertex3D {
                        position: wp.to_array(),
                        normal: wn.to_array(),
                        palette_uv: [u, 0.5],
                        ao: 1.0,
                        tangent: [1.0, 0.0, 0.0, 1.0],
                    });
                }
                indices.extend(tri.indices.into_iter().map(|idx| idx + base));
            }
        }
        self.mesh_pass.upload_mesh(device, &vertices, &indices);
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
                let uniform =
                    camera_3d.to_uniform(aspect, scene.light_azimuth, scene.light_elevation);
                self.grid_pass.update_camera(queue, &uniform);
                self.shadow_pass.update_camera(queue, &uniform);
                self.mesh_pass.update_camera(queue, &uniform);
                self.overlay_pass.update_camera(queue, &uniform);

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
                if render_preview {
                    self.shadow_pass.draw(&mut pass);
                } else {
                    self.grid_pass.draw(&mut pass);
                }
                self.mesh_pass.draw(&mut pass);
                if !render_preview {
                    self.overlay_pass.draw(&mut pass);
                }
            }
        }
    }

    /// Captures the current offscreen viewport texture as raw sRGB RGBA8 bytes `(width, height, rgba)`.
    pub fn capture_rgba_bytes(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
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
        device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| RenderError::PreviewEncode(e.to_string()))?
            .map_err(|e| RenderError::PreviewEncode(e.to_string()))?;

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
