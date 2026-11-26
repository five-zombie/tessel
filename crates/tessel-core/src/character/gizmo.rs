use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

use super::nano::BoneId;
use super::rig::CharacterRig;

/// Operation mode for the octahedral/pyramid Stick Gizmo manipulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickGizmoMode {
    /// Rigid rotation around the bone's longitudinal axis.
    #[default]
    Roll,
    /// Lateral tilt (nod) around local X.
    Pitch,
    /// Vertical turn (shake) around local Z/Y perpendicular.
    Yaw,
    /// Distributed twist gradient along a multi-bone descendant chain (tails/tentacles/cloth).
    Twist,
    /// Combined rigid root roll + distributed descendant twist gradient.
    RollTwist,
}

/// Persistent viewport settings for the Stick Gizmo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StickGizmoSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: StickGizmoMode,
    /// Angle snap increment in degrees (`0.0` = free, typical presets: `5.0`, `15.0`, `45.0`).
    #[serde(default)]
    pub snap_deg: f32,
    #[serde(default)]
    pub mirror_lr: bool,
    /// Optional explicit world/local axis lock (`0=X`, `1=Y`, `2=Z`).
    #[serde(default)]
    pub axis_lock: Option<u8>,
    /// Number of bones in chain for `Twist` / `RollTwist` modes.
    #[serde(default = "default_chain_len")]
    pub twist_chain_len: usize,
}

fn default_true() -> bool {
    true
}

fn default_chain_len() -> usize {
    3
}

impl Default for StickGizmoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: StickGizmoMode::Roll,
            snap_deg: 0.0,
            mirror_lr: false,
            axis_lock: None,
            twist_chain_len: default_chain_len(),
        }
    }
}

/// Parameters for applying a single Stick Gizmo pose operation via the Command Bus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StickGizmoParams {
    pub bone_id: BoneId,
    pub mode: StickGizmoMode,
    pub angle_deg: f32,
    #[serde(default = "default_chain_len")]
    pub chain_len: usize,
    #[serde(default)]
    pub snap_deg: f32,
    #[serde(default)]
    pub mirror_lr: bool,
    #[serde(default)]
    pub axis_lock: Option<u8>,
}

/// Snaps `angle_deg` to the nearest multiple of `snap_deg` when `snap_deg > 0`.
pub fn snap_angle_deg(angle_deg: f32, snap_deg: f32) -> f32 {
    if !angle_deg.is_finite() {
        return 0.0;
    }
    if snap_deg.is_finite() && snap_deg > 1e-4 {
        (angle_deg / snap_deg).round() * snap_deg
    } else {
        angle_deg
    }
}

/// Finds the symmetric Left/Right counterpart bone by standard naming conventions
/// (`Left*` <-> `Right*`, `*_L` <-> `*_R`, `*.L` <-> `*.R`).
pub fn find_mirror_bone(rig: &CharacterRig, bone_id: BoneId) -> Option<BoneId> {
    let name = &rig.bone(bone_id)?.name;
    let mirrored = if let Some(rest) = name.strip_prefix("Left") {
        format!("Right{rest}")
    } else if let Some(rest) = name.strip_prefix("Right") {
        format!("Left{rest}")
    } else if let Some(rest) = name.strip_prefix("left_") {
        format!("right_{rest}")
    } else if let Some(rest) = name.strip_prefix("right_") {
        format!("left_{rest}")
    } else if let Some(rest) = name.strip_suffix("_L") {
        format!("{rest}_R")
    } else if let Some(rest) = name.strip_suffix("_R") {
        format!("{rest}_L")
    } else if let Some(rest) = name.strip_suffix(".L") {
        format!("{rest}.R")
    } else {
        let rest = name.strip_suffix(".R")?;
        format!("{rest}.L")
    };
    rig.bones
        .iter()
        .find(|b| b.name == mirrored && b.id != bone_id)
        .map(|b| b.id)
}

