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
		let bound_pipeline = self.bound_pipeline.expect(
			"Vulkan trace_rays has no pipeline bound. The most likely cause is that trace_rays was called before bind_ray_tracing_pipeline. Bind the ray tracing pipeline first.",
		);
		let shader_binding_table = self.device.shader_binding_tables.get(table.0 as usize).expect(
			"Vulkan trace_rays failed because the shader binding table handle names no table of this context. The most likely cause is that the handle came from another context. Trace with tables the recording's context created.",
		);
		let Some(written) = &shader_binding_table.written else {
			panic!(
				"Vulkan trace_rays failed because shader binding table {:?} has never been written. The most likely cause is that the table was traced before write_shader_binding_table gave it records. Write the table for the bound pipeline before tracing with it.",
				shader_binding_table.name,
			);
		};
		assert!(
			written.pipeline == bound_pipeline,
			"Vulkan trace_rays failed because shader binding table {:?} was last written for pipeline {:?}, and pipeline {bound_pipeline:?} is bound. The most likely cause is that the pipeline was created again, with more hit groups, and the table wasn't written for the new one. Write the table for the bound pipeline before tracing with it.",
			shader_binding_table.name,
			written.pipeline,
		);
		let (ray_generation, callable) = (written.ray_generation, written.callable);

		// A height or depth of 0 counts as 1, as one- and two-dimensional extents leave them, and a width of 0 traces
		// nothing.
		if extent.width() == 0 {
			return;
		}
		let (width, height, depth) = (extent.width(), extent.height().max(1), extent.depth().max(1));
		let invocations = u64::from(width) * u64::from(height) * u64::from(depth);
		let limit = self.device.ray_tracing("trace rays").max_ray_dispatch_invocation_count;
		assert!(
			invocations <= u64::from(limit),
			"Vulkan trace_rays launches more invocations than the device allows. The most likely cause is a launch sized for more rays than one trace can run. invocations={invocations}, limit={limit}. Split the launch into several traces.",
		);

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
