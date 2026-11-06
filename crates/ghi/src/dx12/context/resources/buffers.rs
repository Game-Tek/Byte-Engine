use super::super::*;

impl Device {
	pub fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			T::layout(builder.length),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Static,
			builder.name,
		);
		BufferHandle(BaseBufferHandle::from_index(handle), std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: buffer::Builder,
	) -> DynamicBufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			<T as crate::buffer::BufferContents>::layout(builder.length),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Dynamic,
			builder.name,
		);
		DynamicBufferHandle(BaseBufferHandle::from_index(handle), std::marker::PhantomData)
	}

	/// Shares a buffer this context owns so another context of the same device can record copies into it.
	///
	/// Keep this context alive for as long as any context uses the imported buffer. Next, pass the value to
	/// [`Self::import_buffer`] on that context.
	///
	/// # Panics
	///
	/// Panics when the handle names no buffer of this context or a buffer with one representation per frame.
	pub fn share_buffer<T: ?Sized>(&self, buffer_handle: BufferHandle<T>) -> crate::dx12::factory::SharedBuffer<T> {
		let buffer = self
			.buffer(buffer_handle.0)
			.expect("Buffer sharing failed. The most likely cause is that the handle is stale or belongs to another context.");
		assert!(
			buffer.frame_resources.is_none(),
			"Buffer sharing failed. The most likely cause is that the handle names a dynamic buffer."
		);
		let resource = buffer
			.memory
			.resource
			.clone()
			.expect("Buffer sharing failed. The most likely cause is that the buffer is empty and has no native resource.");
		crate::dx12::factory::SharedBuffer {
			resource,
			size: buffer.size,
			uses: buffer.uses,
			access: buffer.access,
			heap_kind: buffer.memory.heap_kind,
			contents: std::marker::PhantomData,
		}
	}

	/// Imports a buffer another context shared and returns the handle this context records copies with.
	///
	/// This context only records GPU copies with the handle. CPU access stays with the owning context, which also
	/// keeps the buffer alive, so the import has no CPU storage of its own.
	pub fn import_buffer<T: ?Sized>(&mut self, buffer: crate::dx12::factory::SharedBuffer<T>) -> BufferHandle<T> {
		let layout = Layout::new::<()>();
		self.buffers.push(Buffer {
			memory: BufferMemory {
				data: std::ptr::without_provenance_mut(layout.align()),
				layout,
				// Equal generations mark the absent CPU storage as already uploaded, so no sync ever writes it.
				host_generation: 0,
				uploaded_generation: 0,
				resource: Some(buffer.resource),
				mapped: std::ptr::null_mut(),
				heap_kind: buffer.heap_kind,
				transferred: false,
			},
			size: buffer.size,
			uses: buffer.uses,
			access: buffer.access,
			frame_resources: None,
			name: None,
		});
		BufferHandle(
			BaseBufferHandle::from_index((self.buffers.len() - 1) as u64),
			std::marker::PhantomData,
		)
	}

	pub fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64 {
		let buffer = self
			.buffer(buffer_handle)
			.expect("Invalid DX12 buffer address request. The most likely cause is that the handle is stale.");
		assert!(
			buffer.memory.heap_kind != BufferHeapKind::Readback,
			"Invalid DX12 readback-buffer address request. The most likely cause is that CPU readback memory was selected for a GPU-address operation. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#readback-heap-resources."
		);
		buffer
			.memory
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	pub fn get_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		let buffer = self
			.buffer_mut(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		Self::take_gpu_writes(buffer);
		let buffer = &*buffer;
		let pointer = Self::typed_buffer_pointer::<T>(buffer);
		// SAFETY: Typed handles preserve the allocation's layout; zero-sized buffers carry an aligned sentinel pointer.
		unsafe { &*pointer }
	}

	pub fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: BufferHandle<T>,
	) -> &mut T {
		let buffer = self
			.buffer_mut(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		// A mutable view reads too, as on backends whose CPU and GPU share the buffer's memory.
		Self::take_gpu_writes(buffer);
		Self::mark_host_write(&mut buffer.memory);
		let pointer = Self::typed_buffer_pointer::<T>(buffer);
		// SAFETY: Typed handles preserve the allocation's layout and `&mut self` guarantees exclusive CPU access.
		unsafe { &mut *pointer }
	}

	/// Copies what the GPU wrote to a CPU-readable buffer through its mapping into the CPU copy, which views read.
	fn take_gpu_writes(buffer: &mut Buffer) {
		if buffer.memory.heap_kind.host_readable() && !buffer.memory.mapped.is_null() && !buffer.memory.transferred {
			// SAFETY: The mapping and the CPU copy each cover the buffer's size, and are separate allocations.
			unsafe { std::ptr::copy_nonoverlapping(buffer.memory.mapped, buffer.memory.data, buffer.size) };
		}
	}

	/// Returns the typed CPU view of a buffer's host storage, sized by the allocation's recorded byte count.
	fn typed_buffer_pointer<T: ?Sized + crate::buffer::BufferContents>(buffer: &Buffer) -> *mut T {
		<T as crate::buffer::BufferContents>::from_raw_parts(buffer.memory.data, buffer.size).expect(
			"Failed to map a typed DX12 buffer. The most likely cause is that the buffer has no sufficiently large, aligned CPU storage.",
		)
	}

	/// Transfers the mapped range to a higher-level owner without manufacturing an unbounded reference.
	///
	/// # Safety
	///
	/// The caller must keep the buffer alive and prevent concurrent access for the lifetime of the returned mapping.
	pub unsafe fn transfer_buffer_mapping<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let buffer = self
			.buffer_mut(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		let pointer = Self::typed_buffer_pointer::<T>(buffer);
		let byte_count = T::byte_count(pointer);
		if buffer.memory.mapped.is_null() {
			// Without a native mapping the owner writes the CPU copy, which every later sync uploads whole.
			Self::mark_host_write(&mut buffer.memory);
			// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
			return unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), byte_count) };
		}
		// The owner writes the mapped memory from other threads, where no later sync sees its writes, so the CPU copy is
		// uploaded one last time and retired.
		let size = buffer.size;
		Self::sync_buffer_memory(&mut buffer.memory, size, buffer.access);
		buffer.memory.transferred = true;
		// SAFETY: The mapping spans the buffer's size for as long as its resource lives, which the caller guarantees by
		// keeping the context alive.
		unsafe { crate::buffer::Mapping::from_raw_parts(buffer.memory.mapped, byte_count) }
	}

	pub(crate) fn buffer_resource_state(
		&self,
		buffer: BaseBufferHandle,
	) -> Option<(DeviceAccesses, BufferHeapKind, bool, bool)> {
		self.buffer(buffer).map(|buffer| {
			(
				buffer.access,
				buffer.memory.heap_kind,
				buffer.memory.resource.is_some(),
				!buffer.memory.mapped.is_null(),
			)
		})
	}

	pub(crate) fn buffer_frame_resource_state(&self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<bool> {
		// A sequence without its own frame copy reports no resource instead of falling back to the base storage.
		self.buffer(buffer).map(|buffer| {
			if sequence_index == 0 {
				return buffer.memory.resource.is_some();
			}
			buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize)?.as_ref())
				.is_some_and(|memory| memory.resource.is_some())
		})
	}

	#[cfg(test)]
	pub(crate) fn buffer_native_size_for_sequence(&mut self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<u64> {
		let resource = self.buffer_resource_for_sequence(buffer, sequence_index)?;
		Some(unsafe { resource.GetDesc() }.Width)
	}

	pub(crate) fn buffer_is_in_common_state(&self, buffer: BaseBufferHandle) -> Option<bool> {
		self.buffer(buffer)
			.and_then(|buffer_data| buffer_data.memory.resource.as_ref())
			.map(|resource| {
				self.buffer_states
					.get(&Self::native_resource_key(resource))
					.copied()
					.unwrap_or(BufferBarrierState::COMMON)
					== BufferBarrierState::COMMON
			})
	}

	pub(crate) fn buffer_bytes(&self, buffer: BaseBufferHandle, size: usize) -> Option<Vec<u8>> {
		let buffer_data = self.buffer(buffer)?;
		if size > buffer_data.size {
			return None;
		}
		// SAFETY: Buffer shadow storage is non-null, stable, and contains at least the checked number of initialized bytes.
		Some(unsafe { std::slice::from_raw_parts(buffer_data.memory.data, size).to_vec() })
	}

	pub(crate) fn buffer_bytes_for_sequence(
		&self,
		buffer: BaseBufferHandle,
		size: usize,
		sequence_index: u8,
	) -> Option<Vec<u8>> {
		let (data, buffer_size) = self.buffer_storage_parts_for_sequence(buffer, sequence_index)?;
		if size > buffer_size {
			return None;
		}
		// SAFETY: Sequence storage is non-null, stable, and contains at least the checked number of initialized bytes.
		Some(unsafe { std::slice::from_raw_parts(data, size).to_vec() })
	}

	/// Returns bytes currently visible through a host-mapped DX12 buffer resource.
	#[cfg(test)]
	pub(crate) fn buffer_mapped_bytes_for_sequence(
		&mut self,
		buffer: BaseBufferHandle,
		size: usize,
		sequence_index: u8,
	) -> Option<Vec<u8>> {
		self.ensure_buffer_frame_storage(buffer, sequence_index);
		let buffer_data = self.buffer(buffer)?;
		if size > buffer_data.size {
			return None;
		}
		let mapped = buffer_data.memory(sequence_index).mapped;
		if mapped.is_null() {
			return None;
		}
		// SAFETY: The retained resource mapping covers at least the checked logical buffer size.
		Some(unsafe { std::slice::from_raw_parts(mapped, size).to_vec() })
	}
}
