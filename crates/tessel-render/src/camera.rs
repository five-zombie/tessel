use glam::{Mat4, Vec3};

use crate::vertex::CameraUniform;

const MIN_PITCH: f32 = -1.52;
const MAX_PITCH: f32 = 1.52;
const MIN_DISTANCE: f32 = 0.25;
const MAX_DISTANCE: f32 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub const fn dir(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPreset {
    Iso,
    Front,
    Back,
    Right,
    Left,
    Top,
}

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
    pub orthographic: bool,
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
            orthographic: false,
        }
    }

    pub fn set_orthographic(&mut self, orthographic: bool) {
        self.orthographic = orthographic;
    }

    pub fn set_telephoto(&mut self) {
        self.orthographic = false;
        self.fov_y_radians = 22.0_f32.to_radians();
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

    pub fn zoom_step(&mut self, multiplier: f32) {
        self.distance = (self.distance * multiplier).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn set_preset(&mut self, preset: CameraPreset) {
        match preset {
            CameraPreset::Iso => {
                self.yaw = 0.72;
                self.pitch = 0.45;
            }
            CameraPreset::Front => {
                self.yaw = 0.0;
                self.pitch = 0.05;
            }
            CameraPreset::Back => {
                self.yaw = std::f32::consts::PI;
                self.pitch = 0.05;
            }
            CameraPreset::Right => {
                self.yaw = std::f32::consts::FRAC_PI_2;
                self.pitch = 0.05;
            }
            CameraPreset::Left => {
                self.yaw = -std::f32::consts::FRAC_PI_2;
                self.pitch = 0.05;
            }
            CameraPreset::Top => {
                self.yaw = 0.0;
                self.pitch = MAX_PITCH;
            }
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let safe_aspect = aspect.max(0.01);
        let view = Mat4::look_at_rh(self.position(), self.target, Vec3::Y);
        let proj = if self.orthographic {
            let half_h = (self.distance * (self.fov_y_radians * 0.5).tan()).max(0.25);
            let half_w = half_h * safe_aspect;
            Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, -self.far * 0.5, self.far)
        } else {
            Mat4::perspective_rh(self.fov_y_radians, safe_aspect, self.near, self.far)
        };
        proj * view
    }

    pub fn to_uniform(
        &self,
        aspect: f32,
        light_azimuth: f32,
        light_elevation: f32,
    ) -> CameraUniform {
        let vp = self.view_proj(aspect);
        let inv_vp = vp.inverse();
        let pos = self.position();
        let cos_e = light_elevation.cos();
        let light_dir = Vec3::new(
            cos_e * light_azimuth.sin(),
            light_elevation.sin(),
            cos_e * light_azimuth.cos(),
        )
        .normalize_or_zero();
        CameraUniform {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: inv_vp.to_cols_array_2d(),
            camera_pos: [pos.x, pos.y, pos.z, 1.0],
            light_dir: [light_dir.x, light_dir.y, light_dir.z, 0.0],
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

/// Closest parameter `s` along line `(line_origin + s * line_dir)` to ray `(ray_origin + t * ray_dir)`.
pub fn closest_point_on_axis(
    ray_origin: Vec3,
    ray_dir: Vec3,
    line_origin: Vec3,
    line_dir: Vec3,
) -> Option<(f32, f32)> {
    let w0 = line_origin - ray_origin;
    let a = line_dir.dot(line_dir);
    let b = line_dir.dot(ray_dir);
    let c = ray_dir.dot(ray_dir);
    let d = line_dir.dot(w0);
    let e = ray_dir.dot(w0);
    let denom = a * c - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    let s = (b * e - c * d) / denom;
    let t = (a * e - b * d) / denom;
    if t < 0.0 {
        return None;
    }
    let p_line = line_origin + line_dir * s;
    let p_ray = ray_origin + ray_dir * t;
    Some((s, (p_line - p_ray).length()))
}

/// Picks the closest X/Y/Z move gizmo axis handle at `gizmo_origin`.
pub fn pick_move_gizmo(
    ray_origin: Vec3,
    ray_dir: Vec3,
    gizmo_origin: Vec3,
    axis_len: f32,
    hit_radius: f32,
) -> Option<GizmoAxis> {
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in GizmoAxis::ALL {
        let Some((s, dist)) = closest_point_on_axis(ray_origin, ray_dir, gizmo_origin, axis.dir())
        else {
            continue;
        };
        if s >= 0.05
            && s <= axis_len + 0.12
            && dist <= hit_radius
            && best.is_none_or(|(_, best_d)| dist < best_d)
        {
            best = Some((axis, dist));
        }
    }
    best.map(|(a, _)| a)
}

/// Active 3D transform gizmo mode for the selected scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformGizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
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

        cam.set_orthographic(true);
        let ortho_vp = cam.view_proj(16.0 / 9.0);
        assert!(ortho_vp.is_finite());
        assert!(ortho_vp.inverse().is_finite());

        cam.set_telephoto();
        assert!(!cam.orthographic);
        assert!((cam.fov_y_radians.to_degrees() - 22.0).abs() < 1e-4);
        assert!(cam.view_proj(16.0 / 9.0).is_finite());
    }
}
