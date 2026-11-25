use glam::{Quat, Vec3};

use crate::error::CoreError;

use super::nano::BoneId;
use super::rig::CharacterRig;

/// Analytic two-bone IK: rotates `root` and `mid` so the chain end reaches
/// `target`. Returns world-space rotation deltas `(root_delta, mid_delta)`
/// applied as `new_world = delta * old_world`.
///
/// `pole` steers the elbow/knee bend plane; when it is parallel to the aim
/// direction (or zero) a stable perpendicular fallback is picked, so the
/// solver never returns NaN. Out-of-reach targets clamp to max extension;
/// targets inside the inner radius clamp to max folding.
pub fn solve_two_bone_ik(
    root_pos: Vec3,
    mid_pos: Vec3,
    end_pos: Vec3,
    target: Vec3,
    pole: Vec3,
) -> Option<(Quat, Quat)> {
    if !root_pos.is_finite() || !mid_pos.is_finite() || !end_pos.is_finite() || !target.is_finite()
    {
        return None;
    }
    let l1 = (mid_pos - root_pos).length();
    let l2 = (end_pos - mid_pos).length();
    if l1 < 1e-6 || l2 < 1e-6 {
        return None;
    }
    let mut to_target = target - root_pos;
    let mut dist = to_target.length();
    if dist < 1e-6 {
        return None;
    }
    to_target /= dist;
    // Clamp reach: [|l1-l2|, l1+l2] with a small epsilon to keep angles valid.
    let max_reach = (l1 + l2) * (1.0 - 1e-4);
    let min_reach = (l1 - l2).abs() * (1.0 + 1e-4);
    dist = dist.clamp(min_reach.max(1e-6), max_reach.max(1e-6));

    // Bend direction: pole component perpendicular to the aim axis.
    let mut bend = pole - to_target * pole.dot(to_target);
    if bend.length_squared() < 1e-10 {
        bend = perpendicular_fallback(to_target);
    }
    bend = bend.normalize_or_zero();
    if bend.length_squared() < 1e-10 {
        return None;
    }

    // Law of cosines: distance from root along the aim axis to the new elbow.
    let a1 = (l1 * l1 - l2 * l2 + dist * dist) / (2.0 * dist);
    let h_sq = (l1 * l1 - a1 * a1).max(0.0);
    let new_mid = root_pos + to_target * a1 + bend * h_sq.sqrt();

    let old_mid_dir = (mid_pos - root_pos).normalize_or_zero();
    let new_mid_dir = (new_mid - root_pos).normalize_or_zero();
    if old_mid_dir.length_squared() < 1e-10 || new_mid_dir.length_squared() < 1e-10 {
        return None;
    }
    let root_delta = Quat::from_rotation_arc(old_mid_dir, new_mid_dir);

    // End effector rides on the mid joint: rotate its offset, then aim it.
    let old_end_offset = end_pos - mid_pos;
    let rotated_offset = root_delta * old_end_offset;
    if rotated_offset.length_squared() < 1e-12 {
        return None;
    }
    let clamped_target = root_pos + to_target * dist;
    let want_dir = (clamped_target - new_mid).normalize_or_zero();
    let have_dir = rotated_offset.normalize_or_zero();
    if want_dir.length_squared() < 1e-10 || have_dir.length_squared() < 1e-10 {
        return None;
    }
    let mid_delta = Quat::from_rotation_arc(have_dir, want_dir);
    if !mid_delta.is_finite() || !root_delta.is_finite() {
        return None;
    }
    Some((root_delta, mid_delta))
}

/// Any unit vector perpendicular to `axis` (stable fallback chain).
fn perpendicular_fallback(axis: Vec3) -> Vec3 {
    let candidate = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    (candidate - axis * axis.dot(candidate)).normalize_or_zero()
}

