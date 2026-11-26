use super::super::*;

impl Device {
	pub(crate) fn dispatch_compute_native(&mut self, command_buffer_handle: CommandBufferHandle, dispatch: DispatchExtent) {
		let command_list = self.command_list(command_buffer_handle);
		let extent = dispatch.get_extent();
		unsafe {
			command_list.Dispatch(extent.width(), extent.height(), extent.depth());
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Encodes a native DX12 indirect compute dispatch command.
	pub(crate) fn dispatch_compute_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		entry_index: usize,
		sequence_index: u8,
	) {
		self.execute_indirect_native(
			command_buffer_handle,
			base_buffer_handle,
			sequence_index,
			D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
			crate::command_buffer::indirect_entry_range::<[u32; 3], N>(entry_index),
		);
	}

	/// Encodes one `ExecuteIndirect` of the `entry` byte range of a GPU buffer. Each argument type caches its own
	/// command signature, whose stride is one record.
	pub(crate) fn execute_indirect_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		sequence_index: u8,
		argument_type: D3D12_INDIRECT_ARGUMENT_TYPE,
		entry: std::ops::Range<usize>,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let buffer_size = self.buffer(base_buffer_handle).size;
		assert!(
			entry.end <= buffer_size,
			"DX12 indirect entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={}, buffer_size={buffer_size}",
			entry.end,
		);
		let resource = self.buffer_resource_for_sequence(base_buffer_handle, sequence_index).expect(
			"Missing DX12 indirect argument buffer. The most likely cause is that the buffer is empty. Build indirect buffers with at least one record.",
		);
		let command_signature = if argument_type == D3D12_INDIRECT_ARGUMENT_TYPE_DRAW {
			&mut self.indirect_draw_signature
		} else if argument_type == D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH_MESH {
			&mut self.indirect_mesh_dispatch_signature
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
			unsafe { self.device.CreateCommandSignature(&description, None, command_signature) }.expect(
				"Failed to create a DX12 command signature. The most likely cause is that the device was removed or is out of memory.",
			);
		}
		let command_signature = command_signature.clone().expect("The command signature was created above.");

		// Draw and dispatch records are 16 and 12 bytes, so every selected offset is on DX12's required four-byte boundary.
		unsafe {
			self.transition_tracked_buffer(&command_list, &resource, BufferBarrierState::INDIRECT_ARGUMENT);
			command_list.ExecuteIndirect(&command_signature, 1, &resource, entry.start as u64, None, 0);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}
}
