use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::character::{BoneId, VisibilityKey, VisibilityTrack};
use crate::error::CoreError;
use crate::palette::PaletteIndex;
use crate::scene::{NodeId, SceneState};

use super::face::Face;
use super::lowpoly::LowPolyMesh;
use super::npr::AlphaMode;

/// Parameters for generating a deterministic procedural ribbon / trail mesh
/// following either a skeletal bone or a scene node trajectory across an animation clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RibbonParams {
    pub name: String,
    #[serde(default)]
    pub source_bone: Option<BoneId>,
    #[serde(default)]
    pub source_node: Option<NodeId>,
    #[serde(default)]
    pub explicit_points: Vec<Vec3>,
    #[serde(default)]
    pub clip_index: usize,
    #[serde(default)]
    pub start_time: f32,
    #[serde(default = "default_end_time")]
    pub end_time: f32,
    #[serde(default = "default_samples")]
    pub samples: usize,
    #[serde(default = "default_width_start")]
    pub width_start: f32,
    #[serde(default = "default_width_end")]
    pub width_end: f32,
    pub material: PaletteIndex,
    #[serde(default)]
    pub bake_visibility_track: bool,
}

fn default_end_time() -> f32 {
    1.0
}

fn default_samples() -> usize {
    16
}

fn default_width_start() -> f32 {
    0.16
}

fn default_width_end() -> f32 {
    0.02
}

/// Samples world-space trajectory points along `[start_time, end_time]` from `source_bone`,
/// `source_node`, or `explicit_points`.
pub fn sample_trajectory_points(
    scene: &SceneState,
    params: &RibbonParams,
) -> Result<Vec<Vec3>, CoreError> {
    if !params.explicit_points.is_empty() {
        if params.explicit_points.len() < 2 || params.explicit_points.iter().any(|p| !p.is_finite())
        {
            return Err(CoreError::OutOfBounds);
        }
        return Ok(params.explicit_points.clone());
    }

    if !params.start_time.is_finite() || !params.end_time.is_finite() {
        return Err(CoreError::OutOfBounds);
    }
    let samples = params.samples.clamp(2, 128);
    let t0 = params.start_time.max(0.0);
    let t1 = params.end_time.max(t0 + 1e-3);

    let mut eval_scene = scene.clone();
    let mut points = Vec::with_capacity(samples);

    for i in 0..samples {
        let frac = i as f32 / (samples - 1) as f32;
        let t = t0 + (t1 - t0) * frac;

        if let Some(rig) = eval_scene.character_rig.as_mut() {
            if params.clip_index < rig.animations.len() {
                rig.active_animation = params.clip_index;
                rig.playback_time = t;
                rig.apply_active_animation();
            }
        }

        let pos = if let Some(bid) = params.source_bone {
            eval_scene
                .character_rig
                .as_ref()
                .and_then(|r| r.world_pose_matrices().get(&bid).copied())
                .map(|m| m.transform_point3(Vec3::ZERO))
                .ok_or(CoreError::OutOfBounds)?
        } else if let Some(nid) = params.source_node {
            if !eval_scene.nodes.contains_key(&nid) {
                return Err(CoreError::NodeNotFound(nid));
            }
            eval_scene
                .node_world_matrix(nid)
                .transform_point3(Vec3::ZERO)
        } else {
            return Err(CoreError::OutOfBounds);
        };
        points.push(pos);
    }

    Ok(points)
}

