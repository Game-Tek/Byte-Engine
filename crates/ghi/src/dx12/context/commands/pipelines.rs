use super::super::*;

impl Device {
	/// Binds a pipeline's root signature, its pipeline state or ray-tracing state object, and its raster topology.
	pub(crate) fn bind_pipeline_native_state(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.index() as usize) else {
			return;
		};

		if let Some(layout) = self.pipeline_layouts.get(pipeline.layout.index() as usize) {
			unsafe {
				match pipeline.kind {
					PipelineKind::Compute | PipelineKind::RayTracing => {
						command_list.SetComputeRootSignature(&layout.root_signature)
					}
					PipelineKind::Raster => command_list.SetGraphicsRootSignature(&layout.root_signature),
				}
			}
			self.native_counters.root_signature_bind_count += 1;
		}
		if let Some(pipeline_state) = &pipeline.pipeline_state {
			unsafe { command_list.SetPipelineState(pipeline_state) };
			self.native_counters.pipeline_state_bind_count += 1;
		}
		if let Some(ray_tracing) = &pipeline.ray_tracing {
			unsafe { command_list.SetPipelineState1(&ray_tracing.state_object) };
			self.native_counters.pipeline_state_bind_count += 1;
		}
		if matches!(pipeline.kind, PipelineKind::Raster) {
			unsafe { command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
			self.native_counters.primitive_topology_set_count += 1;
			// D3D12 reads vertex strides from the views, so vertex buffers bound before the pipeline take its strides now.
			let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
			command_buffer.vertex_strides.clone_from(&pipeline.vertex_strides);
			let mut restrided = false;
			for (slot, view) in command_buffer.vertex_buffer_views.iter_mut().enumerate() {
				let stride = pipeline.vertex_strides.get(slot).copied().unwrap_or(0);
				restrided |= view.StrideInBytes != stride;
				view.StrideInBytes = stride;
			}
			if restrided {
				unsafe { command_list.IASetVertexBuffers(0, Some(&command_buffer.vertex_buffer_views)) };
			}
		}
	}
}