/// Computes the local longitudinal axis of `bone_id` (pointing toward its first child, or `+Y` for leaf bones).
pub fn bone_longitudinal_axis(rig: &CharacterRig, bone_id: BoneId) -> Vec3 {
    if let Some(child) = rig.bones.iter().find(|b| b.parent == Some(bone_id)) {
        let dir = child.rest_local.translation.normalize_or_zero();
        if dir.length_squared() > 1e-6 {
            return dir;
        }
    }
    Vec3::Y
}

/// Collects a linear chain starting at `root` up to `max_len` bones (following first child).
fn collect_descendant_chain(rig: &CharacterRig, root: BoneId, max_len: usize) -> Vec<BoneId> {
    let mut chain = vec![root];
    let limit = max_len.clamp(1, 32);
    let mut curr = root;
    while chain.len() < limit {
        if let Some(child) = rig.bones.iter().find(|b| b.parent == Some(curr)) {
            if chain.contains(&child.id) {
                break;
            }
            chain.push(child.id);
            curr = child.id;
        } else {
            break;
        }
    }
    chain
}

/// Computes the rotation axis for a stick gizmo mode (respecting optional `axis_lock`).
fn resolve_gizmo_axis(
    rig: &CharacterRig,
    bone_id: BoneId,
    mode: StickGizmoMode,
    axis_lock: Option<u8>,
) -> Vec3 {
    if let Some(axis) = axis_lock {
        return match axis {
            0 => Vec3::X,
            1 => Vec3::Y,
            _ => Vec3::Z,
        };
    }
    let long_axis = bone_longitudinal_axis(rig, bone_id);
    match mode {
        StickGizmoMode::Roll | StickGizmoMode::Twist | StickGizmoMode::RollTwist => long_axis,
        StickGizmoMode::Pitch => {
            let ref_up = if long_axis.x.abs() < 0.9 {
                Vec3::X
            } else {
                Vec3::Z
            };
            let pitch = (ref_up - long_axis * ref_up.dot(long_axis)).normalize_or_zero();
            if pitch.length_squared() > 1e-6 {
                pitch
            } else {
                Vec3::X
            }
        }
        StickGizmoMode::Yaw => {
            let pitch = resolve_gizmo_axis(rig, bone_id, StickGizmoMode::Pitch, None);
            let yaw = long_axis.cross(pitch).normalize_or_zero();
            if yaw.length_squared() > 1e-6 {
                yaw
            } else {
                Vec3::Z
            }
        }
    }
}

