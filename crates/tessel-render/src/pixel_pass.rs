use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const PIXEL_SHADER_WGSL: &str = include_str!("../../../assets/shaders/pixel_2d.wgsl");

/// Minimum 2D pixel zoom (1%), allowing a 4096×4096 canvas to fit even in small viewports/screenshots.
pub const MIN_PIXEL_ZOOM: f32 = 0.01;
/// Maximum 2D pixel zoom (6400%).
pub const MAX_PIXEL_ZOOM: f32 = 64.0;

/// 2D pan/zoom camera state for `AppMode::Pixel`.
#[derive(Debug, Clone, Copy)]
pub struct PixelCamera2D {
    pub pan: [f32; 2],
    pub zoom: f32,
    pub show_grid: bool,
}

impl PixelCamera2D {
    pub fn new() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 14.0,
            show_grid: true,
        }
    }

    pub fn reset(&mut self) {
        self.pan = [0.0, 0.0];
        self.zoom = 14.0;
    }

    /// Centers the canvas and adjusts zoom so the entire canvas fits inside `viewport_size` with margin.
    pub fn fit_canvas(&mut self, viewport_size: [f32; 2], canvas_size: (u32, u32)) {
        self.pan = [0.0, 0.0];
        let vw = viewport_size[0];
        let vh = viewport_size[1];
        let cw = canvas_size.0 as f32;
        let ch = canvas_size.1 as f32;
        if vw.is_finite() && vh.is_finite() && vw > 1.0 && vh > 1.0 && cw > 0.0 && ch > 0.0 {
            let fit = ((vw * 0.90) / cw).min((vh * 0.90) / ch);
            self.zoom = fit.clamp(MIN_PIXEL_ZOOM, MAX_PIXEL_ZOOM);
        } else {
            self.zoom = 14.0;
        }
    }

    /// Fits the camera to the scene's active pixel canvas if present, otherwise resets to defaults.
    pub fn fit_active_canvas(&mut self, viewport_size: [f32; 2], scene: &tessel_core::SceneState) {
        if let Some(canvas) = scene
            .active_pixel_node()
            .and_then(|id| scene.pixel_canvases.get(&id))
        {
            self.fit_canvas(viewport_size, (canvas.width, canvas.height));
        } else {
            self.reset();
        }
    }

    pub fn zoom_by(&mut self, scroll_delta: f32) {
        if !scroll_delta.is_finite() {
            return;
        }
        let base = if self.zoom.is_finite() {
            self.zoom
        } else {
            14.0
        };
        let factor = (1.0 + scroll_delta * 0.002).clamp(0.75, 1.35);
        self.zoom = (base * factor).clamp(MIN_PIXEL_ZOOM, MAX_PIXEL_ZOOM);
    }

    pub fn zoom_step(&mut self, multiplier: f32) {
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return;
        }
        let base = if self.zoom.is_finite() {
            self.zoom
        } else {
            14.0
        };
        self.zoom = (base * multiplier).clamp(MIN_PIXEL_ZOOM, MAX_PIXEL_ZOOM);
    }

    /// Converts a local viewport pixel position (logical or physical matching `viewport_size`)
    /// into a canvas pixel coordinate `(x, y)` if inside bounds.
    pub fn screen_to_canvas(
        &self,
        local_pos: [f32; 2],
        viewport_size: [f32; 2],
        canvas_size: (u32, u32),
    ) -> Option<(u32, u32)> {
        let zoom = if self.zoom.is_finite() {
            self.zoom.max(0.01)
        } else {
            1.0
        };
        let cx = viewport_size[0] * 0.5 + self.pan[0];
        let cy = viewport_size[1] * 0.5 + self.pan[1];
        let px = (local_pos[0] - cx) / zoom + canvas_size.0 as f32 * 0.5;
        let py = (local_pos[1] - cy) / zoom + canvas_size.1 as f32 * 0.5;
        if px >= 0.0 && py >= 0.0 && px < canvas_size.0 as f32 && py < canvas_size.1 as f32 {
            Some((px.floor() as u32, py.floor() as u32))
        } else {
            None
        }
    }
}

impl Default for PixelCamera2D {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct PixelViewUniform {
    viewport_size: [f32; 2],
    canvas_size: [f32; 2],
    pan: [f32; 2],
    zoom: f32,
    show_grid: f32,
    pixels_per_point: f32,
    _pad: [f32; 3],
}

/// Renders a crisp nearest-neighbor 2D pixel canvas with checkerboard and optional grid overlay.
pub struct PixelPass {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    canvas_size: (u32, u32),
    canvas_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

impl PixelPass {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tessel-pixel-2d-shader"),
            source: wgpu::ShaderSource::Wgsl(PIXEL_SHADER_WGSL.into()),
        });

