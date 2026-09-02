use super::super::*;

impl Device {
	/// Records DX12 ray dispatch metadata from GHI shader binding table ranges.
	pub(crate) fn trace_rays_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		binding_tables: crate::rt::BindingTables,
		x: u32,
		y: u32,
		z: u32,
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::RayTracing) {
			return;
		}
		let state_object = pipeline.ray_tracing_state_object.clone();
		if self.command_buffers.get(command_buffer_handle.0 as usize).is_none() {
			return;
		}
		let Some(raygen) = self.ray_generation_shader_record(binding_tables.raygen, sequence_index) else {
			return;
		};
		let Some(miss) = self.shader_table_range(binding_tables.miss, sequence_index) else {
			return;
		};
		let Some(hit) = self.shader_table_range(binding_tables.hit, sequence_index) else {
			return;
		};
		let callable = if let Some(callable) = binding_tables.callable {
			let Some(callable) = self.shader_table_range(callable, sequence_index) else {
				return;
			};
			callable
		} else {
			D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::default()
		};

		let _desc = D3D12_DISPATCH_RAYS_DESC {
			RayGenerationShaderRecord: raygen,
			MissShaderTable: miss,
			HitGroupTable: hit,
			CallableShaderTable: callable,
			Width: x,
			Height: y,
			Depth: z,
		};
		if state_object.is_some() {
			if let Some(command_list) = self
				.command_buffers
				.get(command_buffer_handle.0 as usize)
				.and_then(|command_buffer| command_buffer.command_list.clone())
				.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList4>().ok())
			{
				unsafe {
					command_list.DispatchRays(&_desc);
				}
				self.mark_command_buffer_work(command_buffer_handle);
			}
		}
		self.trace_rays_record_count += 1;
	}

	pub(crate) fn ray_generation_shader_record(
		&mut self,
		range: BufferStridedRange,
		sequence_index: u8,
	) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE> {
		assert!(
			range.size != 0
				&& range.size.is_multiple_of(
					windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as usize
				) && range.size <= windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_MAX_SHADER_RECORD_STRIDE as usize,
			"Invalid DX12 ray-generation shader record size. The most likely cause is that the shader binding table record is empty, not 32-byte aligned, or larger than 4096 bytes.",
		);
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
			StartAddress: self.shader_table_address(&range, sequence_index)?,
			SizeInBytes: range.size as u64,
		})
	}

	pub(crate) fn shader_table_range(
		&mut self,
		range: BufferStridedRange,
		sequence_index: u8,
	) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE> {
		assert!(
			range.size == 0
				|| (range.stride != 0
					&& range.stride.is_multiple_of(
						windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as usize,
					) && range.stride
					<= windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_MAX_SHADER_RECORD_STRIDE as usize
					&& range.size.is_multiple_of(range.stride)),
			"Invalid DX12 shader table stride or size. The most likely cause is that a nonempty table does not contain complete 32-byte-aligned records with a stride no larger than 4096 bytes.",
		);
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
			StartAddress: self.shader_table_address(&range, sequence_index)?,
			SizeInBytes: range.size as u64,
			StrideInBytes: range.stride as u64,
		})
	}

	pub(crate) fn shader_table_address(&mut self, range: &BufferStridedRange, sequence_index: u8) -> Option<u64> {
		let buffer_size = self.buffer(range.buffer_offset.buffer)?.size;
		let heap_kind = self.buffer_heap_kind_for_sequence(range.buffer_offset.buffer, sequence_index)?;
		Self::validate_buffer_heap_access(heap_kind, BufferBarrierState::ACCELERATION_STRUCTURE_INPUT);
		let end = range.buffer_offset.offset.checked_add(range.size).expect(
			"DX12 shader table range overflowed. The most likely cause is that its offset and size came from invalid shader binding table metadata.",
		);
		assert!(
			end <= buffer_size,
			"DX12 shader table range exceeds the buffer. The most likely cause is that its offset or size came from stale shader binding table metadata. range_end={end}, buffer_size={buffer_size}",
		);
		let address = self.buffer_address_for_sequence(range.buffer_offset.buffer, sequence_index);
		if address == 0 {
			return None;
		}
		let address = address.checked_add(range.buffer_offset.offset as u64).expect(
			"DX12 shader table address overflowed. The most likely cause is an invalid native resource address or offset.",
		);
		assert!(
			address.is_multiple_of(windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as u64,),
			"DX12 shader table address is misaligned. The most likely cause is that the table offset is not a multiple of 64 bytes.",
		);
		Some(address)
	}
}
