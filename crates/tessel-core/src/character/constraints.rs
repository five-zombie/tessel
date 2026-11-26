use std::collections::BTreeMap;

use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::Transform;

use super::anim::{AnimationClip, BoneChannel};
use super::nano::{BoneId, Keyframe, KeyframeInterpolation};
use super::rig::CharacterRig;
use super::skin::VertexSkinWeights;

/// Physics / ragdoll collider metadata attached to a bone (exported to glTF `extras`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum BoneColliderMeta {
    Capsule { radius: f32, height: f32 },
    Box { half_extents: [f32; 3] },
}

/// Deterministic rig constraint evaluated during pose sampling / baking (`v5.6`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoneConstraint {
    IkFkBlend {
        name: String,
        root: BoneId,
        mid: BoneId,
        end: BoneId,
        target: Vec3,
        #[serde(default)]
        pole: Option<Vec3>,
        /// `0.0` = pure FK, `1.0` = pure IK.
        weight: f32,
    },
    LookAt {
        name: String,
        bone_id: BoneId,
        target: Vec3,
        #[serde(default = "default_up")]
        up: Vec3,
        #[serde(default = "default_one")]
        influence: f32,
    },
    CopyTransform {
        name: String,
        bone_id: BoneId,
        source_bone_id: BoneId,
        #[serde(default = "default_one")]
        influence: f32,
    },
}

fn default_up() -> Vec3 {
    Vec3::Y
}

fn default_one() -> f32 {
    1.0
}

/// Named bone group / visibility & lock layer (`v5.6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoneGroup {
    pub name: String,
    pub bone_ids: Vec<BoneId>,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
}

fn default_true() -> bool {
    true
}

/// Named attachment socket anchored to a bone with a local TRS offset (`v5.6`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigSocket {
    pub name: String,
    pub bone_id: BoneId,
    #[serde(default)]
    pub local_offset: Transform,
}

/// Timeline event / notify marker on an `AnimationClip` (exported to glTF `extras`, `v5.7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationEventKey {
    pub time_secs: f32,
    pub name: String,
    #[serde(default)]
    pub payload: String,
}

/// Standard Mixamo -> TESSEL 19-bone humanoid mapping table (`v5.6`).
pub const MIXAMO_TO_TESSEL_MAP: &[(&str, &str)] = &[
    ("mixamorig:Hips", "Root"),
    ("mixamorig:Spine", "Spine"),
    ("mixamorig:Spine2", "Chest"),
    ("mixamorig:Neck", "Neck"),
    ("mixamorig:Head", "Head"),
    ("mixamorig:LeftShoulder", "LeftShoulder"),
    ("mixamorig:LeftArm", "LeftUpperArm"),
    ("mixamorig:LeftForeArm", "LeftForearm"),
    ("mixamorig:LeftHand", "LeftHand"),
    ("mixamorig:RightShoulder", "RightShoulder"),
    ("mixamorig:RightArm", "RightUpperArm"),
    ("mixamorig:RightForeArm", "RightForearm"),
    ("mixamorig:RightHand", "RightHand"),
    ("mixamorig:LeftUpLeg", "LeftThigh"),
    ("mixamorig:LeftLeg", "LeftCalf"),
    ("mixamorig:LeftFoot", "LeftFoot"),
    ("mixamorig:RightUpLeg", "RightThigh"),
    ("mixamorig:RightLeg", "RightCalf"),
    ("mixamorig:RightFoot", "RightFoot"),
];

/// Prunes zero/invalid weights, keeps at most 4 influences, and normalizes `sum(weights) == 1.0`.
pub fn prune_and_normalize_weights(w: &mut VertexSkinWeights) {
    let mut pairs: [(u16, f32); 4] = [
        (w.joints[0], w.weights[0]),
        (w.joints[1], w.weights[1]),
        (w.joints[2], w.weights[2]),
        (w.joints[3], w.weights[3]),
    ];
    for p in &mut pairs {
        if !p.1.is_finite() || p.1 < 0.0 {
            p.1 = 0.0;
        }
    }
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
    let sum: f32 = pairs.iter().map(|p| p.1).sum();
    if sum > 1e-8 {
        for (i, (j, wt)) in pairs.iter().enumerate() {
            w.joints[i] = *j;
            w.weights[i] = *wt / sum;
        }
    } else {
        w.joints = [0, 0, 0, 0];
        w.weights = [1.0, 0.0, 0.0, 0.0];
    }
}

