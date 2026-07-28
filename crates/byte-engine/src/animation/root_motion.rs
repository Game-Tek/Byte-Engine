//! Root-motion extraction and composition utilities.

use resource_management::resources::skeleton::LocalTransform;

use super::math::{conjugate_quaternion, multiply_quaternion, nlerp_quaternion};

/// The `RootMotionDelta` struct carries one frame's local translation and rotation change to gameplay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotionDelta {
	pub translation: [f32; 3],
	pub rotation: [f32; 4],
}

impl RootMotionDelta {
	pub const IDENTITY: Self = Self {
		translation: [0.0; 3],
		rotation: [0.0, 0.0, 0.0, 1.0],
	};

	/// Calculates the shortest local transform delta between two sampled root poses.
	pub fn between(previous: LocalTransform, current: LocalTransform) -> Self {
		Self {
			translation: subtract3(current.translation, previous.translation),
			rotation: multiply_quaternion(current.rotation, conjugate_quaternion(previous.rotation)),
		}
	}

	/// Composes this delta followed by `next`.
	///
	/// Translation remains in the skeleton root's parent space, so segment
	/// translations add directly. Use this to join root-motion segments across
	/// a looping clip boundary.
	pub fn then(self, next: Self) -> Self {
		Self {
			translation: add3(self.translation, next.translation),
			rotation: multiply_quaternion(next.rotation, self.rotation),
		}
	}

	/// Blends two root-motion deltas for pose blending.
	pub fn blend(self, other: Self, factor: f32) -> Self {
		let factor = factor.clamp(0.0, 1.0);
		Self {
			translation: std::array::from_fn(|component| {
				self.translation[component] + (other.translation[component] - self.translation[component]) * factor
			}),
			rotation: nlerp_quaternion(self.rotation, other.rotation, factor),
		}
	}
}

impl Default for RootMotionDelta {
	fn default() -> Self {
		Self::IDENTITY
	}
}

/// Extracts one root node's motion and resets its translation and rotation to a reference pose.
///
/// Keep `previous_pose` unmodified between frames. The current root keeps its
/// sampled scale because scale is not locomotion.
pub fn extract_root_motion(
	previous_pose: &[LocalTransform],
	current_pose: &mut [LocalTransform],
	root_node: usize,
	reference: LocalTransform,
) -> Result<RootMotionDelta, RootMotionError> {
	if previous_pose.len() != current_pose.len() {
		return Err(RootMotionError::PoseLength {
			previous: previous_pose.len(),
			current: current_pose.len(),
		});
	}
	let previous = previous_pose
		.get(root_node)
		.copied()
		.ok_or(RootMotionError::RootNodeOutOfRange {
			root_node,
			pose_len: current_pose.len(),
		})?;
	let current = current_pose[root_node];
	let delta = RootMotionDelta::between(previous, current);
	current_pose[root_node].translation = reference.translation;
	current_pose[root_node].rotation = reference.rotation;
	Ok(delta)
}

/// Calculates a forward loop-wrap delta by joining the end and start segments.
pub fn forward_loop_root_motion(
	previous: LocalTransform,
	loop_end: LocalTransform,
	loop_start: LocalTransform,
	current: LocalTransform,
) -> RootMotionDelta {
	RootMotionDelta::between(previous, loop_end).then(RootMotionDelta::between(loop_start, current))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootMotionError {
	PoseLength { previous: usize, current: usize },
	RootNodeOutOfRange { root_node: usize, pose_len: usize },
}

impl std::fmt::Display for RootMotionError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::PoseLength { previous, current } => write!(
				formatter,
				"Root-motion poses have different node counts. The most likely cause is comparing poses with {previous} and {current} nodes."
			),
			Self::RootNodeOutOfRange { root_node, pose_len } => write!(
				formatter,
				"Root-motion node is outside the pose. The most likely cause is selecting node {root_node} in a pose with {pose_len} nodes."
			),
		}
	}
}

impl std::error::Error for RootMotionError {}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] + right[component])
}

fn subtract3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] - right[component])
}

#[cfg(test)]
mod tests {
	use std::f32::consts::FRAC_PI_2;

	use resource_management::resources::skeleton::LocalTransform;

	use super::{extract_root_motion, forward_loop_root_motion, RootMotionDelta};
	use crate::animation::math::quaternion_exp;

	fn root(translation: [f32; 3], yaw: f32) -> LocalTransform {
		LocalTransform {
			translation,
			rotation: quaternion_exp([0.0, yaw, 0.0]),
			scale: [2.0; 3],
		}
	}

	#[test]
	fn extraction_returns_delta_and_makes_current_pose_in_place() {
		let previous = [root([1.0, 0.0, 0.0], 0.0)];
		let mut current = [root([3.0, 0.0, 1.0], FRAC_PI_2)];
		let reference = LocalTransform::identity();
		let delta = extract_root_motion(&previous, &mut current, 0, reference).expect("expected test value");
		assert_eq!(delta.translation, [2.0, 0.0, 1.0]);
		assert_eq!(current[0].translation, reference.translation);
		assert_eq!(current[0].rotation, reference.rotation);
		assert_eq!(current[0].scale, [2.0; 3]);
	}

	#[test]
	fn loop_delta_does_not_move_back_to_the_clip_start() {
		let delta = forward_loop_root_motion(
			root([9.0, 0.0, 0.0], 0.0),
			root([10.0, 0.0, 0.0], 0.0),
			root([0.0, 0.0, 0.0], 0.0),
			root([2.0, 0.0, 0.0], 0.0),
		);
		assert_eq!(
			delta,
			RootMotionDelta {
				translation: [3.0, 0.0, 0.0],
				rotation: [0.0, 0.0, 0.0, 1.0],
			}
		);
	}
}
