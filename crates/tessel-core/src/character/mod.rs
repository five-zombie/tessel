pub mod anim;
mod bone;
pub mod cloth;
pub mod gizmo;
pub mod ik;
mod nano;
mod rig;
mod skin;

pub use anim::{
    AnimationClip, BoneChannel, MorphWeightKey, MorphWeightTrack, NodeTransformTrack,
    ParentSwitchKey, ParentSwitchTrack, VisibilityKey, VisibilityTrack,
};
pub use bone::Bone;
pub use cloth::{
    assign_bone_grid_mask, bake_cloth_grid_animation, create_bone_grid, BoneGridResult,
    BoneGridSpec, ClothWindParams,
};
pub use gizmo::{
    apply_stick_gizmo_pose, bone_longitudinal_axis, find_mirror_bone, snap_angle_deg,
    StickGizmoMode, StickGizmoParams, StickGizmoSettings,
};
pub use ik::{solve_chain_ik, solve_for_rig, solve_two_bone_ik};
pub use nano::{BoneId, Keyframe, KeyframeInterpolation};
pub use rig::{
    CharacterRig, ClipLayer, ContactChain, GroundingOptions, SampledBonePose, SkinBinding,
    SurfaceHit, SurfaceSource, SurfaceTarget,
};
pub use skin::{AutoSkinSolver, SkinnedBoundsCache, VertexSkinWeights};
