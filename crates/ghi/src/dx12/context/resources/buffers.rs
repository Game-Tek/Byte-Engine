use super::super::*;

impl Device {
	pub fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Static,
		);
		BufferHandle(BaseBufferHandle(handle), std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Dynamic,
		);
		DynamicBufferHandle(BaseBufferHandle(handle), std::marker::PhantomData)
	}

	pub fn get_buffer_address(&self, _buffer_handle: BaseBufferHandle) -> u64 {
		self.buffer(_buffer_handle)
			.and_then(|buffer| buffer.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	pub(crate) fn buffer_address_for_sequence(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) -> u64 {
		self.buffer_resource_for_sequence(buffer_handle, sequence_index)
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		let buffer = self
			.buffer(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		unsafe { &*(buffer.data as *const T) }
	}

	pub fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: BufferHandle<T>) -> &'a mut T {
		let buffer = self
			.buffer(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		unsafe { &mut *(buffer.data as *mut T) }
	}

	pub(crate) fn buffer_resource_state(
		&self,
		buffer: BaseBufferHandle,
	) -> Option<(DeviceAccesses, BufferHeapKind, bool, bool)> {
		self.buffer(buffer).map(|buffer| {
			(
				buffer.access,
				buffer.heap_kind,
				buffer.resource.is_some(),
				!buffer.mapped.is_null(),
			)
		})
	}

	pub(crate) fn buffer_frame_resource_state(&self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<bool> {
		self.buffer(buffer).map(|buffer| {
			if sequence_index == 0 {
				return buffer.resource.is_some();
			}
			buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.and_then(|resource| resource.resource.as_ref())
				.is_some()
		})
	}

	#[cfg(test)]
	pub(crate) fn buffer_native_size_for_sequence(&mut self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<u64> {
		let resource = self.buffer_resource_for_sequence(buffer, sequence_index)?;
		Some(unsafe { resource.GetDesc() }.Width)
	}

	pub(crate) fn buffer_is_in_common_state(&self, buffer: BaseBufferHandle) -> Option<bool> {
		self.buffer(buffer)
			.and_then(|buffer_data| buffer_data.resource.as_ref())
			.map(|resource| {
				self.buffer_states
					.get(&Self::native_resource_key(resource))
					.copied()
					.unwrap_or(D3D12_RESOURCE_STATE_COMMON)
					== D3D12_RESOURCE_STATE_COMMON
			})
	}

	pub(crate) fn buffer_bytes(&self, buffer: BaseBufferHandle, size: usize) -> Option<Vec<u8>> {
		let buffer_data = self.buffer(buffer)?;
		if size > buffer_data.size {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(buffer_data.data, size).to_vec() })
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
		let mapped = if sequence_index == 0 {
			buffer_data.mapped
		} else {
			buffer_data
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.map(|resource| resource.mapped)
				.unwrap_or(buffer_data.mapped)
		};
		if mapped.is_null() {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(mapped, size).to_vec() })
	}
}
