use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::scene::{NodeId, Transform};

use super::anim::AnimationClip;
use super::bone::Bone;
use super::ik::solve_chain_ik;
use super::nano::{BoneId, Keyframe, KeyframeInterpolation};
use super::rig::CharacterRig;

/// Specification for generating an `M × N` multi-chain cloth bone grid.
/// Each column `0..cols` forms a skeletal chain from row `0` (pinned root) to `rows - 1` (free tip).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneGridSpec {
    pub prefix: String,
    pub cols: usize,
    pub rows: usize,
    pub origin: Vec3,
    pub width: f32,
    pub height: f32,
    #[serde(default = "default_axis_u")]
    pub axis_u: Vec3,
    #[serde(default = "default_axis_v")]
    pub axis_v: Vec3,
    #[serde(default)]
    pub parent_bone: Option<BoneId>,
    #[serde(default = "default_true")]
    pub pin_top_row: bool,
}

fn default_axis_u() -> Vec3 {
    Vec3::X
}

fn default_axis_v() -> Vec3 {
    -Vec3::Y
}

fn default_true() -> bool {
    true
}

impl Default for BoneGridSpec {
    fn default() -> Self {
        Self {
            prefix: "Cloth".to_string(),
            cols: 4,
            rows: 4,
            origin: Vec3::new(0.0, 1.4, -0.15),
            width: 0.6,
            height: 0.9,
            axis_u: default_axis_u(),
            axis_v: default_axis_v(),
            parent_bone: None,
            pin_top_row: true,
        }
    }
}

/// Deterministic kinematic wind/gravity bake parameters for a cloth bone grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothWindParams {
    pub clip_name: String,
    pub wind_dir: Vec3,
    pub wind_strength: f32,
    pub gravity: f32,
    pub turbulence: f32,
    #[serde(default)]
    pub phase_offset: f32,
    pub duration_secs: f32,
    pub fps: f32,
}

impl Default for ClothWindParams {
    fn default() -> Self {
        Self {
            clip_name: "ClothWind".to_string(),
            wind_dir: Vec3::new(0.0, 0.0, -1.0),
            wind_strength: 0.25,
            gravity: 0.15,
            turbulence: 0.12,
            phase_offset: 0.0,
            duration_secs: 2.0,
            fps: 12.0,
        }
    }
}

/// Result of creating a multi-chain cloth bone grid.
#[derive(Debug, Clone, PartialEq)]
pub struct BoneGridResult {
    /// Column-major grid: `columns[col][row] = BoneId`.
    pub columns: Vec<Vec<BoneId>>,
}

impl BoneGridResult {
    pub fn all_bone_ids(&self) -> Vec<BoneId> {
        self.columns
            .iter()
            .flat_map(|col| col.iter().copied())
            .collect()
    }
}

