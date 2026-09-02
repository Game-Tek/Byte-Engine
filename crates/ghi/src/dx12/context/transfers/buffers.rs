use super::*;

impl Device {
	pub(crate) fn copy_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			let source_heap = self
				.buffer_heap_kind_for_sequence(copy.source_buffer, sequence_index)
				.expect("Invalid DX12 buffer copy source. The most likely cause is that the source handle is stale.");
			let destination_heap = self
				.buffer_heap_kind_for_sequence(copy.destination_buffer, sequence_index)
				.expect("Invalid DX12 buffer copy destination. The most likely cause is that the destination handle is stale.");
			// Validate both native accesses before the CPU shadow copy mutates observable buffer contents.
			Self::validate_buffer_heap_access(source_heap, BufferBarrierState::COPY_SOURCE);
			Self::validate_buffer_heap_access(destination_heap, BufferBarrierState::COPY_DESTINATION);
			self.copy_buffer_shadow(copy, sequence_index);
			self.record_buffer_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	pub(crate) fn clear_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handles: &[BaseBufferHandle],
		sequence_index: u8,
	) {
		for &buffer_handle in buffer_handles {
			if self.buffer_needs_cpu_shadow_clear(buffer_handle) {
				self.clear_buffer_shadow(buffer_handle, sequence_index);
			}
		}

		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let mut gpu_clear_buffers = SmallVec::<[(BaseBufferHandle, ID3D12Resource); 16]>::new();
		let mut clear_barriers = EnhancedBarrierBatch::default();
		for &buffer_handle in buffer_handles {
			let Some(buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
				continue;
			};
			if buffer.access.intersects(DeviceAccesses::GpuWrite)
				&& buffer.heap_kind == BufferHeapKind::Default
				&& buffer.size != 0
				&& buffer.size % std::mem::size_of::<u32>() == 0
			{
				if gpu_clear_buffers.iter().any(|(handle, _)| *handle == buffer_handle) {
					continue;
				}
				self.transition_tracked_buffer_into(
					buffer_handle,
					&buffer.resource,
					BufferBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
					&mut clear_barriers,
				);
				gpu_clear_buffers.push((buffer_handle, buffer.resource));
			}
		}
		Self::submit_resource_barriers(&command_list, &clear_barriers);

		for &buffer_handle in buffer_handles {
			let Some(buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
				continue;
			};
			if buffer.access.intersects(DeviceAccesses::GpuWrite)
				&& buffer.heap_kind == BufferHeapKind::Default
				&& buffer.size != 0
				&& buffer.size % std::mem::size_of::<u32>() == 0
			{
				let description = Self::raw_buffer_clear_uav_desc(buffer.size);
				self.prepare_clear_descriptor(command_buffer_handle, &buffer.resource, &description);
			}
		}
		self.flush_pending_clear_descriptor_copies(command_buffer_handle);

		for &buffer_handle in buffer_handles {
			let batched = gpu_clear_buffers.iter().any(|(handle, _)| *handle == buffer_handle);
			self.record_buffer_clear(command_buffer_handle, buffer_handle, sequence_index, !batched);
		}
	}

	/// Returns whether a buffer clear must update CPU-visible shadow storage.
	pub(crate) fn buffer_needs_cpu_shadow_clear(&self, buffer_handle: BaseBufferHandle) -> bool {
		self.buffer(buffer_handle)
			.map(|buffer| buffer.access.intersects(DeviceAccesses::CpuRead | DeviceAccesses::CpuWrite))
			.unwrap_or(false)
	}

	pub(crate) fn clear_buffer_shadow(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) {
		let Some((data, size)) = self.buffer_storage_parts_mut_for_sequence(buffer_handle, sequence_index) else {
			return;
		};
		if size == 0 {
			return;
		}

		unsafe {
			std::ptr::write_bytes(data, 0, size);
		}
		self.sync_buffer_for_sequence(buffer_handle, sequence_index);
	}

	pub(crate) fn copy_buffer_shadow(&mut self, copy: &crate::BufferCopyDescriptor, sequence_index: u8) {
		// Resolve handles through `buffer` instead of indexing storage directly. Dynamic buffer handles carry
		// `DYNAMIC_BUFFER_HANDLE_FLAG`, so the raw handle value is not always a valid index into `buffers`.
		let Some(source) = self.buffer_storage_parts_for_sequence(copy.source_buffer, sequence_index) else {
			return;
		};
		let Some(destination) = self.buffer_storage_parts_mut_for_sequence(copy.destination_buffer, sequence_index) else {
			return;
		};

		let source_end = copy.source_offset.saturating_add(copy.size);
		let destination_end = copy.destination_offset.saturating_add(copy.size);
		if source_end > source.1 || destination_end > destination.1 {
			panic!(
				"Failed to copy DX12 buffer data from {:?} offset {} to {:?} offset {} for {} bytes. The most likely cause is that the requested source or destination range is outside the buffer allocation. Source size: {} bytes. Destination size: {} bytes.",
				copy.source_buffer,
				copy.source_offset,
				copy.destination_buffer,
				copy.destination_offset,
				copy.size,
				source.1,
				destination.1
			);
		}
		if copy.size == 0 {
			return;
		}

		unsafe {
			let source = source.0.add(copy.source_offset);
			let destination = destination.0.add(copy.destination_offset);
			std::ptr::copy(source, destination, copy.size);
		}
		self.sync_buffer_for_sequence(copy.destination_buffer, sequence_index);
	}

	pub(crate) fn record_buffer_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferCopyDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(source) = self.copy_buffer_info_for_sequence(copy.source_buffer, sequence_index) else {
			return;
		};
		let Some(destination) = self.copy_buffer_info_for_sequence(copy.destination_buffer, sequence_index) else {
			return;
		};
		if destination.access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		let source_end = copy.source_offset.saturating_add(copy.size);
		let destination_end = copy.destination_offset.saturating_add(copy.size);
		if source_end > source.size || destination_end > destination.size {
			panic!(
				"Failed to record DX12 buffer copy from {:?} offset {} to {:?} offset {} for {} bytes. The most likely cause is that the requested source or destination range is outside the GPU buffer allocation. Source size: {} bytes. Destination size: {} bytes.",
				copy.source_buffer,
				copy.source_offset,
				copy.destination_buffer,
				copy.destination_offset,
				copy.size,
				source.size,
				destination.size
			);
		}

		self.transition_tracked_buffer(
			&command_list,
			copy.source_buffer,
			&source.resource,
			BufferBarrierState::COPY_SOURCE,
		);
		self.transition_tracked_buffer(
			&command_list,
			copy.destination_buffer,
			&destination.resource,
			BufferBarrierState::COPY_DESTINATION,
		);
		unsafe {
			command_list.CopyBufferRegion(
				&destination.resource,
				copy.destination_offset as u64,
				&source.resource,
				copy.source_offset as u64,
				copy.size as u64,
			);
		}
		self.transition_tracked_buffer(
			&command_list,
			copy.destination_buffer,
			&destination.resource,
			BufferBarrierState::COMMON,
		);
		self.transition_tracked_buffer(
			&command_list,
			copy.source_buffer,
			&source.resource,
			BufferBarrierState::COMMON,
		);
		self.mark_command_buffer_work(command_buffer_handle);
		self.buffer_copy_count += 1;
	}

	pub(crate) fn record_buffer_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
		transition_before_clear: bool,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(destination_buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
			return;
		};
		let destination_size = destination_buffer.size;
		let destination_access = destination_buffer.access;
		let destination_heap_kind = destination_buffer.heap_kind;
		let destination = destination_buffer.resource;
		if destination_size == 0 || destination_access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}
		if destination_access.intersects(DeviceAccesses::GpuWrite)
			&& destination_heap_kind == BufferHeapKind::Default
			&& destination_size % std::mem::size_of::<u32>() == 0
		{
			// Default-heap GPU-writable buffers use descriptors staged together by `clear_buffers`.
			let Some(descriptor) = self.take_prepared_clear_descriptor(command_buffer_handle, &destination) else {
				return;
			};

			if transition_before_clear {
				self.transition_tracked_buffer(
					&command_list,
					buffer_handle,
					&destination,
					BufferBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
				);
			}
			unsafe {
				self.bind_active_staged_descriptor_heaps(command_buffer_handle);
				command_list.ClearUnorderedAccessViewUint(descriptor.gpu, descriptor.cpu, &destination, &[0, 0, 0, 0], &[]);
			}
			self.mark_command_buffer_work(command_buffer_handle);
			self.buffer_clear_count += 1;
			return;
		}
		let (Some(upload), mapped, _) = self.create_buffer_resource(destination_size, DeviceAccesses::HostToDevice) else {
			return;
		};
		if mapped.is_null() {
			return;
		}

		self.transition_tracked_buffer(
			&command_list,
			buffer_handle,
			&destination,
			BufferBarrierState::COPY_DESTINATION,
		);
		unsafe {
			std::ptr::write_bytes(mapped, 0, destination_size);
			command_list.CopyBufferRegion(&destination, 0, &upload, 0, destination_size as u64);
		}
		self.transition_tracked_buffer(&command_list, buffer_handle, &destination, BufferBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		self.buffer_clear_count += 1;
	}

	pub(crate) fn copy_buffer_info_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<BufferCopyInfo> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let resource = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let heap_kind = self.buffer_heap_kind_for_sequence(buffer_handle, sequence_index)?;
		let buffer = self.buffer(buffer_handle)?;
		Some(BufferCopyInfo {
			resource,
			access: buffer.access,
			heap_kind,
			size: buffer.size,
		})
	}
}