        let initial_uniform = PixelViewUniform {
            viewport_size: [800.0, 600.0],
            canvas_size: [32.0, 32.0],
            pan: [0.0, 0.0],
            zoom: 14.0,
            show_grid: 1.0,
            pixels_per_point: 1.0,
            _pad: [0.0; 3],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tessel-pixel-view-uniform"),
            contents: bytemuck::bytes_of(&initial_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tessel-pixel-bgl"),
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
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tessel-pixel-nearest-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (canvas_texture, canvas_view) = create_canvas_texture(device, 32, 32);
        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &uniform_buffer,
            &canvas_view,
            &sampler,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tessel-pixel-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tessel-pixel-pipeline"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            uniform_buffer,
            bind_group_layout,
            sampler,
            canvas_size: (32, 32),
            canvas_texture,
            bind_group,
        }
    }

    pub fn upload_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), crate::error::RenderError> {
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        if width == 0 || height == 0 || width > max_dim || height > max_dim {
            tracing::warn!(
                width,
                height,
                max_dim,
                "PixelPass::upload_canvas rejected out-of-range canvas dimensions"
            );
            return Err(crate::error::RenderError::InvalidTextureUpload(format!(
                "pixel canvas dimensions {width}x{height} exceed valid range 1..={max_dim}"
            )));
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected_len != Some(rgba.len()) {
            tracing::warn!(
                width,
                height,
                actual_len = rgba.len(),
                ?expected_len,
                "PixelPass::upload_canvas rejected mismatched RGBA buffer length"
            );
            return Err(crate::error::RenderError::InvalidTextureUpload(format!(
                "pixel canvas RGBA buffer length {} does not match {width}x{height}x4",
                rgba.len()
            )));
        }
        if (width, height) != self.canvas_size {
            self.canvas_size = (width, height);
            let (tex, view) = create_canvas_texture(device, width, height);
            self.bind_group = create_bind_group(
                device,
                &self.bind_group_layout,
                &self.uniform_buffer,
                &view,
                &self.sampler,
            );
            self.canvas_texture = tex;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.canvas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub fn update_view(
        &self,
        queue: &wgpu::Queue,
        viewport_size: (u32, u32),
        camera: &PixelCamera2D,
        pixels_per_point: f32,
    ) {
        let ppp = if pixels_per_point.is_finite() {
            pixels_per_point.max(0.5)
        } else {
            1.0
        };
        let pan_x = if camera.pan[0].is_finite() {
            camera.pan[0]
        } else {
            0.0
        };
        let pan_y = if camera.pan[1].is_finite() {
            camera.pan[1]
        } else {
            0.0
        };
        let zoom = if camera.zoom.is_finite() {
            camera.zoom.clamp(MIN_PIXEL_ZOOM, MAX_PIXEL_ZOOM)
        } else {
            14.0
        };
        let uniform = PixelViewUniform {
            viewport_size: [viewport_size.0 as f32, viewport_size.1 as f32],
            canvas_size: [self.canvas_size.0 as f32, self.canvas_size.1 as f32],
            pan: [pan_x * ppp, pan_y * ppp],
            zoom: zoom * ppp,
            show_grid: if camera.show_grid { 1.0 } else { 0.0 },
            pixels_per_point: ppp,
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_canvas_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tessel-pixel-canvas-tex"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    canvas_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tessel-pixel-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(canvas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_camera_fits_4k_canvas_and_maps_sub_unit_zoom_coordinates() {
        let mut cam = PixelCamera2D::new();
        cam.fit_canvas([1280.0, 720.0], (4096, 4096));
        assert_eq!(cam.pan, [0.0, 0.0]);
        assert!(
            cam.zoom > 0.14 && cam.zoom < 0.17,
            "expected ~0.158 zoom for 4K in 720p viewport, got {}",
            cam.zoom
        );
        // Center of viewport maps to center of 4096x4096 canvas.
        assert_eq!(
            cam.screen_to_canvas([640.0, 360.0], [1280.0, 720.0], (4096, 4096)),
            Some((2048, 2048))
        );
        // Zooming out clamps cleanly to MIN_PIXEL_ZOOM (0.01), never stuck at 2.0,
        // and non-finite inputs must never poison zoom state.
        cam.zoom_step(0.001);
        assert!((cam.zoom - MIN_PIXEL_ZOOM).abs() < 1e-6);
        cam.zoom_by(f32::NAN);
        cam.zoom_step(f32::INFINITY);
        cam.zoom_step(-2.0);
        assert!(cam.zoom.is_finite());
        assert!((cam.zoom - MIN_PIXEL_ZOOM).abs() < 1e-6);
    }
}
