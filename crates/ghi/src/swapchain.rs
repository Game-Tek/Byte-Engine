use crate::{PrivateHandle, PrivateHandles};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SwapchainHandle(pub(crate) u64);

impl From<SwapchainHandle> for PrivateHandles {
	fn from(val: SwapchainHandle) -> Self {
		PrivateHandles::Swapchain(val)
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
