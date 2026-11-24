use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Nano-Unit: Stable bone identifier inside a `CharacterRig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BoneId(pub u32);

/// Interpolation curve between this keyframe and the next keyframe on a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeInterpolation {
    #[default]
    Linear,
    Step,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl KeyframeInterpolation {
    pub fn apply(self, alpha: f32) -> f32 {
        let t = alpha.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Step => 0.0,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
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
