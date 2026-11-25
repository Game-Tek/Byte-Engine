use super::*;

impl Device {
	/// Records buffer-to-buffer copies on the GPU.
	pub(crate) fn copy_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			self.record_buffer_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	pub(crate) fn clear_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handles: &[BaseBufferHandle],
		sequence_index: u8,
	) {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let mut gpu_clear_buffers = SmallVec::<[(BaseBufferHandle, ID3D12Resource); 16]>::new();
		let mut clear_barriers = EnhancedBarrierBatch::default();
		for &buffer_handle in buffer_handles {
			let Some(buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
				continue;
			};
			if Self::clears_as_unordered_access(buffer.size) {
				if gpu_clear_buffers.iter().any(|(handle, _)| *handle == buffer_handle) {
					continue;
				}
				self.transition_tracked_buffer_into(
					&buffer.resource,
					BufferBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
					&mut clear_barriers,
				);
				gpu_clear_buffers.push((buffer_handle, buffer.resource));
			}
		}
		clear_barriers.submit(&command_list);

		for &buffer_handle in buffer_handles {
			let Some(buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
				continue;
			};
			if Self::clears_as_unordered_access(buffer.size) {
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

	/// Returns whether a buffer of `size` bytes clears through an unordered-access view, which clears whole 32-bit
	/// words. Other buffers clear by copying from zeroed memory.
	fn clears_as_unordered_access(size: usize) -> bool {
		size != 0 && size % std::mem::size_of::<u32>() == 0
	}

	pub(crate) fn record_buffer_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferCopyDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let Some(source) = self.copy_buffer_info_for_sequence(copy.source_buffer, sequence_index) else {
			return;
		};
		let Some(destination) = self.copy_buffer_info_for_sequence(copy.destination_buffer, sequence_index) else {
			return;
		};
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

		self.transition_tracked_buffer(&command_list, &source.resource, BufferBarrierState::COPY_SOURCE);
		self.transition_tracked_buffer(&command_list, &destination.resource, BufferBarrierState::COPY_DESTINATION);
		unsafe {
			command_list.CopyBufferRegion(
				&destination.resource,
				copy.destination_offset as u64,
				&source.resource,
				copy.source_offset as u64,
				copy.size as u64,
			);
		}
		self.transition_tracked_buffer(&command_list, &destination.resource, BufferBarrierState::COMMON);
		self.transition_tracked_buffer(&command_list, &source.resource, BufferBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
		self.native_counters.buffer_copy_count += 1;
	}

	pub(crate) fn record_buffer_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
		transition_before_clear: bool,
	) {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let Some(destination_buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
			return;
		};
		let destination_size = destination_buffer.size;
		let destination = destination_buffer.resource;
		if destination_size == 0 {
			return;
		}
		if Self::clears_as_unordered_access(destination_size) {
			// Shader-writable buffers use descriptors staged together by `clear_buffers`.
			let Some(descriptor) = self.take_prepared_clear_descriptor(command_buffer_handle, &destination) else {
				return;
			};

			if transition_before_clear {
				self.transition_tracked_buffer(
					&command_list,
					&destination,
					BufferBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
				);
			}
			unsafe {
				command_list.ClearUnorderedAccessViewUint(descriptor.gpu, descriptor.cpu, &destination, &[0, 0, 0, 0], &[]);
			}
			self.mark_command_buffer_work(command_buffer_handle);
			self.native_counters.buffer_clear_count += 1;
			return;
		}
		// D3D12 zeroes the committed resource, so the copy source needs no write.
		let (Some(upload), ..) = self.create_buffer_resource(destination_size, crate::HostAccess::Write) else {
			return;
		};
		self.transition_tracked_buffer(&command_list, &destination, BufferBarrierState::COPY_DESTINATION);
		unsafe {
			command_list.CopyBufferRegion(&destination, 0, &upload, 0, destination_size as u64);
		}
		self.transition_tracked_buffer(&command_list, &destination, BufferBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		self.native_counters.buffer_clear_count += 1;
	}

	pub(crate) fn copy_buffer_info_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<BufferCopyInfo> {
		let resource = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let buffer = self.buffer(buffer_handle)?;
		Some(BufferCopyInfo {
			resource,
			size: buffer.size,
		})
	}
}
