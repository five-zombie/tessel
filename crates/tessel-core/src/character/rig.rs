use std::collections::BTreeMap;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{NodeId, Transform};

use super::anim::AnimationClip;
use super::bone::Bone;
use super::nano::BoneId;

/// Rigid binding of a scene part node to a single skeletal bone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinBinding {
    pub node_id: NodeId,
    pub bone_id: BoneId,
}

/// Unit Anchor: Hierarchical humanoid/custom character rig with starter animations.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CharacterRig {
    pub bones: Vec<Bone>,
    pub bindings: Vec<SkinBinding>,
    pub animations: Vec<AnimationClip>,
    pub active_animation: usize,
    pub playback_time: f32,
    pub playing: bool,
    /// Live sampled morph weights `(node, target) -> weight`, refreshed by
    /// `apply_active_animation`. The viewport renderer does not deform meshes
    /// with these yet; export bakes animated weights into glTF channels.
    #[serde(default)]
    pub morph_weights: BTreeMap<(NodeId, String), f32>,
}

impl CharacterRig {
    /// Creates a standard 7-bone biped humanoid skeleton with starter Idle & Walk animations.
    pub fn biped_template() -> Self {
        let bone = |id: u32, name: &str, parent: Option<u32>, pos: Vec3| {
            Bone::new(
                BoneId(id),
                name,
                parent.map(BoneId),
                Transform {
                    translation: pos,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
            )
        };

        let bones = vec![
            bone(0, "Root", None, Vec3::new(0.0, 0.8, 0.0)),
            bone(1, "Torso", Some(0), Vec3::new(0.0, 0.3, 0.0)),
            bone(2, "Head", Some(1), Vec3::new(0.0, 0.45, 0.0)),
            bone(3, "LeftArm", Some(1), Vec3::new(-0.35, 0.35, 0.0)),
            bone(4, "RightArm", Some(1), Vec3::new(0.35, 0.35, 0.0)),
            bone(5, "LeftLeg", Some(0), Vec3::new(-0.18, -0.05, 0.0)),
            bone(6, "RightLeg", Some(0), Vec3::new(0.18, -0.05, 0.0)),
        ];

        Self {
            bones,
            bindings: Vec::new(),
            animations: vec![AnimationClip::starter_idle(), AnimationClip::starter_walk()],
            active_animation: 0,
            playback_time: 0.0,
            playing: false,
            morph_weights: BTreeMap::new(),
        }
    }

    pub fn bone(&self, id: BoneId) -> Option<&Bone> {
        self.bones.iter().find(|b| b.id == id)
    }

    pub fn bone_mut(&mut self, id: BoneId) -> Option<&mut Bone> {
        self.bones.iter_mut().find(|b| b.id == id)
    }

    /// Computes world-space transform matrices for every bone in hierarchy order.
    pub fn world_pose_matrices(&self) -> BTreeMap<BoneId, Mat4> {
        let mut world = BTreeMap::new();
        for bone in &self.bones {
            let local = Mat4::from_scale_rotation_translation(
                bone.pose_local.scale,
                bone.pose_local.rotation,
                bone.pose_local.translation,
            );
            let mat = match bone.parent.and_then(|pid| world.get(&pid)) {
                Some(&parent_mat) => parent_mat * local,
                None => local,
            };
            world.insert(bone.id, mat);
        }
        world
    }

    /// Evaluates the active `AnimationClip` at `playback_time` into bone poses.
    pub fn apply_active_animation(&mut self) {
        let Some(clip) = self.animations.get(self.active_animation).cloned() else {
            return;
        };
        let t = if clip.duration_secs > 0.0 {
            self.playback_time % clip.duration_secs
        } else {
            0.0
        };
        for bone in &mut self.bones {
            bone.pose_local = bone.rest_local;
            if let Some(sampled) = clip.sample_bone(bone.id, t) {
                bone.pose_local.translation = bone.rest_local.translation + sampled.translation;
                bone.pose_local.rotation = bone.rest_local.rotation * sampled.rotation;
                bone.pose_local.scale = bone.rest_local.scale * sampled.scale;
            }
        }
        self.morph_weights.clear();
        for (node, target, weight) in clip.sample_morph_weights(t) {
            if weight.is_finite() {
                self.morph_weights.insert((node, target), weight);
            }
        }
    }

    /// Picks the closest bone joint within `pick_radius` of a world-space ray.
    pub fn pick_bone(&self, ray_origin: Vec3, ray_dir: Vec3, pick_radius: f32) -> Option<BoneId> {
        let dir = ray_dir.normalize_or_zero();
        if dir.length_squared() < 1e-6 {
            return None;
        }
        let mats = self.world_pose_matrices();
        let mut best: Option<(BoneId, f32)> = None;
        for (id, mat) in mats {
            let joint = mat.transform_point3(Vec3::ZERO);
            let to_joint = joint - ray_origin;
            let proj = to_joint.dot(dir);
            if proj <= 0.0 {
                continue;
            }
            let closest_pt = ray_origin + dir * proj;
            let dist = (joint - closest_pt).length();
            if dist <= pick_radius && best.is_none_or(|(_, best_d)| dist < best_d) {
                best = Some((id, dist));
            }
        }
        best.map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_bone_selects_head_joint_along_ray() {
        let rig = CharacterRig::biped_template();
        // Head is at y = 0.8 + 0.3 + 0.45 = 1.55
        let hit = rig.pick_bone(Vec3::new(0.0, 1.55, 3.0), Vec3::new(0.0, 0.0, -1.0), 0.15);
        assert_eq!(hit, Some(BoneId(2)));
    }

    #[test]
    fn apply_active_animation_samples_morph_weights() {
        let mut rig = CharacterRig::biped_template();
        let node = NodeId(3);
        rig.animations[0].upsert_morph_weight(node, "Smile", 0.0, 0.0);
        rig.animations[0].upsert_morph_weight(node, "Smile", 2.0, 1.0);
        rig.playback_time = 0.5;
        rig.apply_active_animation();
        let w = rig.morph_weights.get(&(node, "Smile".to_string())).copied();
        assert!((w.unwrap_or(-1.0) - 0.25).abs() < 1e-6);
    }
}
