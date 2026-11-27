use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{NodeId, SceneState};
use crate::voxel::greedy_mesh_volume;

use super::nano::BoneId;
use super::rig::CharacterRig;

/// Normalized 4-bone skinning influence for a single vertex (glTF `JOINTS_0` / `WEIGHTS_0`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VertexSkinWeights {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// Precomputed rest-pose skeleton segments and inverse bind matrices for auto-weight skinning.
#[derive(Debug, Clone)]
pub struct AutoSkinSolver {
    scale: f32,
    node_rigid_joint: BTreeMap<NodeId, u16>,
    node_allowed_joints: BTreeMap<NodeId, Vec<u16>>,
    segments: Vec<(Vec3, Vec3)>,
    /// Lateral side tag per bone (`+1` = +X left limb, `-1` = -X right limb, `0` = midline).
    bone_sides: Vec<i8>,
    /// Whether each bone participates in smooth auto-weight deformation (`Bone::deform`).
    bone_deform: Vec<bool>,
    inverse_bind_matrices: Vec<[f32; 16]>,
}

impl AutoSkinSolver {
    /// Builds a rest-pose skinning solver from `rig`, scaled uniformly by `scale`.
    /// Returns `None` when the rig has no bones or when zero bones have `deform == true`
    /// and no valid rigid bindings exist on rigged nodes.
    pub fn from_rig(rig: &CharacterRig, scale: f32) -> Option<Self> {
        if rig.bones.is_empty() {
            return None;
        }
        let has_deform_bone = rig.bones.iter().any(|b| b.deform);
        let has_valid_binding = rig
            .bindings
            .iter()
            .any(|b| rig.should_skin_node(b.node_id) && rig.bone(b.bone_id).is_some());
        if !has_deform_bone && !has_valid_binding {
            return None;
        }
        let s = scale.max(1e-6);

        let mut world_rest: BTreeMap<BoneId, Mat4> = BTreeMap::new();
        let mut bone_index_by_id: BTreeMap<BoneId, u16> = BTreeMap::new();
        for (idx, bone) in rig.bones.iter().enumerate() {
            let Ok(joint) = u16::try_from(idx) else {
                break;
            };
            bone_index_by_id.insert(bone.id, joint);
            let local = Mat4::from_scale_rotation_translation(
                bone.rest_local.scale,
                bone.rest_local.rotation,
                bone.rest_local.translation * s,
            );
            let mat = match bone.parent.and_then(|pid| world_rest.get(&pid)) {
                Some(&parent_mat) => parent_mat * local,
                None => local,
            };
            world_rest.insert(bone.id, mat);
        }

        let mut children_pos: BTreeMap<BoneId, Vec<Vec3>> = BTreeMap::new();
        for bone in &rig.bones {
            if let (Some(pid), Some(mat)) = (bone.parent, world_rest.get(&bone.id)) {
                children_pos
                    .entry(pid)
                    .or_default()
                    .push(mat.transform_point3(Vec3::ZERO));
            }
        }

        let mut segments = Vec::with_capacity(rig.bones.len());
        let mut bone_sides = Vec::with_capacity(rig.bones.len());
        let mut bone_deform = Vec::with_capacity(rig.bones.len());
        let mut inverse_bind_matrices = Vec::with_capacity(rig.bones.len());

        for bone in &rig.bones {
            let Some(&mat) = world_rest.get(&bone.id) else {
                continue;
            };
            let start = mat.transform_point3(Vec3::ZERO);
            let end = if let Some(kids) = children_pos.get(&bone.id) {
                let sum = kids.iter().copied().fold(Vec3::ZERO, |acc, p| acc + p);
                sum / (kids.len() as f32)
            } else if let Some(parent_pos) = bone
                .parent
                .and_then(|pid| world_rest.get(&pid))
                .map(|m| m.transform_point3(Vec3::ZERO))
            {
                let is_whole_leg = bone.name.contains("Leg");
                if is_whole_leg && start.y < parent_pos.y {
                    Vec3::new(start.x, 0.0, start.z)
                } else {
                    let dir = (start - parent_pos).normalize_or_zero();
                    let raw_end = start + dir * (0.15 * s);
                    Vec3::new(raw_end.x, raw_end.y.max(0.0), raw_end.z)
                }
            } else {
                start + Vec3::Y * (0.20 * s)
            };

            let mid_x = (start.x + end.x) * 0.5;
            let side = if mid_x > 0.04 * s {
                1
            } else if mid_x < -0.04 * s {
                -1
            } else {
                0
            };
            segments.push((start, end));
            bone_sides.push(side);
            bone_deform.push(bone.deform);
            inverse_bind_matrices.push(mat.inverse().to_cols_array());
        }

        let mut node_rigid_joint = BTreeMap::new();
        for binding in &rig.bindings {
            if let Some(&joint) = bone_index_by_id.get(&binding.bone_id) {
                node_rigid_joint.insert(binding.node_id, joint);
            }
        }
        let mut node_allowed_joints = BTreeMap::new();
        for (&nid, mask) in &rig.node_bone_masks {
            let joints: Vec<u16> = mask
                .iter()
                .filter_map(|bid| bone_index_by_id.get(bid).copied())
                .collect();
            if !joints.is_empty() {
                node_allowed_joints.insert(nid, joints);
            }
        }

        Some(Self {
            scale: s,
            node_rigid_joint,
            node_allowed_joints,
            segments,
            bone_sides,
            bone_deform,
            inverse_bind_matrices,
        })
    }

