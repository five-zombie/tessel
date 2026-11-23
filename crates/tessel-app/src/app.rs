use std::sync::Arc;

use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use tessel_ai::{dispatch_mcp_message_with_preview, LiveMcpServerHandle};
use tessel_core::{Command, HistoryStack, PixelCoord, SceneState};
use tessel_render::{OrbitCamera, PixelCamera2D, RenderError, ViewportRenderer};

use crate::input::handle_global_shortcuts;
use crate::project_files::ProjectFileManager;
use crate::settings::{load_api_key_from_keyring, AppPaths, AppSettings};
use crate::ui;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),

    #[error("render initialization error: {0}")]
    Render(#[from] RenderError),
}

pub fn run() -> Result<(), AppError> {
    let event_loop = EventLoop::new()?;
    let mut app = TesselApp::default();
    event_loop.run_app(&mut app)?;
    if let Some(err) = app.fatal_error {
        return Err(err);
    }
    Ok(())
}

struct StudioState {
    scene: SceneState,
    history: HistoryStack,
    camera_3d: OrbitCamera,
    camera_2d: PixelCamera2D,
    pixel_drag_start: Option<PixelCoord>,
    gizmo_drag: Option<ui::viewport::GizmoDragState>,
    export_dialog: ui::export_dialog::ExportDialogState,
    ai_state: ui::inspector::DirectAiUiState,
    command_palette: ui::command_palette::CommandPaletteState,
    quick_start: ui::quick_start::QuickStartState,
    project_files: ProjectFileManager,
    paths: AppPaths,
    settings: AppSettings,
    live_mcp: Option<LiveMcpServerHandle>,
}

impl StudioState {
    fn poll_live_mcp(&mut self, preview_png: Option<&[u8]>) {
        let Some(handle) = &self.live_mcp else {
            return;
        };
        while let Ok(req) = handle.rx.try_recv() {
            let reply = dispatch_mcp_message_with_preview(
                &req.payload,
                &mut self.scene,
                &mut self.history,
                &self.export_dialog.output_dir,
                preview_png,
            );
            let _ = req.reply_tx.send(reply);
        }
    }

    fn toggle_live_mcp(&mut self) {
        if let Some(handle) = self.live_mcp.take() {
            handle.stop();
            tracing::info!("Live MCP HTTP server stopped");
        } else {
            match LiveMcpServerHandle::start(38421) {
                Ok(handle) => {
                    tracing::info!(
                        "Live MCP HTTP server listening on http://127.0.0.1:{}/mcp (token: {})",
                        handle.port,
                        handle.token
                    );
                    self.live_mcp = Some(handle);
                }
                Err(err) => {
                    tracing::error!("Failed to start live MCP server: {err}");
                }
            }
        }
    }
}

impl Default for StudioState {
    fn default() -> Self {
        let paths = AppPaths::resolve();
        let settings = AppSettings::load(&paths);
        let project_files = ProjectFileManager::with_recovery_path(paths.recovery_file.clone());
        let has_recovery = project_files.has_recovery_snapshot();

        let mut scene = SceneState::new();
        let _ = Command::CreatePixelCanvas {
            name: "Sprite 32x32".into(),
            width: 32,
            height: 32,
        }
        .apply(&mut scene);
        let _ = Command::CreateVoxelVolume {
            name: "Starter Voxel".into(),
        }
        .apply(&mut scene);
        if let Some(voxel_id) = scene.active_voxel_node() {
            let _ = Command::FillVoxelBox {
                node_id: voxel_id,
                min: tessel_core::VoxelCoord::new(-2, 0, -2),
                max: tessel_core::VoxelCoord::new(2, 4, 2),
                color: Some(scene.active_color),
                hollow: false,
            }
            .apply(&mut scene);
        }

        let export_dialog = ui::export_dialog::ExportDialogState {
            output_dir: settings.allowed_export_dir.clone(),
            ..Default::default()
        };

        let mut ai_state = ui::inspector::DirectAiUiState::default();
        ai_state.config.provider = settings.ai_provider;
        ai_state.config.model = settings.ai_model.clone();
        ai_state.config.base_url = settings.ai_base_url.clone();
        ai_state.config.api_key = load_api_key_from_keyring(settings.ai_provider);

        Self {
            scene,
            history: HistoryStack::new(),
            camera_3d: OrbitCamera::new(),
            camera_2d: PixelCamera2D::new(),
            pixel_drag_start: None,
            gizmo_drag: None,
            export_dialog,
            ai_state,
            command_palette: ui::command_palette::CommandPaletteState::default(),
            quick_start: ui::quick_start::QuickStartState {
                open: true,
                has_recovery,
                error_message: None,
            },
            project_files,
            paths,
            settings,
            live_mcp: None,
        }
    }
}

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    software_fallback: bool,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    viewport_renderer: ViewportRenderer,
    viewport_texture_id: egui::TextureId,
}

