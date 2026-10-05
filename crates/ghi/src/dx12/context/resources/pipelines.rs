use super::super::*;

impl Device {
	pub(crate) fn pipeline_has_native_state(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.pipeline_state.is_some())
	}

	pub(crate) fn pipeline_state_bind_count(&self) -> usize {
		self.counters.pipeline_state_bind_count
	}

	pub(crate) fn compute_pipeline_state_create_attempt_count(&self) -> usize {
		self.counters.compute_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_create_attempt_count(&self) -> usize {
		self.counters.graphics_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_last_error(&self) -> Option<i32> {
		self.counters.graphics_pipeline_state_last_error
	}

	pub(crate) fn hlsl_specialization_compile_count(&self) -> usize {
		self.counters.hlsl_specialization_compile_count
	}

	pub(crate) fn ray_tracing_state_object_create_attempt_count(&self) -> usize {
		self.counters.ray_tracing_state_object_create_attempt_count
	}

	pub(crate) fn pipeline_has_ray_tracing_state_object(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.ray_tracing_state_object.is_some())
	}

	pub(crate) fn ray_tracing_shader_identifier_count(&self, pipeline: PipelineHandle) -> Option<usize> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.ray_tracing_shader_identifiers.len())
	}

	pub(crate) fn compute_dispatch_encode_count(&self) -> usize {
		self.counters.compute_dispatch_encode_count
	}

	pub(crate) fn indirect_dispatch_encode_count(&self) -> usize {
		self.counters.indirect_dispatch_encode_count
	}

	pub(crate) fn trace_rays_record_count(&self) -> usize {
		self.counters.trace_rays_record_count
	}

	pub(crate) fn mesh_dispatch_encode_count(&self) -> usize {
		self.counters.mesh_dispatch_encode_count
	}

	pub(crate) fn vertex_buffer_bind_count(&self) -> usize {
		self.counters.vertex_buffer_bind_count
	}

	pub(crate) fn index_buffer_bind_count(&self) -> usize {
		self.counters.index_buffer_bind_count
	}

	pub(crate) fn draw_encode_count(&self) -> usize {
		self.counters.draw_encode_count
	}

	pub(crate) fn draw_indexed_encode_count(&self) -> usize {
		self.counters.draw_indexed_encode_count
	}

	pub(crate) fn render_target_bind_count(&self) -> usize {
		self.counters.render_target_bind_count
	}

	pub(crate) fn render_target_clear_count(&self) -> usize {
		self.counters.render_target_clear_count
	}

	pub(crate) fn render_pass_end_count(&self) -> usize {
		self.counters.render_pass_end_count
	}

	pub(crate) fn depth_stencil_bind_count(&self) -> usize {
		self.counters.depth_stencil_bind_count
	}

	pub(crate) fn depth_stencil_clear_count(&self) -> usize {
		self.counters.depth_stencil_clear_count
	}

	pub(crate) fn viewport_set_count(&self) -> usize {
		self.counters.viewport_set_count
	}

	pub(crate) fn scissor_set_count(&self) -> usize {
		self.counters.scissor_set_count
	}

	pub(crate) fn primitive_topology_set_count(&self) -> usize {
		self.counters.primitive_topology_set_count
	}

	pub(crate) fn swapchain_backbuffer_bind_count(&self) -> usize {
		self.counters.swapchain_backbuffer_bind_count
	}

	pub(crate) fn swapchain_present_transition_count(&self) -> usize {
		self.counters.swapchain_present_transition_count
	}

	pub(crate) fn uav_barrier_count(&self) -> usize {
		self.counters.uav_barrier_count
	}
}
