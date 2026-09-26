use std::alloc::Layout;

use crate::{DeviceAccesses, PrivateHandle, PrivateHandles, Uses, graphics_hardware_interface};

/// The `BufferContents` trait lets one typed buffer API cover both fixed-size values and runtime-length arrays.
///
/// A [`crate::BufferHandle<T>`] with a [`crate::Pod`] `T` holds exactly one `T`. A `BufferHandle<[T]>` holds as many
/// `T` elements as [`Builder::length`] requested when [`crate::context::ContextCreate::build_buffer`] created it.
/// CPU views of an array buffer are ordinary slices, so indexing past the allocated length panics instead of
/// reading unrelated memory.
///
/// The trait is sealed. Only `T` and `[T]` for a [`crate::Pod`] `T` implement it.
pub trait BufferContents: sealed::Sealed {
	/// Returns the allocation layout for these contents and the builder's optional element count.
	///
	/// # Panics
	///
	/// Panics when a single value receives a length, when an array receives none, or when the array size overflows.
	fn layout(length: Option<usize>) -> Layout;

	/// Returns a typed pointer over `byte_count` mapped bytes, or `None` when the range cannot hold these contents.
	///
	/// An array view covers every whole element that fits in `byte_count`.
	fn from_raw_parts(pointer: *mut u8, byte_count: usize) -> Option<*mut Self>;

	/// Returns the number of bytes a pointer from [`Self::from_raw_parts`] covers.
	fn byte_count(pointer: *const Self) -> usize;
}

mod sealed {
	pub trait Sealed {}
	impl<T: crate::Pod> Sealed for T {}
	impl<T: crate::Pod> Sealed for [T] {}
}

impl<T: crate::Pod> BufferContents for T {
	fn layout(length: Option<usize>) -> Layout {
		assert!(
			length.is_none(),
			"Invalid buffer length. The most likely cause is that buffer::Builder::length was set for a single-value buffer type; use a slice type such as `[T]` for runtime-length buffers."
		);
		Layout::new::<T>()
	}

	fn from_raw_parts(pointer: *mut u8, byte_count: usize) -> Option<*mut T> {
		if std::mem::size_of::<T>() == 0 {
			return Some(std::ptr::NonNull::<T>::dangling().as_ptr());
		}

		(byte_count >= std::mem::size_of::<T>()
			&& !pointer.is_null()
			&& (pointer as usize).is_multiple_of(std::mem::align_of::<T>()))
		.then_some(pointer.cast::<T>())
	}

	fn byte_count(_: *const T) -> usize {
		std::mem::size_of::<T>()
	}
}

impl<T: crate::Pod> BufferContents for [T] {
	fn layout(length: Option<usize>) -> Layout {
		let length = length.expect(
			"Missing buffer length. The most likely cause is that an array buffer was built without calling buffer::Builder::length.",
		);
		assert!(
			std::mem::size_of::<T>() != 0,
			"Invalid array buffer element. The most likely cause is that the element type is zero-sized and cannot give the array a byte size."
		);
		Layout::array::<T>(length).expect(
			"Invalid buffer length. The most likely cause is that the element count times the element size overflows addressable memory.",
		)
	}

	fn from_raw_parts(pointer: *mut u8, byte_count: usize) -> Option<*mut [T]> {
		let element_size = std::mem::size_of::<T>();
		if element_size == 0 {
			return None;
		}

		let length = byte_count / element_size;
		// An empty slice still needs a non-null, aligned pointer, but never reads through it.
		let elements = if length == 0 {
			std::ptr::NonNull::<T>::dangling().as_ptr()
		} else if !pointer.is_null() && (pointer as usize).is_multiple_of(std::mem::align_of::<T>()) {
			pointer.cast::<T>()
		} else {
			return None;
		};
		Some(std::ptr::slice_from_raw_parts_mut(elements, length))
	}

	fn byte_count(pointer: *const [T]) -> usize {
		pointer.len() * std::mem::size_of::<T>()
	}
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
	pub(crate) length: Option<usize>,
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
			length: None,
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

	/// Sets the element count of an array buffer.
	///
	/// Set a length exactly when the buffer type is a slice, such as `BufferHandle<[u32]>`. Building a slice buffer
	/// without a length, or a single-value buffer with one, panics. See [`BufferContents`].
	pub fn length(mut self, length: usize) -> Self {
		self.length = Some(length);
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
	use super::{BufferContents, Mapping};

	#[repr(C, align(64))]
	#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
	struct AlignedZeroSized;

	#[test]
	fn raw_parts_preserve_zst_alignment_and_reject_invalid_storage() {
		let zero_sized = <AlignedZeroSized as BufferContents>::from_raw_parts(std::ptr::null_mut(), 0)
			.expect("A zero-sized POD value should not require mapped storage");
		assert!(!zero_sized.is_null());
		assert!((zero_sized as usize).is_multiple_of(std::mem::align_of::<AlignedZeroSized>()));

		assert!(<u32 as BufferContents>::from_raw_parts(std::ptr::null_mut(), std::mem::size_of::<u32>()).is_none());
		let mut storage = [0u32; 2];
		assert!(<u32 as BufferContents>::from_raw_parts(storage.as_mut_ptr().cast(), std::mem::size_of::<u16>()).is_none());
		assert!(
			<u32 as BufferContents>::from_raw_parts(storage.as_mut_ptr().cast::<u8>().wrapping_add(1), std::mem::size_of::<u32>(),)
				.is_none()
		);
		assert_eq!(
			<u32 as BufferContents>::from_raw_parts(storage.as_mut_ptr().cast(), std::mem::size_of::<u32>()),
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
