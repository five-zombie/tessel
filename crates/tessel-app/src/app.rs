use std::sync::Arc;

use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use tessel_ai::{dispatch_mcp_message_with_preview, LiveMcpServerHandle};
use tessel_core::{Command, HistoryStack, PixelCoord, SceneState};
use tessel_render::{CameraPreset, OrbitCamera, PixelCamera2D, RenderError, ViewportRenderer};

use crate::input::handle_global_shortcuts;
use crate::project_files::ProjectFileManager;
use crate::settings::{
    load_api_key_from_keyring, load_or_create_mcp_token, rotate_mcp_token, AppPaths, AppSettings,
};
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
    camera_3d_target: Option<OrbitCamera>,
    showcase_camera: OrbitCamera,
    showcase_camera_target: Option<OrbitCamera>,
    display_light_azimuth: f32,
    display_light_elevation: f32,
    camera_2d: PixelCamera2D,
    pixel_drag_start: Option<PixelCoord>,
    gizmo_drag: Option<ui::viewport::GizmoDragState>,
    scene_gadget_drag: Option<ui::viewport::SceneGadgetDrag>,
    free_view: bool,
    render_preview: bool,
    export_dialog: ui::export_dialog::ExportDialogState,
    settings_dialog: ui::settings_dialog::SettingsDialogState,
    mcp_dialog: ui::mcp_dialog::McpDialogState,
    ai_state: ui::inspector::DirectAiUiState,
    command_palette: ui::command_palette::CommandPaletteState,
    quick_start: ui::quick_start::QuickStartState,
    about_open: bool,
    confirm_new_project: bool,
    confirm_exit: bool,
    quit_confirmed: bool,
    project_files: ProjectFileManager,
    paths: AppPaths,
    settings: AppSettings,
    mcp_token: String,
    live_mcp: Option<LiveMcpServerHandle>,
}

/// Live-viewport camera tools for MCP agents (orbit/zoom/pan/preset/look-at).
/// Only the GUI app hosts a camera, so these are intercepted in
/// [`StudioState::poll_live_mcp`] instead of going through the shared
/// [`tessel_ai::translator::execute_tool_call`] path (which correctly rejects
/// them as needing a live viewport).
fn camera_state_json(cam: &OrbitCamera) -> serde_json::Value {
    serde_json::json!({
        "target": cam.target.to_array(),
        "yaw": cam.yaw,
        "pitch": cam.pitch,
        "distance": cam.distance,
        "orthographic": cam.orthographic,
        "fov_deg": cam.fov_y_radians.to_degrees(),
    })
}

fn read_vec3(args: &serde_json::Value, key: &str) -> Result<glam::Vec3, tessel_ai::AiError> {
    use tessel_ai::AiError;
    let arr = args
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AiError::InvalidArguments(format!("missing '{key}' [x, y, z]")))?;
    if arr.len() != 3 {
        return Err(AiError::InvalidArguments(format!(
            "'{key}' must have 3 numbers"
        )));
    }
    let mut out = [0.0f32; 3];
    for (i, v) in arr.iter().enumerate() {
        out[i] = v
            .as_f64()
            .ok_or_else(|| AiError::InvalidArguments(format!("'{key}[{i}]' must be a number")))?
            as f32;
    }
    let vec = glam::Vec3::from_array(out);
    if !vec.is_finite() {
        return Err(AiError::InvalidArguments(format!("'{key}' must be finite")));
    }
    Ok(vec)
}

fn read_finite(args: &serde_json::Value, key: &str) -> Result<f32, tessel_ai::AiError> {
    use tessel_ai::AiError;
    args.get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| AiError::InvalidArguments(format!("missing finite number '{key}'")))
}

pub fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Exponentially eases `current` toward `target`. Returns `true` while still moving.
pub fn smooth_step_camera(current: &mut OrbitCamera, target: OrbitCamera, dt: f32) -> bool {
    let alpha = 1.0 - (-12.0 * dt.clamp(0.001, 0.05)).exp();
    let dyaw = shortest_angle_delta(current.yaw, target.yaw);
    let dpitch = target.pitch - current.pitch;
    let ddist = target.distance - current.distance;
    let dtarget = target.target - current.target;
    let dfov = target.fov_y_radians - current.fov_y_radians;
    current.orthographic = target.orthographic;
    if dtarget.length() < 1e-4
        && dyaw.abs() < 1e-4
        && dpitch.abs() < 1e-4
        && ddist.abs() < 1e-4
        && dfov.abs() < 1e-4
    {
        *current = target;
        return false;
    }
    current.target = current.target.lerp(target.target, alpha);
    current.yaw += dyaw * alpha;
    current.pitch += dpitch * alpha;
    current.distance += ddist * alpha;
    current.fov_y_radians += dfov * alpha;
    true
}

