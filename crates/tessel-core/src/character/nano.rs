use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Nano-Unit: Stable bone identifier inside a `CharacterRig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BoneId(pub u32);

/// Interpolation curve between this keyframe and the next keyframe on a channel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeInterpolation {
    #[default]
    Linear,
    Step,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Custom 1D cubic Bezier easing curve with control points `P1=(p1x, p1y)` and `P2=(p2x, p2y)`.
    CubicBezier {
        p1x: f32,
        p1y: f32,
        p2x: f32,
        p2y: f32,
    },
}

impl KeyframeInterpolation {
    pub fn apply(self, alpha: f32) -> f32 {
        if !alpha.is_finite() {
            return 0.0;
        }
        let t = alpha.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Step => 0.0,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::EaseInOut => t * t * (3.0 - 2.0 * t),
            Self::CubicBezier { p1x, p1y, p2x, p2y } => {
                let x1 = if p1x.is_finite() {
                    p1x.clamp(0.0, 1.0)
                } else {
                    0.25
                };
                let x2 = if p2x.is_finite() {
                    p2x.clamp(0.0, 1.0)
                } else {
                    0.75
                };
                let y1 = if p1y.is_finite() {
                    p1y.clamp(-1.0, 2.0)
                } else {
                    0.1
                };
                let y2 = if p2y.is_finite() {
                    p2y.clamp(-1.0, 2.0)
                } else {
                    0.9
                };
                eval_cubic_bezier(t, x1, y1, x2, y2)
            }
        }
    }
}

fn cubic_bezier_1d(u: f32, c1: f32, c2: f32) -> f32 {
    let inv = 1.0 - u;
    3.0 * inv * inv * u * c1 + 3.0 * inv * u * u * c2 + u * u * u
}

fn eval_cubic_bezier(target_x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    let mut u = target_x;
    for _ in 0..12 {
        let x = cubic_bezier_1d(u, x1, x2);
        if (x - target_x).abs() < 1e-5 {
            break;
        }
        if x < target_x {
            lo = u;
        } else {
            hi = u;
        }
        u = 0.5 * (lo + hi);
    }
    cubic_bezier_1d(u, y1, y2)
}

/// Nano-Unit: Single pose keyframe at `time_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time_secs: f32,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    #[serde(default)]
    pub interpolation: KeyframeInterpolation,
}

impl Keyframe {
    pub fn rotation_only(time_secs: f32, rotation: Quat) -> Self {
        Self {
            time_secs,
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
            interpolation: KeyframeInterpolation::Linear,
        }
    }
}
