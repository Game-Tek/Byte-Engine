//! Root-motion deltas that the animation graph player reports to gameplay.

use math::{Orientation, Vector};
use resource_management::resources::{ParentSpace, skeleton::LocalTransform};

/// The `RootMotionDelta` struct carries one frame's local translation and rotation change to gameplay.
///
/// Its default is [`Self::IDENTITY`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RootMotionDelta {
	/// Translation to apply in the skeleton root's parent space.
	pub translation: Vector<ParentSpace>,
	/// Rotation to apply after the previous root rotation.
	pub rotation: Orientation,
}

impl RootMotionDelta {
	/// The delta that preserves the current gameplay transform.
	pub const IDENTITY: Self = Self {
		translation: Vector::zero(),
		rotation: Orientation::identity(),
	};

	/// Calculates the shortest local transform delta between two sampled root poses.
	pub fn between(previous: LocalTransform, current: LocalTransform) -> Self {
		Self {
			translation: current.translation - previous.translation,
			rotation: current.rotation.compose(previous.rotation.inverse()),
		}
	}

	/// Composes this delta followed by `next`.
	///
	/// Translation remains in the skeleton root's parent space, so segment
	/// translations add directly. Use this to join root-motion segments across
	/// a looping clip boundary.
	pub fn then(self, next: Self) -> Self {
		Self {
			translation: self.translation + next.translation,
			rotation: next.rotation.compose(self.rotation),
		}
	}

	/// Blends two root-motion deltas for pose blending.
	pub fn blend(self, other: Self, factor: f32) -> Self {
		let factor = factor.clamp(0.0, 1.0);
		Self {
			translation: self.translation.lerp(other.translation, factor),
			rotation: self.rotation.nlerp(other.rotation, factor),
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Scale, Vector};
	use resource_management::resources::{ParentSpace, skeleton::LocalTransform};

	use super::RootMotionDelta;

	fn root(translation: [f32; 3], yaw: f32) -> LocalTransform {
		LocalTransform {
			translation: Vector::from_array(translation),
			rotation: Orientation::try_from_rotation_vector(Vector::<ParentSpace>::new(0.0, yaw, 0.0)).unwrap(),
			scale: Scale::new(2.0, 2.0, 2.0),
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
				translation: Vector::new(3.0, 0.0, 0.0),
				rotation: Orientation::identity(),
			}
		);
	}
}
