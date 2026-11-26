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

	/// Zeroes each buffer. The transitions of the buffers that clear through an unordered-access view go in one batch
	/// before the clears.
	pub(crate) fn clear_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handles: &[BaseBufferHandle],
		sequence_index: u8,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let buffers = buffer_handles
			.iter()
			.filter_map(|&buffer_handle| self.copy_buffer_info_for_sequence(buffer_handle, sequence_index))
			.collect::<SmallVec<[BufferCopyInfo; 16]>>();
		let mut clear_barriers = EnhancedBarrierBatch::default();
		for (index, buffer) in buffers.iter().enumerate() {
			// A buffer listed twice transitions once.
			if Self::clears_as_unordered_access(buffer.size)
				&& !buffers[..index].iter().any(|earlier| earlier.resource == buffer.resource)
			{
				self.transition_tracked_buffer_into(
					&buffer.resource,
					BufferBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
					true,
					&mut clear_barriers,
				);
			}
		}
		clear_barriers.submit(&command_list);

		for buffer in buffers {
			self.record_buffer_clear(command_buffer_handle, &command_list, buffer);
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
		let command_list = self.command_list(command_buffer_handle);
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
	}

	/// Zeroes one buffer: through an unordered-access view, which `clear_buffers` transitioned, when it holds whole
	/// 32-bit words, and otherwise by copying from zeroed upload memory.
	fn record_buffer_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		buffer: BufferCopyInfo,
	) {
		let BufferCopyInfo { resource, size } = buffer;
		if Self::clears_as_unordered_access(size) {
			let (gpu, cpu) =
				self.clear_uav_descriptors(command_buffer_handle, &resource, &Self::raw_buffer_clear_uav_desc(size));
			unsafe { command_list.ClearUnorderedAccessViewUint(gpu, cpu, &resource, &[0, 0, 0, 0], &[]) };
		} else {
			// D3D12 zeroes the committed resource, so the copy source needs no write.
			let (Some(upload), ..) = self.create_buffer_resource(size, crate::HostAccess::Write) else {
				return;
			};
			self.transition_tracked_buffer(command_list, &resource, BufferBarrierState::COPY_DESTINATION);
			unsafe { command_list.CopyBufferRegion(&resource, 0, &upload, 0, size as u64) };
			self.transition_tracked_buffer(command_list, &resource, BufferBarrierState::COMMON);
			self.retain_command_buffer_resource(command_buffer_handle, &upload);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	pub(crate) fn copy_buffer_info_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<BufferCopyInfo> {
		let resource = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let buffer = self.buffer(buffer_handle);
		Some(BufferCopyInfo {
			resource,
			size: buffer.size,
		})
	}
}
