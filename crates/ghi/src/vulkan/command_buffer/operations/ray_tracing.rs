use super::*;

impl CommandBufferRecording<'_> {
	/// Binds a ray tracing pipeline and sets the stack its shaders need, which the pipeline leaves dynamic.
	pub(super) fn bind_ray_tracing_pipeline_internal(&mut self, pipeline_handle: graphics_hardware_interface::PipelineHandle) {
		self.record_pipeline_bind(vk::PipelineBindPoint::RAY_TRACING_KHR, pipeline_handle);
		let stack_size = self
			.device
			.ray_tracing_pipeline_state(pipeline_handle, "bind_ray_tracing_pipeline")
			.stack_size;
		let command_buffer = self.get_command_buffer().command_buffer;
		let functions = &self.device.ray_tracing("bind a ray tracing pipeline").pipeline;
		// SAFETY: The bound pipeline declares its stack size dynamic, and the size covers its deepest call.
		unsafe { functions.cmd_set_ray_tracing_pipeline_stack_size(command_buffer, stack_size) };
	}
}

impl crate::command_buffer::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	/// Launches the bound pipeline's ray generation shader once for each point of `extent`, with `table`'s callable
	/// region holding the closest hit of each hit record.
	fn trace_rays(&mut self, table: crate::ShaderBindingTableHandle, extent: ::utils::Extent) {
		let bound_pipeline = crate::pipelines::ray_tracing::bound_pipeline("Vulkan", self.bound_pipeline);
		let shader_binding_table = self
			.device
			.shader_binding_tables
			.get(table.index() as usize)
			.unwrap_or_else(|| crate::pipelines::ray_tracing::unknown_table_trace("Vulkan"));
		crate::pipelines::ray_tracing::check_table_pipeline(
			"Vulkan",
			shader_binding_table.name.as_deref(),
			shader_binding_table.written.as_ref().map(|written| written.pipeline),
			bound_pipeline,
		);
		// The check panics for a table that was never written.
		let Some(written) = &shader_binding_table.written else {
			unreachable!()
		};
		let (ray_generation, callable) = (written.ray_generation, written.callable);

		let Some(grid @ [width, height, depth]) = crate::pipelines::ray_tracing::launch_grid(extent) else {
			return;
		};
		let limit = self.device.ray_tracing("trace rays").max_ray_dispatch_invocation_count;
		crate::pipelines::ray_tracing::check_launch("Vulkan", grid, u64::from(limit));

		let command_buffer = self.prepare_shader_work();
		// Rays come from ray queries, so the trace needs no miss or hit region.
		let unused = vk::StridedDeviceAddressRegionKHR::default();
		let functions = &self.device.ray_tracing("trace rays").pipeline;
		// SAFETY: The table was written for the bound pipeline, its buffer lives until every recording that traced
		// with it completes, and the regions are aligned as the device requires.
		unsafe {
			functions.cmd_trace_rays(
				command_buffer,
				&ray_generation,
				&unused,
				&unused,
				&callable,
				width,
				height,
				depth,
			);
		}
	}
}