/// Solves IK against a live rig and returns the LOCAL pose rotations to set
/// on `root` and `mid`. Validates the parent chain `root <- mid <- end`.
/// `pole` defaults to world +Z (knees/elbows bend forward) when `None`.
pub fn solve_for_rig(
    rig: &CharacterRig,
    root: BoneId,
    mid: BoneId,
    end: BoneId,
    target: Vec3,
    pole: Option<Vec3>,
) -> Result<(Quat, Quat), CoreError> {
    if !target.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let pole = pole.unwrap_or(Vec3::Z);
    if !pole.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let mid_bone = rig.bone(mid).ok_or(CoreError::OutOfBounds)?;
    if mid_bone.parent != Some(root) {
        return Err(CoreError::OutOfBounds);
    }
    let end_bone = rig.bone(end).ok_or(CoreError::OutOfBounds)?;
    if end_bone.parent != Some(mid) {
        return Err(CoreError::OutOfBounds);
    }
    rig.bone(root).ok_or(CoreError::OutOfBounds)?;

    let world = rig.world_pose_matrices();
    let joint_pos = |id: BoneId| -> Option<Vec3> {
        world
            .get(&id)
            .map(|m| m.transform_point3(Vec3::ZERO))
            .filter(|p| p.is_finite())
    };
    let root_pos = joint_pos(root).ok_or(CoreError::OutOfBounds)?;
    let mid_pos = joint_pos(mid).ok_or(CoreError::OutOfBounds)?;
    let end_pos = joint_pos(end).ok_or(CoreError::OutOfBounds)?;

    let (root_delta, mid_delta) = solve_two_bone_ik(root_pos, mid_pos, end_pos, target, pole)
        .ok_or(CoreError::OutOfBounds)?;

    // Convert world deltas to local rotations: local_new = parent_world^-1 * world_new.
    // NOTE: mid's parent is root, whose world rotation CHANGES with root_delta,
    // so mid must use the updated parent rotation, not the pre-solve one.
    let parent_old_rot = |id: BoneId| -> Option<Quat> {
        let bone = rig.bone(id)?;
        let parent_world = bone
            .parent
            .and_then(|pid| world.get(&pid))
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let (_, parent_rot, _) = parent_world.to_scale_rotation_translation();
        parent_rot.is_finite().then_some(parent_rot)
    };
    let root_world_old = world
        .get(&root)
        .map(|m| m.to_scale_rotation_translation().1)
        .ok_or(CoreError::OutOfBounds)?;
    let mid_world_old = world
        .get(&mid)
        .map(|m| m.to_scale_rotation_translation().1)
        .ok_or(CoreError::OutOfBounds)?;
    if !root_world_old.is_finite() || !mid_world_old.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let root_world_new = (root_delta * root_world_old).normalize();
    let root_local = (parent_old_rot(root)
        .ok_or(CoreError::OutOfBounds)?
        .conjugate()
        * root_world_new)
        .normalize();
    let root_new_rot = root_world_new;
    // Mid rides on the rotated root: pre-mid-delta world includes root_delta.
    let mid_world_new = (mid_delta * root_delta * mid_world_old).normalize();
    let mid_local = (root_new_rot.conjugate() * mid_world_new).normalize();
    if !root_local.is_finite() || !mid_local.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    Ok((root_local, mid_local))
}

/// Applies world-space deltas to a FK chain and returns the end position.
/// Test/debug helper mirroring the solver math.
pub fn apply_deltas_end_pos(
    root_pos: Vec3,
    mid_pos: Vec3,
    end_pos: Vec3,
    root_delta: Quat,
    mid_delta: Quat,
) -> Vec3 {
    let new_mid = root_pos + root_delta * (mid_pos - root_pos);
    new_mid + mid_delta * root_delta * (end_pos - mid_pos)
}

