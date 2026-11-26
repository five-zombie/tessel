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

/// World-space surface contact hit returned by vertical raycast grounding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceHit {
    pub y: f32,
    pub normal: Vec3,
}

/// 3-bone limb contact chain (`root` -> `mid` -> `end`) for generalized foot/wheel/quadruped grounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactChain {
    pub root: BoneId,
    pub mid: BoneId,
    pub end: BoneId,
}

/// Configuration options for v4.9 Surface-Raycast Grounding v2.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundingOptions {
    pub solve_foot_ik: bool,
    pub align_foot_normal: bool,
    pub pelvis_tilt: bool,
    /// Height above reference foot/root to start downward rays (prevents snapping to overhead bridges/caves).
    pub step_up_clearance: f32,
    /// Half-length of heel-to-toe multi-ray footprint sweep (`0.0` = single center ray).
    pub footprint_half_length: f32,
    /// Optional explicit 3-bone contact chains; when empty, auto-discovers humanoid Left/Right leg chains.
    pub contact_chains: Vec<ContactChain>,
}

impl Default for GroundingOptions {
    fn default() -> Self {
        Self {
            solve_foot_ik: false,
            align_foot_normal: false,
            pelvis_tilt: false,
            step_up_clearance: 1.0,
            footprint_half_length: 0.08,
            contact_chains: Vec::new(),
        }
    }
}

/// Single surface source (either a `LowPolyMesh` with a prebuilt `MeshBvh`, or a `VoxelVolume`) in world space.
#[derive(Debug, Clone)]
pub enum SurfaceSource<'a> {
    Mesh {
        mesh: &'a crate::mesh::LowPolyMesh,
        bvh: crate::mesh::MeshBvh,
        world_mat: Mat4,
        world_inv: Mat4,
        normal_mat: Mat4,
    },
    Voxel {
        volume: &'a crate::voxel::VoxelVolume,
        unit_scale: f32,
        world_mat: Mat4,
        world_inv: Mat4,
        normal_mat: Mat4,
    },
}

/// Parameter bundle for clamping an animation clip onto flat or mesh/voxel surfaces.
#[derive(Debug, Clone, Copy)]
pub struct ClipGroundingRequest<'a> {
    pub clip_index: usize,
    pub bone_prefix: &'a str,
    pub fallback_floor_y: f32,
    pub fps: f32,
    pub bounds: Option<&'a super::skin::SkinnedBoundsCache>,
    pub surface: Option<&'a SurfaceTarget<'a>>,
    pub options: &'a GroundingOptions,
}
#[derive(Debug, Clone)]
pub struct SurfaceTarget<'a> {
    pub sources: Vec<SurfaceSource<'a>>,
}

impl<'a> SurfaceTarget<'a> {
    pub fn from_scene(scene: &'a crate::scene::SceneState, node_id: NodeId) -> Option<Self> {
        Self::from_scene_nodes(scene, &[node_id])
    }

