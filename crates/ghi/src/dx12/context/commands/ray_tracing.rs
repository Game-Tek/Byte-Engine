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
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
			StartAddress: self.shader_table_address(&range, sequence_index)?,
			SizeInBytes: range.size as u64,
			StrideInBytes: range.stride as u64,
		})
	}

	pub(crate) fn shader_table_address(&mut self, range: &BufferStridedRange, sequence_index: u8) -> Option<u64> {
		let address = self.buffer_address_for_sequence(range.buffer_offset.buffer, sequence_index);
		if address == 0 {
			return None;
		}
		Some(address + range.buffer_offset.offset as u64)
	}
}