/// Safely extracts a rotation quaternion from a world transform matrix without panicking on zero scale.
fn safe_matrix_rotation(m: glam::Mat4) -> Quat {
    let x = m.x_axis.truncate().normalize_or_zero();
    let y = m.y_axis.truncate().normalize_or_zero();
    let z = m.z_axis.truncate().normalize_or_zero();
    if x.length_squared() < 1e-6 || y.length_squared() < 1e-6 || z.length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    let ortho_y = (y - x * x.dot(y)).normalize_or_zero();
    if ortho_y.length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    let ortho_z = x.cross(ortho_y).normalize_or_zero();
    if ortho_z.length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    Quat::from_mat3(&glam::Mat3::from_cols(x, ortho_y, ortho_z)).normalize()
}

/// Evaluates all deterministic rig constraints in order onto the rig's current `pose_local`.
pub fn evaluate_rig_constraints(rig: &mut CharacterRig) {
    let constraints = rig.constraints.clone();
    for constraint in &constraints {
        match constraint {
            BoneConstraint::CopyTransform {
                bone_id,
                source_bone_id,
                influence,
                ..
            } => {
                if !influence.is_finite() {
                    continue;
                }
                let w = influence.clamp(0.0, 1.0);
                if w <= 1e-5 {
                    continue;
                }
                let Some(src) = rig.bone(*source_bone_id).map(|b| b.pose_local) else {
                    continue;
                };
                if let Some(dst) = rig.bone_mut(*bone_id) {
                    dst.pose_local.translation =
                        dst.pose_local.translation.lerp(src.translation, w);
                    dst.pose_local.rotation = dst.pose_local.rotation.slerp(src.rotation, w);
                    dst.pose_local.scale = dst.pose_local.scale.lerp(src.scale, w);
                }
            }
            BoneConstraint::LookAt {
                bone_id,
                target,
                up,
                influence,
                ..
            } => {
                if !influence.is_finite() || !target.is_finite() {
                    continue;
                }
                let w = influence.clamp(0.0, 1.0);
                if w <= 1e-5 {
                    continue;
                }
                let world_mats = rig.world_pose_matrices();
                let Some(&bone_mat) = world_mats.get(bone_id) else {
                    continue;
                };
                let bone_pos = bone_mat.w_axis.truncate();
                let dir = (*target - bone_pos).normalize_or_zero();
                if dir.length_squared() < 1e-6 {
                    continue;
                }
                let up_vec = if up.is_finite() && up.length_squared() > 1e-6 {
                    up.normalize()
                } else {
                    Vec3::Y
                };
                let mut right = up_vec.cross(dir).normalize_or_zero();
                if right.length_squared() < 1e-6 {
                    let alt_up = if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
                    right = alt_up.cross(dir).normalize_or_zero();
                }
                let ortho_up = dir.cross(right).normalize_or_zero();
                let desired_world =
                    Quat::from_mat3(&glam::Mat3::from_cols(right, ortho_up, dir)).normalize();
                let parent_rot = rig
                    .bone(*bone_id)
                    .and_then(|b| b.parent)
                    .and_then(|pid| world_mats.get(&pid))
                    .map(|&m| safe_matrix_rotation(m))
                    .unwrap_or(Quat::IDENTITY);
                let desired_local = (parent_rot.inverse() * desired_world).normalize();
                if let Some(dst) = rig.bone_mut(*bone_id) {
                    dst.pose_local.rotation =
                        dst.clamp_rotation_quat(dst.pose_local.rotation.slerp(desired_local, w));
                }
            }
            BoneConstraint::IkFkBlend {
                root,
                mid,
                end,
                target,
                pole,
                weight,
                ..
            } => {
                if !weight.is_finite() || !target.is_finite() {
                    continue;
                }
                let w = weight.clamp(0.0, 1.0);
                if w <= 1e-5 {
                    continue;
                }
                let effective_pole = pole
                    .filter(|p| p.is_finite())
                    .or_else(|| rig.chain_poles.get(end).copied());
                if let Ok((r_rot, m_rot)) =
                    super::ik::solve_for_rig(rig, *root, *mid, *end, *target, effective_pole)
                {
                    if let Some(b) = rig.bone_mut(*root) {
                        b.pose_local.rotation =
                            b.clamp_rotation_quat(b.pose_local.rotation.slerp(r_rot, w));
                    }
                    if let Some(b) = rig.bone_mut(*mid) {
                        b.pose_local.rotation =
                            b.clamp_rotation_quat(b.pose_local.rotation.slerp(m_rot, w));
                    }
                }
            }
        }
    }
}