    /// Collects all valid `LowPolyMesh` and `VoxelVolume` surface sources for `node_ids`.
    pub fn from_scene_nodes(
        scene: &'a crate::scene::SceneState,
        node_ids: &[NodeId],
    ) -> Option<Self> {
        let mut sources = Vec::new();
        for &node_id in node_ids {
            let world_mat = scene.node_world_matrix(node_id);
            if world_mat.determinant().abs() <= 1e-8 {
                continue;
            }
            let world_inv = world_mat.inverse();
            let normal_mat = world_inv.transpose();
            if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
                if !mesh.faces.is_empty() {
                    sources.push(SurfaceSource::Mesh {
                        mesh,
                        bvh: crate::mesh::MeshBvh::from_mesh(mesh),
                        world_mat,
                        world_inv,
                        normal_mat,
                    });
                    continue;
                }
            }
            if let Some(volume) = scene.voxel_volumes.get(&node_id) {
                if !volume.is_empty() {
                    sources.push(SurfaceSource::Voxel {
                        volume,
                        unit_scale: scene.unit_scale.max(1e-4),
                        world_mat,
                        world_inv,
                        normal_mat,
                    });
                }
            }
        }
        (!sources.is_empty()).then_some(Self { sources })
    }

    /// Casts a vertical downward ray (`-Y`) starting at `(world_x, start_y, world_z)` across all
    /// surface sources and returns the highest hit below `start_y` (with fallback from `y = 10_000.0`
    /// if the character starts below all surfaces).
    pub fn sample_surface_hit(
        &self,
        world_x: f32,
        world_z: f32,
        start_y: f32,
    ) -> Option<SurfaceHit> {
        let local_start = if start_y.is_finite() {
            start_y
        } else {
            10_000.0
        };
        self.cast_downward_from(world_x, world_z, local_start)
            .or_else(|| {
                if local_start < 9_999.0 {
                    self.cast_downward_from(world_x, world_z, 10_000.0)
                } else {
                    None
                }
            })
    }

    fn cast_downward_from(&self, world_x: f32, world_z: f32, start_y: f32) -> Option<SurfaceHit> {
        let ray_origin_w = Vec3::new(world_x, start_y, world_z);
        let mut best: Option<SurfaceHit> = None;

        for source in &self.sources {
            let candidate = match source {
                SurfaceSource::Mesh {
                    mesh: _,
                    bvh,
                    world_mat,
                    world_inv,
                    normal_mat,
                } => {
                    let local_orig = world_inv.transform_point3(ray_origin_w);
                    let local_dir = world_inv.transform_vector3(-Vec3::Y).normalize_or_zero();
                    bvh.raycast(local_orig, local_dir).and_then(|hit| {
                        let wp = world_mat.transform_point3(hit.point);
                        if !wp.y.is_finite() || wp.y > start_y + 1e-2 {
                            return None;
                        }
                        let mut wn = normal_mat.transform_vector3(hit.normal).normalize_or_zero();
                        if wn.length_squared() < 1e-6 {
                            wn = Vec3::Y;
                        } else if wn.y < 0.0 {
                            wn = -wn;
                        }
                        Some(SurfaceHit {
                            y: wp.y,
                            normal: wn,
                        })
                    })
                }
                SurfaceSource::Voxel {
                    volume,
                    unit_scale,
                    world_mat,
                    world_inv,
                    normal_mat,
                } => {
                    let volume = *volume;
                    let unit_scale = *unit_scale;
                    let local_orig = world_inv.transform_point3(ray_origin_w);
                    let local_dir = world_inv.transform_vector3(-Vec3::Y).normalize_or_zero();
                    crate::voxel::ops::raycast_voxels(
                        volume, local_orig, local_dir, unit_scale, 512,
                    )
                    .and_then(|hit| {
                        let top_local = Vec3::new(
                            (hit.voxel.x as f32 + 0.5) * unit_scale,
                            (hit.voxel.y as f32 + 1.0) * unit_scale,
                            (hit.voxel.z as f32 + 0.5) * unit_scale,
                        );
                        let wp = world_mat.transform_point3(top_local);
                        if !wp.y.is_finite() || wp.y > start_y + 1e-2 {
                            return None;
                        }
                        let mut wn = normal_mat.transform_vector3(Vec3::Y).normalize_or_zero();
                        if wn.length_squared() < 1e-6 {
                            wn = Vec3::Y;
                        } else if wn.y < 0.0 {
                            wn = -wn;
                        }
                        Some(SurfaceHit {
                            y: wp.y,
                            normal: wn,
                        })
                    })
                }
            };

            if let Some(hit) = candidate {
                if best.is_none_or(|b| hit.y > b.y) {
                    best = Some(hit);
                }
            }
        }
        best
    }

    /// Multi-ray heel+toe footprint sweep returning the highest contact height and averaged normal.
    pub fn sample_surface_footprint(
        &self,
        world_x: f32,
        world_z: f32,
        start_y: f32,
        forward_xz: Vec3,
        half_length: f32,
    ) -> Option<SurfaceHit> {
        let center_hit = self.sample_surface_hit(world_x, world_z, start_y);
        if half_length <= 1e-4 {
            return center_hit;
        }
        let fwd = Vec3::new(forward_xz.x, 0.0, forward_xz.z).normalize_or_zero();
        if fwd.length_squared() < 1e-6 {
            return center_hit;
        }
        let toe_hit = self.sample_surface_hit(
            world_x + fwd.x * half_length,
            world_z + fwd.z * half_length,
            start_y,
        );
        let heel_hit = self.sample_surface_hit(
            world_x - fwd.x * half_length,
            world_z - fwd.z * half_length,
            start_y,
        );

        let mut max_y = f32::NEG_INFINITY;
        let mut normal_sum = Vec3::ZERO;
        let mut count = 0u32;
        for h in [center_hit, toe_hit, heel_hit].into_iter().flatten() {
            max_y = max_y.max(h.y);
            normal_sum += h.normal;
            count += 1;
        }
        if count == 0 || !max_y.is_finite() {
            return None;
        }
        let normal = normal_sum.normalize_or_zero();
        Some(SurfaceHit {
            y: max_y,
            normal: if normal.length_squared() > 1e-6 {
                normal
            } else {
                Vec3::Y
            },
        })
    }

    /// Casts a vertical downward ray (`-Y`) at `(world_x, world_z)` and returns the surface hit Y in world space.
    pub fn sample_surface_y(&self, world_x: f32, world_z: f32) -> Option<f32> {
        self.sample_surface_hit(world_x, world_z, 10_000.0)
            .map(|h| h.y)
    }
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
    /// `apply_active_animation` and applied live in `tessel-render`.
    /// Transient runtime cache — skipped in JSON serialization because tuple keys are not valid JSON object keys.
    #[serde(skip)]
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
    /// Persistent per-chain IK pole vectors keyed by end-effector `BoneId` (`v5.5`).
    #[serde(default)]
    pub chain_poles: BTreeMap<BoneId, Vec3>,
    /// Deterministic rig constraints evaluated during pose sampling (`v5.6`).
    #[serde(default)]
    pub constraints: Vec<super::constraints::BoneConstraint>,
    /// Named bone groups / visibility & lock layers (`v5.6`).
    #[serde(default)]
    pub bone_groups: BTreeMap<String, super::constraints::BoneGroup>,
    /// Named attachment sockets anchored to bones (`v5.6`).
    #[serde(default)]
    pub sockets: Vec<super::constraints::RigSocket>,
    /// Named saved poses for one-click apply / X-flipped paste (`v5.7`).
    #[serde(default)]
    pub pose_library: BTreeMap<String, BTreeMap<BoneId, Transform>>,
    /// Optional dedicated root-motion bone ID (`v5.6`).
    #[serde(default)]
    pub root_motion_bone: Option<BoneId>,
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
            ..Self::default()
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
            ..Self::default()
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
        let mut world: BTreeMap<BoneId, Mat4> = BTreeMap::new();
        for bone in &self.bones {
            let local = Mat4::from_scale_rotation_translation(
                bone.pose_local.scale,
                bone.pose_local.rotation,
                bone.pose_local.translation,
            );
            let mat = match bone.parent.and_then(|pid| world.get(&pid)) {
                Some(&parent_mat) => {
                    if bone.inherit_rotation && bone.inherit_scale {
                        parent_mat * local
                    } else {
                        let (ps, pr, pt) = parent_mat.to_scale_rotation_translation();
                        let eff_s = if bone.inherit_scale { ps } else { Vec3::ONE };
                        let eff_r = if bone.inherit_rotation {
                            pr
                        } else {
                            Quat::IDENTITY
                        };
                        let world_pos = pt + pr * (ps * bone.pose_local.translation);
                        Mat4::from_scale_rotation_translation(
                            eff_s * bone.pose_local.scale,
                            (eff_r * bone.pose_local.rotation).normalize(),
                            world_pos,
                        )
                    }
                }
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
            if self.playback_time >= 0.0 && (self.playback_time - clip.duration_secs).abs() <= 1e-4
            {
                clip.duration_secs
            } else {
                self.playback_time.rem_euclid(clip.duration_secs)
            }
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
                let matches = if layer.bone_prefix.is_empty() {
                    !bone.name.contains('_')
                } else {
                    bone.name.starts_with(&layer.bone_prefix)
                };
                if !matches {
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
        if !self.constraints.is_empty() {
            super::constraints::evaluate_rig_constraints(self);
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
            let bone_rest_inv = if bone_rest.determinant().abs() > 1e-8 {
                bone_rest.inverse()
            } else {
                Mat4::IDENTITY
            };
            bone_world * offset * bone_rest_inv * base_node_mat
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
        self.clamp_sub_skeleton_to_surface(bone_prefix, floor_y, bounds, None, false)
            .map(|(root_id, min_y, delta_y, _)| (root_id, min_y, delta_y))
    }

    /// Grounds a sub-skeleton onto either a flat `fallback_floor_y` or an uneven target mesh
    /// `surface` via vertical `-Y` raycast. When `solve_foot_ik` is true and humanoid leg chains
    /// exist, solves two-bone leg IK for `LeftFoot` and `RightFoot` to plant both feet on slopes.
    /// Returns `(root_id, min_y_before, delta_y, target_surface_y)`.
    pub fn clamp_sub_skeleton_to_surface(
        &mut self,
        bone_prefix: &str,
        fallback_floor_y: f32,
        bounds: Option<&super::skin::SkinnedBoundsCache>,
        surface: Option<&SurfaceTarget<'_>>,
        solve_foot_ik: bool,
    ) -> Option<(BoneId, f32, f32, f32)> {
        self.clamp_sub_skeleton_to_surface_v2(
            bone_prefix,
            fallback_floor_y,
            bounds,
            surface,
            &GroundingOptions {
                solve_foot_ik,
                ..GroundingOptions::default()
            },
        )
    }

    /// Grounding v2: local-start overhang-safe raycast, multi-surface (mesh + voxel),
    /// heel+toe footprint sweep, optional foot normal alignment, pelvis tilt, and custom contact chains.
    pub fn clamp_sub_skeleton_to_surface_v2(
        &mut self,
        bone_prefix: &str,
        fallback_floor_y: f32,
        bounds: Option<&super::skin::SkinnedBoundsCache>,
        surface: Option<&SurfaceTarget<'_>>,
        options: &GroundingOptions,
    ) -> Option<(BoneId, f32, f32, f32)> {
        let (root_id, bone_ids) = self.sub_skeleton_root_and_bones(bone_prefix)?;
        let world_mats = self.world_pose_matrices();
        let min_pt = if let Some(pt) =
            bounds.and_then(|cache| cache.sub_skeleton_min_point(self, bone_prefix))
        {
            pt
        } else {
            let mut best: Option<Vec3> = None;
            for id in &bone_ids {
                if let Some(mat) = world_mats.get(id) {
                    let p = mat.transform_point3(Vec3::ZERO);
                    if p.is_finite() && best.is_none_or(|b| p.y < b.y) {
                        best = Some(p);
                    }
                }
            }
            best?
        };

        let root_world = world_mats
            .get(&root_id)
            .map(|m| m.transform_point3(Vec3::ZERO))
            .unwrap_or(min_pt);
        let root_rot_before = self
            .bones
            .iter()
            .find(|b| b.id == root_id)
            .map(|b| b.pose_local.rotation)
            .unwrap_or(Quat::IDENTITY);
        let forward_xz = (root_rot_before * Vec3::Z).normalize_or_zero();
        let clearance = if options.step_up_clearance.is_finite() {
            options.step_up_clearance.clamp(0.05, 20.0)
        } else {
            1.0
        };
        let half_len = if options.footprint_half_length.is_finite() {
            options.footprint_half_length.clamp(0.0, 1.0)
        } else {
            0.08
        };

        let find_bone_by_suffix = |suffix: &str| -> Option<BoneId> {
            let target_name = format!("{bone_prefix}{suffix}");
            self.bones
                .iter()
                .find(|b| b.name == target_name)
                .map(|b| b.id)
        };
        let chains: Vec<ContactChain> = if !options.contact_chains.is_empty() {
            options.contact_chains.clone()
        } else {
            let mut auto = Vec::new();
            for (r, m, e) in [
                ("LeftThigh", "LeftCalf", "LeftFoot"),
                ("RightThigh", "RightCalf", "RightFoot"),
            ] {
                if let (Some(root), Some(mid), Some(end)) = (
                    find_bone_by_suffix(r),
                    find_bone_by_suffix(m),
                    find_bone_by_suffix(e),
                ) {
                    auto.push(ContactChain { root, mid, end });
                }
            }
            auto
        };

        let mut target_y = fallback_floor_y;
        let mut chain_hits: Vec<Option<SurfaceHit>> = vec![None; chains.len()];

        if let Some(surf) = surface {
            let ref_start_y = min_pt.y.max(root_world.y) + clearance;
            for (i, ch) in chains.iter().enumerate() {
                if let Some(fm) = world_mats.get(&ch.end) {
                    let fp = fm.transform_point3(Vec3::ZERO);
                    let ray_start = fp.y + clearance;
                    chain_hits[i] =
                        surf.sample_surface_footprint(fp.x, fp.z, ray_start, forward_xz, half_len);
                }
            }
            let valid_ys: Vec<f32> = chain_hits.iter().flatten().map(|h| h.y).collect();
            if options.solve_foot_ik && !valid_ys.is_empty() {
                target_y = valid_ys.iter().copied().fold(f32::INFINITY, f32::min);
            } else if let Some(hit) = surf
                .sample_surface_footprint(min_pt.x, min_pt.z, ref_start_y, forward_xz, half_len)
                .or_else(|| {
                    surf.sample_surface_footprint(
                        root_world.x,
                        root_world.z,
                        ref_start_y,
                        forward_xz,
                        half_len,
                    )
                })
            {
                target_y = hit.y;
            }
        }

        let delta_y = target_y - min_pt.y;
        let root = self.bones.iter_mut().find(|b| b.id == root_id)?;
        root.pose_local.translation.y += delta_y;

        if options.pelvis_tilt && chains.len() >= 2 {
            if let (Some(h0), Some(h1)) = (chain_hits[0], chain_hits[1]) {
                if let (Some(m0), Some(m1)) = (
                    world_mats.get(&chains[0].end),
                    world_mats.get(&chains[1].end),
                ) {
                    let p0 = m0.transform_point3(Vec3::ZERO);
                    let p1 = m1.transform_point3(Vec3::ZERO);
                    let horiz_span = Vec3::new(p0.x - p1.x, 0.0, p0.z - p1.z).length();
                    if horiz_span > 0.05 {
                        let fwd_axis = if forward_xz.length_squared() > 1e-6 {
                            forward_xz
                        } else {
                            Vec3::Z
                        };
                        let left_axis = Vec3::Y.cross(fwd_axis).normalize_or_zero();
                        let sign = if (p0 - p1).dot(left_axis) < 0.0 {
                            -1.0
                        } else {
                            1.0
                        };
                        let tilt_rad = (sign * (h0.y - h1.y) / horiz_span)
                            .atan()
                            .clamp(-0.35, 0.35);
                        let tilt_q = Quat::from_axis_angle(fwd_axis, tilt_rad * 0.5);
                        root.pose_local.rotation = (tilt_q * root.pose_local.rotation).normalize();
                    }
                }
            }
        }
        let root_rot = root.pose_local.rotation;

        if (options.solve_foot_ik || options.align_foot_normal) && surface.is_some() {
            if let Some(surf) = surface {
                let updated_mats = self.world_pose_matrices();
                let sole_offset = chains
                    .iter()
                    .filter_map(|c| updated_mats.get(&c.end))
                    .map(|m| m.transform_point3(Vec3::ZERO).y)
                    .fold(f32::INFINITY, f32::min);
                let foot_clearance = if sole_offset.is_finite() {
                    (sole_offset - target_y).max(0.0)
                } else {
                    0.0
                };
                let knee_pole = (root_rot * Vec3::Z).normalize_or_zero();

                for (i, ch) in chains.iter().enumerate() {
                    let Some(foot_mat) = updated_mats.get(&ch.end) else {
                        continue;
                    };
                    let fp = foot_mat.transform_point3(Vec3::ZERO);
                    let Some(hit) = chain_hits[i].or_else(|| {
                        surf.sample_surface_footprint(
                            fp.x,
                            fp.z,
                            fp.y + clearance,
                            forward_xz,
                            half_len,
                        )
                    }) else {
                        continue;
                    };

                    if options.solve_foot_ik {
                        let goal = Vec3::new(fp.x, hit.y + foot_clearance, fp.z);
                        if (goal.y - fp.y).abs() > 1e-3 {
                            if let Ok((r_rot, m_rot)) = super::ik::solve_for_rig(
                                self,
                                ch.root,
                                ch.mid,
                                ch.end,
                                goal,
                                Some(knee_pole),
                            ) {
                                if let Some(tb) = self.bone_mut(ch.root) {
                                    tb.pose_local.rotation = r_rot;
                                }
                                if let Some(cb) = self.bone_mut(ch.mid) {
                                    cb.pose_local.rotation = m_rot;
                                }
                            }
                        }
                    }

                    if options.align_foot_normal {
                        let n = hit.normal.normalize_or_zero();
                        if n.length_squared() > 1e-6 && n.y > 0.1 {
                            let raw_angle = Vec3::Y.dot(n).clamp(-1.0, 1.0).acos();
                            let max_angle = 45.0_f32.to_radians();
                            let tilt_world = if raw_angle > 1e-4 {
                                let axis = Vec3::Y.cross(n).normalize_or_zero();
                                if axis.length_squared() > 1e-6 {
                                    Quat::from_axis_angle(axis, raw_angle.min(max_angle))
                                } else {
                                    Quat::IDENTITY
                                }
                            } else {
                                Quat::IDENTITY
                            };
                            let post_ik_mats = self.world_pose_matrices();
                            let parent_world_rot = self
                                .bones
                                .iter()
                                .find(|b| b.id == ch.end)
                                .and_then(|b| b.parent)
                                .and_then(|pid| post_ik_mats.get(&pid))
                                .map(|m| m.to_scale_rotation_translation().1)
                                .unwrap_or(Quat::IDENTITY);
                            let foot_world_rot = post_ik_mats
                                .get(&ch.end)
                                .map(|m| m.to_scale_rotation_translation().1)
                                .unwrap_or(parent_world_rot);
                            let desired_world = (tilt_world * foot_world_rot).normalize();
                            let desired_local =
                                (parent_world_rot.inverse() * desired_world).normalize();
                            if let Some(fb) = self.bone_mut(ch.end) {
                                fb.pose_local.rotation = desired_local;
                            }
                        }
                    }
                }
            }
        }

        Some((root_id, min_pt.y, delta_y, target_y))
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
        let duration_secs = if duration_secs.is_finite() {
            duration_secs.clamp(0.05, 600.0)
        } else {
            1.0
        };
        let steps = ((duration_secs * fps.clamp(1.0, 120.0)).round() as usize).clamp(1, 2048);

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
        let opts = GroundingOptions::default();
        self.clamp_clip_to_surface_with(ClipGroundingRequest {
            clip_index,
            bone_prefix,
            fallback_floor_y: floor_y,
            fps,
            bounds,
            surface: None,
            options: &opts,
        })
    }

    /// Grounding v2 clip baker: samples in-between frames at `fps` and grounds onto multi-surface
    /// targets with overhang clearance, footprint sweep, optional foot IK, normal alignment, and pelvis tilt.
    pub fn clamp_clip_to_surface_with(
        &self,
        req: ClipGroundingRequest<'_>,
    ) -> Option<AnimationClip> {
        let ClipGroundingRequest {
            clip_index,
            bone_prefix,
            fallback_floor_y,
            fps,
            bounds,
            surface,
            options,
        } = req;
        let mut clip = self.animations.get(clip_index)?.clone();
        let (root_id, _) = self.sub_skeleton_root_and_bones(bone_prefix)?;
        let duration = if clip.duration_secs.is_finite() {
            clip.duration_secs.clamp(0.05, 600.0)
        } else {
            1.0
        };
        let steps = ((duration * fps.clamp(1.0, 120.0)).round() as usize).clamp(1, 2048);

        let mut leg_ids: Vec<BoneId> = Vec::new();
        if (options.solve_foot_ik || options.align_foot_normal) && surface.is_some() {
            if !options.contact_chains.is_empty() {
                for ch in &options.contact_chains {
                    if options.solve_foot_ik {
                        leg_ids.push(ch.root);
                        leg_ids.push(ch.mid);
                    }
                    if options.align_foot_normal {
                        leg_ids.push(ch.end);
                    }
                }
            } else {
                let suffixes: &[&str] = match (options.solve_foot_ik, options.align_foot_normal) {
                    (true, true) => &[
                        "LeftThigh",
                        "LeftCalf",
                        "LeftFoot",
                        "RightThigh",
                        "RightCalf",
                        "RightFoot",
                    ],
                    (true, false) => &["LeftThigh", "LeftCalf", "RightThigh", "RightCalf"],
                    (false, true) => &["LeftFoot", "RightFoot"],
                    (false, false) => &[],
                };
                for suffix in suffixes {
                    let target_name = format!("{bone_prefix}{suffix}");
                    if let Some(b) = self.bones.iter().find(|b| b.name == target_name) {
                        leg_ids.push(b.id);
                    }
                }
            }
        }

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
            let _ = sim.clamp_sub_skeleton_to_surface_v2(
                bone_prefix,
                fallback_floor_y,
                bounds,
                surface,
                options,
            );
            let mut record_ids = vec![root_id];
            record_ids.extend_from_slice(&leg_ids);
            for bid in record_ids {
                if let Some(bone) = sim.bones.iter().find(|b| b.id == bid) {
                    let rot_delta = bone.rest_local.rotation.inverse() * bone.pose_local.rotation;
                    let trans_delta = bone.pose_local.translation - bone.rest_local.translation;
                    let existing_interp = clip
                        .channels
                        .iter()
                        .find(|c| c.bone_id == bid)
                        .and_then(|c| c.keyframes.iter().find(|k| (k.time_secs - t).abs() <= 1e-3))
                        .map(|k| k.interpolation)
                        .unwrap_or(crate::character::KeyframeInterpolation::Linear);
                    let scale_delta = Vec3::new(
                        if bone.rest_local.scale.x.abs() > 1e-6 {
                            bone.pose_local.scale.x / bone.rest_local.scale.x
                        } else {
                            1.0
                        },
                        if bone.rest_local.scale.y.abs() > 1e-6 {
                            bone.pose_local.scale.y / bone.rest_local.scale.y
                        } else {
                            1.0
                        },
                        if bone.rest_local.scale.z.abs() > 1e-6 {
                            bone.pose_local.scale.z / bone.rest_local.scale.z
                        } else {
                            1.0
                        },
                    );
                    clip.upsert_keyframe(
                        bid,
                        crate::character::Keyframe {
                            time_secs: t,
                            translation: trans_delta,
                            rotation: rot_delta,
                            scale: scale_delta,
                            interpolation: existing_interp,
                        },
                    );
                }
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

    #[test]
    fn grounding_v2_under_bridge_overhang_hits_lower_floor_not_bridge() {
        let mut scene = crate::scene::SceneState::new();
        let mat = crate::palette::PaletteIndex::new(3).unwrap();
        // Lower floor at y = 0.0
        let floor_id = scene.allocate_node_id();
        scene.nodes.insert(
            floor_id,
            crate::scene::SceneNode {
                id: floor_id,
                name: "Floor".into(),
                kind: crate::scene::NodeKind::LowPolyMesh,
                visible: true,
                parent: None,
                transform: Transform::default(),
            },
        );
        scene.lowpoly_meshes.insert(
            floor_id,
            crate::mesh::ops::create_primitive(crate::mesh::PrimitiveKind::Plane, 6.0, mat),
        );
        // Overhead bridge deck at y = 3.0
        let bridge_id = scene.allocate_node_id();
        scene.nodes.insert(
            bridge_id,
            crate::scene::SceneNode {
                id: bridge_id,
                name: "Bridge".into(),
                kind: crate::scene::NodeKind::LowPolyMesh,
                visible: true,
                parent: None,
                transform: Transform {
                    translation: Vec3::new(0.0, 3.0, 0.0),
                    ..Transform::default()
                },
            },
        );
        scene.lowpoly_meshes.insert(
            bridge_id,
            crate::mesh::ops::create_primitive(crate::mesh::PrimitiveKind::Plane, 6.0, mat),
        );

        let surf = SurfaceTarget::from_scene_nodes(&scene, &[floor_id, bridge_id]).unwrap();
        // Character stands at y = 0.2 under the y = 3.0 bridge: local ray starts at 0.2 + 1.0 = 1.2 -> hits y = 0.0!
        let hit = surf.sample_surface_hit(0.0, 0.0, 1.2).unwrap();
        assert!(
            hit.y.abs() < 1e-3,
            "expected lower floor at y=0.0 under overhang, got {}",
            hit.y
        );
        // Character standing ON the bridge at y = 3.2: local ray starts at 4.2 -> hits y = 3.0!
        let top_hit = surf.sample_surface_hit(0.0, 0.0, 4.2).unwrap();
        assert!(
            (top_hit.y - 3.0).abs() < 1e-3,
            "expected bridge deck at y=3.0, got {}",
            top_hit.y
        );
    }

    #[test]
    fn grounding_v2_supports_voxel_surface_and_foot_normal_alignment() {
        let mut scene = crate::scene::SceneState::new();
        let mat = crate::palette::PaletteIndex::new(4).unwrap();
        // Voxel platform with voxels at y = 0..=4 (top at y = 0.5m at unit_scale 0.1).
        let vox_id = scene.allocate_node_id();
        scene.nodes.insert(
            vox_id,
            crate::scene::SceneNode {
                id: vox_id,
                name: "VoxelBlock".into(),
                kind: crate::scene::NodeKind::VoxelVolume,
                visible: true,
                parent: None,
                transform: Transform::default(),
            },
        );
        let mut vol = crate::voxel::VoxelVolume::new();
        for x in -4..=4 {
            for z in -4..=4 {
                vol.set(crate::voxel::VoxelCoord::new(x, 4, z), Some(mat));
            }
        }
        scene.voxel_volumes.insert(vox_id, vol);

        let surf = SurfaceTarget::from_scene_nodes(&scene, &[vox_id]).unwrap();
        let hit = surf.sample_surface_hit(0.0, 0.0, 1.5).unwrap();
        assert!(
            (hit.y - 0.5).abs() < 1e-3,
            "expected voxel top at y=0.5m, got {}",
            hit.y
        );
        assert!(hit.normal.y > 0.99);
    }
}
