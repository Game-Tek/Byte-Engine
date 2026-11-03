use super::super::*;

impl Device {
	pub(crate) fn pipeline_has_native_state(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.index() as usize)
			.map(|pipeline| pipeline.pipeline_state.is_some())
	}

	pub(crate) fn pipeline_state_bind_count(&self) -> usize {
		self.native_counters.pipeline_state_bind_count
	}

	pub(crate) fn compute_pipeline_state_create_attempt_count(&self) -> usize {
		self.native_counters.compute_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_create_attempt_count(&self) -> usize {
		self.native_counters.graphics_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_last_error(&self) -> Option<i32> {
		self.native_counters.graphics_pipeline_state_last_error
	}

	pub(crate) fn hlsl_specialization_compile_count(&self) -> usize {
		self.native_counters.hlsl_specialization_compile_count
	}

	pub(crate) fn compute_dispatch_encode_count(&self) -> usize {
		self.native_counters.compute_dispatch_encode_count
	}

	pub(crate) fn indirect_dispatch_encode_count(&self) -> usize {
		self.native_counters.indirect_dispatch_encode_count
	}

	pub(crate) fn mesh_dispatch_encode_count(&self) -> usize {
		self.native_counters.mesh_dispatch_encode_count
	}

	pub(crate) fn vertex_buffer_bind_count(&self) -> usize {
		self.native_counters.vertex_buffer_bind_count
	}

	pub(crate) fn index_buffer_bind_count(&self) -> usize {
		self.native_counters.index_buffer_bind_count
	}

	pub(crate) fn draw_encode_count(&self) -> usize {
		self.native_counters.draw_encode_count
	}

	pub(crate) fn draw_indexed_encode_count(&self) -> usize {
		self.native_counters.draw_indexed_encode_count
	}

	pub(crate) fn render_target_bind_count(&self) -> usize {
		self.native_counters.render_target_bind_count
	}

	pub(crate) fn render_target_clear_count(&self) -> usize {
		self.native_counters.render_target_clear_count
	}

	pub(crate) fn render_pass_end_count(&self) -> usize {
		self.native_counters.render_pass_end_count
	}

	pub(crate) fn depth_stencil_bind_count(&self) -> usize {
		self.native_counters.depth_stencil_bind_count
	}

	pub(crate) fn depth_stencil_clear_count(&self) -> usize {
		self.native_counters.depth_stencil_clear_count
	}

	pub(crate) fn viewport_set_count(&self) -> usize {
		self.native_counters.viewport_set_count
	}

	pub(crate) fn scissor_set_count(&self) -> usize {
		self.native_counters.scissor_set_count
	}

	pub(crate) fn primitive_topology_set_count(&self) -> usize {
		self.native_counters.primitive_topology_set_count
	}

	pub(crate) fn swapchain_backbuffer_bind_count(&self) -> usize {
		self.native_counters.swapchain_backbuffer_bind_count
	}

	pub(crate) fn swapchain_present_transition_count(&self) -> usize {
		self.native_counters.swapchain_present_transition_count
	}

	pub(crate) fn uav_barrier_count(&self) -> usize {
		self.native_counters.uav_barrier_count
	}
}