    /// Returns `true` when `node_id` has at least one active skinning influence in this solver
    /// (explicit rigid binding, or at least one `deform == true` bone within its mask / skeleton).
    pub fn node_has_skin_influence(&self, node_id: NodeId) -> bool {
        if self.node_rigid_joint.contains_key(&node_id) {
            return true;
        }
        if let Some(mask) = self.node_allowed_joints.get(&node_id) {
            return mask.iter().any(|&j| {
                self.bone_deform
                    .get(usize::from(j))
                    .copied()
                    .unwrap_or(false)
            });
        }
        self.bone_deform.iter().any(|&d| d)
    }

    /// Column-major 4x4 inverse bind matrices matching `rig.bones` order.
    pub fn inverse_bind_matrices(&self) -> &[[f32; 16]] {
        &self.inverse_bind_matrices
    }

    /// Computes live world-space skinning matrices (`pose_world * inverse_bind`) matching
    /// `rig.bones` order. At rest pose every matrix is `Mat4::IDENTITY`.
    pub fn skin_matrices(&self, rig: &CharacterRig) -> Vec<Mat4> {
        let world_pose = rig.world_pose_matrices();
        rig.bones
            .iter()
            .enumerate()
            .map(|(idx, bone)| {
                let Some(ibm_cols) = self.inverse_bind_matrices.get(idx) else {
                    return Mat4::IDENTITY;
                };
                let Some(&pose) = world_pose.get(&bone.id) else {
                    return Mat4::IDENTITY;
                };
                pose * Mat4::from_cols_array(ibm_cols)
            })
            .collect()
    }

    /// Linear Blend Skinning (LBS) for a world-space point and normal using up to 4 bone influences.
    pub fn deform_point_normal(
        &self,
        skin_mats: &[Mat4],
        weights: VertexSkinWeights,
        world_pos: Vec3,
        world_norm: Vec3,
    ) -> (Vec3, Vec3) {
        let mut out_pos = Vec3::ZERO;
        let mut out_norm = Vec3::ZERO;
        let mut w_sum = 0.0f32;
        for k in 0..4 {
            let w = weights.weights[k];
            if w <= 0.0 {
                continue;
            }
            if let Some(&mat) = skin_mats.get(usize::from(weights.joints[k])) {
                out_pos += mat.transform_point3(world_pos) * w;
                out_norm += mat.transform_vector3(world_norm) * w;
                w_sum += w;
            }
        }
        if w_sum <= 1e-6 {
            return (world_pos, world_norm);
        }
        (out_pos, out_norm.normalize_or_zero())
    }

