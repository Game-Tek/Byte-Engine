use super::super::*;

impl Device {
	/// Queues one retained clear descriptor for a batched copy before command-list submission.
	pub(crate) fn queue_clear_descriptor_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		destination: D3D12_CPU_DESCRIPTOR_HANDLE,
		source: D3D12_CPU_DESCRIPTOR_HANDLE,
	) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		command_buffer
			.pending_clear_descriptor_copies
			.push(PendingDescriptorCopy { destination, source });
	}

	/// Reserves and queues one shader-visible descriptor for a later clear in the current batch.
	pub(crate) fn prepare_clear_descriptor(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		description: &D3D12_UNORDERED_ACCESS_VIEW_DESC,
	) -> bool {
		let Some((heap, descriptor_offset)) = self.reserve_staged_descriptor_range(command_buffer_handle, false, 1) else {
			return false;
		};
		let Some(cpu_descriptor) = self.retained_clear_uav_descriptor(resource, description) else {
			return false;
		};
		let destination = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
		let source = self.descriptor_cpu_handle(
			&cpu_descriptor.heap,
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
			cpu_descriptor.slot,
		);
		let gpu = self.descriptor_gpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
		self.queue_clear_descriptor_copy(command_buffer_handle, destination, source);
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return false;
		};
		command_buffer.prepared_clear_descriptors.push(PreparedClearDescriptor {
			resource: Self::native_resource_key(resource),
			cpu: source,
			gpu,
		});
		true
	}

	/// Removes the descriptor prepared for the next clear of this native resource.
	pub(crate) fn take_prepared_clear_descriptor(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
	) -> Option<PreparedClearDescriptor> {
		let command_buffer = self.command_buffers.get_mut(command_buffer_handle.0 as usize)?;
		let resource = Self::native_resource_key(resource);
		let index = command_buffer
			.prepared_clear_descriptors
			.iter()
			.position(|descriptor| descriptor.resource == resource)?;
		Some(command_buffer.prepared_clear_descriptors.remove(index))
	}

	/// Copies queued clear descriptors, combining adjacent source and destination slots into one native call.
	pub(crate) fn flush_pending_clear_descriptor_copies(&mut self, command_buffer_handle: CommandBufferHandle) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		let copies = &mut command_buffer.pending_clear_descriptor_copies;
		if copies.is_empty() {
			return;
		}

		let increment = self.descriptor_handle_increment_sizes[0] as usize;
		let mut first = 0usize;
		while first < copies.len() {
			let mut end = first + 1;
			while end < copies.len()
				&& copies[end].destination.ptr == copies[end - 1].destination.ptr.saturating_add(increment)
				&& copies[end].source.ptr == copies[end - 1].source.ptr.saturating_add(increment)
			{
				end += 1;
			}
			unsafe {
				self.device.CopyDescriptorsSimple(
					(end - first) as u32,
					copies[first].destination,
					copies[first].source,
					D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
				);
			}
			self.clear_descriptor_copy_call_count += 1;
			first = end;
		}
		copies.clear();
	}
}