impl GpuState {
    fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(Arc::clone(&window))?;

        let (adapter, software_fallback) =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })) {
                Some(adapter) => (adapter, false),
                None => {
                    tracing::warn!(
                        "No hardware GPU adapter found; falling back to software adapter"
                    );
                    let fallback = pollster::block_on(instance.request_adapter(
                        &wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::LowPower,
                            compatible_surface: Some(&surface),
                            force_fallback_adapter: true,
                        },
                    ))
                    .ok_or(RenderError::NoGpuAdapter)?;
                    (fallback, true)
                }
            };

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("tessel-device"),
                ..Default::default()
            },
            None,
        ))?;

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or(RenderError::UnsupportedSurfaceFormat)?;

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let egui_ctx = egui::Context::default();
        ui::theme::apply_tactile_precision_theme(&egui_ctx);

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let mut egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1, false);
        let viewport_renderer =
            ViewportRenderer::new(&device, surface_config.width, surface_config.height);
        let viewport_texture_id = egui_renderer.register_native_texture(
            &device,
            viewport_renderer.color_view(),
            wgpu::FilterMode::Linear,
        );

        Ok(Self {
            window,
            surface,
            surface_config,
            device,
            queue,
            software_fallback,
            egui_ctx,
            egui_state,
            egui_renderer,
            viewport_renderer,
            viewport_texture_id,
        })
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    fn render(&mut self, studio: &mut StudioState) -> Result<(), RenderError> {
        let preview_png = if studio.live_mcp.is_some() {
            self.viewport_renderer
                .capture_png_bytes(&self.device, &self.queue)
                .ok()
        } else {
            None
        };
        studio.poll_live_mcp(preview_png.as_deref());
        studio.project_files.tick_autosave(&studio.scene);

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut desired_viewport = self.viewport_renderer.size();
        let mut hover_voxel = None;
        let mut active_gizmo_axis = None;
        let mut quick_action = ui::quick_start::QuickStartAction::None;
        let mut top_actions = ui::top_bar::TopBarActions {
            toggle_mcp: false,
            save_project: false,
            open_project: false,
        };

        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            handle_global_shortcuts(
                ctx,
                &mut studio.scene,
                &mut studio.history,
                &mut studio.camera_3d,
                &mut studio.camera_2d,
            );
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
                studio.command_palette.open = !studio.command_palette.open;
            }
            if !ctx.wants_keyboard_input() {
                if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::E)) {
                    studio.export_dialog.open = true;
                }
                if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                    top_actions.save_project = true;
                }
                if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
                    top_actions.open_project = true;
                }
            }
            let mcp_port = studio.live_mcp.as_ref().map(|h| h.port);
            let bar = ui::top_bar::show_top_bar(
                ctx,
                &mut studio.scene,
                &mut studio.history,
                self.software_fallback,
                &mut studio.export_dialog.open,
                mcp_port,
            );
            top_actions.toggle_mcp |= bar.toggle_mcp;
            top_actions.save_project |= bar.save_project;
            top_actions.open_project |= bar.open_project;

            ui::status_bar::show_status_bar(
                ctx,
                &studio.scene,
                &studio.history,
                &studio.camera_3d,
                &studio.camera_2d,
            );
            ui::timeline::show_timeline(ctx, &mut studio.scene, &mut studio.history);
            ui::toolbar::show_toolbar(ctx, &mut studio.scene, &mut studio.history);
            ui::inspector::show_inspector(
                ctx,
                &mut studio.scene,
                &mut studio.history,
                &mut studio.ai_state,
                &studio.export_dialog.output_dir,
            );
            let (vp_size, hv, g_axis) = ui::viewport::show_viewport(
                ctx,
                self.viewport_texture_id,
                &mut studio.scene,
                &mut studio.history,
                &mut studio.camera_3d,
                &mut studio.camera_2d,
                &mut studio.pixel_drag_start,
                &mut studio.gizmo_drag,
            );
            desired_viewport = vp_size;
            hover_voxel = hv;
            active_gizmo_axis = g_axis;
            ui::export_dialog::show_export_dialog(ctx, &studio.scene, &mut studio.export_dialog);
            quick_action = ui::quick_start::show_quick_start(
                ctx,
                &mut studio.quick_start,
                &studio.settings.recent_projects,
            );
            ui::command_palette::show_command_palette(
                ctx,
                &mut studio.command_palette,
                &mut studio.scene,
                &mut studio.history,
                &mut studio.camera_3d,
                &mut studio.camera_2d,
                &mut studio.export_dialog.open,
            );
        });

        match quick_action {
            ui::quick_start::QuickStartAction::RecoverSession => {
                match studio.project_files.load_recovery_snapshot() {
                    Ok(loaded) => {
                        studio.scene = loaded;
                        studio.history = HistoryStack::new();
                    }
                    Err(err) => {
                        studio.quick_start.open = true;
                        studio.quick_start.error_message = Some(err);
                    }
                }
            }
            ui::quick_start::QuickStartAction::OpenRecent(path) => {
                match studio.project_files.load_from_path(path.clone()) {
                    Ok(loaded) => {
                        studio.scene = loaded;
                        studio.history = HistoryStack::new();
                        studio.settings.record_recent_project(path);
                        studio.settings.save(&studio.paths);
                    }
                    Err(err) => {
                        studio.quick_start.open = true;
                        studio.quick_start.error_message = Some(err);
                    }
                }
            }
            other => {
                ui::quick_start::apply_quick_start_preset(
                    &other,
                    &mut studio.scene,
                    &mut studio.history,
                );
            }
        }

        if top_actions.toggle_mcp {
            studio.toggle_live_mcp();
        }
        if top_actions.save_project {
            let thumb = self
                .viewport_renderer
                .capture_png_bytes(&self.device, &self.queue)
                .ok();
            match studio
                .project_files
                .save_with_dialog(&studio.scene, thumb.as_deref())
            {
                Ok(Some(path)) => {
                    studio.settings.record_recent_project(path);
                    studio.settings.save(&studio.paths);
                }
                Ok(None) => {}
                Err(err) => tracing::error!("Save project failed: {err}"),
            }
        }
        if top_actions.open_project {
            match studio.project_files.open_with_dialog() {
                Ok(Some(loaded)) => {
                    studio.scene = loaded;
                    studio.history = HistoryStack::new();
                    if let Some(path) = studio.project_files.current_path.clone() {
                        studio.settings.record_recent_project(path);
                        studio.settings.save(&studio.paths);
                    }
                }
                Ok(None) => {}
                Err(err) => tracing::error!("Open project failed: {err}"),
            }
        }

        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);

        if self
            .viewport_renderer
            .resize(&self.device, desired_viewport.0, desired_viewport.1)
        {
            self.egui_renderer.update_egui_texture_from_wgpu_texture(
                &self.device,
                self.viewport_renderer.color_view(),
                wgpu::FilterMode::Linear,
                self.viewport_texture_id,
            );
        }

        self.viewport_renderer.sync_scene(
            &self.device,
            &self.queue,
            &studio.scene,
            hover_voxel,
            active_gizmo_axis,
        );

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(err) => return Err(RenderError::SurfaceTexture(err)),
        };

        let surface_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tessel-frame-encoder"),
            });

        self.viewport_renderer.render(
            &self.queue,
            &mut encoder,
            &studio.scene,
            &studio.camera_3d,
            &studio.camera_2d,
            self.window.scale_factor() as f32,
        );

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.surface_config.width, self.surface_config.height],
            pixels_per_point: self.window.scale_factor() as f32,
        };

        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, image_delta);
        }

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tessel-egui-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
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
            self.egui_renderer.render(
                &mut render_pass.forget_lifetime(),
                &paint_jobs,
                &screen_descriptor,
            );
        }

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

#[derive(Default)]
struct TesselApp {
    gpu: Option<GpuState>,
    studio: StudioState,
    fatal_error: Option<AppError>,
}

impl ApplicationHandler for TesselApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Tessel")
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(800.0, 520.0));

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                match GpuState::new(window) {
                    Ok(gpu) => {
                        gpu.window.request_redraw();
                        self.gpu = Some(gpu);
                    }
                    Err(err) => {
                        self.fatal_error = Some(AppError::Render(err));
                        event_loop.exit();
                    }
                }
            }
            Err(err) => {
                tracing::error!("failed to create window: {err}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        let response = gpu.egui_state.on_window_event(&gpu.window, &event);
        if response.repaint {
            gpu.window.request_redraw();
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.resize_surface(size.width, size.height);
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = gpu.render(&mut self.studio) {
                    tracing::error!("render error: {err}");
                }
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}
