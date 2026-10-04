use super::super::*;

impl Device {
	pub(crate) fn dispatch_compute_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		dispatch: DispatchExtent,
	) {
		let Some(pipeline) = pipeline_handle.and_then(|pipeline_handle| self.pipelines.get(pipeline_handle.0 as usize)) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::Compute) || pipeline.pipeline_state.is_none() {
			return;
		}
		let Some(command_list) = self.command_list(command_buffer_handle) else {
			return;
		};
		let extent = dispatch.get_extent();
		unsafe {
			command_list.Dispatch(extent.width(), extent.height(), extent.depth());
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.counters.compute_dispatch_encode_count += 1;
	}

	/// Encodes a native DX12 indirect compute dispatch command.
	pub(crate) fn dispatch_compute_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		entry_index: usize,
		sequence_index: u8,
	) {
		if self.execute_indirect_native(
			command_buffer_handle,
			base_buffer_handle,
			sequence_index,
			D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
			|| crate::command_buffer::indirect_entry_range::<[u32; 3], N>(entry_index),
		) {
			self.counters.indirect_dispatch_encode_count += 1;
		}
	}

	/// Encodes one `ExecuteIndirect` of the buffer record that `entry` returns, and returns whether it recorded work.
	///
	/// `entry` runs after the command list and buffer lookups, so a command without either records nothing and checks
	/// nothing. Each argument type caches its own command signature, whose stride is one record.
	pub(crate) fn execute_indirect_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		sequence_index: u8,
		argument_type: D3D12_INDIRECT_ARGUMENT_TYPE,
		entry: impl FnOnce() -> std::ops::Range<usize>,
	) -> bool {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return false;
		};
		let Some(buffer_size) = self.buffer(base_buffer_handle).map(|buffer| buffer.size) else {
			return false;
		};
		let entry = entry();
		assert!(
			entry.end <= buffer_size,
			"DX12 indirect entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={}, buffer_size={buffer_size}",
			entry.end,
		);
		let Some(resource) = self.buffer_resource_for_sequence(base_buffer_handle, sequence_index) else {
			return false;
		};
		let command_signature = if argument_type == D3D12_INDIRECT_ARGUMENT_TYPE_DRAW {
			&mut self.indirect_draw_signature
		} else {
			&mut self.indirect_dispatch_signature
		};
		if command_signature.is_none() {
			let argument = D3D12_INDIRECT_ARGUMENT_DESC {
				Type: argument_type,
				Anonymous: D3D12_INDIRECT_ARGUMENT_DESC_0::default(),
			};
			let description = D3D12_COMMAND_SIGNATURE_DESC {
				ByteStride: entry.len() as u32,
				NumArgumentDescs: 1,
				pArgumentDescs: &argument,
				NodeMask: 0,
			};
			// A failed creation leaves the cache empty, so the next indirect command tries again.
			if unsafe { self.device.CreateCommandSignature(&description, None, command_signature) }.is_err() {
				return false;
			}
		}
		let Some(command_signature) = command_signature.clone() else {
			return false;
		};

		// Draw and dispatch records are 16 and 12 bytes, so every selected offset is on DX12's required four-byte boundary.
		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				base_buffer_handle,
				&resource,
				BufferBarrierState::INDIRECT_ARGUMENT,
			);
			command_list.ExecuteIndirect(&command_signature, 1, &resource, entry.start as u64, None, 0);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		true
	}
}
