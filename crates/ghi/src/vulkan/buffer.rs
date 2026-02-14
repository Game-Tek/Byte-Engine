use ash::vk;

use crate::{vulkan::{Handle, HandleLike, Next}, Uses};

/// When `true`, dynamic buffers use a persistent CPU-writable source buffer
/// plus per-frame staging buffers. The source buffer is copied (CPU memcpy)
/// into the current frame's staging buffer every frame, which is then copied
/// to the GPU buffer via `vkCmdCopyBuffer2`. This avoids data races between
/// frames while allowing the CPU to write only when data actually changes.
pub(crate) const PERSISTENT_WRITE: bool = true;

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	pub(crate) next: Option<BufferHandle>,
	pub(crate) staging: Option<BufferHandle>,
	/// When `PERSISTENT_WRITE` is enabled on a dynamic buffer, this points to the
	/// shared, persistent CPU-writable source buffer. The user writes here and
	/// data is memcpy'd to per-frame staging buffers each frame.
	pub(crate) source: Option<BufferHandle>,
	pub(crate) buffer: vk::Buffer,
	pub(crate) size: usize,
	pub(crate) device_address: vk::DeviceAddress,
	pub(crate) pointer: *mut u8,
	pub(crate) uses: Uses,
	pub(crate) access: crate::DeviceAccesses,
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BufferHandle(pub(crate) u64);

impl Into<Handle> for BufferHandle {
	fn into(self) -> Handle {
		Handle::Buffer(self)
	}
}

impl HandleLike for BufferHandle {
	type Item = Buffer;

	fn build(value: u64) -> Self {
		BufferHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Buffer {
		&collection[self.0 as usize]
	}
}

impl Next for Buffer {
	type Handle = BufferHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}
