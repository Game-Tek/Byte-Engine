use crate::{PrivateHandle, PrivateHandles};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SwapchainHandle(pub(crate) u64);

impl Into<PrivateHandles> for SwapchainHandle {
	fn into(self) -> PrivateHandles {
		PrivateHandles::Swapchain(self)
	}
}

impl PrivateHandle for SwapchainHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}
