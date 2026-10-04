//! Flipbook playback, skeletal animation sampling, and pose blending.
//!
//! Use [`flipbook::Flipbook`] to select images from a timed sequence.
//!
//! Use [`sample_local_pose`] to sample clips before applying [`blend`] or
//! [`inertialization`]. Build renderer-facing matrices with
//! [`write_global_pose`], then send those matrices through the renderer's
//! `UpdatePose` message.

pub mod blend;
pub mod flipbook;
pub mod graph;
pub mod inertialization;
pub(crate) mod math;
/// Packed animation storage and allocation-free pose sampling.
pub mod packed;
pub mod root_motion;
/// Local-pose sampling, global-pose construction, and pose comparison.
pub mod skeletal;

pub use skeletal::{
	AnimationBonePositionComparison, AnimationComparisonError, BonePositionDifference, PoseError,
	compare_animation_bone_positions, sample_local_pose, sample_pose, write_global_pose,
};

/// Builds a skeleton node for tests that author small hierarchies.
#[cfg(test)]
fn test_node(
	name: Option<&str>,
	parent: Option<u32>,
	rest_local: resource_management::resources::skeleton::LocalTransform,
) -> resource_management::resources::skeleton::SkeletonNode {
	resource_management::resources::skeleton::SkeletonNode {
		name: name.map(Into::into),
		parent,
		rest_local,
	}
}
