use super::super::*;

impl Device {
	pub(crate) fn dispatch_compute_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		dispatch: DispatchExtent,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::Compute) || pipeline.pipeline_state.is_none() {
			return;
		}
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let extent = dispatch.get_extent();
		unsafe {
			command_list.Dispatch(extent.width(), extent.height(), extent.depth());
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.compute_dispatch_encode_count += 1;
	}

	/// Encodes a native DX12 indirect compute dispatch command.
	pub(crate) fn dispatch_compute_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		entry_index: usize,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let buffer_size = {
			let Some(buffer) = self.buffer(base_buffer_handle) else {
				return;
			};
			buffer.size
		};
		assert!(
			entry_index < N,
			"DX12 indirect dispatch entry is out of bounds. The most likely cause is that entry_index exceeds the typed indirect buffer length. entry_index={entry_index}, entry_count={N}",
		);
		let argument_size = std::mem::size_of::<[u32; 3]>();
		let argument_offset = entry_index.checked_mul(argument_size).expect(
			"DX12 indirect dispatch offset overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		let argument_end = argument_offset.checked_add(argument_size).expect(
			"DX12 indirect dispatch range overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		assert!(
			argument_end <= buffer_size,
			"DX12 indirect dispatch entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={argument_end}, buffer_size={}",
			buffer_size,
		);
		let argument_offset = u64::try_from(argument_offset).expect(
			"DX12 indirect dispatch offset exceeds the native address range. The most likely cause is that the host address space is wider than DX12 GPU offsets.",
		);
		let Some(resource) = self.buffer_resource_for_sequence(base_buffer_handle, sequence_index) else {
			return;
		};
		let Some(command_signature) = self.indirect_dispatch_command_signature() else {
			return;
		};

		// A 12-byte dispatch record keeps every selected offset on DX12's required four-byte boundary.
		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				base_buffer_handle,
				&resource,
				BufferBarrierState::INDIRECT_ARGUMENT,
			);
			command_list.ExecuteIndirect(&command_signature, 1, &resource, argument_offset, None, 0);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.indirect_dispatch_encode_count += 1;
	}

	pub(crate) fn indirect_dispatch_command_signature(&mut self) -> Option<ID3D12CommandSignature> {
		if let Some(command_signature) = self.indirect_dispatch_signature.clone() {
			return Some(command_signature);
		}

		let argument = D3D12_INDIRECT_ARGUMENT_DESC {
			Type: D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
			Anonymous: D3D12_INDIRECT_ARGUMENT_DESC_0::default(),
		};
		let description = D3D12_COMMAND_SIGNATURE_DESC {
			ByteStride: std::mem::size_of::<[u32; 3]>() as u32,
			NumArgumentDescs: 1,
			pArgumentDescs: &argument,
			NodeMask: 0,
		};
		let mut command_signature: Option<ID3D12CommandSignature> = None;
		unsafe {
			self.device
				.CreateCommandSignature(&description, None, &mut command_signature)
				.ok()?;
		}
		let command_signature = command_signature?;
		self.indirect_dispatch_signature = Some(command_signature.clone());
		Some(command_signature)
	}
}