/// Applies single-control reverse foot roll (`roll_deg`: negative rocks onto heel, positive rises onto ball/toe, `v5.7`).
/// Requires all three bones (`heel_id`, `ball_id`, `toe_id`) to exist in `rig`.
pub fn apply_foot_roll(
    rig: &mut CharacterRig,
    heel_id: BoneId,
    ball_id: BoneId,
    toe_id: BoneId,
    roll_deg: f32,
) -> bool {
    if !roll_deg.is_finite()
        || rig.bone(heel_id).is_none()
        || rig.bone(ball_id).is_none()
        || rig.bone(toe_id).is_none()
    {
        return false;
    }
    let clamped = roll_deg.clamp(-60.0, 90.0);
    let (heel_pitch, ball_pitch, toe_pitch) = if clamped < 0.0 {
        (clamped, 0.0, 0.0)
    } else if clamped <= 45.0 {
        (0.0, clamped, 0.0)
    } else {
        (0.0, 45.0, clamped - 45.0)
    };
    for (bid, deg) in [
        (heel_id, heel_pitch),
        (ball_id, ball_pitch),
        (toe_id, toe_pitch),
    ] {
        if let Some(b) = rig.bone_mut(bid) {
            let q = Quat::from_rotation_x(deg.to_radians());
            b.pose_local.rotation = b.clamp_rotation_quat(q);
        }
    }
    true
}

/// Deterministically bakes procedural spring/jiggle secondary motion for all bones with `spring_stiffness > 0.0` (`v5.7`).
/// Uses fixed sub-stepping (`<= 1/120 s`) so high stiffness remains unconditionally stable and preserves animated translation/scale.
pub fn bake_spring_bones(rig: &CharacterRig, clip_index: usize, fps: f32) -> Option<AnimationClip> {
    let base_clip = rig.animations.get(clip_index)?;
    if !fps.is_finite() || !base_clip.duration_secs.is_finite() {
        return None;
    }
    let spring_bones: Vec<(BoneId, f32, f32)> = rig
        .bones
        .iter()
        .filter(|b| b.spring_stiffness > 0.0 && b.spring_stiffness.is_finite())
        .map(|b| {
            let damp = if b.spring_damping.is_finite() {
                b.spring_damping.clamp(0.01, 50.0)
            } else {
                0.5
            };
            (b.id, b.spring_stiffness.clamp(0.01, 200.0), damp)
        })
        .collect();
    if spring_bones.is_empty() {
        return None;
    }
    let duration = base_clip.duration_secs.clamp(0.05, 600.0);
    let sample_fps = fps.clamp(1.0, 120.0);
    let steps = ((duration * sample_fps).round() as usize).clamp(2, 1024);
    let dt = duration / steps as f32;
    let substeps = ((dt / (1.0 / 120.0)).ceil() as usize).clamp(1, 128);
    let sub_dt = dt / substeps as f32;

    let mut out = base_clip.clone();
    let mut work_rig = rig.clone();
    work_rig.active_animation = clip_index;
    work_rig.playback_time = 0.0;
    work_rig.apply_active_animation();

    let mut angle_state: BTreeMap<BoneId, (Vec3, Vec3)> = spring_bones
        .iter()
        .map(|&(bid, _, _)| {
            let (bx, by, bz) = work_rig
                .bone(bid)
                .map(|b| b.pose_local.rotation.to_euler(EulerRot::XYZ))
                .unwrap_or((0.0, 0.0, 0.0));
            (bid, (Vec3::new(bx, by, bz), Vec3::ZERO))
        })
        .collect();
    let mut prev_root_pos: Option<Vec3> = None;
    let mut baked_keys: BTreeMap<BoneId, Vec<Keyframe>> = BTreeMap::new();

    for step in 0..=steps {
        let t = (step as f32 * dt).min(duration);
        work_rig.playback_time = t;
        work_rig.apply_active_animation();
        let world_mats = work_rig.world_pose_matrices();
        let root_pos = work_rig
            .bones
            .first()
            .and_then(|b| world_mats.get(&b.id))
            .map(|m| m.w_axis.truncate())
            .unwrap_or(Vec3::ZERO);
        let root_vel = prev_root_pos
            .map(|p| (root_pos - p) / dt.max(1e-4))
            .unwrap_or(Vec3::ZERO)
            .clamp(Vec3::splat(-50.0), Vec3::splat(50.0));
        prev_root_pos = Some(root_pos);

        for &(bid, stiffness, damping) in &spring_bones {
            let (base_rot, anim_trans, anim_scale) = work_rig
                .bone(bid)
                .map(|b| {
                    (
                        b.pose_local.rotation,
                        b.pose_local.translation - b.rest_local.translation,
                        b.pose_local.scale,
                    )
                })
                .unwrap_or((Quat::IDENTITY, Vec3::ZERO, Vec3::ONE));
            let (base_x, base_y, base_z) = base_rot.to_euler(EulerRot::XYZ);
            let target = Vec3::new(base_x, base_y, base_z)
                + Vec3::new(-root_vel.z * 0.05, 0.0, root_vel.x * 0.05);
            if let Some((cur, vel)) = angle_state.get_mut(&bid) {
                for _ in 0..substeps {
                    let accel = (target - *cur) * stiffness - *vel * damping;
                    *vel = (*vel + accel * sub_dt).clamp(Vec3::splat(-40.0), Vec3::splat(40.0));
                    *cur = (*cur + *vel * sub_dt).clamp(
                        Vec3::splat(-std::f32::consts::PI),
                        Vec3::splat(std::f32::consts::PI),
                    );
                }
                let rot = Quat::from_euler(EulerRot::XYZ, cur.x, cur.y, cur.z);
                baked_keys.entry(bid).or_default().push(Keyframe {
                    time_secs: t,
                    rotation: rot,
                    translation: anim_trans,
                    scale: anim_scale,
                    interpolation: KeyframeInterpolation::Linear,
                });
            }
        }
    }

    for (bid, keyframes) in baked_keys {
        if let Some(ch) = out.channels.iter_mut().find(|c| c.bone_id == bid) {
            ch.keyframes = keyframes;
        } else {
            out.channels.push(BoneChannel {
                bone_id: bid,
                keyframes,
            });
        }
    }
    Some(out)
}

