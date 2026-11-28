use base64::Engine as _;
use serde_json::{json, Value};
use tessel_core::SceneState;
use tessel_render::{OrbitCamera, PixelCamera2D, ViewportRenderer};

use crate::error::AiError;

fn parse_opt_u32_clamped(
    args: &Value,
    field: &str,
    default: u32,
    min: u32,
    max: u32,
) -> Result<u32, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                AiError::InvalidArguments(format!("'{field}' must be a non-negative integer"))
            })?;
            let clamped = n.clamp(u64::from(min), u64::from(max)) as u32;
            Ok(clamped)
        }
    }
}

fn parse_opt_bool_default(args: &Value, field: &str, default: bool) -> Result<bool, AiError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(AiError::InvalidArguments(format!(
            "'{field}' must be a boolean"
        ))),
    }
}

/// Validated parameters for `viewport_screenshot`.
pub struct ScreenshotArgs {
    pub width: u32,
    pub height: u32,
    pub preview: bool,
    pub camera: OrbitCamera,
}

/// Parses and strictly validates `viewport_screenshot` arguments.
pub fn parse_screenshot_args(args: &Value) -> Result<ScreenshotArgs, AiError> {
    let width = parse_opt_u32_clamped(args, "width", 640, 16, 1024)?;
    let height = parse_opt_u32_clamped(args, "height", 480, 16, 1024)?;
    let preview = parse_opt_bool_default(args, "preview", false)?;
    let camera = screenshot_camera(args)?;
    Ok(ScreenshotArgs {
        width,
        height,
        preview,
        camera,
    })
}

/// Builds a `PixelCamera2D` fitted to the active pixel canvas (if in Pixel mode) so
/// headless screenshots capture full 4K canvases instead of a cropped corner.
fn pixel_camera_for_scene(scene: &SceneState, width: u32, height: u32) -> PixelCamera2D {
    let mut cam = PixelCamera2D::new();
    if scene.active_mode == tessel_core::AppMode::Pixel {
        cam.fit_active_canvas([width as f32, height as f32], scene);
    }
    cam
}

/// Renders `scene` offscreen using any `wgpu::Device`/`Queue` + `ViewportRenderer` and returns
/// the MCP `content` array (text + sRGB PNG image block). Shared by stdio MCP and live GUI HTTP MCP.
pub fn render_screenshot_mcp_content(
    viewport: &mut ViewportRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneState,
    args: &Value,
) -> Result<Value, AiError> {
    let parsed = parse_screenshot_args(args)?;
    let w = parsed.width;
    let h = parsed.height;
    let preview = parsed.preview;
    let cam = parsed.camera;
    let cam_2d = pixel_camera_for_scene(scene, w, h);
    viewport.resize(device, w, h);
    viewport.sync_scene(device, queue, scene, None, None);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tessel-offscreen-screenshot"),
    });
    viewport.render(queue, &mut encoder, scene, &cam, &cam_2d, 1.0, preview);
    queue.submit(Some(encoder.finish()));
    let png = viewport
        .capture_png_bytes(device, queue)
        .map_err(|e| AiError::InvalidArguments(format!("viewport capture failed: {e}")))?;
    Ok(json!([
        { "type": "text", "text": format!("viewport {w}x{h} sRGB PNG") },
        { "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png" },
    ]))
}

/// Parsed parameters for a 3D turntable GIF render (validated before touching any GPU).
pub struct TurntableArgs {
    pub target_path: std::path::PathBuf,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub fps: u32,
    pub preview: bool,
    pub clip_name: Option<String>,
    pub clip_index: Option<usize>,
    pub camera: OrbitCamera,
}

