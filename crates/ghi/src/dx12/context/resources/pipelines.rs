use super::super::*;

impl Device {
	pub(crate) fn pipeline_has_native_state(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.pipeline_state.is_some())
	}

	pub(crate) fn pipeline_state_bind_count(&self) -> usize {
		self.pipeline_state_bind_count
	}

	pub(crate) fn compute_pipeline_state_create_attempt_count(&self) -> usize {
		self.compute_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_create_attempt_count(&self) -> usize {
		self.graphics_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_last_error(&self) -> Option<i32> {
		self.graphics_pipeline_state_last_error
	}

	pub(crate) fn hlsl_specialization_compile_count(&self) -> usize {
		self.hlsl_specialization_compile_count
	}

	pub(crate) fn ray_tracing_state_object_create_attempt_count(&self) -> usize {
		self.ray_tracing_state_object_create_attempt_count
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

	pub(crate) fn supports_native_ray_tracing(&self) -> bool {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
		let result = unsafe {
			self.device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS5,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS5).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS5>() as u32,
			)
		};
		result.is_ok() && options.RaytracingTier != D3D12_RAYTRACING_TIER_NOT_SUPPORTED
	}

	pub(crate) fn supports_native_mesh_shaders(&self) -> bool {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS7::default();
		let result = unsafe {
			self.device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS7,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS7).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS7>() as u32,
			)
		};
		result.is_ok() && options.MeshShaderTier != D3D12_MESH_SHADER_TIER_NOT_SUPPORTED
	}

	pub(crate) fn compute_dispatch_encode_count(&self) -> usize {
		self.compute_dispatch_encode_count
	}

	pub(crate) fn indirect_dispatch_encode_count(&self) -> usize {
		self.indirect_dispatch_encode_count
	}

	pub(crate) fn trace_rays_record_count(&self) -> usize {
		self.trace_rays_record_count
	}

	pub(crate) fn mesh_dispatch_encode_count(&self) -> usize {
		self.mesh_dispatch_encode_count
	}

	pub(crate) fn vertex_buffer_bind_count(&self) -> usize {
		self.vertex_buffer_bind_count
	}

	pub(crate) fn index_buffer_bind_count(&self) -> usize {
		self.index_buffer_bind_count
	}

	pub(crate) fn draw_encode_count(&self) -> usize {
		self.draw_encode_count
	}

	pub(crate) fn draw_indexed_encode_count(&self) -> usize {
		self.draw_indexed_encode_count
	}

	pub(crate) fn render_target_bind_count(&self) -> usize {
		self.render_target_bind_count
	}

	pub(crate) fn render_target_clear_count(&self) -> usize {
		self.render_target_clear_count
	}

	pub(crate) fn render_pass_end_count(&self) -> usize {
		self.render_pass_end_count
	}

	pub(crate) fn depth_stencil_bind_count(&self) -> usize {
		self.depth_stencil_bind_count
	}

	pub(crate) fn depth_stencil_clear_count(&self) -> usize {
		self.depth_stencil_clear_count
	}

	pub(crate) fn viewport_set_count(&self) -> usize {
		self.viewport_set_count
	}

	pub(crate) fn scissor_set_count(&self) -> usize {
		self.scissor_set_count
	}

	pub(crate) fn primitive_topology_set_count(&self) -> usize {
		self.primitive_topology_set_count
	}

	pub(crate) fn swapchain_backbuffer_bind_count(&self) -> usize {
		self.swapchain_backbuffer_bind_count
	}

	pub(crate) fn swapchain_present_transition_count(&self) -> usize {
		self.swapchain_present_transition_count
	}

	pub(crate) fn uav_barrier_count(&self) -> usize {
		self.uav_barrier_count
	}
}
