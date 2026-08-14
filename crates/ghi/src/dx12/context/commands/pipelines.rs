use super::super::*;

impl Device {
	pub(crate) fn bind_pipeline_root_signature(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let Some(root_signature) = self
			.pipeline_root_signatures
			.get(pipeline.layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())
		else {
			return;
		};

		unsafe {
			match pipeline.kind {
				PipelineKind::Compute | PipelineKind::RayTracing => command_list.SetComputeRootSignature(&root_signature),
				PipelineKind::Raster => command_list.SetGraphicsRootSignature(&root_signature),
			}
		}
		self.root_signature_bind_count += 1;
	}

	pub(crate) fn bind_pipeline_state(&mut self, command_buffer_handle: CommandBufferHandle, pipeline_handle: PipelineHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline_state) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.and_then(|pipeline| pipeline.pipeline_state.clone())
		else {
			return;
		};

		unsafe {
			command_list.SetPipelineState(&pipeline_state);
		}
		self.pipeline_state_bind_count += 1;
	}

	pub(crate) fn bind_pipeline_native_state(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		self.bind_pipeline_root_signature(command_buffer_handle, pipeline_handle);
		self.bind_pipeline_state(command_buffer_handle, pipeline_handle);
		self.bind_ray_tracing_state_object(command_buffer_handle, pipeline_handle);
		self.bind_primitive_topology(command_buffer_handle, pipeline_handle);
	}

	pub(crate) fn bind_ray_tracing_state_object(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		let Some(state_object) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.and_then(|pipeline| pipeline.ray_tracing_state_object.clone())
		else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
			.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList4>().ok())
		else {
			return;
		};
		unsafe {
			command_list.SetPipelineState1(&state_object);
		}
		self.pipeline_state_bind_count += 1;
	}

	pub(crate) fn bind_primitive_topology(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		let Some(Pipeline {
			kind: PipelineKind::Raster,
			..
		}) = self.pipelines.get(pipeline_handle.0 as usize)
		else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		unsafe {
			command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
		}
		self.primitive_topology_set_count += 1;
	}
}
