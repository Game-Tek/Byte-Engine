use crate::{DeviceAccesses, PrivateHandle, PrivateHandles, Uses, graphics_hardware_interface};

/// Returns a typed pointer only when the mapped byte range satisfies Rust's reference requirements.
pub(crate) fn typed_buffer_pointer<T: crate::Pod>(pointer: *mut u8, byte_count: usize) -> Option<*mut T> {
	if std::mem::size_of::<T>() == 0 {
		return Some(std::ptr::NonNull::<T>::dangling().as_ptr());
	}

	(byte_count >= std::mem::size_of::<T>()
		&& !pointer.is_null()
		&& (pointer as usize).is_multiple_of(std::mem::align_of::<T>()))
	.then_some(pointer.cast::<T>())
}

/// The `Mapping` struct transfers exclusive CPU access to one persistently mapped buffer.
///
/// A mapping does not own the backend allocation. The context that created it must remain
/// alive until the mapping and every region derived from it are no longer used.
pub struct Mapping {
	address: usize,
	byte_count: usize,
}

impl Mapping {
	/// Creates an exclusive mapping capability for backend-owned memory.
	///
	/// # Safety
	///
	/// `pointer..pointer + byte_count` must remain allocated and mapped until this
	/// capability is discarded. No other CPU mapping may access that range while
	/// this capability or any region derived from it exists.
	pub(crate) unsafe fn from_raw_parts(pointer: *mut u8, byte_count: usize) -> Self {
		assert!(
			!pointer.is_null(),
			"Buffer mapping transfer failed. The most likely cause is that the buffer was not created with CPU-visible memory."
		);
		Self {
			address: pointer as usize,
			byte_count,
		}
	}

	/// Returns the mapped byte count.
	pub fn byte_count(&self) -> usize {
		self.byte_count
	}

	/// Consumes the mapping and returns its address and byte count for an exclusive region owner.
	pub fn into_raw_parts(self) -> (usize, usize) {
		(self.address, self.byte_count)
	}
}

/// The `Builder` struct configures buffer creation parameters that can be shared across static and dynamic buffer constructors.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
}

impl<'a> Builder<'a> {
	/// Creates a buffer builder with GPU read and write access.
	///
	/// The default name is `None`.
	pub fn new(resource_uses: Uses) -> Self {
		Self {
			name: None,
			resource_uses,
			device_accesses: DeviceAccesses::DeviceOnly,
		}
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	pub fn device_accesses(mut self, device_accesses: DeviceAccesses) -> Self {
		self.device_accesses = device_accesses;
		self
	}
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct BufferHandle(pub(crate) u64);

impl From<BufferHandle> for graphics_hardware_interface::Handles {
	fn from(val: BufferHandle) -> Self {
		graphics_hardware_interface::Handles::Buffer(graphics_hardware_interface::BaseBufferHandle(val.0))
	}
}

impl From<BufferHandle> for PrivateHandles {
	fn from(val: BufferHandle) -> Self {
		PrivateHandles::Buffer(val)
	}
}

impl PrivateHandle for BufferHandle {
	fn new(i: u64) -> Self {
		BufferHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[cfg(test)]
mod tests {
	use super::{Mapping, typed_buffer_pointer};

	#[repr(C, align(64))]
	#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
	struct AlignedZeroSized;

	#[test]
	fn typed_buffer_pointer_preserves_zst_alignment_and_rejects_invalid_storage() {
		let zero_sized = typed_buffer_pointer::<AlignedZeroSized>(std::ptr::null_mut(), 0)
			.expect("A zero-sized POD value should not require mapped storage");
		assert!(!zero_sized.is_null());
		assert!((zero_sized as usize).is_multiple_of(std::mem::align_of::<AlignedZeroSized>()));

		assert!(typed_buffer_pointer::<u32>(std::ptr::null_mut(), std::mem::size_of::<u32>()).is_none());
		let mut storage = [0u32; 2];
		assert!(typed_buffer_pointer::<u32>(storage.as_mut_ptr().cast(), std::mem::size_of::<u16>()).is_none());
		assert!(
			typed_buffer_pointer::<u32>(storage.as_mut_ptr().cast::<u8>().wrapping_add(1), std::mem::size_of::<u32>(),)
				.is_none()
		);
		assert_eq!(
			typed_buffer_pointer::<u32>(storage.as_mut_ptr().cast(), std::mem::size_of::<u32>()),
			Some(storage.as_mut_ptr())
		);
	}

	#[test]
	fn mapping_transfers_address_and_size_without_borrowing() {
		let mut bytes = [0u8; 8];
		let pointer = bytes.as_mut_ptr();
		// SAFETY: The stack array remains alive and exclusively borrowed until the mapping is consumed below.
		let mapping = unsafe { Mapping::from_raw_parts(pointer, bytes.len()) };

		assert_eq!(mapping.byte_count(), bytes.len());
		assert_eq!(mapping.into_raw_parts(), (pointer as usize, bytes.len()));
	}
}