/// Validates `viewport_render_turntable_gif` arguments and output path before GPU allocation.
pub fn parse_turntable_args(
    allowed_export_dir: &std::path::Path,
    args: &Value,
) -> Result<TurntableArgs, AiError> {
    let filename = args
        .get("filename")
        .and_then(Value::as_str)
        .ok_or_else(|| AiError::InvalidArguments("missing 'filename'".into()))?;
    let target_path = tessel_io::resolve_safe_export_path(allowed_export_dir, filename)?;
    let width = parse_opt_u32_clamped(args, "width", 320, 64, 512)?;
    let height = parse_opt_u32_clamped(args, "height", 320, 64, 512)?;
    let frames = parse_opt_u32_clamped(args, "frames", 16, 4, 36)?;
    let fps = parse_opt_u32_clamped(args, "fps", 12, 2, 30)?;
    let preview = parse_opt_bool_default(args, "preview", true)?;
    let clip_name = match args.get("clip_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(AiError::InvalidArguments(
                "'clip_name' must be a string".into(),
            ))
        }
    };
    let clip_index = match args.get("clip_index") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| {
                AiError::InvalidArguments("'clip_index' must be a non-negative integer".into())
            })?;
            Some(
                usize::try_from(raw)
                    .map_err(|_| AiError::InvalidArguments("'clip_index' out of range".into()))?,
            )
        }
    };
    let camera = screenshot_camera(args)?;
    Ok(TurntableArgs {
        target_path,
        width,
        height,
        frames,
        fps,
        preview,
        clip_name,
        clip_index,
        camera,
    })
}

/// Renders a 360-degree animated turntable `.gif` offscreen and writes it to `parsed.target_path`.
pub fn render_turntable_gif_with_renderer(
    viewport: &mut ViewportRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneState,
    parsed: &TurntableArgs,
) -> Result<Value, AiError> {
    if let Some(parent) = parsed.target_path.parent() {
        std::fs::create_dir_all(parent).map_err(tessel_io::IoError::from)?;
    }

    let mut work_scene = scene.clone();
    let active_clip_duration: Option<f32> = work_scene.character_rig.as_mut().and_then(|rig| {
        if let Some(ci) = parsed.clip_index {
            if ci < rig.animations.len() {
                rig.active_animation = ci;
                return Some(rig.animations[ci].duration_secs);
            }
            return None;
        }
        if let Some(ref cname) = parsed.clip_name {
            let idx = rig.animations.iter().position(|c| c.name == *cname)?;
            rig.active_animation = idx;
            return Some(rig.animations[idx].duration_secs);
        }
        if !rig.clip_layers.is_empty() {
            let dur = rig
                .clip_layers
                .iter()
                .filter_map(|l| rig.animations.get(l.clip_index).map(|c| c.duration_secs))
                .fold(0.0_f32, f32::max);
            if dur > 0.0 {
                return Some(dur);
            }
        }
        None
    });

    viewport.resize(device, parsed.width, parsed.height);
    let mut gif_bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(
            &mut gif_bytes,
            parsed.width as u16,
            parsed.height as u16,
            &[],
        )
        .map_err(|e| AiError::InvalidArguments(format!("gif encoder init failed: {e}")))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| AiError::InvalidArguments(format!("gif repeat failed: {e}")))?;

        let delay_cs = (100 / parsed.fps).max(3) as u16;
        let base_yaw = parsed.camera.yaw;
        for i in 0..parsed.frames {
            let frac = i as f32 / parsed.frames as f32;
            let mut cam = parsed.camera;
            cam.yaw = base_yaw + frac * std::f32::consts::TAU;
            if let Some(dur) = active_clip_duration {
                if let Some(rig) = work_scene.character_rig.as_mut() {
                    rig.playback_time = frac * dur;
                    rig.apply_active_animation();
                    work_scene.mark_modified();
                }
            }
            viewport.sync_scene(device, queue, &work_scene, None, None);
            work_scene.dirty_viewport = false;
            let mut cmd = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tessel-turntable-frame"),
            });
            let cam_2d = pixel_camera_for_scene(&work_scene, parsed.width, parsed.height);
            viewport.render(
                queue,
                &mut cmd,
                &work_scene,
                &cam,
                &cam_2d,
                1.0,
                parsed.preview,
            );
            queue.submit(Some(cmd.finish()));
            let (w, h, mut rgba) = viewport
                .capture_rgba_bytes(device, queue)
                .map_err(|e| AiError::InvalidArguments(format!("turntable capture failed: {e}")))?;
            let mut frame = gif::Frame::from_rgba_speed(w as u16, h as u16, &mut rgba, 10);
            frame.delay = delay_cs;
            encoder
                .write_frame(&frame)
                .map_err(|e| AiError::InvalidArguments(format!("gif frame encode failed: {e}")))?;
        }
    }

    let size = gif_bytes.len();
    std::fs::write(&parsed.target_path, gif_bytes).map_err(tessel_io::IoError::from)?;
    Ok(json!({
        "ok": true,
        "output_path": parsed.target_path.display().to_string(),
        "file_size_bytes": size,
        "width": parsed.width,
        "height": parsed.height,
        "frames": parsed.frames,
        "fps": parsed.fps
    }))
}

