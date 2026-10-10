use ash::vk;

use crate::{Uses, buffer::BufferHandle};

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	/// The mapped buffer that receives CPU writes before a transfer copies them into `buffer`, for a buffer whose own
	/// memory is not mapped.
	pub(crate) staging: Option<BufferHandle>,
	pub(crate) buffer: vk::Buffer,
	pub(crate) size: usize,
	pub(crate) device_address: vk::DeviceAddress,
	pub(crate) pointer: crate::vulkan::MappedMemoryPointer,
	/// Dedicated memory backing `buffer`, released together with it when the buffer is replaced.
	pub(crate) allocation: Option<crate::AllocationHandle>,
	pub(crate) uses: Uses,
	pub(crate) access: crate::DeviceAccesses,
	/// Whether another context owns `buffer`, which this context then never destroys.
	pub(crate) imported: bool,
}