    /// Computes normalized 4-bone weights for a vertex at `world_pos`.
    /// Honors explicit rigid `SkinBinding` on `node_id` if present; otherwise uses smooth
    /// inverse-distance-4th-power falloff across bone capsule segments.
    pub fn weights_for_vertex(
        &self,
        node_id: Option<NodeId>,
        world_pos: Vec3,
    ) -> VertexSkinWeights {
        if let Some(joint) = node_id.and_then(|nid| self.node_rigid_joint.get(&nid).copied()) {
            return VertexSkinWeights {
                joints: [joint, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
        }

        let eps = 0.02 * self.scale;
        let mut top_joints = [0u16; 4];
        let mut top_weights = [0.0f32; 4];

        let allowed = node_id.and_then(|nid| self.node_allowed_joints.get(&nid));
        let lateral_thresh = 0.03 * self.scale;
        for (idx, &(a, b)) in self.segments.iter().enumerate() {
            let Ok(joint) = u16::try_from(idx) else {
                break;
            };
            if !self.bone_deform.get(idx).copied().unwrap_or(true) {
                continue;
            }
            if let Some(mask) = allowed {
                if !mask.contains(&joint) {
                    continue;
                }
            }
            // Prevent cross-side limb bleeding: a vertex clearly on +X ignores -X limb bones and vice versa.
            let side = self.bone_sides.get(idx).copied().unwrap_or(0);
            if side != 0
                && world_pos.x.abs() > lateral_thresh
                && (f32::from(side) * world_pos.x) < 0.0
            {
                continue;
            }
            let d = point_to_segment_distance(world_pos, a, b);
            let inv = 1.0 / (d + eps);
            let w = inv * inv * inv * inv;
            if w > top_weights[3] {
                top_weights[3] = w;
                top_joints[3] = joint;
                let mut k = 3;
                while k > 0 && top_weights[k] > top_weights[k - 1] {
                    top_weights.swap(k, k - 1);
                    top_joints.swap(k, k - 1);
                    k -= 1;
                }
            }
        }

        let sum: f32 = top_weights.iter().sum();
        if !sum.is_finite() || sum <= 0.0 {
            if let Some(mask) = allowed {
                // Respect node_bone_masks strictly (ignoring lateral filter) so a masked
                // sub-skeleton never leaks outside its mask; if every masked bone has
                // deform == false, return zero weights so the vertex remains undeformed.
                let closest_deform_in_mask = mask
                    .iter()
                    .copied()
                    .filter(|&j| {
                        self.bone_deform
                            .get(usize::from(j))
                            .copied()
                            .unwrap_or(false)
                    })
                    .min_by(|&a, &b| {
                        let da = self
                            .segments
                            .get(usize::from(a))
                            .map(|&(sa, sb)| point_to_segment_distance(world_pos, sa, sb))
                            .unwrap_or(f32::INFINITY);
                        let db = self
                            .segments
                            .get(usize::from(b))
                            .map(|&(sa, sb)| point_to_segment_distance(world_pos, sa, sb))
                            .unwrap_or(f32::INFINITY);
                        da.total_cmp(&db)
                    });
                return match closest_deform_in_mask {
                    Some(joint) => VertexSkinWeights {
                        joints: [joint, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    },
                    None => VertexSkinWeights {
                        joints: [0, 0, 0, 0],
                        weights: [0.0, 0.0, 0.0, 0.0],
                    },
                };
            }
            return match self
                .bone_deform
                .iter()
                .position(|&d| d)
                .and_then(|i| u16::try_from(i).ok())
            {
                Some(joint) => VertexSkinWeights {
                    joints: [joint, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                },
                None => VertexSkinWeights {
                    joints: [0, 0, 0, 0],
                    weights: [0.0, 0.0, 0.0, 0.0],
                },
            };
        }

        // Prune negligible distant bone influences (< 2% of total weight) before normalizing
        // so distant limbs (e.g. Hand -> Boot) never introduce floating-point drift.
        let cutoff = sum * 0.02;
        for k in 1..4 {
            if top_weights[k] < cutoff {
                top_weights[k] = 0.0;
                top_joints[k] = 0;
            }
        }
        let pruned_sum: f32 = top_weights.iter().sum();
        for w in &mut top_weights {
            *w /= pruned_sum;
        }
        top_weights[0] = (1.0 - top_weights[1] - top_weights[2] - top_weights[3]).max(0.0);

        VertexSkinWeights {
            joints: top_joints,
            weights: top_weights,
        }
    }
}

fn point_to_segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= 1e-10 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

#[derive(Debug, Clone, Copy)]
struct SkinnedVertexSample {
    rest_world_pos: Vec3,
    weights: VertexSkinWeights,
    dominant_joint: u16,
}

/// Precomputed rest-pose vertices and skinning weights from visible scene meshes for
/// fast, allocation-free per-frame skinned Y-bounds evaluation (e.g. floor grounding).
#[derive(Debug, Clone)]
pub struct SkinnedBoundsCache {
    solver: AutoSkinSolver,
    samples: Vec<SkinnedVertexSample>,
}

impl SkinnedBoundsCache {
    /// Builds a skinned bounds cache for `rig` from all visible, non-overridden low-poly
    /// and voxel meshes in `scene`.
    pub fn from_scene(scene: &SceneState, rig: &CharacterRig) -> Option<Self> {
        Self::from_scene_excluding(scene, rig, None)
    }

    /// Builds a skinned bounds cache while optionally excluding a static surface node
    /// (e.g. a terrain mesh that the character is being grounded onto).
    pub fn from_scene_excluding(
        scene: &SceneState,
        rig: &CharacterRig,
        exclude_node: Option<NodeId>,
    ) -> Option<Self> {
        let excluded: Vec<NodeId> = exclude_node.into_iter().collect();
        Self::from_scene_excluding_many(scene, rig, &excluded)
    }

    /// Precomputes skinned vertex weights while excluding multiple surface/terrain nodes.
    pub fn from_scene_excluding_many(
        scene: &SceneState,
        rig: &CharacterRig,
        excluded_nodes: &[NodeId],
    ) -> Option<Self> {
        let solver = AutoSkinSolver::from_rig(rig, 1.0)?;
        let mut samples = Vec::new();

        let is_skinned_visible = |node_id: NodeId| -> bool {
            if excluded_nodes.contains(&node_id) {
                return false;
            }
            if !rig.should_skin_node(node_id) {
                return false;
            }
            if rig.node_parent_overrides.contains_key(&node_id) {
                return false;
            }
            rig.node_visibility
                .get(&node_id)
                .copied()
                .unwrap_or_else(|| scene.nodes.get(&node_id).map(|n| n.visible).unwrap_or(true))
        };

        for (&node_id, mesh) in &scene.lowpoly_meshes {
            if !is_skinned_visible(node_id) || mesh.vertices.is_empty() {
                continue;
            }
            let node_mat = scene.node_rest_world_matrix(node_id);
            samples.reserve(mesh.vertices.len());
            for &v in &mesh.vertices {
                let wp = node_mat.transform_point3(v);
                let weights = solver.weights_for_vertex(Some(node_id), wp);
                samples.push(SkinnedVertexSample {
                    rest_world_pos: wp,
                    weights,
                    dominant_joint: weights.joints[0],
                });
            }
        }

        for (&node_id, vol) in &scene.voxel_volumes {
            if !is_skinned_visible(node_id) || vol.is_empty() {
                continue;
            }
            let node_mat = scene.node_rest_world_matrix(node_id);
            let mesh = greedy_mesh_volume(vol, scene.unit_scale);
            samples.reserve(mesh.vertices.len());
            for v in mesh.vertices {
                let wp = node_mat.transform_point3(Vec3::from_array(v.position));
                let weights = solver.weights_for_vertex(Some(node_id), wp);
                samples.push(SkinnedVertexSample {
                    rest_world_pos: wp,
                    weights,
                    dominant_joint: weights.joints[0],
                });
            }
        }

        Some(Self { solver, samples })
    }

    /// Evaluates the world-space position of the lowest skinned vertex whose dominant bone
    /// belongs to `bone_prefix`.
    pub fn sub_skeleton_min_point(&self, rig: &CharacterRig, bone_prefix: &str) -> Option<Vec3> {
        if self.samples.is_empty() {
            return None;
        }
        let mut in_sub = vec![false; rig.bones.len()];
        let mut any_bone = false;
        for (idx, b) in rig.bones.iter().enumerate() {
            let matches = if bone_prefix.is_empty() {
                !b.name.contains('_')
            } else {
                b.name.starts_with(bone_prefix)
            };
            if matches {
                in_sub[idx] = true;
                any_bone = true;
            }
        }
        if !any_bone {
            return None;
        }

        let skin_mats = self.solver.skin_matrices(rig);
        let mut min_pt: Option<Vec3> = None;
        for sample in &self.samples {
            if !in_sub
                .get(usize::from(sample.dominant_joint))
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let mut pos = Vec3::ZERO;
            let mut w_sum = 0.0f32;
            for k in 0..4 {
                let w = sample.weights.weights[k];
                if w <= 0.0 {
                    continue;
                }
                if let Some(&mat) = skin_mats.get(usize::from(sample.weights.joints[k])) {
                    pos += mat.transform_point3(sample.rest_world_pos) * w;
                    w_sum += w;
                }
            }
            let deformed = if w_sum <= 1e-6 {
                sample.rest_world_pos
            } else {
                pos
            };
            if deformed.is_finite() && min_pt.is_none_or(|best| deformed.y < best.y) {
                min_pt = Some(deformed);
            }
        }

        min_pt
    }

    /// Evaluates the minimum world-space Y coordinate among all skinned vertices whose
    /// dominant bone belongs to `bone_prefix`. Returns `None` if no vertices belong to
    /// that sub-skeleton (callers should fall back to joint positions).
    pub fn sub_skeleton_min_y(&self, rig: &CharacterRig, bone_prefix: &str) -> Option<f32> {
        self.sub_skeleton_min_point(rig, bone_prefix).map(|p| p.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::SkinBinding;

    #[test]
    fn auto_skin_weights_assigns_head_and_legs_smoothly() {
        let rig = CharacterRig::biped_template();
        let solver = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        assert_eq!(solver.inverse_bind_matrices().len(), 7);

        // Vertex near Head (0, 1.65, 0) -> dominant joint 2 (Head)
        let head_w = solver.weights_for_vertex(None, Vec3::new(0.0, 1.65, 0.0));
        assert_eq!(head_w.joints[0], 2);
        assert!(head_w.weights[0] > 0.8);
        assert!((head_w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);

        // Vertex near LeftLeg foot (+0.11, 0.15, 0) -> dominant joint 5 (LeftLeg, +X in +Z Front RH)
        let leg_w = solver.weights_for_vertex(None, Vec3::new(0.11, 0.15, 0.0));
        assert_eq!(leg_w.joints[0], 5);
        assert!(leg_w.weights[0] > 0.8);
        assert!((leg_w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn auto_skin_solver_honors_explicit_rigid_binding() {
        let mut rig = CharacterRig::biped_template();
        let prop_node = NodeId(42);
        rig.bindings.push(SkinBinding {
            node_id: prop_node,
            bone_id: BoneId(4), // RightArm
        });
        let solver = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        let w = solver.weights_for_vertex(Some(prop_node), Vec3::new(0.0, 1.65, 0.0));
        assert_eq!(w.joints, [4, 0, 0, 0]);
        assert_eq!(w.weights, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn skin_matrices_identity_at_rest_and_deform_on_rotation() {
        let mut rig = CharacterRig::biped_template();
        let solver = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();

        let rest_mats = solver.skin_matrices(&rig);
        let p = Vec3::new(0.0, 1.65, 0.0);
        let w = solver.weights_for_vertex(None, p);
        let (rp, _) = solver.deform_point_normal(&rest_mats, w, p, Vec3::Z);
        assert!((rp - p).length() < 1e-4);

        rig.bone_mut(BoneId(2)).unwrap().pose_local.rotation =
            glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let posed_mats = solver.skin_matrices(&rig);
        let (dp, _) = solver.deform_point_normal(&posed_mats, w, p, Vec3::Z);
        assert!(
            dp.z > 0.05,
            "expected posed head vertex to move +Z, got {dp:?}"
        );
    }

    #[test]
    fn humanoid_hand_leaf_does_not_pull_boot_and_no_cross_side_bleeding() {
        let rig = CharacterRig::humanoid_template();
        let solver = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        // Right boot vertex (-0.11, 0.05, 0.05) must be dominated by RightFoot (18) or RightCalf (17),
        // NEVER by RightHand (12) or LeftFoot (15).
        let w = solver.weights_for_vertex(None, Vec3::new(-0.11, 0.05, 0.05));
        assert!(
            w.joints[0] == 18 || w.joints[0] == 17,
            "expected RightFoot(18)/RightCalf(17), got joints={:?} weights={:?}",
            w.joints,
            w.weights
        );
        // Must have zero weight on LeftFoot (15) or LeftCalf (14)
        for k in 0..4 {
            if w.weights[k] > 0.0 {
                assert_ne!(w.joints[k], 15);
                assert_ne!(w.joints[k], 14);
                assert_ne!(w.joints[k], 12);
            }
        }
    }

    #[test]
    fn skinned_bounds_cache_grounds_protruding_boot_sole_and_falls_back_when_empty() {
        let mut scene = SceneState::new();
        let node_id = scene.allocate_node_id();
        // Boot mesh with bottom sole at y = -0.05 (below humanoid LeftFoot joint at y ~ 0.08).
        let mut mesh = crate::mesh::LowPolyMesh::new();
        mesh.vertices = vec![
            Vec3::new(0.05, -0.05, -0.08),
            Vec3::new(0.17, -0.05, 0.12),
            Vec3::new(0.11, 0.10, 0.0),
        ];
        scene.nodes.insert(
            node_id,
            crate::scene::SceneNode {
                id: node_id,
                name: "LeftBoot".into(),
                kind: crate::scene::NodeKind::LowPolyMesh,
                visible: true,
                parent: None,
                transform: crate::scene::Transform::default(),
            },
        );
        scene.lowpoly_meshes.insert(node_id, mesh);

        let mut rig = CharacterRig::humanoid_template();
        let cache = SkinnedBoundsCache::from_scene(&scene, &rig).unwrap();

        let min_y_before = cache.sub_skeleton_min_y(&rig, "").unwrap();
        assert!(
            (min_y_before - (-0.05)).abs() < 1e-4,
            "expected boot sole min_y = -0.05, got {min_y_before}"
        );

        let (_, reported_min, delta_y) = rig
            .clamp_sub_skeleton_to_floor_with_bounds("", 0.0, Some(&cache))
            .unwrap();
        assert!((reported_min - (-0.05)).abs() < 1e-4);
        assert!((delta_y - 0.05).abs() < 1e-4);

        let min_y_after = cache.sub_skeleton_min_y(&rig, "").unwrap();
        assert!(
            min_y_after.abs() < 1e-4,
            "expected clamped boot sole min_y ~ 0.0, got {min_y_after}"
        );

        // Unknown / unmeshed sub-skeleton returns None so callers fall back to joints.
        assert!(cache.sub_skeleton_min_y(&rig, "P2_").is_none());
    }

    #[test]
    fn auto_skin_solver_skips_non_deform_helper_bones() {
        let mut rig = CharacterRig::biped_template();
        // Disable deform on Head (BoneId 2) so vertices near the head fall back to Torso/Root, never Head.
        rig.bone_mut(BoneId(2)).unwrap().deform = false;
        let solver = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        let w = solver.weights_for_vertex(None, Vec3::new(0.0, 1.65, 0.0));
        for k in 0..4 {
            if w.weights[k] > 0.0 {
                assert_ne!(
                    w.joints[k], 2,
                    "non-deform helper bone must not receive smooth auto-skin weight"
                );
            }
        }
        assert!((w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);

        // Masked vertex across X=0 must stay within its allowed mask, never snapping to global Root (0).
        let masked_node = NodeId(77);
        rig.node_bone_masks.insert(masked_node, vec![BoneId(5)]); // LeftLeg (+X)
        let solver_masked = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        let w_cross =
            solver_masked.weights_for_vertex(Some(masked_node), Vec3::new(-0.5, 0.2, 0.0));
        assert_eq!(
            w_cross.joints[0], 5,
            "masked fallback must stay in node_bone_masks"
        );

        // Mask containing ONLY non-deform bone(s) must never leak out to global Root (0).
        let helper_masked_node = NodeId(78);
        rig.node_bone_masks
            .insert(helper_masked_node, vec![BoneId(2)]); // Head (deform = false)
        let solver_helper = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        assert!(!solver_helper.node_has_skin_influence(helper_masked_node));
        let w_helper =
            solver_helper.weights_for_vertex(Some(helper_masked_node), Vec3::new(0.0, 1.65, 0.0));
        assert_eq!(w_helper.weights, [0.0, 0.0, 0.0, 0.0]);

        // All bones deform=false with no rigid bindings -> solver returns None.
        for b in &mut rig.bones {
            b.deform = false;
        }
        assert!(AutoSkinSolver::from_rig(&rig, 1.0).is_none());

        // All bones deform=false WITH one rigid binding -> unbound nodes get zero weights, not bone 0.
        let prop_node = NodeId(79);
        let unbound_node = NodeId(80);
        rig.bindings.push(SkinBinding {
            node_id: prop_node,
            bone_id: BoneId(0),
        });
        let solver_rigid_only = AutoSkinSolver::from_rig(&rig, 1.0).unwrap();
        assert!(solver_rigid_only.node_has_skin_influence(prop_node));
        assert!(!solver_rigid_only.node_has_skin_influence(unbound_node));
        let w_unbound =
            solver_rigid_only.weights_for_vertex(Some(unbound_node), Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(w_unbound.weights, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn skinned_bounds_cache_excludes_unrigged_nodes() {
        let mut scene = SceneState::new();
        let node_id = scene.allocate_node_id();
        let mut mesh = crate::mesh::LowPolyMesh::new();
        mesh.vertices = vec![
            Vec3::new(0.0, -2.5, 0.0),
            Vec3::new(1.0, -2.5, 0.0),
            Vec3::new(0.5, -2.0, 0.5),
        ];
        scene.nodes.insert(
            node_id,
            crate::scene::SceneNode {
                id: node_id,
                name: "TerrainFloor".into(),
                kind: crate::scene::NodeKind::LowPolyMesh,
                visible: true,
                parent: None,
                transform: crate::scene::Transform::default(),
            },
        );
        scene.lowpoly_meshes.insert(node_id, mesh);

        let mut rig = CharacterRig::humanoid_template();
        rig.unrigged_nodes.insert(node_id);
        let cache = SkinnedBoundsCache::from_scene(&scene, &rig).unwrap();
        assert!(
            cache.sub_skeleton_min_y(&rig, "").is_none(),
            "unrigged nodes must not be sampled into SkinnedBoundsCache"
        );
    }
}
