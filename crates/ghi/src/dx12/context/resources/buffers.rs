use super::super::*;

impl Device {
	/// Builds every copy of a buffer; see [`crate::context::ContextCreate::build_buffer`].
	pub fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = self.create_buffer(
			T::layout(builder.length),
			builder.resource_uses,
			builder.host_access,
			builder.copies,
			builder.name,
		);
		BufferHandle(BaseBufferHandle::from_index(handle), std::marker::PhantomData)
	}

	/// Builds a buffer the CPU writes and writes every copy with `initialize`; see
	/// [`crate::context::ContextCreate::build_buffer_with`].
	pub fn build_buffer_with<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: buffer::Builder,
		initialize: impl Fn(&mut T),
	) -> BufferHandle<T> {
		crate::buffer::assert_initializable(builder.host_access);
		let handle = self.build_buffer::<T>(builder);
		let buffer = self.buffer(handle.into());
		for copy in 0..buffer.copies.count(self.frames) {
			// SAFETY: The copy is mapped storage of `T`'s layout, and no command has used the new buffer yet.
			initialize(unsafe { &mut *buffer.typed_pointer::<T>(copy) });
		}
		handle
	}

	/// Shares a buffer this context owns so another context of the same device can record copies into it.
	///
	/// D3D12 zeroes committed resources, so the buffer holds zeros or what this context wrote before any copy the
	/// other context records. Keep this context alive for as long as any context uses the imported buffer. Next, pass
	/// the value to [`Self::import_buffer`] on that context.
	///
	/// # Panics
	///
	/// Panics when the handle names no buffer of this context or a buffer with one representation per frame.
	pub fn share_buffer<T: ?Sized>(&mut self, buffer_handle: BufferHandle<T>) -> crate::dx12::factory::SharedBuffer<T> {
		let buffer = self.buffer(buffer_handle.0);
		assert!(
			buffer.copies == crate::Copies::One,
			"Buffer sharing failed. The most likely cause is that the buffer has a copy per frame. Share a buffer built with Copies::One."
		);
		let resource = buffer
			.memory
			.resource
			.clone()
			.expect("Buffer sharing failed. The most likely cause is that the buffer is empty and has no native resource. Share a buffer with a nonzero size.");
		crate::dx12::factory::SharedBuffer {
			resource,
			size: buffer.size,
			uses: buffer.uses,
			heap_kind: buffer.memory.heap_kind,
			contents: std::marker::PhantomData,
		}
	}

	/// Imports a buffer another context shared and returns the handle this context records copies with.
	///
	/// This context only records GPU copies with the handle. CPU access stays with the owning context, which also
	/// keeps the buffer alive, so the import has no CPU mapping of its own.
	pub fn import_buffer<T: ?Sized>(&mut self, buffer: crate::dx12::factory::SharedBuffer<T>) -> BufferHandle<T> {
		self.buffers.push(Buffer {
			memory: BufferMemory {
				resource: Some(buffer.resource),
				mapped: std::ptr::null_mut(),
				heap_kind: buffer.heap_kind,
			},
			frame_copies: Vec::new(),
			size: buffer.size,
			align: 1,
			uses: buffer.uses,
			host_access: crate::HostAccess::None,
			copies: crate::Copies::One,
			name: None,
		});
		BufferHandle(
			BaseBufferHandle::from_index((self.buffers.len() - 1) as u64),
			std::marker::PhantomData,
		)
	}

	pub fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64 {
		let buffer = self.buffer(buffer_handle);
		buffer
			.memory
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	/// Gives the whole mapping of a one-copy buffer to the caller; see
	/// [`crate::context::Context::map_unsynchronized`].
	///
	/// # Safety
	///
	/// The caller must keep the buffer alive and prevent concurrent access for the lifetime of the returned mapping.
	pub unsafe fn map_unsynchronized<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let buffer = self.buffer(buffer_handle.into());
		crate::buffer::assert_unsynchronized_mappable(buffer.host_access, buffer.copies);
		let pointer = buffer.typed_pointer::<T>(0);
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), T::byte_count(pointer)) }
	}

	/// Returns the copy of a buffer the CPU writes that frame sequence `sequence_index` uses; see
	/// [`crate::frame::Frame::map`].
	pub(crate) fn map_for_sequence<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: BufferHandle<T>,
		sequence_index: u8,
	) -> &mut T {
		let buffer = self.buffer(buffer_handle.into());
		crate::buffer::assert_frame_mappable(buffer.host_access, buffer.copies);
		// SAFETY: Starting the frame waited for the last frame of this sequence, so no GPU work uses the copy, and
		// `&mut self` keeps CPU access exclusive.
		unsafe { &mut *buffer.typed_pointer::<T>(sequence_index) }
	}

	/// Returns the copy of a buffer the CPU reads that frame sequence `sequence_index` uses, or `None` before a frame
	/// of that sequence completed; see [`crate::frame::Frame::read`].
	pub(crate) fn read_for_sequence<T: ?Sized + crate::buffer::BufferContents>(
		&self,
		buffer_handle: BufferHandle<T>,
		sequence_index: u8,
		completed: bool,
	) -> Option<&T> {
		let buffer = self.buffer(buffer_handle.into());
		crate::buffer::assert_frame_readable(buffer.host_access, buffer.copies);
		// SAFETY: The completed frame's GPU writes reached the cached mapping, and nothing writes it until this frame's
		// commands run.
		completed.then(|| unsafe { &*buffer.typed_pointer::<T>(sequence_index) })
	}
}
