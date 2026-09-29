//! Root-motion deltas that the animation graph player reports to gameplay.

use resource_management::resources::skeleton::LocalTransform;

use super::math::{add3, conjugate_quaternion, lerp3, multiply_quaternion, nlerp_quaternion, sub3};

/// The `RootMotionDelta` struct carries one frame's local translation and rotation change to gameplay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotionDelta {
	/// Translation to apply in the skeleton root's parent space.
	pub translation: [f32; 3],
	/// Rotation to apply after the previous root rotation.
	pub rotation: [f32; 4],
}

impl RootMotionDelta {
	/// The delta that preserves the current gameplay transform.
	pub const IDENTITY: Self = Self {
		translation: [0.0; 3],
		rotation: [0.0, 0.0, 0.0, 1.0],
	};

	/// Calculates the shortest local transform delta between two sampled root poses.
	pub fn between(previous: LocalTransform, current: LocalTransform) -> Self {
		Self {
			translation: sub3(current.translation, previous.translation),
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
			translation: lerp3(self.translation, other.translation, factor),
			rotation: nlerp_quaternion(self.rotation, other.rotation, factor),
		}
	}
}

impl Default for RootMotionDelta {
	fn default() -> Self {
		Self::IDENTITY
	}
}

#[cfg(test)]
mod tests {
	use resource_management::resources::skeleton::LocalTransform;

	use super::RootMotionDelta;
	use crate::animation::math::quaternion_exp;

	fn root(translation: [f32; 3], yaw: f32) -> LocalTransform {
		LocalTransform {
			translation,
			rotation: quaternion_exp([0.0, yaw, 0.0]),
			scale: [2.0; 3],
		}
	}

	#[test]
	fn loop_delta_does_not_move_back_to_the_clip_start() {
		// Join the segment up to the clip end with the segment from the clip start, as a loop wrap does.
		let delta = RootMotionDelta::between(root([9.0, 0.0, 0.0], 0.0), root([10.0, 0.0, 0.0], 0.0)).then(
			RootMotionDelta::between(root([0.0, 0.0, 0.0], 0.0), root([2.0, 0.0, 0.0], 0.0)),
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