/// Exponentially eases display light angles toward target angles. Returns `true` while still moving.
pub fn smooth_step_light(
    cur_az: &mut f32,
    cur_el: &mut f32,
    target_az: f32,
    target_el: f32,
    dt: f32,
) -> bool {
    let alpha = 1.0 - (-12.0 * dt.clamp(0.001, 0.05)).exp();
    let daz = shortest_angle_delta(*cur_az, target_az);
    let del = target_el - *cur_el;
    if daz.abs() < 1e-4 && del.abs() < 1e-4 {
        *cur_az = target_az;
        *cur_el = target_el;
        return false;
    }
    *cur_az = (*cur_az + daz * alpha).rem_euclid(std::f32::consts::TAU);
    *cur_el += del * alpha;
    true
}

fn select_camera_target(args: &serde_json::Value) -> Result<bool, tessel_ai::AiError> {
    match args.get("camera").and_then(serde_json::Value::as_str) {
        None | Some("viewport") => Ok(false),
        Some("scene") => Ok(true),
        Some(_) => Err(tessel_ai::AiError::InvalidArguments(
            "'camera' must be 'viewport' or 'scene'".into(),
        )),
    }
}

fn handle_camera_tool_call(
    tool_name: &str,
    args: &serde_json::Value,
    viewport_cam: &mut OrbitCamera,
    scene_cam: &mut OrbitCamera,
) -> Result<serde_json::Value, tessel_ai::AiError> {
    use tessel_ai::AiError;
    let is_scene = select_camera_target(args)?;
    let cam_label = if is_scene { "scene" } else { "viewport" };
    if tool_name == "camera_get" {
        let selected: &OrbitCamera = if is_scene { scene_cam } else { viewport_cam };
        return Ok(serde_json::json!({
            "ok": true,
            "target_camera": cam_label,
            "camera": camera_state_json(selected),
            "viewport": camera_state_json(viewport_cam),
            "scene": camera_state_json(scene_cam),
        }));
    }
    let cam = if is_scene { scene_cam } else { viewport_cam };
    match tool_name {
        "camera_set_pose" => {
            let mut any = false;
            if args.get("target").is_some() {
                cam.target = read_vec3(args, "target")?;
                any = true;
            }
            if args.get("yaw_deg").is_some() {
                cam.yaw = read_finite(args, "yaw_deg")?.to_radians();
                any = true;
            } else if args.get("yaw").is_some() {
                cam.yaw = read_finite(args, "yaw")?;
                any = true;
            }
            if args.get("pitch_deg").is_some() {
                cam.pitch = read_finite(args, "pitch_deg")?
                    .to_radians()
                    .clamp(-1.52, 1.52);
                any = true;
            } else if args.get("pitch").is_some() {
                cam.pitch = read_finite(args, "pitch")?.clamp(-1.52, 1.52);
                any = true;
            }
            if args.get("distance").is_some() {
                let d = read_finite(args, "distance")?;
                if d <= 0.0 {
                    return Err(AiError::InvalidArguments(
                        "'distance' must be a finite number > 0".into(),
                    ));
                }
                cam.distance = d.clamp(0.25, 120.0);
                any = true;
            }
            if let Some(telephoto) = args.get("telephoto").and_then(serde_json::Value::as_bool) {
                if telephoto {
                    cam.set_telephoto();
                } else {
                    cam.fov_y_radians = 45.0_f32.to_radians();
                }
                any = true;
            }
            if let Some(ortho) = args
                .get("orthographic")
                .and_then(serde_json::Value::as_bool)
            {
                cam.set_orthographic(ortho);
                any = true;
            }
            if !any {
                return Err(AiError::InvalidArguments(
                    "camera_set_pose needs at least one of target, yaw_deg/yaw, pitch_deg/pitch, distance, orthographic, telephoto".into(),
                ));
            }
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_look_at" => {
            cam.target = read_vec3(args, "target")?;
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_zoom" => {
            if let Some(f) = args.get("factor").and_then(serde_json::Value::as_f64) {
                let f = f as f32;
                if !f.is_finite() || f <= 0.0 {
                    return Err(AiError::InvalidArguments(
                        "'factor' must be a finite number > 0".into(),
                    ));
                }
                cam.zoom_step(f.clamp(0.001, 1000.0));
            } else if let Some(d) = args.get("distance").and_then(serde_json::Value::as_f64) {
                let d = d as f32;
                if !d.is_finite() || d <= 0.0 {
                    return Err(AiError::InvalidArguments(
                        "'distance' must be a finite number > 0".into(),
                    ));
                }
                cam.zoom_step((d / cam.distance.max(1e-6)).clamp(0.001, 1000.0));
            } else {
                return Err(AiError::InvalidArguments(
                    "camera_zoom needs 'factor' or 'distance'".into(),
                ));
            }
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_orbit" => {
            let (dx, dy) = (read_finite(args, "dx")?, read_finite(args, "dy")?);
            cam.orbit(dx, dy);
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_pan" => {
            let (dx, dy) = (read_finite(args, "dx")?, read_finite(args, "dy")?);
            cam.pan(dx, dy);
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_reset" => {
            if is_scene {
                *cam = default_showcase_camera();
            } else {
                cam.reset();
            }
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        "camera_preset" => {
            let name = args
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let preset = match name {
                "iso" => CameraPreset::Iso,
                "front" => CameraPreset::Front,
                "back" => CameraPreset::Back,
                "right" => CameraPreset::Right,
                "left" => CameraPreset::Left,
                "top" => CameraPreset::Top,
                _ => {
                    return Err(AiError::InvalidArguments(
                        "unknown preset; use iso|front|back|right|left|top".into(),
                    ));
                }
            };
            cam.set_preset(preset);
            Ok(
                serde_json::json!({ "ok": true, "target_camera": cam_label, "camera": camera_state_json(cam) }),
            )
        }
        other => Err(AiError::UnknownTool(other.to_string())),
    }
}

impl StudioState {
    fn poll_live_mcp(
        &mut self,
        mut capture_preview: impl FnMut() -> Option<Vec<u8>>,
        mut render_offscreen: impl FnMut(
            &str,
            &SceneState,
            &std::path::Path,
            &serde_json::Value,
        ) -> Result<serde_json::Value, tessel_ai::AiError>,
    ) {
        let Some(handle) = &self.live_mcp else {
            return;
        };
        let mut cached_preview: Option<Option<Vec<u8>>> = None;
        while let Ok(req) = handle.rx.try_recv() {
            let wants_preview = req
                .payload
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(|u| u.as_str())
                == Some("tessel://viewport/preview");
            let preview_bytes = if wants_preview {
                cached_preview
                    .get_or_insert_with(&mut capture_preview)
                    .as_deref()
            } else {
                None
            };
            let is_tools_call = req
                .payload
                .get("method")
                .and_then(serde_json::Value::as_str)
                == Some("tools/call");
            let call_name = if is_tools_call {
                req.payload
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
            } else {
                ""
            };
            let reply = if call_name.starts_with("camera_") {
                let params = req
                    .payload
                    .get("params")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let id = req
                    .payload
                    .get("id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let mut vp_goal = self.camera_3d_target.unwrap_or(self.camera_3d);
                let mut sc_goal = self.showcase_camera_target.unwrap_or(self.showcase_camera);
                let result = match handle_camera_tool_call(
                    call_name,
                    &args,
                    &mut vp_goal,
                    &mut sc_goal,
                ) {
                    Ok(val) => {
                        if call_name != "camera_get" {
                            if select_camera_target(&args).unwrap_or(false) {
                                self.showcase_camera_target = Some(sc_goal);
                            } else {
                                self.camera_3d_target = Some(vp_goal);
                            }
                        }
                        serde_json::json!({
                            "content": [{ "type": "text", "text": val.to_string() }],
                            "isError": false
                        })
                    }
                    Err(err) => serde_json::json!({
                        "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                        "isError": true
                    }),
                };
                Some(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }))
            } else if call_name == "viewport_set_preview" {
                let id = req
                    .payload
                    .get("id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let preview = req
                    .payload
                    .get("params")
                    .and_then(|p| p.get("arguments"))
                    .and_then(|a| a.get("preview"))
                    .and_then(serde_json::Value::as_bool);
                let result = match preview {
                    Some(p) => {
                        self.render_preview = p;
                        let val = serde_json::json!({ "ok": true, "preview": p });
                        serde_json::json!({
                            "content": [{ "type": "text", "text": val.to_string() }],
                            "isError": false
                        })
                    }
                    None => {
                        let err = tessel_ai::AiError::InvalidArguments(
                            "missing boolean 'preview'".into(),
                        );
                        serde_json::json!({
                            "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                            "isError": true
                        })
                    }
                };
                Some(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }))
            } else if call_name == "viewport_screenshot"
                || call_name == "viewport_render_turntable_gif"
            {
                let id = req
                    .payload
                    .get("id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let args = req
                    .payload
                    .get("params")
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let result = match render_offscreen(
                    call_name,
                    &self.scene,
                    &self.export_dialog.output_dir,
                    &args,
                ) {
                    Ok(val) => {
                        let content = if call_name == "viewport_screenshot" {
                            val
                        } else {
                            serde_json::json!([{ "type": "text", "text": val.to_string() }])
                        };
                        serde_json::json!({
                            "content": content,
                            "isError": false
                        })
                    }
                    Err(err) => serde_json::json!({
                        "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                        "isError": true
                    }),
                };
                Some(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }))
            } else {
                dispatch_mcp_message_with_preview(
                    &req.payload,
                    &mut self.scene,
                    &mut self.history,
                    &self.export_dialog.output_dir,
                    preview_bytes,
                )
            };
            let _ = req.reply_tx.send(reply);
        }
    }

    fn toggle_live_mcp(&mut self) {
        if let Some(handle) = self.live_mcp.take() {
            handle.stop();
            tracing::info!("Live MCP HTTP server stopped");
        } else {
            match LiveMcpServerHandle::start_with_token(38421, self.mcp_token.clone()) {
                Ok(handle) => {
                    tracing::info!(
                        "Live MCP HTTP server listening on http://127.0.0.1:{}/mcp",
                        handle.port
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

fn default_showcase_camera() -> OrbitCamera {
    let mut cam = OrbitCamera::new();
    cam.yaw = -0.65;
    cam.pitch = 0.28;
    cam.distance = 4.2;
    cam
}

impl Default for StudioState {
    fn default() -> Self {
        let paths = AppPaths::resolve();
        let settings = AppSettings::load(&paths);
        let mcp_token = load_or_create_mcp_token(&paths);
        let mut project_files = ProjectFileManager::with_recovery_path(paths.recovery_file.clone());
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
        project_files.mark_baseline_revision(scene.revision);
        project_files.set_autosave_paused(has_recovery);

        let export_dialog = ui::export_dialog::ExportDialogState {
            output_dir: settings.allowed_export_dir.clone(),
            ..Default::default()
        };
        let settings_dialog = ui::settings_dialog::SettingsDialogState::from_settings(&settings);

        let mut ai_state = ui::inspector::DirectAiUiState::default();
        ai_state.config.provider = settings.ai_provider;
        ai_state.config.model = settings.ai_model.clone();
        ai_state.config.base_url = settings.ai_base_url.clone();
        ai_state.config.api_key = load_api_key_from_keyring(settings.ai_provider);

        let display_light_azimuth = scene.light_azimuth;
        let display_light_elevation = scene.light_elevation;

        Self {
            scene,
            history: HistoryStack::new(),
            camera_3d: OrbitCamera::new(),
            camera_3d_target: None,
            showcase_camera: default_showcase_camera(),
            showcase_camera_target: None,
            display_light_azimuth,
            display_light_elevation,
            camera_2d: PixelCamera2D::new(),
            pixel_drag_start: None,
            gizmo_drag: None,
            scene_gadget_drag: None,
            free_view: true,
            render_preview: false,
            export_dialog,
            settings_dialog,
            mcp_dialog: ui::mcp_dialog::McpDialogState::default(),
            ai_state,
            command_palette: ui::command_palette::CommandPaletteState::default(),
            quick_start: ui::quick_start::QuickStartState {
                open: true,
                has_recovery,
                error_message: None,
                example_thumbs: None,
            },
            about_open: false,
            confirm_new_project: false,
            confirm_exit: false,
            quit_confirmed: false,
            project_files,
            paths,
            settings,
            mcp_token,
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
    offscreen_screenshot_renderer: Option<ViewportRenderer>,
    viewport_texture_id: egui::TextureId,
    last_window_title: String,
}

/// Formats the OS window title showing active `.tessel` filename and unsaved `*` indicator.
pub fn format_window_title(current_path: Option<&std::path::Path>, unsaved: bool) -> String {
    let name = current_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Untitled");
    if unsaved {
        format!("Tessel — {name}*")
    } else {
        format!("Tessel — {name}")
    }
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
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui_ctx.set_fonts(fonts);

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
            offscreen_screenshot_renderer: None,
            viewport_texture_id,
            last_window_title: "Tessel".into(),
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
        let device = &self.device;
        let queue = &self.queue;
        let main_vp = &self.viewport_renderer;
        let offscreen_vp = &mut self.offscreen_screenshot_renderer;
        studio.poll_live_mcp(
            || main_vp.capture_png_bytes(device, queue).ok(),
            |tool, scene, export_dir, args| {
                let vp =
                    offscreen_vp.get_or_insert_with(|| ViewportRenderer::new(device, 640, 480));
                if tool == "viewport_screenshot" {
                    tessel_ai::headless::render_screenshot_mcp_content(
                        vp, device, queue, scene, args,
                    )
                } else {
                    let parsed = tessel_ai::headless::parse_turntable_args(export_dir, args)?;
                    tessel_ai::headless::render_turntable_gif_with_renderer(
                        vp, device, queue, scene, &parsed,
                    )
                }
            },
        );
        studio
            .project_files
            .set_autosave_paused(studio.quick_start.open && studio.quick_start.has_recovery);
        studio.project_files.tick_autosave(&studio.scene);

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut desired_viewport = self.viewport_renderer.size();
        let mut hover_voxel = None;
        let mut active_gizmo_axis = None;
        let mut quick_action = ui::quick_start::QuickStartAction::None;
        let mut top_actions = ui::top_bar::TopBarActions::default();
        let mut toggle_mcp_server = false;
        let mut rotate_mcp = false;
        let mut save_and_quit = false;
        let mut export_action = ui::export_dialog::ExportAction::None;

        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            let dt = ctx.input(|i| i.unstable_dt).clamp(0.001, 0.05);
            if let Some(goal) = studio.camera_3d_target {
                if smooth_step_camera(&mut studio.camera_3d, goal, dt) {
                    ctx.request_repaint();
                } else {
                    studio.camera_3d_target = None;
                }
            }
            if let Some(goal) = studio.showcase_camera_target {
                if smooth_step_camera(&mut studio.showcase_camera, goal, dt) {
                    ctx.request_repaint();
                } else {
                    studio.showcase_camera_target = None;
                }
            }
            if matches!(
                studio.scene_gadget_drag,
                Some(ui::viewport::SceneGadgetDrag::Sun { .. })
            ) {
                studio.display_light_azimuth = studio.scene.light_azimuth;
                studio.display_light_elevation = studio.scene.light_elevation;
            } else if smooth_step_light(
                &mut studio.display_light_azimuth,
                &mut studio.display_light_elevation,
                studio.scene.light_azimuth,
                studio.scene.light_elevation,
                dt,
            ) {
                ctx.request_repaint();
            }

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
                if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::I)) {
                    top_actions.import_mesh = true;
                }
                if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::N)) {
                    top_actions.new_project = true;
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
            top_actions.open_mcp |= bar.open_mcp;
            top_actions.save_project |= bar.save_project;
            top_actions.open_project |= bar.open_project;
            top_actions.new_project |= bar.new_project;
            top_actions.import_mesh |= bar.import_mesh;
            top_actions.open_settings |= bar.open_settings;
            top_actions.open_welcome |= bar.open_welcome;
            top_actions.open_about |= bar.open_about;

            ui::status_bar::show_status_bar(
                ctx,
                &studio.scene,
                &studio.history,
                &studio.camera_3d,
                &studio.camera_2d,
                studio.free_view,
                studio.project_files.has_unsaved_changes(studio.scene.revision),
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
                &mut studio.camera_3d_target,
                &mut studio.showcase_camera,
                &mut studio.showcase_camera_target,
                (studio.display_light_azimuth, studio.display_light_elevation),
                &mut studio.camera_2d,
                &mut studio.pixel_drag_start,
                &mut studio.gizmo_drag,
                &mut studio.scene_gadget_drag,
                &mut studio.free_view,
                &mut studio.render_preview,
            );
            desired_viewport = vp_size;
            hover_voxel = hv;
            active_gizmo_axis = g_axis;
            export_action = ui::export_dialog::show_export_dialog(
                ctx,
                &studio.scene,
                &studio.camera_3d,
                &mut studio.export_dialog,
            );
            ui::settings_dialog::show_settings_dialog(
                ctx,
                &mut studio.settings_dialog,
                &mut studio.settings,
                &studio.paths,
                &mut studio.ai_state.config,
                &mut studio.export_dialog.output_dir,
                studio.live_mcp.as_ref(),
                &mut studio.mcp_dialog.open,
            );
            let mcp_action = ui::mcp_dialog::show_mcp_dialog(
                ctx,
                &mut studio.mcp_dialog,
                studio.live_mcp.as_ref(),
                &studio.mcp_token,
                &studio.export_dialog.output_dir,
            );
            toggle_mcp_server |= mcp_action.toggle_server;
            rotate_mcp |= mcp_action.rotate_token;
            quick_action = ui::quick_start::show_quick_start(
                ctx,
                &mut studio.quick_start,
                &studio.settings.recent_projects,
            );
            ui::quick_start::show_about_dialog(ctx, &mut studio.about_open);
            ui::command_palette::show_command_palette(
                ctx,
                &mut studio.command_palette,
                &mut studio.scene,
                &mut studio.history,
                &mut studio.camera_3d,
                &mut studio.camera_2d,
                &mut studio.export_dialog.open,
                &mut top_actions.import_mesh,
                &mut top_actions.new_project,
            );
            if top_actions.new_project {
                studio.confirm_new_project = true;
                top_actions.new_project = false;
            }
            if studio.confirm_new_project {
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    studio.confirm_new_project = false;
                } else {
                    let mut open = studio.confirm_new_project;
                    egui::Window::new("New Project")
                        .open(&mut open)
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .fixed_size(egui::vec2(360.0, 120.0))
                        .show(ctx, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Are you sure you want to start a new empty project? Any unsaved changes in the current scene will be lost.",
                                )
                                .size(12.0)
                                .color(ui::theme::tokens::ON_SURFACE),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                let confirm_btn = egui::Button::new(
                                    egui::RichText::new("Start New Project")
                                        .strong()
                                        .color(ui::theme::tokens::ON_PRIMARY),
                                )
                                .fill(ui::theme::tokens::ERROR)
                                .corner_radius(egui::CornerRadius::same(4));
                                if ui.add(confirm_btn).clicked() {
                                    top_actions.new_project = true;
                                    studio.confirm_new_project = false;
                                }
                                if ui.button("Cancel").clicked() {
                                    studio.confirm_new_project = false;
                                }
                            });
                        });
                    studio.confirm_new_project = open && studio.confirm_new_project;
                }
            }
            if studio.confirm_exit {
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    studio.confirm_exit = false;
                } else {
                    let mut open = studio.confirm_exit;
                    egui::Window::new("Unsaved Changes")
                        .open(&mut open)
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .fixed_size(egui::vec2(380.0, 125.0))
                        .show(ctx, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "You have unsaved changes in this project. Would you like to save before closing Tessel?",
                                )
                                .size(12.0)
                                .color(ui::theme::tokens::ON_SURFACE),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                let save_btn = egui::Button::new(
                                    egui::RichText::new("Save & Quit")
                                        .strong()
                                        .color(ui::theme::tokens::ON_PRIMARY),
                                )
                                .fill(ui::theme::tokens::PRIMARY_CONTAINER)
                                .corner_radius(egui::CornerRadius::same(4));
                                if ui.add(save_btn).clicked() {
                                    save_and_quit = true;
                                    studio.confirm_exit = false;
                                }
                                let discard_btn = egui::Button::new(
                                    egui::RichText::new("Discard & Quit")
                                        .strong()
                                        .color(ui::theme::tokens::ON_PRIMARY),
                                )
                                .fill(ui::theme::tokens::ERROR)
                                .corner_radius(egui::CornerRadius::same(4));
                                if ui.add(discard_btn).clicked() {
                                    studio.quit_confirmed = true;
                                    studio.confirm_exit = false;
                                }
                                if ui.button("Cancel").clicked() {
                                    studio.confirm_exit = false;
                                }
                            });
                        });
                    studio.confirm_exit = open && studio.confirm_exit;
                }
            }
        });

        match quick_action {
            ui::quick_start::QuickStartAction::OpenExample(example) => {
                match example.load_copy() {
                    Ok(loaded) => {
                        studio.scene = loaded;
                        studio.history = HistoryStack::new();
                        studio.camera_3d_target = None;
                        studio.showcase_camera_target = None;
                        studio.display_light_azimuth = studio.scene.light_azimuth;
                        studio.display_light_elevation = studio.scene.light_elevation;
                        studio.camera_2d.reset();
                        if let Some((target, yaw_deg, pitch_deg, distance)) = example.camera_pose()
                        {
                            studio.camera_3d.target = glam::Vec3::from(target);
                            studio.camera_3d.yaw = yaw_deg.to_radians();
                            studio.camera_3d.pitch = pitch_deg.to_radians();
                            studio.camera_3d.distance = distance;
                        } else {
                            studio.camera_3d.reset();
                        }
                        studio.pixel_drag_start = None;
                        studio.gizmo_drag = None;
                        studio.scene_gadget_drag = None;
                        studio.export_dialog.status_message = None;
                        // Keep current_path = None so Ctrl+S always prompts Save As
                        // and never overwrites the original example file.
                        studio.project_files.current_path = None;
                        studio
                            .project_files
                            .mark_baseline_revision(studio.scene.revision);
                    }
                    Err(err) => {
                        studio.quick_start.open = true;
                        studio.quick_start.error_message = Some(err);
                    }
                }
            }
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

        if let ui::export_dialog::ExportAction::TurntableGif(args) = export_action {
            let vp = self.offscreen_screenshot_renderer.get_or_insert_with(|| {
                ViewportRenderer::new(&self.device, args.width, args.height)
            });
            match tessel_ai::headless::render_turntable_gif_with_renderer(
                vp,
                &self.device,
                &self.queue,
                &studio.scene,
                &args,
            ) {
                Ok(val) => {
                    let path = val
                        .get("output_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("turntable.gif");
                    let bytes = val
                        .get("file_size_bytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    studio.export_dialog.status_message = Some(Ok(format!(
                        "Saved {path} ({bytes} bytes, {} frames @ {} fps)",
                        args.frames, args.fps
                    )));
                }
                Err(err) => {
                    studio.export_dialog.status_message = Some(Err(err.to_string()));
                }
            }
        }

        if top_actions.open_settings {
            studio.settings_dialog.open = true;
        }
        if top_actions.open_welcome {
            studio.quick_start.error_message = None;
            studio.quick_start.open = true;
        }
        if top_actions.open_about {
            studio.about_open = true;
        }
        if top_actions.import_mesh {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("3D Mesh (.obj, .glb)", &["obj", "glb"])
                .pick_file()
            {
                match tessel_io::import_mesh_from_path(&path, studio.scene.active_color) {
                    Ok((name, mesh)) => {
                        let _ = studio.history.execute(
                            &mut studio.scene,
                            Command::Transaction(vec![
                                Command::ImportLowPolyMesh { name, mesh },
                                Command::SetMode(tessel_core::AppMode::LowPoly),
                            ]),
                        );
                    }
                    Err(err) => tracing::error!("Import 3D mesh failed: {err}"),
                }
            }
        }
        if top_actions.open_mcp {
            studio.mcp_dialog.open = true;
        }
        if toggle_mcp_server {
            studio.toggle_live_mcp();
        }
        if rotate_mcp {
            let next = rotate_mcp_token(&studio.paths);
            studio.mcp_token = next.clone();
            if let Some(handle) = studio.live_mcp.as_mut() {
                handle.set_token(next);
            }
            studio.mcp_dialog.status_message =
                Some("Rotated MCP token. Update your client config.".into());
        }
        if top_actions.save_project || save_and_quit {
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
                    if save_and_quit {
                        studio.quit_confirmed = true;
                    }
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
        if top_actions.new_project {
            studio.scene = SceneState::new();
            studio.history = HistoryStack::new();
            studio.camera_3d.reset();
            studio.camera_3d_target = None;
            studio.showcase_camera = default_showcase_camera();
            studio.showcase_camera_target = None;
            studio.display_light_azimuth = studio.scene.light_azimuth;
            studio.display_light_elevation = studio.scene.light_elevation;
            studio.camera_2d.reset();
            studio.pixel_drag_start = None;
            studio.gizmo_drag = None;
            studio.scene_gadget_drag = None;
            studio.export_dialog.status_message = None;
            studio.project_files.current_path = None;
            studio
                .project_files
                .mark_baseline_revision(studio.scene.revision);
        }

        let desired_title = format_window_title(
            studio.project_files.current_path.as_deref(),
            studio
                .project_files
                .has_unsaved_changes(studio.scene.revision),
        );
        if desired_title != self.last_window_title {
            self.window.set_title(&desired_title);
            self.last_window_title = desired_title;
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

        self.viewport_renderer.render_with_light(
            &self.queue,
            &mut encoder,
            &studio.scene,
            &studio.camera_3d,
            &studio.camera_2d,
            self.window.scale_factor() as f32,
            studio.render_preview,
            studio.display_light_azimuth,
            studio.display_light_elevation,
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

        let mut attrs = Window::default_attributes()
            .with_title("Tessel")
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(800.0, 520.0));
        if let Some(icon) = crate::icon::load_window_icon() {
            attrs = attrs.with_window_icon(Some(icon));
        }

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
            WindowEvent::CloseRequested => {
                if self
                    .studio
                    .project_files
                    .has_unsaved_changes(self.studio.scene.revision)
                    && !self.studio.quit_confirmed
                {
                    self.studio.confirm_exit = true;
                    gpu.window.request_redraw();
                } else {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                gpu.resize_surface(size.width, size.height);
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = gpu.render(&mut self.studio) {
                    tracing::error!("render error: {err}");
                }
                if self.studio.quit_confirmed {
                    event_loop.exit();
                    return;
                }
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_window_title_shows_filename_and_dirty_star() {
        assert_eq!(format_window_title(None, false), "Tessel — Untitled");
        assert_eq!(format_window_title(None, true), "Tessel — Untitled*");
        let path = std::path::Path::new("/tmp/hero.tessel");
        assert_eq!(
            format_window_title(Some(path), false),
            "Tessel — hero.tessel"
        );
        assert_eq!(
            format_window_title(Some(path), true),
            "Tessel — hero.tessel*"
        );
    }

    #[test]
    fn camera_tools_drive_viewport_state() {
        let mut cam = OrbitCamera::new();
        let mut scene_cam = default_showcase_camera();
        assert!((cam.distance - 9.0).abs() < 1e-6);

        // Absolute zoom + readback.
        let res = handle_camera_tool_call(
            "camera_zoom",
            &serde_json::json!({ "distance": 3.0 }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!((res["camera"]["distance"].as_f64().unwrap() - 3.0).abs() < 1e-4);

        // Factor zoom in halves the distance.
        handle_camera_tool_call(
            "camera_zoom",
            &serde_json::json!({ "factor": 0.5 }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!((cam.distance - 1.5).abs() < 1e-4);

        // Look-at + orbit + pan move state observably.
        handle_camera_tool_call(
            "camera_look_at",
            &serde_json::json!({ "target": [1.0, 0.5, -2.0] }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert_eq!(cam.target, glam::Vec3::new(1.0, 0.5, -2.0));
        let before = (cam.yaw, cam.pitch);
        handle_camera_tool_call(
            "camera_orbit",
            &serde_json::json!({ "dx": 40.0, "dy": -15.0 }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!((cam.yaw - before.0).abs() > 1e-6);
        assert!((cam.pitch - before.1).abs() > 1e-6);
        let target_before = cam.target;
        handle_camera_tool_call(
            "camera_pan",
            &serde_json::json!({ "dx": 30.0, "dy": 10.0 }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!((cam.target - target_before).length() > 1e-6);

        // Preset + reset restore defaults.
        handle_camera_tool_call(
            "camera_preset",
            &serde_json::json!({ "name": "front" }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!(cam.yaw.abs() < 1e-6);
        handle_camera_tool_call(
            "camera_reset",
            &serde_json::json!({}),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert!((cam.distance - 9.0).abs() < 1e-6);

        // Scene/projector camera routing + camera_set_pose.
        let pose_res = handle_camera_tool_call(
            "camera_set_pose",
            &serde_json::json!({
                "camera": "scene",
                "target": [0.0, 1.2, 0.0],
                "yaw_deg": 90.0,
                "pitch_deg": 20.0,
                "distance": 5.0
            }),
            &mut cam,
            &mut scene_cam,
        )
        .unwrap();
        assert_eq!(pose_res["target_camera"], "scene");
        assert!((scene_cam.yaw - 90.0_f32.to_radians()).abs() < 1e-5);
        assert!((scene_cam.distance - 5.0).abs() < 1e-5);
        // Viewport camera remained untouched.
        assert!((cam.distance - 9.0).abs() < 1e-6);
    }

    #[test]
    fn smooth_step_camera_and_light_converge() {
        let mut cam = OrbitCamera::new();
        let mut goal = OrbitCamera::new();
        goal.yaw = 2.4;
        goal.pitch = 0.8;
        goal.distance = 4.0;
        goal.target = glam::Vec3::new(1.0, 1.5, -0.5);
        let mut moving = true;
        for _ in 0..120 {
            moving = smooth_step_camera(&mut cam, goal, 0.016);
            if !moving {
                break;
            }
        }
        assert!(!moving);
        assert!((cam.distance - goal.distance).abs() < 1e-4);

        let mut az = 0.1_f32;
        let mut el = 0.4_f32;
        let mut light_moving = true;
        for _ in 0..120 {
            light_moving = smooth_step_light(&mut az, &mut el, 5.9, 1.1, 0.016);
            if !light_moving {
                break;
            }
        }
        assert!(!light_moving);
        assert!((el - 1.1).abs() < 1e-4);
    }

    #[test]
    fn camera_tools_reject_bad_input() {
        let mut cam = OrbitCamera::new();
        let mut scene_cam = default_showcase_camera();
        for (name, v) in [
            ("camera_zoom", serde_json::json!({})),
            ("camera_zoom", serde_json::json!({ "factor": -1.0 })),
            ("camera_zoom", serde_json::json!({ "distance": 0.0 })),
            ("camera_orbit", serde_json::json!({ "dx": 1.0 })),
            (
                "camera_look_at",
                serde_json::json!({ "target": [0.0, 0.0] }),
            ),
            (
                "camera_preset",
                serde_json::json!({ "name": "upside-down" }),
            ),
            ("camera_preset", serde_json::json!({})),
            ("camera_set_pose", serde_json::json!({})),
            ("camera_set_pose", serde_json::json!({ "distance": 0.0 })),
            ("camera_get", serde_json::json!({ "camera": "invalid" })),
            ("camera_nope", serde_json::json!({})),
        ] {
            assert!(
                handle_camera_tool_call(name, &v, &mut cam, &mut scene_cam).is_err(),
                "{name} should be rejected"
            );
        }
        // Failed calls leave the camera untouched.
        assert!((cam.distance - 9.0).abs() < 1e-6);
    }
}