/// Creates an `M × N` cloth bone grid inside `rig`.
/// Each column is a parent-child chain (`r=0 -> r=1 -> ... -> r=rows-1`).
pub fn create_bone_grid(
    rig: &mut CharacterRig,
    spec: &BoneGridSpec,
) -> Result<BoneGridResult, CoreError> {
    if !spec.origin.is_finite()
        || !spec.width.is_finite()
        || !spec.height.is_finite()
        || !spec.axis_u.is_finite()
        || !spec.axis_v.is_finite()
        || spec.width <= 1e-4
        || spec.height <= 1e-4
    {
        return Err(CoreError::OutOfBounds);
    }
    let cols = spec.cols.clamp(2, 16);
    let rows = spec.rows.clamp(2, 16);
    let u_dir = spec.axis_u.normalize_or_zero();
    let v_dir = spec.axis_v.normalize_or_zero();
    if u_dir.length_squared() < 1e-6 || v_dir.length_squared() < 1e-6 {
        return Err(CoreError::OutOfBounds);
    }

    let effective_parent = if spec.pin_top_row {
        spec.parent_bone
    } else {
        None
    };
    let parent_world = if let Some(pid) = effective_parent {
        if rig.bone(pid).is_none() {
            return Err(CoreError::OutOfBounds);
        }
        rig.world_rest_matrices()
            .get(&pid)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY)
    } else {
        glam::Mat4::IDENTITY
    };
    let parent_world_inv = parent_world.inverse();

    let mut next_id = rig
        .bones
        .iter()
        .map(|b| b.id.0)
        .max()
        .map_or(0, |m| m.saturating_add(1));

    let prefix = if spec.prefix.trim().is_empty() {
        "Cloth"
    } else {
        spec.prefix.trim()
    };

    let row_step = spec.height / (rows - 1) as f32;
    let mut columns = Vec::with_capacity(cols);

    for c in 0..cols {
        let u_frac = if cols > 1 {
            c as f32 / (cols - 1) as f32 - 0.5
        } else {
            0.0
        };
        let mut col_ids = Vec::with_capacity(rows);
        let mut prev_world_pos = Vec3::ZERO;
        let mut prev_id = effective_parent;

        for r in 0..rows {
            let world_pos =
                spec.origin + u_dir * (u_frac * spec.width) + v_dir * (r as f32 * row_step);
            let local_pos = if r == 0 {
                parent_world_inv.transform_point3(world_pos)
            } else {
                // Local translation relative to parent bone (which has identity rest rotation)
                parent_world_inv.transform_vector3(world_pos - prev_world_pos)
            };
            let id = BoneId(next_id);
            next_id = next_id.saturating_add(1);
            let name = format!("{prefix}_c{c}_r{r}");
            let bone = Bone::new(
                id,
                name,
                prev_id,
                Transform {
                    translation: local_pos,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
            );
            rig.bones.push(bone);
            col_ids.push(id);
            prev_world_pos = world_pos;
            prev_id = Some(id);
        }
        columns.push(col_ids);
    }

    Ok(BoneGridResult { columns })
}

/// Assigns a mesh node's `bone_mask` to a subset of cloth grid cells (e.g. a cape/skirt panel).
pub fn assign_bone_grid_mask(rig: &mut CharacterRig, node_id: NodeId, bone_ids: Vec<BoneId>) {
    if bone_ids.is_empty() {
        rig.node_bone_masks.remove(&node_id);
    } else {
        rig.node_bone_masks.insert(node_id, bone_ids);
    }
}

