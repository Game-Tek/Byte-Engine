#[derive(Clone, Debug)]
/// The `UpdatePose` struct carries one renderable's global skeleton pose from gameplay to rendering.
///
/// The pose holds one affine transform per skeleton node, the form skinning reads, so rendering stores it as sent.
pub struct UpdatePose {
	handle: Handle,
	global_matrices: Vec<AffineMatrix>,
}

impl UpdatePose {
	/// Creates a pose update for one renderable.
	pub fn new(handle: Handle, global_matrices: Vec<AffineMatrix>) -> Self {
		Self { handle, global_matrices }
	}

	/// Returns the renderable that owns the pose.
	pub fn handle(&self) -> Handle {
		self.handle
	}

	/// Returns the global transform of each skeleton joint.
	pub fn global_matrices(&self) -> &[AffineMatrix] {
		&self.global_matrices
	}
}

impl Message for UpdatePose {}

impl TargetedMessage for UpdatePose {
	type Payload = Vec<AffineMatrix>;

	fn from_handle_and_payload(handle: Handle, pose: Self::Payload) -> Self {
		Self::new(handle, pose)
	}
}

use math::AffineMatrix;

use crate::core::{factory::Handle, message::Message, targeted_message::TargetedMessage};
