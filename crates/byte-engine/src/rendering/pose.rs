use math::Matrix4;

use crate::core::{factory::Handle, message::Message};

#[derive(Clone, Debug)]
/// The `UpdatePose` struct carries one renderable's global skeleton pose from gameplay to rendering.
pub struct UpdatePose {
	handle: Handle,
	global_matrices: Vec<Matrix4>,
}

impl UpdatePose {
	pub fn new(handle: Handle, global_matrices: Vec<Matrix4>) -> Self {
		Self { handle, global_matrices }
	}

	pub fn handle(&self) -> Handle {
		self.handle
	}

	pub fn global_matrices(&self) -> &[Matrix4] {
		&self.global_matrices
	}
}

impl Message for UpdatePose {}

#[cfg(test)]
mod tests {
	use math::mat::MatNew4 as _;

	use super::*;
	use crate::core::factory::Factory;

	#[test]
	fn update_pose_preserves_renderable_and_global_matrices() {
		let handle = Factory::new().create(());
		let matrices = vec![Matrix4::new(
			1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
		)];
		let update = UpdatePose::new(handle, matrices.clone());

		assert_eq!(update.handle(), handle);
		assert_eq!(update.global_matrices(), matrices);
	}
}
