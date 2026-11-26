use glam::{EulerRot, Quat};
use serde::{Deserialize, Serialize};

use crate::scene::Transform;

use super::constraints::BoneColliderMeta;
use super::nano::BoneId;

fn default_true() -> bool {
    true
}

fn default_damping() -> f32 {
    0.5
}

/// Sub-Unit: Single skeletal joint with rest pose, editable local pose, and optional rigging/physics metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub id: BoneId,
    pub name: String,
    pub parent: Option<BoneId>,
    pub rest_local: Transform,
    pub pose_local: Transform,
    /// Optional Euler XYZ rotation limits in degrees `(min_deg, max_deg)` (`v5.5`).
    #[serde(default)]
    pub rotation_limits_deg: Option<([f32; 3], [f32; 3])>,
    /// Whether this bone inherits parent rotation (`v5.5`, default `true`).
    #[serde(default = "default_true")]
    pub inherit_rotation: bool,
    /// Whether this bone inherits parent scale (`v5.5`, default `true`).
    #[serde(default = "default_true")]
    pub inherit_scale: bool,
    /// Whether this bone is connected head-to-tail to its parent (`v5.5`, default `false`).
    #[serde(default)]
    pub connected: bool,
    /// Whether this bone participates in glTF skin deformation (`v5.6`, default `true`; `false` = helper/control bone).
    #[serde(default = "default_true")]
    pub deform: bool,
    /// Procedural spring stiffness for secondary motion baking (`v5.7`, `0.0` = disabled).
    #[serde(default)]
    pub spring_stiffness: f32,
    /// Procedural spring damping for secondary motion baking (`v5.7`).
    #[serde(default = "default_damping")]
    pub spring_damping: f32,
    /// Skeleton LOD tier (`0` = always active, `>= 1` = detail bone cullable at distance, `v5.7`).
    #[serde(default)]
    pub lod_level: u8,
    /// Optional physics/ragdoll collider metadata exported to glTF `extras` (`v5.7`).
    #[serde(default)]
    pub physics_collider: Option<BoneColliderMeta>,
}

impl Bone {
    pub fn new(
        id: BoneId,
        name: impl Into<String>,
        parent: Option<BoneId>,
        rest_local: Transform,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            parent,
            rest_local,
            pose_local: rest_local,
            rotation_limits_deg: None,
            inherit_rotation: true,
            inherit_scale: true,
            connected: false,
            deform: true,
            spring_stiffness: 0.0,
            spring_damping: default_damping(),
            lod_level: 0,
            physics_collider: None,
        }
    }

    /// Clamps a local rotation quaternion against `rotation_limits_deg` if configured.
    /// Guards against inverted or non-finite bounds so it never panics.
    pub fn clamp_rotation_quat(&self, q: Quat) -> Quat {
        let Some((min_d, max_d)) = self.rotation_limits_deg else {
            return q;
        };
        if !min_d.iter().all(|v| v.is_finite()) || !max_d.iter().all(|v| v.is_finite()) {
            return q;
        }
        let (rx, ry, rz) = q.to_euler(EulerRot::XYZ);
        let cx = rx
            .to_degrees()
            .clamp(min_d[0].min(max_d[0]), min_d[0].max(max_d[0]))
            .to_radians();
        let cy = ry
            .to_degrees()
            .clamp(min_d[1].min(max_d[1]), min_d[1].max(max_d[1]))
            .to_radians();
        let cz = rz
            .to_degrees()
            .clamp(min_d[2].min(max_d[2]), min_d[2].max(max_d[2]))
            .to_radians();
        Quat::from_euler(EulerRot::XYZ, cx, cy, cz)
    }
}
