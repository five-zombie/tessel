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

/// A single bone's sampled pose: world-space joint position plus the
/// local rotation that produces it (matches `SetBoneRotation` space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampledBonePose {
    pub bone_id: BoneId,
    pub world_position: Vec3,
    pub local_rotation: Quat,
}

/// Independent animation clip layer applied to bones matching `bone_prefix`
/// (enables simultaneous per-character playback, e.g. P1 `Walk` + P2 `ShotFall`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipLayer {
    pub clip_index: usize,
    pub bone_prefix: String,
    #[serde(default)]
    pub time_offset: f32,
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
    /// `apply_active_animation` and applied live in `tessel-render` as well as glTF export.
    #[serde(default)]
    pub morph_weights: BTreeMap<(NodeId, String), f32>,
    /// Optional per-node bone allowlist restricting `AutoSkinSolver` influences
    /// (e.g. isolate leg/pelvis meshes from hanging arm bones).
    #[serde(default)]
    pub node_bone_masks: BTreeMap<NodeId, Vec<BoneId>>,
    /// Live sampled visibility overrides `node_id -> visible`, refreshed by
    /// `apply_active_animation` from the active clip's `visibility_tracks`.
    #[serde(default)]
    pub node_visibility: BTreeMap<NodeId, bool>,
    /// Live sampled dynamic parent-switch / prop-drop overrides `node_id -> ParentSwitchKey`,
    /// refreshed by `apply_active_animation` from `parent_switch_tracks`.
    #[serde(default)]
    pub node_parent_overrides: BTreeMap<NodeId, super::anim::ParentSwitchKey>,
    /// Live sampled per-node local TRS delta overrides `node_id -> Keyframe`,
    /// refreshed by `apply_active_animation` from `node_transform_tracks`.
    #[serde(default)]
    pub node_transform_overrides: BTreeMap<NodeId, super::nano::Keyframe>,
    /// Optional per-prefix clip layers evaluated on top of `active_animation`.
    #[serde(default)]
    pub clip_layers: Vec<ClipLayer>,
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
            // Right-Handed +Y Up, +Z Front: character's Left is +X, Right is -X.
            bone(3, "LeftArm", Some(1), Vec3::new(0.22, 0.35, 0.0)),
            bone(4, "RightArm", Some(1), Vec3::new(-0.22, 0.35, 0.0)),
            bone(5, "LeftLeg", Some(0), Vec3::new(0.11, -0.05, 0.0)),
            bone(6, "RightLeg", Some(0), Vec3::new(-0.11, -0.05, 0.0)),
        ];

        Self {
            bones,
            bindings: Vec::new(),
            animations: vec![AnimationClip::starter_idle(), AnimationClip::starter_walk()],
            active_animation: 0,
            playback_time: 0.0,
            playing: false,
            morph_weights: BTreeMap::new(),
            node_bone_masks: BTreeMap::new(),
            node_visibility: BTreeMap::new(),
            node_parent_overrides: BTreeMap::new(),
            node_transform_overrides: BTreeMap::new(),
            clip_layers: Vec::new(),
        }
    }

    /// Creates a 19-bone 1:1 humanoid skeleton (~1.82m tall, Right-Handed +Y Up, +Z Front: Left=+X, Right=-X).
    pub fn humanoid_template() -> Self {
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
            bone(0, "Root", None, Vec3::new(0.0, 0.92, 0.0)),
            bone(1, "Spine", Some(0), Vec3::new(0.0, 0.16, 0.0)),
            bone(2, "Chest", Some(1), Vec3::new(0.0, 0.24, 0.0)),
            bone(3, "Neck", Some(2), Vec3::new(0.0, 0.16, 0.0)),
            bone(4, "Head", Some(3), Vec3::new(0.0, 0.12, 0.0)),
            // Left arm (+X)
            bone(5, "LeftShoulder", Some(2), Vec3::new(0.10, 0.10, 0.0)),
            bone(6, "LeftUpperArm", Some(5), Vec3::new(0.12, -0.02, 0.0)),
            bone(7, "LeftForearm", Some(6), Vec3::new(0.04, -0.28, 0.0)),
            bone(8, "LeftHand", Some(7), Vec3::new(0.03, -0.24, 0.02)),
            // Right arm (-X)
            bone(9, "RightShoulder", Some(2), Vec3::new(-0.10, 0.10, 0.0)),
            bone(10, "RightUpperArm", Some(9), Vec3::new(-0.12, -0.02, 0.0)),
            bone(11, "RightForearm", Some(10), Vec3::new(-0.04, -0.28, 0.0)),
            bone(12, "RightHand", Some(11), Vec3::new(-0.03, -0.24, 0.02)),
            // Left leg (+X)
            bone(13, "LeftThigh", Some(0), Vec3::new(0.11, -0.04, 0.0)),
            bone(14, "LeftCalf", Some(13), Vec3::new(0.0, -0.38, 0.0)),
            bone(15, "LeftFoot", Some(14), Vec3::new(0.0, -0.40, 0.04)),
            // Right leg (-X)
            bone(16, "RightThigh", Some(0), Vec3::new(-0.11, -0.04, 0.0)),
            bone(17, "RightCalf", Some(16), Vec3::new(0.0, -0.38, 0.0)),
            bone(18, "RightFoot", Some(17), Vec3::new(0.0, -0.40, 0.04)),
        ];

        Self {
            bones,
            bindings: Vec::new(),
            animations: vec![
                AnimationClip::humanoid_idle(),
                AnimationClip::humanoid_walk(),
            ],
            active_animation: 0,
            playback_time: 0.0,
            playing: false,
            morph_weights: BTreeMap::new(),
            node_bone_masks: BTreeMap::new(),
            node_visibility: BTreeMap::new(),
            node_parent_overrides: BTreeMap::new(),
            node_transform_overrides: BTreeMap::new(),
            clip_layers: Vec::new(),
        }
    }

    pub fn bone(&self, id: BoneId) -> Option<&Bone> {
        self.bones.iter().find(|b| b.id == id)
    }

    pub fn bone_mut(&mut self, id: BoneId) -> Option<&mut Bone> {
        self.bones.iter_mut().find(|b| b.id == id)
    }

    /// Computes world-space rest transform matrices for every bone in hierarchy order.
    pub fn world_rest_matrices(&self) -> BTreeMap<BoneId, Mat4> {
        let mut world = BTreeMap::new();
        for bone in &self.bones {
            let local = Mat4::from_scale_rotation_translation(
                bone.rest_local.scale,
                bone.rest_local.rotation,
                bone.rest_local.translation,
            );
            let mat = match bone.parent.and_then(|pid| world.get(&pid)) {
                Some(&parent_mat) => parent_mat * local,
                None => local,
            };
            world.insert(bone.id, mat);
        }
        world
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
    /// Samples `clip_index` at `time_secs` (wrapping within the clip
    /// duration) without touching live poses. Returns one entry per bone in
    /// hierarchy order, mirroring `apply_active_animation` math. `None` for
    /// a missing clip.
    pub fn sample_clip_pose(
        &self,
        clip_index: usize,
        time_secs: f32,
    ) -> Option<Vec<SampledBonePose>> {
        let clip = self.animations.get(clip_index)?;
        let t = if clip.duration_secs > 0.0 {
            time_secs.max(0.0) % clip.duration_secs
        } else {
            0.0
        };
        let mut world_mats: BTreeMap<BoneId, Mat4> = BTreeMap::new();
        let mut out = Vec::with_capacity(self.bones.len());
        for bone in &self.bones {
            let sampled = clip.sample_bone(bone.id, t);
            let local_t =
                bone.rest_local.translation + sampled.map(|k| k.translation).unwrap_or(Vec3::ZERO);
            let local_r =
                bone.rest_local.rotation * sampled.map(|k| k.rotation).unwrap_or(Quat::IDENTITY);
            let local_s = bone.rest_local.scale * sampled.map(|k| k.scale).unwrap_or(Vec3::ONE);
            let local = Mat4::from_scale_rotation_translation(local_s, local_r, local_t);
            let world = match bone.parent.and_then(|p| world_mats.get(&p)) {
                Some(parent_world) => *parent_world * local,
                None => local,
            };
            let (_, _, world_pos) = world.to_scale_rotation_translation();
            world_mats.insert(bone.id, world);
            out.push(SampledBonePose {
                bone_id: bone.id,
                world_position: world_pos,
                local_rotation: local_r,
            });
        }
        Some(out)
    }

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
        self.node_visibility.clear();
        for (node, visible) in clip.sample_node_visibility(t) {
            self.node_visibility.insert(node, visible);
        }
        self.node_parent_overrides.clear();
        for (node, key) in clip.sample_parent_switches(t) {
            self.node_parent_overrides.insert(node, key);
        }
        self.node_transform_overrides.clear();
        for (node, kf) in clip.sample_node_transforms(t) {
            self.node_transform_overrides.insert(node, kf);
        }

        // Apply optional per-prefix clip layers on top of the active clip.
        let layers = self.clip_layers.clone();
        for layer in layers {
            let Some(layer_clip) = self.animations.get(layer.clip_index).cloned() else {
                continue;
            };
            let lt = if layer_clip.duration_secs > 0.0 {
                (self.playback_time + layer.time_offset).rem_euclid(layer_clip.duration_secs)
            } else {
                0.0
            };
            for bone in &mut self.bones {
                if !layer.bone_prefix.is_empty() && !bone.name.starts_with(&layer.bone_prefix) {
                    continue;
                }
                bone.pose_local = bone.rest_local;
                if let Some(sampled) = layer_clip.sample_bone(bone.id, lt) {
                    bone.pose_local.translation = bone.rest_local.translation + sampled.translation;
                    bone.pose_local.rotation = bone.rest_local.rotation * sampled.rotation;
                    bone.pose_local.scale = bone.rest_local.scale * sampled.scale;
                }
            }
            for (node, target, weight) in layer_clip.sample_morph_weights(lt) {
                if weight.is_finite() {
                    self.morph_weights.insert((node, target), weight);
                }
            }
            for (node, visible) in layer_clip.sample_node_visibility(lt) {
                self.node_visibility.insert(node, visible);
            }
            for (node, key) in layer_clip.sample_parent_switches(lt) {
                self.node_parent_overrides.insert(node, key);
            }
            for (node, kf) in layer_clip.sample_node_transforms(lt) {
                self.node_transform_overrides.insert(node, kf);
            }
        }
    }

    /// Computes the live world transform matrix of a node driven by a dynamic `ParentSwitchKey`.
    pub fn compute_node_override_matrix(
        key: super::anim::ParentSwitchKey,
        base_node_mat: Mat4,
        world_mats: &BTreeMap<BoneId, Mat4>,
        rest_mats: &BTreeMap<BoneId, Mat4>,
    ) -> Mat4 {
        if let Some(bid) = key.bone_id {
            let bone_world = world_mats.get(&bid).copied().unwrap_or(Mat4::IDENTITY);
            let bone_rest = rest_mats.get(&bid).copied().unwrap_or(Mat4::IDENTITY);
            let offset = Mat4::from_rotation_translation(key.rotation, key.position);
            bone_world * offset * bone_rest.inverse() * base_node_mat
        } else {
            let (base_scale, _, _) = base_node_mat.to_scale_rotation_translation();
            Mat4::from_scale_rotation_translation(base_scale, key.rotation, key.position)
        }
    }

    /// Finds the root bone and all bone IDs belonging to `bone_prefix`
    /// (`""` matches primary bones without `'_'`; otherwise matches `name.starts_with(bone_prefix)`).
    pub fn sub_skeleton_root_and_bones(&self, bone_prefix: &str) -> Option<(BoneId, Vec<BoneId>)> {
        let mut root_id = None;
        let mut ids = Vec::new();
        for b in &self.bones {
            let matches = if bone_prefix.is_empty() {
                !b.name.contains('_')
            } else {
                b.name.starts_with(bone_prefix)
            };
            if matches {
                if b.parent.is_none() && root_id.is_none() {
                    root_id = Some(b.id);
                }
                ids.push(b.id);
            }
        }
        root_id.map(|r| (r, ids))
    }

    /// Shifts the sub-skeleton's root bone Y translation so its lowest joint sits at `floor_y`.
    /// Returns `(root_id, min_y_before, delta_y)`.
    pub fn clamp_sub_skeleton_to_floor(
        &mut self,
        bone_prefix: &str,
        floor_y: f32,
    ) -> Option<(BoneId, f32, f32)> {
        self.clamp_sub_skeleton_to_floor_with_bounds(bone_prefix, floor_y, None)
    }

    /// Shifts the sub-skeleton's root bone Y translation so its lowest skinned mesh vertex
    /// (when `bounds` has vertices for this sub-skeleton) or lowest joint sits at `floor_y`.
    /// Returns `(root_id, min_y_before, delta_y)`.
    pub fn clamp_sub_skeleton_to_floor_with_bounds(
        &mut self,
        bone_prefix: &str,
        floor_y: f32,
        bounds: Option<&super::skin::SkinnedBoundsCache>,
    ) -> Option<(BoneId, f32, f32)> {
        let (root_id, bone_ids) = self.sub_skeleton_root_and_bones(bone_prefix)?;
        let min_y = if let Some(skinned_min_y) =
            bounds.and_then(|cache| cache.sub_skeleton_min_y(self, bone_prefix))
        {
            skinned_min_y
        } else {
            let world_mats = self.world_pose_matrices();
            let mut joint_min_y = f32::INFINITY;
            for id in &bone_ids {
                if let Some(mat) = world_mats.get(id) {
                    joint_min_y = joint_min_y.min(mat.transform_point3(Vec3::ZERO).y);
                }
            }
            joint_min_y
        };
        if !min_y.is_finite() {
            return None;
        }
        let delta_y = floor_y - min_y;
        let root = self.bones.iter_mut().find(|b| b.id == root_id)?;
        root.pose_local.translation.y += delta_y;
        Some((root_id, min_y, delta_y))
    }

    /// Bakes the current `clip_layers` composition (or active clip if `clip_layers` is empty)
    /// into a standalone `AnimationClip` ready for `.glb` / `.gif` export.
    pub fn bake_clip_layers(
        &self,
        name: String,
        fps: f32,
        duration_override: Option<f32>,
        clamp_floor_y: Option<f32>,
    ) -> Option<AnimationClip> {
        self.bake_clip_layers_with_bounds(name, fps, duration_override, clamp_floor_y, None)
    }

    /// Bakes the current `clip_layers` composition into a standalone `AnimationClip`,
    /// optionally clamping each sub-skeleton to `clamp_floor_y` using skinned mesh vertices
    /// when `bounds` is provided.
    pub fn bake_clip_layers_with_bounds(
        &self,
        name: String,
        fps: f32,
        duration_override: Option<f32>,
        clamp_floor_y: Option<f32>,
        bounds: Option<&super::skin::SkinnedBoundsCache>,
    ) -> Option<AnimationClip> {
        if self.bones.is_empty() || self.animations.is_empty() {
            return None;
        }
        let duration_secs = if let Some(d) = duration_override {
            d.max(0.05)
        } else if !self.clip_layers.is_empty() {
            self.clip_layers
                .iter()
                .filter_map(|l| self.animations.get(l.clip_index).map(|c| c.duration_secs))
                .fold(0.0_f32, f32::max)
                .max(0.1)
        } else {
            self.animations
                .get(self.active_animation)
                .map(|c| c.duration_secs)
                .unwrap_or(1.0)
                .max(0.1)
        };
        let steps = ((duration_secs * fps.clamp(1.0, 120.0)).round() as usize).max(1);

        // Collect all sub-skeleton prefixes present in the rig for optional floor clamping.
        let mut prefixes: Vec<String> = Vec::new();
        for b in &self.bones {
            if b.parent.is_none() {
                let pfx = match b.name.find('_') {
                    Some(pos) => b.name[..=pos].to_string(),
                    None => String::new(),
                };
                if !prefixes.contains(&pfx) {
                    prefixes.push(pfx);
                }
            }
        }

        let mut baked = AnimationClip::new_empty(name, duration_secs);
        let mut sim = self.clone();
        let mut last_vis: std::collections::BTreeMap<NodeId, bool> =
            std::collections::BTreeMap::new();

        for step in 0..=steps {
            let t = if step == steps {
                duration_secs
            } else {
                (step as f32 / steps as f32) * duration_secs
            };
            // Avoid modulo wrap to 0.0 on the final frame by clamping slightly below duration.
            sim.playback_time = if step == steps {
                (duration_secs - 1e-4).max(0.0)
            } else {
                t
            };
            sim.apply_active_animation();
            if let Some(fy) = clamp_floor_y {
                for pfx in &prefixes {
                    let _ = sim.clamp_sub_skeleton_to_floor_with_bounds(pfx, fy, bounds);
                }
            }
            for b in &sim.bones {
                let rot_delta = b.rest_local.rotation.inverse() * b.pose_local.rotation;
                let trans_delta = b.pose_local.translation - b.rest_local.translation;
                let scale_delta = Vec3::new(
                    if b.rest_local.scale.x.abs() > 1e-6 {
                        b.pose_local.scale.x / b.rest_local.scale.x
                    } else {
                        1.0
                    },
                    if b.rest_local.scale.y.abs() > 1e-6 {
                        b.pose_local.scale.y / b.rest_local.scale.y
                    } else {
                        1.0
                    },
                    if b.rest_local.scale.z.abs() > 1e-6 {
                        b.pose_local.scale.z / b.rest_local.scale.z
                    } else {
                        1.0
                    },
                );
                baked.upsert_keyframe(
                    b.id,
                    crate::character::Keyframe {
                        time_secs: t,
                        translation: trans_delta,
                        rotation: rot_delta,
                        scale: scale_delta,
                        interpolation: crate::character::KeyframeInterpolation::Linear,
                    },
                );
            }
            for ((node_id, target), &weight) in &sim.morph_weights {
                baked.upsert_morph_weight(*node_id, target, t, weight);
            }
            for (&node_id, &visible) in &sim.node_visibility {
                if step == 0 || last_vis.get(&node_id).copied() != Some(visible) {
                    baked.upsert_visibility_key(node_id, t, visible);
                    last_vis.insert(node_id, visible);
                }
            }
            for (&node_id, &ps_key) in &sim.node_parent_overrides {
                baked.upsert_parent_switch_key(
                    node_id,
                    super::anim::ParentSwitchKey {
                        time_secs: t,
                        ..ps_key
                    },
                );
            }
            for (&node_id, &nt_kf) in &sim.node_transform_overrides {
                baked.upsert_node_transform_key(
                    node_id,
                    super::nano::Keyframe {
                        time_secs: t,
                        ..nt_kf
                    },
                );
            }
        }
        Some(baked)
    }

    /// Samples in-between frames of `clip_index` at `fps` and adjusts the sub-skeleton's
    /// root bone Y translation keyframes so the character never penetrates or floats above `floor_y`.
    pub fn clamp_clip_to_floor(
        &self,
        clip_index: usize,
        bone_prefix: &str,
        floor_y: f32,
        fps: f32,
    ) -> Option<AnimationClip> {
        self.clamp_clip_to_floor_with_bounds(clip_index, bone_prefix, floor_y, fps, None)
    }

    /// Samples in-between frames of `clip_index` at `fps` and adjusts the sub-skeleton's
    /// root bone Y translation keyframes so the lowest skinned mesh vertex (or joint fallback)
    /// sits flush on `floor_y`.
    pub fn clamp_clip_to_floor_with_bounds(
        &self,
        clip_index: usize,
        bone_prefix: &str,
        floor_y: f32,
        fps: f32,
        bounds: Option<&super::skin::SkinnedBoundsCache>,
    ) -> Option<AnimationClip> {
        let mut clip = self.animations.get(clip_index)?.clone();
        let (root_id, _) = self.sub_skeleton_root_and_bones(bone_prefix)?;
        let duration = clip.duration_secs.max(0.05);
        let steps = ((duration * fps.clamp(1.0, 120.0)).round() as usize).max(1);

        let mut sim = self.clone();
        sim.clip_layers.clear();
        sim.active_animation = clip_index;

        for step in 0..=steps {
            let t = if step == steps {
                duration
            } else {
                (step as f32 / steps as f32) * duration
            };
            sim.playback_time = if step == steps {
                (duration - 1e-4).max(0.0)
            } else {
                t
            };
            sim.apply_active_animation();
            let _ = sim.clamp_sub_skeleton_to_floor_with_bounds(bone_prefix, floor_y, bounds);
            if let Some(root) = sim.bones.iter().find(|b| b.id == root_id) {
                let rot_delta = root.rest_local.rotation.inverse() * root.pose_local.rotation;
                let trans_delta = root.pose_local.translation - root.rest_local.translation;
                let existing_interp = clip
                    .channels
                    .iter()
                    .find(|c| c.bone_id == root_id)
                    .and_then(|c| c.keyframes.iter().find(|k| (k.time_secs - t).abs() <= 1e-3))
                    .map(|k| k.interpolation)
                    .unwrap_or(crate::character::KeyframeInterpolation::Linear);
                clip.upsert_keyframe(
                    root_id,
                    crate::character::Keyframe {
                        time_secs: t,
                        translation: trans_delta,
                        rotation: rot_delta,
                        scale: Vec3::ONE,
                        interpolation: existing_interp,
                    },
                );
            }
        }
        Some(clip)
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

    #[test]
    fn sample_clip_pose_matches_rest_and_keyframes() {
        let rig = CharacterRig::biped_template();
        // Idle (clip 0) has Torso translation keys [0, .03, 0]; LeftArm is static.
        let at_half = rig.sample_clip_pose(0, 0.5).unwrap();
        assert_eq!(at_half.len(), rig.bones.len());
        let torso = at_half.iter().find(|s| s.bone_id == BoneId(1)).unwrap();
        // Rest torso world pos is (0, 1.1, 0); +0.015 breathing offset at t=0.5.
        assert!((torso.world_position.y - 1.115).abs() < 1e-4);
        assert!(torso.local_rotation.angle_between(Quat::IDENTITY) < 1e-6);
        // Unknown clip returns None.
        assert!(rig.sample_clip_pose(99, 0.0).is_none());
        // Empty rig samples fine (zero bones).
        let empty = CharacterRig::default();
        assert_eq!(empty.sample_clip_pose(0, 0.0), None);
    }
}