/// Generates a double-sided procedural ribbon strip mesh along `points` with width taper,
/// UV coordinates (`u = progress`, `v = 0|1`), and `COLOR_0` alpha fade from head (`1.0`) to tail (`0.0`).
pub fn generate_ribbon_mesh(
    points: &[Vec3],
    width_start: f32,
    width_end: f32,
    material: PaletteIndex,
) -> Result<LowPolyMesh, CoreError> {
    if points.len() < 2
        || !width_start.is_finite()
        || !width_end.is_finite()
        || width_start < 0.0
        || width_end < 0.0
    {
        return Err(CoreError::OutOfBounds);
    }

    let mut mesh = LowPolyMesh::new();
    mesh.smooth_shading = true;
    mesh.material.double_sided = true;
    mesh.material.alpha_mode = AlphaMode::Blend;
    mesh.material.unlit = true;

    let n = points.len();
    for (i, &pt) in points.iter().enumerate() {
        let frac = i as f32 / (n - 1) as f32;
        let tangent = if i == 0 {
            (points[1] - points[0]).normalize_or_zero()
        } else if i + 1 == n {
            (points[n - 1] - points[n - 2]).normalize_or_zero()
        } else {
            (points[i + 1] - points[i - 1]).normalize_or_zero()
        };
        let tangent = if tangent.length_squared() < 1e-6 {
            Vec3::Z
        } else {
            tangent
        };
        let ref_up = if tangent.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let side = tangent.cross(ref_up).normalize_or_zero();
        let half_w = (width_start + (width_end - width_start) * frac) * 0.5;
        let alpha = (1.0 - frac).clamp(0.0, 1.0);

        let left_idx = mesh.vertices.len();
        mesh.vertices.push(pt - side * half_w);
        mesh.vertex_colors.insert(left_idx, [1.0, 1.0, 1.0, alpha]);

        let right_idx = mesh.vertices.len();
        mesh.vertices.push(pt + side * half_w);
        mesh.vertex_colors.insert(right_idx, [1.0, 1.0, 1.0, alpha]);
    }

    for i in 0..(n - 1) {
        let u0 = i as f32 / (n - 1) as f32;
        let u1 = (i + 1) as f32 / (n - 1) as f32;
        let l0 = i * 2;
        let r0 = i * 2 + 1;
        let l1 = (i + 1) * 2;
        let r1 = (i + 1) * 2 + 1;
        mesh.faces.push(Face {
            vertices: vec![l0, r0, r1, l1],
            material,
            uvs: vec![[u0, 0.0], [u0, 1.0], [u1, 1.0], [u1, 0.0]],
        });
    }

    Ok(mesh)
}

/// Builds a deterministic STEP visibility track for `node_id` so the baked ribbon appears
/// strictly during `[start_time, end_time]` in glTF 2.0 export.
pub fn build_ribbon_visibility_track(
    node_id: NodeId,
    start_time: f32,
    end_time: f32,
) -> VisibilityTrack {
    let t0 = start_time.max(0.0);
    let t1 = end_time.max(t0 + 1e-3);
    let mut keys = Vec::new();
    if t0 > 1e-3 {
        keys.push(VisibilityKey {
            time_secs: 0.0,
            visible: false,
        });
    }
    keys.push(VisibilityKey {
        time_secs: t0,
        visible: true,
    });
    keys.push(VisibilityKey {
        time_secs: t1,
        visible: false,
    });
    VisibilityTrack {
        node: node_id,
        keys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_ribbon_generates_tapered_strip_with_alpha_fade() {
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.5, 0.5),
            Vec3::new(0.0, 1.0, 1.0),
            Vec3::new(0.0, 1.2, 1.8),
        ];
        let mesh = generate_ribbon_mesh(&pts, 0.2, 0.02, PaletteIndex::new(1).unwrap()).unwrap();
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.faces.len(), 3);
        assert!(mesh.material.double_sided);
        assert_eq!(mesh.material.alpha_mode, AlphaMode::Blend);
        assert!((mesh.vertex_colors[&0][3] - 1.0).abs() < 1e-5);
        assert!((mesh.vertex_colors[&7][3] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn sample_trajectory_rejects_non_finite_time_bounds() {
        let scene = SceneState::new();
        let params = RibbonParams {
            name: "BadTime".to_string(),
            source_bone: Some(BoneId(0)),
            source_node: None,
            explicit_points: Vec::new(),
            clip_index: 0,
            start_time: f32::NAN,
            end_time: f32::INFINITY,
            samples: 8,
            width_start: 0.1,
            width_end: 0.02,
            material: PaletteIndex::new(1).unwrap(),
            bake_visibility_track: false,
        };
        assert!(sample_trajectory_points(&scene, &params).is_err());
    }
}