/// Applies a Stick Gizmo operation to `rig` and returns the list of `(BoneId, previous_pose_rotation)`
/// so the Command Bus can undo/redo cleanly.
pub fn apply_stick_gizmo_pose(
    rig: &mut CharacterRig,
    params: &StickGizmoParams,
) -> Result<Vec<(BoneId, Quat)>, CoreError> {
    if !params.angle_deg.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    if rig.bone(params.bone_id).is_none() {
        return Err(CoreError::OutOfBounds);
    }

    let snapped_deg = snap_angle_deg(params.angle_deg, params.snap_deg);
    let total_rad = snapped_deg.to_radians();

    let mut targets = vec![(params.bone_id, total_rad)];
    if params.mirror_lr {
        if let Some(mirror_id) = find_mirror_bone(rig, params.bone_id) {
            // Pitch mirrors with same sign; Roll/Yaw/Twist mirror with negated sign across X=0 plane.
            let mirror_rad = match params.mode {
                StickGizmoMode::Pitch => total_rad,
                _ => -total_rad,
            };
            targets.push((mirror_id, mirror_rad));
        }
    }

    let mut previous = Vec::new();
    for (root_id, rad) in targets {
        match params.mode {
            StickGizmoMode::Roll | StickGizmoMode::Pitch | StickGizmoMode::Yaw => {
                let axis = resolve_gizmo_axis(rig, root_id, params.mode, params.axis_lock);
                let delta = Quat::from_axis_angle(axis, rad);
                if let Some(bone) = rig.bone_mut(root_id) {
                    previous.push((root_id, bone.pose_local.rotation));
                    bone.pose_local.rotation = (bone.pose_local.rotation * delta).normalize();
                }
            }
            StickGizmoMode::Twist => {
                let chain = collect_descendant_chain(rig, root_id, params.chain_len.max(2));
                let n = chain.len() as f32;
                // Distributed twist gradient: each segment receives a progressive fraction summing to total_rad
                let weight_sum = n * (n + 1.0) * 0.5;
                for (i, &bid) in chain.iter().enumerate() {
                    let seg_rad = rad * ((i + 1) as f32 / weight_sum);
                    let axis =
                        resolve_gizmo_axis(rig, bid, StickGizmoMode::Twist, params.axis_lock);
                    let delta = Quat::from_axis_angle(axis, seg_rad);
                    if let Some(bone) = rig.bone_mut(bid) {
                        previous.push((bid, bone.pose_local.rotation));
                        bone.pose_local.rotation = (bone.pose_local.rotation * delta).normalize();
                    }
                }
            }
            StickGizmoMode::RollTwist => {
                let chain = collect_descendant_chain(rig, root_id, params.chain_len.max(2));
                let root_roll = rad * 0.5;
                let twist_budget = rad * 0.5;
                let n = chain.len() as f32;
                let weight_sum = n * (n + 1.0) * 0.5;
                for (i, &bid) in chain.iter().enumerate() {
                    let seg_rad = if i == 0 {
                        root_roll + twist_budget * (1.0 / weight_sum)
                    } else {
                        twist_budget * ((i + 1) as f32 / weight_sum)
                    };
                    let axis =
                        resolve_gizmo_axis(rig, bid, StickGizmoMode::RollTwist, params.axis_lock);
                    let delta = Quat::from_axis_angle(axis, seg_rad);
                    if let Some(bone) = rig.bone_mut(bid) {
                        previous.push((bid, bone.pose_local.rotation));
                        bone.pose_local.rotation = (bone.pose_local.rotation * delta).normalize();
                    }
                }
            }
        }
    }

    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::Bone;
    use crate::scene::Transform;

    #[test]
    fn twist_distributes_gradient_differently_from_rigid_roll() {
        let mut rig = CharacterRig::default();
        rig.bones
            .push(Bone::new(BoneId(0), "Tail0", None, Transform::default()));
        rig.bones.push(Bone::new(
            BoneId(1),
            "Tail1",
            Some(BoneId(0)),
            Transform {
                translation: Vec3::new(0.0, 0.5, 0.0),
                ..Default::default()
            },
        ));
        rig.bones.push(Bone::new(
            BoneId(2),
            "Tail2",
            Some(BoneId(1)),
            Transform {
                translation: Vec3::new(0.0, 0.5, 0.0),
                ..Default::default()
            },
        ));

        let prev = apply_stick_gizmo_pose(
            &mut rig,
            &StickGizmoParams {
                bone_id: BoneId(0),
                mode: StickGizmoMode::Twist,
                angle_deg: 90.0,
                chain_len: 3,
                snap_deg: 15.0,
                mirror_lr: false,
                axis_lock: None,
            },
        )
        .unwrap();

        assert_eq!(prev.len(), 3);
        let a0 = rig
            .bone(BoneId(0))
            .unwrap()
            .pose_local
            .rotation
            .to_axis_angle()
            .1;
        let a1 = rig
            .bone(BoneId(1))
            .unwrap()
            .pose_local
            .rotation
            .to_axis_angle()
            .1;
        let a2 = rig
            .bone(BoneId(2))
            .unwrap()
            .pose_local
            .rotation
            .to_axis_angle()
            .1;
        assert!(
            a0 < a1 && a1 < a2,
            "twist must increase monotonically along chain"
        );
    }

    #[test]
    fn snap_and_mirror_lr_work() {
        assert!((snap_angle_deg(22.0, 15.0) - 15.0).abs() < 1e-4);
        assert!((snap_angle_deg(23.0, 15.0) - 30.0).abs() < 1e-4);

        let rig = CharacterRig::humanoid_template();
        let left_arm = rig
            .bones
            .iter()
            .find(|b| b.name == "LeftUpperArm")
            .unwrap()
            .id;
        let right_arm = rig
            .bones
            .iter()
            .find(|b| b.name == "RightUpperArm")
            .unwrap()
            .id;
        assert_eq!(find_mirror_bone(&rig, left_arm), Some(right_arm));
    }
}
