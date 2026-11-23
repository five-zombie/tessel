use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{AppMode, SceneState, VoxelCoord};

use crate::camera::OrbitCamera;
use crate::grid_pass::GridPass;
use crate::mesh_pass::MeshPass;
use crate::overlay_pass::OverlayPass;
use crate::pixel_pass::{PixelCamera2D, PixelPass};
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
    mesh_pass: MeshPass,
    overlay_pass: OverlayPass,
    pixel_pass: PixelPass,
    last_uploaded_revision: u64,
    last_hover_voxel: Option<VoxelCoord>,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let safe_size = (width.max(1), height.max(1));
        let (color_texture, color_view) = create_color_target(device, safe_size.0, safe_size.1);
        let (depth_texture, depth_view) = create_depth_target(device, safe_size.0, safe_size.1);
        let grid_pass = GridPass::new(device, VIEWPORT_FORMAT, DEPTH_FORMAT);
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
            mesh_pass,
            overlay_pass,
            pixel_pass,
            last_uploaded_revision: u64::MAX,
            last_hover_voxel: None,
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
    ) {
        let scene_changed = self.last_uploaded_revision != scene.revision;
        let hover_changed = self.last_hover_voxel != hover_voxel;
        if scene_changed || hover_changed {
            self.overlay_pass.update_lines(device, scene, hover_voxel);
            self.last_hover_voxel = hover_voxel;
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

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (node_id, vol) in &scene.voxel_volumes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || vol.is_empty() {
                continue;
            }
            let mesh = greedy_mesh_volume(vol, scene.unit_scale);
            let base = vertices.len() as u32;
            for v in mesh.vertices {
                let u = (f32::from(v.palette_index.get()) + 0.5) / 256.0;
                vertices.push(StudioVertex3D {
                    position: v.position,
                    normal: v.normal,
                    palette_uv: [u, 0.5],
                    ao: v.ao,
                });
            }
            indices.extend(mesh.indices.into_iter().map(|idx| idx + base));
        }

        let bone_mats = scene
            .character_rig
            .as_ref()
            .map(|r| r.world_pose_matrices());

        for (node_id, mesh) in &scene.lowpoly_meshes {
            let visible = scene.nodes.get(node_id).map(|n| n.visible).unwrap_or(true);
            if !visible || mesh.faces.is_empty() {
                continue;
            }
            let bound_mat = scene
                .character_rig
                .as_ref()
                .and_then(|rig| rig.bindings.iter().find(|b| b.node_id == *node_id))
                .and_then(|binding| bone_mats.as_ref().and_then(|m| m.get(&binding.bone_id)))
                .copied();

            let tri = mesh.to_triangulated_flat();
            let base = vertices.len() as u32;
            for v in tri.vertices {
                let u = (f32::from(v.palette_index.get()) + 0.5) / 256.0;
                let (pos, norm) = match bound_mat {
                    Some(mat) => {
                        let p = mat.transform_point3(glam::Vec3::from_array(v.position));
                        let n = mat
                            .transform_vector3(glam::Vec3::from_array(v.normal))
                            .normalize_or_zero();
                        (p.to_array(), n.to_array())
                    }
                    None => (v.position, v.normal),
                };
                vertices.push(StudioVertex3D {
                    position: pos,
                    normal: norm,
                    palette_uv: [u, 0.5],
                    ao: 1.0,
                });
            }
            indices.extend(tri.indices.into_iter().map(|idx| idx + base));
        }
        self.mesh_pass.upload_mesh(device, &vertices, &indices);
        self.last_uploaded_revision = scene.revision;
    }

    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mode: AppMode,
        camera_3d: &OrbitCamera,
        camera_2d: &PixelCamera2D,
        pixels_per_point: f32,
    ) {
        match mode {
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
                let uniform = camera_3d.to_uniform(aspect);
                self.grid_pass.update_camera(queue, &uniform);
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
                self.grid_pass.draw(&mut pass);
                self.mesh_pass.draw(&mut pass);
                self.overlay_pass.draw(&mut pass);
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