impl HeadlessRenderer {
    /// Renders a 360-degree animated turntable `.gif` using this headless GPU context.
    pub fn render_turntable_gif(
        &mut self,
        scene: &SceneState,
        parsed: &TurntableArgs,
    ) -> Result<Value, AiError> {
        render_turntable_gif_with_renderer(
            &mut self.viewport,
            &self.device,
            &self.queue,
            scene,
            parsed,
        )
    }
}

/// Owned headless GPU context for agent screenshots. Created lazily on the
/// first `viewport_screenshot` call so servers without a GPU keep working
/// for every other tool.
pub struct HeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    viewport: ViewportRenderer,
}

impl HeadlessRenderer {
    pub fn new(width: u32, height: u32) -> Result<Self, AiError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            },
        ))
        .or_else(|| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: true,
            }))
        })
        .ok_or_else(|| {
            AiError::InvalidArguments(
                "viewport_screenshot needs a GPU adapter (none found; software GL like lavapipe also works)".into(),
            )
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("tessel-headless-mcp"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .map_err(|e| AiError::InvalidArguments(format!("GPU device request failed: {e}")))?;
        let viewport = ViewportRenderer::new(&device, width.max(16), height.max(16));
        Ok(Self {
            device,
            queue,
            viewport,
        })
    }

    /// Renders `scene` from `cam` into PNG bytes (sRGB, unpremultiplied RGBA).
    pub fn render_png(
        &mut self,
        scene: &SceneState,
        cam: &OrbitCamera,
        width: u32,
        height: u32,
        render_preview: bool,
    ) -> Result<Vec<u8>, AiError> {
        let w = width.clamp(16, 1024);
        let h = height.clamp(16, 1024);
        self.viewport.resize(&self.device, w, h);
        self.viewport
            .sync_scene(&self.device, &self.queue, scene, None, None);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tessel-headless-frame"),
            });
        let cam_2d = pixel_camera_for_scene(scene, w, h);
        self.viewport.render(
            &self.queue,
            &mut encoder,
            scene,
            cam,
            &cam_2d,
            1.0,
            render_preview,
        );
        self.queue.submit(Some(encoder.finish()));
        self.viewport
            .capture_png_bytes(&self.device, &self.queue)
            .map_err(|e| AiError::InvalidArguments(format!("viewport capture failed: {e}")))
    }
}