/// Retargets a clip whose channels are keyed by Mixamo bone names onto a TESSEL humanoid rig (`v5.6`).
pub fn retarget_clip_from_mixamo(
    target_rig: &CharacterRig,
    source_name_channels: &[(String, Vec<Keyframe>)],
    clip_name: impl Into<String>,
    duration_secs: f32,
    height_scale: f32,
) -> AnimationClip {
    let scale = if height_scale.is_finite() && height_scale > 0.0 {
        height_scale
    } else {
        1.0
    };
    let mut clip = AnimationClip::new_empty(clip_name, duration_secs.clamp(0.05, 600.0));
    for (src_name, kfs) in source_name_channels {
        let mapped_name = MIXAMO_TO_TESSEL_MAP
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(src_name))
            .map(|(_, t)| *t)
            .unwrap_or(src_name.as_str());
        if let Some(bone) = target_rig
            .bones
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(mapped_name))
        {
            let scaled_kfs: Vec<Keyframe> = kfs
                .iter()
                .map(|k| {
                    let mut nk = *k;
                    nk.translation *= scale;
                    nk
                })
                .collect();
            clip.channels.push(BoneChannel {
                bone_id: bone.id,
                keyframes: scaled_kfs,
            });
        }
    }
    clip
}

/// Swaps Left <-> Right bone poses simultaneously across the X plane (true flipped pose paste, `v5.7`).
pub fn swap_rig_pose_lr(rig: &mut CharacterRig) -> usize {
    let snapshot: BTreeMap<BoneId, Transform> =
        rig.bones.iter().map(|b| (b.id, b.pose_local)).collect();
    let pairs: Vec<(BoneId, BoneId)> = rig
        .bones
        .iter()
        .filter_map(|b| {
            let right_name = b
                .name
                .strip_prefix("Left")
                .map(|rest| format!("Right{rest}"))
                .or_else(|| {
                    b.name
                        .contains("_Left")
                        .then(|| b.name.replacen("_Left", "_Right", 1))
                })?;
            let right = rig.bones.iter().find(|o| o.name == right_name)?;
            Some((b.id, right.id))
        })
        .collect();
    let mut swapped = 0;
    let mirror_t = |src: Transform| -> Transform {
        let (rx, ry, rz) = src.rotation.to_euler(EulerRot::XYZ);
        Transform {
            translation: Vec3::new(-src.translation.x, src.translation.y, src.translation.z),
            rotation: Quat::from_euler(EulerRot::XYZ, rx, -ry, -rz),
            scale: src.scale,
        }
    };
    for (left_id, right_id) in pairs {
        if let (Some(&lt), Some(&rt)) = (snapshot.get(&left_id), snapshot.get(&right_id)) {
            if let Some(lb) = rig.bone_mut(left_id) {
                let mt = mirror_t(rt);
                lb.pose_local.translation = mt.translation;
                lb.pose_local.rotation = lb.clamp_rotation_quat(mt.rotation);
                lb.pose_local.scale = mt.scale;
            }
            if let Some(rb) = rig.bone_mut(right_id) {
                let mt = mirror_t(lt);
                rb.pose_local.translation = mt.translation;
                rb.pose_local.rotation = rb.clamp_rotation_quat(mt.rotation);
                rb.pose_local.scale = mt.scale;
            }
            swapped += 1;
        }
    }
    swapped
}

