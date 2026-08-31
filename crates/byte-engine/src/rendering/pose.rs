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

use math::Matrix;

use crate::core::{factory::Handle, message::Message, targeted_message::TargetedMessage};