/// Builds an [`OrbitCamera`] from screenshot arguments. Missing fields fall
/// back to a framing that suits ~1-4 unit subjects (closer than the GUI's
/// wide default).
pub fn screenshot_camera(args: &serde_json::Value) -> Result<OrbitCamera, AiError> {
    let mut cam = OrbitCamera::new();
    cam.distance = 3.0;
    cam.target = glam::Vec3::new(0.0, 0.5, 0.0);
    if let Some(t) = args.get("target").filter(|v| !v.is_null()) {
        let arr = t.as_array().ok_or_else(|| {
            AiError::InvalidArguments("screenshot 'target' must be [x, y, z]".into())
        })?;
        if arr.len() != 3 {
            return Err(AiError::InvalidArguments(
                "screenshot 'target' must be [x, y, z]".into(),
            ));
        }
        let mut p = [0.0f32; 3];
        for (i, v) in arr.iter().enumerate() {
            p[i] = v.as_f64().ok_or_else(|| {
                AiError::InvalidArguments("screenshot 'target' must be numbers".into())
            })? as f32;
        }
        let v = glam::Vec3::from_array(p);
        if !v.is_finite() {
            return Err(AiError::InvalidArguments(
                "screenshot 'target' must be finite".into(),
            ));
        }
        cam.target = v;
    }
    let num = |key: &str| -> Result<Option<f32>, AiError> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let n = v.as_f64().ok_or_else(|| {
                    AiError::InvalidArguments(format!("screenshot '{key}' must be a number"))
                })? as f32;
                if !n.is_finite() {
                    return Err(AiError::InvalidArguments(format!(
                        "screenshot '{key}' must be finite"
                    )));
                }
                Ok(Some(n))
            }
        }
    };
    if let Some(yaw_deg) = num("yaw_deg")? {
        cam.yaw = yaw_deg.to_radians();
    } else if let Some(yaw) = num("yaw")? {
        cam.yaw = yaw;
    }
    if let Some(pitch_deg) = num("pitch_deg")? {
        cam.pitch = pitch_deg.to_radians().clamp(-1.52, 1.52);
    } else if let Some(pitch) = num("pitch")? {
        cam.pitch = pitch.clamp(-1.52, 1.52);
    }
    if let Some(distance) = num("distance")? {
        if distance <= 0.0 {
            return Err(AiError::InvalidArguments(
                "screenshot 'distance' must be > 0".into(),
            ));
        }
        cam.distance = distance.clamp(0.25, 120.0);
    }
    if parse_opt_bool_default(args, "telephoto", false)? {
        cam.set_telephoto();
    }
    if parse_opt_bool_default(args, "orthographic", false)? {
        cam.set_orthographic(true);
    }
    Ok(cam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_camera_defaults_frame_small_subjects() {
        let cam = screenshot_camera(&serde_json::json!({})).unwrap();
        assert!((cam.distance - 3.0).abs() < 1e-6);
        assert_eq!(cam.target, glam::Vec3::new(0.0, 0.5, 0.0));
    }

    #[test]
    fn screenshot_camera_honors_and_clamps_args() {
        let cam = screenshot_camera(&serde_json::json!({
            "target": [1.0, 2.0, 3.0],
            "yaw": 1.0,
            "pitch": 99.0,
            "distance": 0.1,
        }))
        .unwrap();
        assert_eq!(cam.target, glam::Vec3::new(1.0, 2.0, 3.0));
        assert!((cam.yaw - 1.0).abs() < 1e-6);
        assert!((cam.pitch - 1.52).abs() < 1e-4, "pitch clamps");
        assert!((cam.distance - 0.25).abs() < 1e-6, "distance clamps to min");

        let cam_deg = screenshot_camera(&serde_json::json!({
            "yaw_deg": 45.0,
            "pitch_deg": 30.0,
            "telephoto": true,
        }))
        .unwrap();
        assert!((cam_deg.yaw - 45.0_f32.to_radians()).abs() < 1e-5);
        assert!((cam_deg.pitch - 30.0_f32.to_radians()).abs() < 1e-5);
        assert!(!cam_deg.orthographic);
        assert!((cam_deg.fov_y_radians.to_degrees() - 22.0).abs() < 1e-4);

        let cam_ortho = screenshot_camera(&serde_json::json!({
            "orthographic": true,
        }))
        .unwrap();
        assert!(cam_ortho.orthographic);
    }

    #[test]
    fn screenshot_camera_rejects_bad_input() {
        for bad in [
            serde_json::json!({ "target": [0.0, 0.0] }),
            serde_json::json!({ "target": ["x", 0.0, 0.0] }),
            serde_json::json!({ "target": [0.0, 0.0, f64::NAN] }),
            serde_json::json!({ "distance": -2.0 }),
            serde_json::json!({ "distance": 0.0 }),
        ] {
            assert!(screenshot_camera(&bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn parse_turntable_args_clamps_and_blocks_traversal() {
        let dir = std::env::temp_dir();
        assert!(parse_turntable_args(&dir, &json!({ "filename": "../escape.gif" })).is_err());
        assert!(parse_turntable_args(&dir, &json!({})).is_err());

        let ok = parse_turntable_args(
            &dir,
            &json!({
                "filename": "spin.gif",
                "width": 9999,
                "height": 10,
                "frames": 100,
                "fps": 1,
                "clip_name": "Walk"
            }),
        )
        .unwrap();
        assert_eq!(ok.width, 512);
        assert_eq!(ok.height, 64);
        assert_eq!(ok.frames, 36);
        assert_eq!(ok.fps, 2);
        assert_eq!(ok.clip_name.as_deref(), Some("Walk"));
    }
}
