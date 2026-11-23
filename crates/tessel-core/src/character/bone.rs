use serde::{Deserialize, Serialize};

use crate::scene::Transform;

use super::nano::BoneId;

/// Sub-Unit: Single skeletal joint with rest pose and editable local pose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub id: BoneId,
    pub name: String,
    pub parent: Option<BoneId>,
    pub rest_local: Transform,
    pub pose_local: Transform,
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
        }
    }
}
