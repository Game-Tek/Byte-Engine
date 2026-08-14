#[derive(Clone, Debug)]
/// The `UpdatePose` struct carries one renderable's global skeleton pose from gameplay to rendering.
pub struct UpdatePose {
	handle: Handle,
	global_matrices: Vec<Matrix>,
}

impl UpdatePose {
	/// Creates a pose update for one renderable.
	pub fn new(handle: Handle, global_matrices: Vec<Matrix>) -> Self {
		Self { handle, global_matrices }
	}

	/// Returns the renderable that owns the pose.
	pub fn handle(&self) -> Handle {
		self.handle
	}

	/// Returns the global matrix for each skeleton joint.
	pub fn global_matrices(&self) -> &[Matrix] {
		&self.global_matrices
	}
}

impl Message for UpdatePose {}

impl TargetedMessage for UpdatePose {
	type Payload = Vec<Matrix>;

	fn from_handle_and_payload(handle: Handle, pose: Self::Payload) -> Self {
		Self::new(handle, pose)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::factory::Factory;

	#[test]
	fn update_pose_preserves_renderable_and_global_matrices() {
		let handle = Factory::new().create(());
		let matrices = vec![Matrix::new(
			1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
		)];
		let update = UpdatePose::new(handle, matrices.clone());

		assert_eq!(update.handle(), handle);
		assert_eq!(update.global_matrices(), matrices);
	}
}

use math::Matrix;
use maths_rs::mat::MatNew4 as _;

use crate::core::{factory::Handle, message::Message, targeted_message::TargetedMessage};
