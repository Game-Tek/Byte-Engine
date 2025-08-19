use ash::vk;

use crate::{vulkan::{BufferHandle, Next}, Uses};

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	pub(crate) next: Option<BufferHandle>,
	pub(crate) staging: Option<BufferHandle>,
	pub(crate) buffer: vk::Buffer,
	pub(crate) size: usize,
	pub(crate) device_address: vk::DeviceAddress,
	pub(crate) pointer: *mut u8,
	pub(crate) uses: Uses,
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Next for Buffer {
	type Handle = BufferHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}
