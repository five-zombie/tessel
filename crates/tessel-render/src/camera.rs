use glam::{Mat4, Vec3};

use crate::vertex::CameraUniform;

const MIN_PITCH: f32 = -1.52;
const MAX_PITCH: f32 = 1.52;
const MIN_DISTANCE: f32 = 1.0;
const MAX_DISTANCE: f32 = 120.0;

/// Right-Handed Y-Up orbit camera for 3D viewport navigation.
#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl OrbitCamera {
    pub fn new() -> Self {
        Self {
            target: Vec3::new(0.0, 0.8, 0.0),
            yaw: 0.72,
            pitch: 0.45,
            distance: 9.0,
            fov_y_radians: 45.0_f32.to_radians(),
            near: 0.05,
            far: 200.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn position(&self) -> Vec3 {
        let cos_p = self.pitch.cos();
        let offset = Vec3::new(
            self.distance * cos_p * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            self.distance * cos_p * self.yaw.cos(),
        );
        self.target + offset
    }

    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw -= delta_x * 0.008;
        self.pitch = (self.pitch + delta_y * 0.008).clamp(MIN_PITCH, MAX_PITCH);
    }

    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        let forward = (self.target - self.position()).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let scale = self.distance * 0.0018;
        self.target += (-right * delta_x + up * delta_y) * scale;
    }

    pub fn zoom(&mut self, scroll_delta: f32) {
        let factor = (1.0 - scroll_delta * 0.0015).clamp(0.7, 1.3);
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let safe_aspect = aspect.max(0.01);
        let view = Mat4::look_at_rh(self.position(), self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y_radians, safe_aspect, self.near, self.far);
        proj * view
    }

    pub fn to_uniform(&self, aspect: f32) -> CameraUniform {
        let vp = self.view_proj(aspect);
        let inv_vp = vp.inverse();
        let pos = self.position();
        CameraUniform {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: inv_vp.to_cols_array_2d(),
            camera_pos: [pos.x, pos.y, pos.z, 1.0],
        }
    }

    /// Computes a world-space picking ray `(origin, direction)` from normalized screen coords
    /// `uv` in `[0.0, 1.0]` (top-left origin).
    pub fn screen_ray(&self, uv: [f32; 2], aspect: f32) -> (Vec3, Vec3) {
        let ndc_x = uv[0] * 2.0 - 1.0;
        let ndc_y = 1.0 - uv[1] * 2.0;
        let inv_vp = self.view_proj(aspect).inverse();
        let near = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_matrices_are_finite_and_invertible() {
        let mut cam = OrbitCamera::new();
        cam.orbit(40.0, -25.0);
        cam.pan(12.0, 8.0);
        cam.zoom(120.0);

        let vp = cam.view_proj(16.0 / 9.0);
        assert!(vp.is_finite());
        assert!(vp.inverse().is_finite());
        assert!(cam.distance >= MIN_DISTANCE && cam.distance <= MAX_DISTANCE);
    }
}
