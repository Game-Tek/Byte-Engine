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
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};

		if let Some(layout) = self.pipeline_layouts.get(pipeline.layout.0 as usize) {
			unsafe {
				match pipeline.kind {
					PipelineKind::Compute | PipelineKind::RayTracing => {
						command_list.SetComputeRootSignature(&layout.root_signature)
					}
					PipelineKind::Raster => command_list.SetGraphicsRootSignature(&layout.root_signature),
				}
			}
			self.counters.root_signature_bind_count += 1;
		}
		if let Some(pipeline_state) = &pipeline.pipeline_state {
			unsafe { command_list.SetPipelineState(pipeline_state) };
			self.counters.pipeline_state_bind_count += 1;
		}
		if let Some(state_object) = &pipeline.ray_tracing_state_object {
			if let Ok(command_list) = command_list.cast::<ID3D12GraphicsCommandList4>() {
				unsafe { command_list.SetPipelineState1(state_object) };
				self.counters.pipeline_state_bind_count += 1;
			}
		}
		if matches!(pipeline.kind, PipelineKind::Raster) {
			unsafe { command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
			self.counters.primitive_topology_set_count += 1;
		}
	}
}