/// Deterministically bakes kinematic cloth wind/gravity motion into an `AnimationClip`
/// by solving each column chain with multi-bone CCD IK (`solve_chain_ik`).
/// Returns the clip index written into `rig.animations`.
pub fn bake_cloth_grid_animation(
    rig: &mut CharacterRig,
    columns: &[Vec<BoneId>],
    params: &ClothWindParams,
) -> Result<usize, CoreError> {
    if columns.is_empty() {
        return Err(CoreError::OutOfBounds);
    }
    if !params.wind_dir.is_finite()
        || !params.wind_strength.is_finite()
        || !params.gravity.is_finite()
        || !params.turbulence.is_finite()
        || !params.phase_offset.is_finite()
        || !params.duration_secs.is_finite()
        || !params.fps.is_finite()
    {
        return Err(CoreError::OutOfBounds);
    }

    let duration = params.duration_secs.clamp(0.1, 120.0);
    let fps = params.fps.clamp(1.0, 60.0);
    let steps = ((duration * fps).round() as usize).clamp(2, 240);
    let wind_dir = params.wind_dir.normalize_or_zero();
    let wind_dir = if wind_dir.length_squared() < 1e-6 {
        Vec3::NEG_Z
    } else {
        wind_dir
    };
    let lateral = wind_dir.cross(Vec3::Y).normalize_or_zero();
    let lateral = if lateral.length_squared() < 1e-6 {
        Vec3::X
    } else {
        lateral
    };

    let clip_name = if params.clip_name.trim().is_empty() {
        "ClothWind".to_string()
    } else {
        params.clip_name.trim().to_string()
    };

    let clip_idx = if let Some(idx) = rig.animations.iter().position(|c| c.name == clip_name) {
        rig.animations[idx].duration_secs = rig.animations[idx].duration_secs.max(duration);
        idx
    } else {
        rig.animations
            .push(AnimationClip::new_empty(clip_name, duration));
        rig.animations.len() - 1
    };

    let rest_world = rig.world_rest_matrices();
    let saved_poses: Vec<(BoneId, Transform)> =
        rig.bones.iter().map(|b| (b.id, b.pose_local)).collect();

    let tau = std::f32::consts::TAU;

    for step in 0..=steps {
        let frac = step as f32 / steps as f32;
        let t = if step == steps {
            duration
        } else {
            frac * duration
        };

        // Reset grid bones to rest before solving each frame cleanly
        for col in columns {
            for &bid in col {
                if let Some(b) = rig.bone_mut(bid) {
                    b.pose_local = b.rest_local;
                }
            }
        }

        for (c_idx, col) in columns.iter().enumerate() {
            if col.len() < 2 {
                continue;
            }
            let Some(&tip_id) = col.last() else {
                continue;
            };
            let Some(&root_id) = col.first() else {
                continue;
            };
            let Some(&tip_rest_mat) = rest_world.get(&tip_id) else {
                continue;
            };
            let Some(&root_rest_mat) = rest_world.get(&root_id) else {
                continue;
            };

            let tip_rest = tip_rest_mat.transform_point3(Vec3::ZERO);
            let root_rest = root_rest_mat.transform_point3(Vec3::ZERO);
            let chain_len = (tip_rest - root_rest).length().max(0.05);

            // Deterministic traveling wave across columns so neighboring chains ripple smoothly
            let col_phase = c_idx as f32 * 0.65 + params.phase_offset;
            let wave = (frac * tau + col_phase).sin();
            let flutter = (frac * tau * 2.0 + col_phase * 1.7).cos();

            let wind_push = wind_dir * (params.wind_strength * chain_len * (0.65 + 0.35 * wave));
            let turb_push = lateral * (params.turbulence * chain_len * flutter * 0.5);
            let grav_sag = Vec3::NEG_Y * (params.gravity * chain_len * 0.25);

            let raw_target = tip_rest + wind_push + turb_push + grav_sag;
            // Clamp target within chain reach from root so CCD converges cleanly
            let offset = raw_target - root_rest;
            let dist = offset.length();
            let target = if dist > chain_len * 0.98 {
                root_rest + offset.normalize_or_zero() * (chain_len * 0.98)
            } else {
                raw_target
            };

            if let Ok(rotations) = solve_chain_ik(rig, col, target, 12, 1e-3) {
                for (bid, rot) in rotations {
                    let kf = Keyframe {
                        time_secs: t,
                        translation: Vec3::ZERO,
                        rotation: rot,
                        scale: Vec3::ONE,
                        interpolation: KeyframeInterpolation::Linear,
                    };
                    if let Some(clip) = rig.animations.get_mut(clip_idx) {
                        clip.upsert_keyframe(bid, kf);
                    }
                }
            }
        }
    }

    // Restore original live pose before returning
    for (bid, pose) in saved_poses {
        if let Some(b) = rig.bone_mut(bid) {
            b.pose_local = pose;
        }
    }

    Ok(clip_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bone_grid_creates_columns_and_bakes_deterministic_wind_clip() {
        let mut rig = CharacterRig::default();
        let spec = BoneGridSpec {
            prefix: "Cape".to_string(),
            cols: 3,
            rows: 4,
            ..Default::default()
        };
        let grid = create_bone_grid(&mut rig, &spec).unwrap();
        assert_eq!(grid.columns.len(), 3);
        assert_eq!(grid.columns[0].len(), 4);
        assert_eq!(rig.bones.len(), 12);

        let params = ClothWindParams {
            clip_name: "CapeWind".to_string(),
            duration_secs: 1.0,
            fps: 6.0,
            ..Default::default()
        };
        let clip_idx = bake_cloth_grid_animation(&mut rig, &grid.columns, &params).unwrap();
        let clip = &rig.animations[clip_idx];
        assert_eq!(clip.name, "CapeWind");
        assert!(!clip.channels.is_empty());
        // Verify loop keyframes were written across multiple frames
        assert!(clip.channels[0].keyframes.len() >= 5);
    }
}