/// Solves multi-bone chain IK (Cyclic Coordinate Descent) for any valid parent-child chain
/// `chain[0] <- chain[1] <- ... <- chain[N-1]` (`N >= 2`) so the end bone `chain[N-1]` reaches `target`.
/// Returns the updated LOCAL pose rotations for `chain[0..N-1]`.
pub fn solve_chain_ik(
    rig: &CharacterRig,
    chain: &[BoneId],
    target: Vec3,
    max_iterations: u32,
    tolerance: f32,
) -> Result<Vec<(BoneId, Quat)>, CoreError> {
    if chain.len() < 2 || chain.len() > 32 || !target.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    rig.bone(chain[0]).ok_or(CoreError::OutOfBounds)?;
    for pair in chain.windows(2) {
        let child = rig.bone(pair[1]).ok_or(CoreError::OutOfBounds)?;
        if child.parent != Some(pair[0]) {
            return Err(CoreError::OutOfBounds);
        }
    }

    let iters = max_iterations.clamp(1, 64);
    let tol = if tolerance.is_finite() {
        tolerance.clamp(1e-4, 1.0)
    } else {
        1e-3
    };
    let end_id = chain[chain.len() - 1];
    let mut sim = rig.clone();

    for _ in 0..iters {
        let world = sim.world_pose_matrices();
        let Some(end_pos) = world.get(&end_id).map(|m| m.transform_point3(Vec3::ZERO)) else {
            return Err(CoreError::OutOfBounds);
        };
        if (end_pos - target).length() <= tol {
            break;
        }
        for &joint_id in chain[..chain.len() - 1].iter().rev() {
            let world = sim.world_pose_matrices();
            let (Some(j_mat), Some(e_mat)) = (world.get(&joint_id), world.get(&end_id)) else {
                continue;
            };
            let j_pos = j_mat.transform_point3(Vec3::ZERO);
            let e_pos = e_mat.transform_point3(Vec3::ZERO);
            let to_end = (e_pos - j_pos).normalize_or_zero();
            let to_tgt = (target - j_pos).normalize_or_zero();
            if to_end.length_squared() < 1e-8 || to_tgt.length_squared() < 1e-8 {
                continue;
            }
            let delta_world = Quat::from_rotation_arc(to_end, to_tgt);
            if !delta_world.is_finite() {
                continue;
            }
            let (_, j_world_rot, _) = j_mat.to_scale_rotation_translation();
            let parent_world_rot = sim
                .bone(joint_id)
                .and_then(|b| b.parent)
                .and_then(|pid| world.get(&pid))
                .map(|m| m.to_scale_rotation_translation().1)
                .unwrap_or(Quat::IDENTITY);
            let new_world_rot = (delta_world * j_world_rot).normalize();
            let new_local_rot = (parent_world_rot.conjugate() * new_world_rot).normalize();
            if new_local_rot.is_finite() {
                if let Some(bone) = sim.bone_mut(joint_id) {
                    bone.pose_local.rotation = new_local_rot;
                }
            }
        }
    }

    let mut out = Vec::with_capacity(chain.len() - 1);
    for &bid in &chain[..chain.len() - 1] {
        let rot = sim
            .bone(bid)
            .map(|b| b.pose_local.rotation)
            .ok_or(CoreError::OutOfBounds)?;
        out.push((bid, rot));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_chain_ik_converges_on_four_bone_chain() {
        let mut rig = CharacterRig::default();
        for i in 0..4u32 {
            rig.bones.push(crate::character::Bone::new(
                BoneId(i),
                format!("Seg{i}"),
                if i == 0 { None } else { Some(BoneId(i - 1)) },
                crate::scene::Transform {
                    translation: if i == 0 {
                        Vec3::ZERO
                    } else {
                        Vec3::new(0.0, 0.5, 0.0)
                    },
                    ..Default::default()
                },
            ));
        }
        let target = Vec3::new(1.0, 0.8, 0.2);
        let chain = [BoneId(0), BoneId(1), BoneId(2), BoneId(3)];
        let rots = solve_chain_ik(&rig, &chain, target, 32, 1e-3).unwrap();
        assert_eq!(rots.len(), 3);
        for (bid, rot) in rots {
            rig.bone_mut(bid).unwrap().pose_local.rotation = rot;
        }
        let mats = rig.world_pose_matrices();
        let end_pos = mats[&BoneId(3)].transform_point3(Vec3::ZERO);
        assert!(
            (end_pos - target).length() < 0.05,
            "4-bone CCD IK did not converge: end={end_pos:?}, target={target:?}"
        );
    }

    fn straight_leg() -> (Vec3, Vec3, Vec3) {
        (
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        )
    }

    #[test]
    fn ik_reaches_forward_target() {
        let (r, m, e) = straight_leg();
        let target = Vec3::new(0.0, 0.5, 0.5);
        let (dr, dm) = solve_two_bone_ik(r, m, e, target, Vec3::Z).unwrap();
        let got = apply_deltas_end_pos(r, m, e, dr, dm);
        assert!((got - target).length() < 1e-4, "got {got:?}");
    }

    #[test]
    fn ik_clamps_beyond_max_reach() {
        let (r, m, e) = straight_leg();
        let target = Vec3::new(0.0, 0.5, 5.0); // far beyond l1+l2 = 1.0
        let (dr, dm) = solve_two_bone_ik(r, m, e, target, Vec3::Z).unwrap();
        let got = apply_deltas_end_pos(r, m, e, dr, dm);
        let want = r + (target - r).normalize_or_zero() * 1.0;
        assert!((got - want).length() < 1e-3, "got {got:?} want {want:?}");
    }

    #[test]
    fn ik_survives_degenerate_pole_and_inputs() {
        let (r, m, e) = straight_leg();
        // Pole parallel to aim: fallback must still solve without NaN.
        let target = Vec3::new(0.0, 1.0, 1.0);
        let (dr, dm) = solve_two_bone_ik(r, m, e, target, Vec3::new(0.0, 0.0, 2.0)).unwrap();
        assert!(dr.is_finite() && dm.is_finite());
        // Zero pole: same fallback path.
        let (dr, dm) = solve_two_bone_ik(r, m, e, target, Vec3::ZERO).unwrap();
        assert!(dr.is_finite() && dm.is_finite());
        // Garbage in: clean None out.
        assert!(solve_two_bone_ik(r, m, e, Vec3::new(f32::NAN, 0.0, 0.0), Vec3::Z).is_none());
        assert!(solve_two_bone_ik(r, r, e, target, Vec3::Z).is_none()); // zero-length l1
        assert!(solve_two_bone_ik(r, m, e, r, Vec3::Z).is_none()); // target on root
    }

    #[test]
    fn ik_validates_rig_chain() {
        let rig = CharacterRig::biped_template();
        // Biped legs are single bones: no mid joint -> chain error, not panic.
        assert!(solve_for_rig(&rig, BoneId(0), BoneId(5), BoneId(6), Vec3::ZERO, None).is_err());
        // Unknown bones error cleanly.
        assert!(solve_for_rig(&rig, BoneId(90), BoneId(91), BoneId(92), Vec3::ZERO, None).is_err());
    }
}
