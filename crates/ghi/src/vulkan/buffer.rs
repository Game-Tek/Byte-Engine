use ash::vk;

use crate::{Copies, HostAccess, Uses};

/// The `Buffer` struct keeps one copy of a GHI buffer: its Vulkan object, the memory behind it, and how the CPU
/// reaches that memory.
///
/// A buffer with [`Copies::PerFrame`] chains one `Buffer` per frame in flight; see
/// [`crate::ResourceCollection::chain`].
#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	pub(crate) buffer: vk::Buffer,
	pub(crate) size: usize,
	pub(crate) device_address: vk::DeviceAddress,
	/// The mapped bytes of a buffer with host access, or null. Host-accessed memory is always coherent, so CPU writes
	/// need no flush and CPU reads no invalidation.
	pub(crate) pointer: crate::vulkan::MappedMemoryPointer,
	/// Dedicated memory backing `buffer`, released together with it when the buffer is replaced.
	pub(crate) allocation: Option<crate::vulkan::AllocationHandle>,
	pub(crate) uses: Uses,
	pub(crate) host_access: HostAccess,
	pub(crate) copies: Copies,
	/// Whether another context owns `buffer`, which this context then never destroys.
	pub(crate) imported: bool,
}
