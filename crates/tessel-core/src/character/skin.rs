use std::collections::BTreeMap;

use glam::{Mat4, Vec3};

use crate::scene::NodeId;

use super::nano::BoneId;
use super::rig::CharacterRig;

/// Normalized 4-bone skinning influence for a single vertex (glTF `JOINTS_0` / `WEIGHTS_0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VertexSkinWeights {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// Precomputed rest-pose skeleton segments and inverse bind matrices for auto-weight skinning.
#[derive(Debug, Clone)]
pub struct AutoSkinSolver {
    scale: f32,
    node_rigid_joint: BTreeMap<NodeId, u16>,
    segments: Vec<(Vec3, Vec3)>,
    inverse_bind_matrices: Vec<[f32; 16]>,
}

impl AutoSkinSolver {
    /// Builds a rest-pose skinning solver from `rig`, scaled uniformly by `scale`.
    pub fn from_rig(rig: &CharacterRig, scale: f32) -> Option<Self> {
        if rig.bones.is_empty() {
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
                if start.y < parent_pos.y {
                    Vec3::new(start.x, 0.0, start.z)
                } else {
                    let dir = (start - parent_pos).normalize_or_zero();
                    start + dir * (0.25 * s)
                }
            } else {
                start + Vec3::Y * (0.25 * s)
            };

            segments.push((start, end));
            inverse_bind_matrices.push(mat.inverse().to_cols_array());
        }

        let mut node_rigid_joint = BTreeMap::new();
        for binding in &rig.bindings {
            if let Some(&joint) = bone_index_by_id.get(&binding.bone_id) {
                node_rigid_joint.insert(binding.node_id, joint);
            }
        }

        Some(Self {
            scale: s,
            node_rigid_joint,
            segments,
            inverse_bind_matrices,
        })
    }

    /// Column-major 4x4 inverse bind matrices matching `rig.bones` order.
    pub fn inverse_bind_matrices(&self) -> &[[f32; 16]] {
        &self.inverse_bind_matrices
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

        for (idx, &(a, b)) in self.segments.iter().enumerate() {
            let Ok(joint) = u16::try_from(idx) else {
                break;
            };
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
            return VertexSkinWeights {
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
        }

        for w in &mut top_weights {
            *w /= sum;
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

        // Vertex near LeftLeg foot (-0.18, 0.15, 0) -> dominant joint 5 (LeftLeg)
        let leg_w = solver.weights_for_vertex(None, Vec3::new(-0.18, 0.15, 0.0));
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
}
