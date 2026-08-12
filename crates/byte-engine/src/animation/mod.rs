//! Skeletal animation sampling, pose blending, and transition utilities.
//!
//! Use [`sample_local_pose`] to sample clips before applying [`blend`] or
//! [`inertialization`]. Build renderer-facing matrices with
//! [`write_global_pose`], then send those matrices through
//! [`crate::rendering::UpdatePose`].

pub mod blend;
pub mod graph;
pub mod inertialization;
mod math;
pub mod packed;
pub mod root_motion;
pub mod skeletal;

pub use skeletal::{
	compare_animation_bone_positions, sample_local_pose, sample_pose, write_global_pose, AnimationBonePositionComparison,
	AnimationComparisonError, BonePositionDifference, PoseError,
};