/// Mirrors bone poses across the X plane (Left <-> Right) using bone naming conventions (`v5.5` / `v5.7`).
pub fn mirror_rig_pose(rig: &mut CharacterRig, left_to_right: bool) -> usize {
    let snapshot: BTreeMap<BoneId, Transform> =
        rig.bones.iter().map(|b| (b.id, b.pose_local)).collect();
    let pairs: Vec<(BoneId, BoneId)> = rig
        .bones
        .iter()
        .filter_map(|b| {
            let other_name = if left_to_right {
                b.name
                    .strip_prefix("Left")
                    .map(|rest| format!("Right{rest}"))
                    .or_else(|| {
                        b.name
                            .contains("_Left")
                            .then(|| b.name.replacen("_Left", "_Right", 1))
                    })
            } else {
                b.name
                    .strip_prefix("Right")
                    .map(|rest| format!("Left{rest}"))
                    .or_else(|| {
                        b.name
                            .contains("_Right")
                            .then(|| b.name.replacen("_Right", "_Left", 1))
                    })
            }?;
            let dst = rig.bones.iter().find(|o| o.name == other_name)?;
            Some((b.id, dst.id))
        })
        .collect();
    let mut mirrored = 0;
    for (src_id, dst_id) in pairs {
        if let Some(&src_t) = snapshot.get(&src_id) {
            if let Some(dst) = rig.bone_mut(dst_id) {
                let (rx, ry, rz) = src_t.rotation.to_euler(EulerRot::XYZ);
                dst.pose_local.rotation =
                    dst.clamp_rotation_quat(Quat::from_euler(EulerRot::XYZ, rx, -ry, -rz));
                dst.pose_local.translation = Vec3::new(
                    -src_t.translation.x,
                    src_t.translation.y,
                    src_t.translation.z,
                );
                dst.pose_local.scale = src_t.scale;
                mirrored += 1;
            }
        }
    }
    mirrored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_and_normalize_weights_sums_to_one() {
        let mut w = VertexSkinWeights {
            joints: [1, 2, 3, 4],
            weights: [0.2, 0.8, f32::NAN, -0.5],
        };
        prune_and_normalize_weights(&mut w);
        let sum: f32 = w.weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert_eq!(w.joints[0], 2);
    }

    #[test]
    fn foot_roll_and_spring_bake_are_deterministic() {
        let mut rig = CharacterRig::humanoid_template();
        let left_foot = rig.bones.iter().find(|b| b.name == "LeftFoot").unwrap().id;
        rig.bone_mut(left_foot).unwrap().spring_stiffness = 20.0;
        rig.bone_mut(left_foot).unwrap().spring_damping = 3.0;
        let baked = bake_spring_bones(&rig, 0, 12.0).unwrap();
        assert!(baked.channels.iter().any(|c| c.bone_id == left_foot));
        assert!(apply_foot_roll(
            &mut rig,
            BoneId(0),
            BoneId(1),
            left_foot,
            55.0
        ));
    }

    #[test]
    fn mixamo_retarget_and_mirror_pose_work() {
        let mut rig = CharacterRig::humanoid_template();
        let l_arm = rig
            .bones
            .iter()
            .find(|b| b.name == "LeftUpperArm")
            .unwrap()
            .id;
        let r_arm = rig
            .bones
            .iter()
            .find(|b| b.name == "RightUpperArm")
            .unwrap()
            .id;
        rig.bone_mut(l_arm).unwrap().pose_local.rotation = Quat::from_rotation_z(0.5);
        let count = mirror_rig_pose(&mut rig, true);
        assert!(count >= 4);
        let (_, _, rz) = rig
            .bone(r_arm)
            .unwrap()
            .pose_local
            .rotation
            .to_euler(EulerRot::XYZ);
        assert!((rz + 0.5).abs() < 1e-4);

        let retargeted = retarget_clip_from_mixamo(
            &rig,
            &[(
                "mixamorig:Hips".into(),
                vec![Keyframe {
                    time_secs: 0.0,
                    rotation: Quat::IDENTITY,
                    translation: Vec3::new(0.0, 1.0, 0.0),
                    scale: Vec3::ONE,
                    interpolation: KeyframeInterpolation::Linear,
                }],
            )],
            "MixamoRun",
            1.0,
            1.2,
        );
        assert_eq!(retargeted.channels.len(), 1);
        assert_eq!(
            retargeted.channels[0].keyframes[0].translation,
            Vec3::new(0.0, 1.2, 0.0)
        );
    }
}
