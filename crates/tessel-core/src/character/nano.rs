use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Nano-Unit: Stable bone identifier inside a `CharacterRig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BoneId(pub u32);

/// Nano-Unit: Single pose keyframe at `time_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time_secs: f32,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Keyframe {
    pub fn rotation_only(time_secs: f32, rotation: Quat) -> Self {
        Self {
            time_secs,
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }
}
