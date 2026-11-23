use tessel_core::SceneState;
use tessel_render::{OrbitCamera, PixelCamera2D, ViewportRenderer};

use crate::error::AiError;

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
        self.viewport.render(
            &self.queue,
            &mut encoder,
            scene,
            cam,
            &PixelCamera2D::new(),
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
    if let Some(t) = args.get("target") {
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
            None => Ok(None),
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
    if let Some(yaw) = num("yaw")? {
        cam.yaw = yaw;
    }
    if let Some(pitch) = num("pitch")? {
        cam.pitch = pitch.clamp(-1.52, 1.52);
    }
    if let Some(distance) = num("distance")? {
        if distance <= 0.0 {
            return Err(AiError::InvalidArguments(
                "screenshot 'distance' must be > 0".into(),
            ));
        }
        cam.distance = distance.clamp(1.0, 120.0);
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
            "distance": 0.5,
        }))
        .unwrap();
        assert_eq!(cam.target, glam::Vec3::new(1.0, 2.0, 3.0));
        assert!((cam.yaw - 1.0).abs() < 1e-6);
        assert!((cam.pitch - 1.52).abs() < 1e-4, "pitch clamps");
        assert!((cam.distance - 1.0).abs() < 1e-6, "distance clamps to min");
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
}
