use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::nano::{BoneId, Keyframe};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneChannel {
    pub bone_id: BoneId,
    pub keyframes: Vec<Keyframe>,
}

/// Keyframe animation clip supporting linear translation/scale & SLERP rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub name: String,
    pub duration_secs: f32,
    pub channels: Vec<BoneChannel>,
}

impl AnimationClip {
    pub fn sample_bone(&self, bone_id: BoneId, time_secs: f32) -> Option<Keyframe> {
        let channel = self.channels.iter().find(|c| c.bone_id == bone_id)?;
        if channel.keyframes.is_empty() {
            return None;
        }
        if channel.keyframes.len() == 1 || time_secs <= channel.keyframes[0].time_secs {
            return Some(channel.keyframes[0]);
        }
        let last = *channel.keyframes.last()?;
        if time_secs >= last.time_secs {
            return Some(last);
        }

        for pair in channel.keyframes.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if time_secs >= a.time_secs && time_secs <= b.time_secs {
                let span = (b.time_secs - a.time_secs).max(1e-5);
                let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
                return Some(Keyframe {
                    time_secs,
                    translation: a.translation.lerp(b.translation, alpha),
                    rotation: a.rotation.slerp(b.rotation, alpha),
                    scale: a.scale.lerp(b.scale, alpha),
                });
            }
        }
        Some(last)
    }

    pub fn starter_idle() -> Self {
        Self {
            name: "Idle".into(),
            duration_secs: 2.0,
            channels: vec![BoneChannel {
                bone_id: BoneId(1), // Torso subtle breathing
                keyframes: vec![
                    Keyframe {
                        time_secs: 0.0,
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                    Keyframe {
                        time_secs: 1.0,
                        translation: Vec3::new(0.0, 0.03, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                    Keyframe {
                        time_secs: 2.0,
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ],
            }],
        }
    }

    pub fn starter_walk() -> Self {
        let fwd = Quat::from_rotation_x(0.45);
        let back = Quat::from_rotation_x(-0.45);
        Self {
            name: "Walk".into(),
            duration_secs: 1.0,
            channels: vec![
                BoneChannel {
                    bone_id: BoneId(3), // LeftArm
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, fwd),
                        Keyframe::rotation_only(0.5, back),
                        Keyframe::rotation_only(1.0, fwd),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(4), // RightArm
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, back),
                        Keyframe::rotation_only(0.5, fwd),
                        Keyframe::rotation_only(1.0, back),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(5), // LeftLeg
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, back),
                        Keyframe::rotation_only(0.5, fwd),
                        Keyframe::rotation_only(1.0, back),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(6), // RightLeg
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, fwd),
                        Keyframe::rotation_only(0.5, back),
                        Keyframe::rotation_only(1.0, fwd),
                    ],
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::CharacterRig;

    #[test]
    fn walk_animation_interpolates_smoothly_and_updates_world_matrices() {
        let mut rig = CharacterRig::biped_template();
        rig.active_animation = 1; // Walk
        rig.playback_time = 0.25;
        rig.apply_active_animation();
        let mats = rig.world_pose_matrices();
        assert_eq!(mats.len(), 7);
        assert!(mats[&BoneId(5)].is_finite());
    }
}
