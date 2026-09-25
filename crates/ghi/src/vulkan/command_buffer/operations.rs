use super::*;

mod compute;
mod raster;
mod ray_tracing;
mod transfer;

/// Consumes a whole non-image resource (buffer or acceleration structure), which has no layout to transition.
fn vulkan_consumption(handle: Handles, stages: vk::PipelineStageFlags2, access: vk::AccessFlags2) -> VulkanConsumption {
	VulkanConsumption {
		handle,
		stages,
		access,
		layout: vk::ImageLayout::UNDEFINED,
		range: None,
	}
}

impl CommandBufferRecording<'_> {
	fn buffer_resource(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> Handles {
		Handles::Buffer(self.get_internal_buffer_handle(handle))
	}

	fn image_resource(&self, handle: graphics_hardware_interface::BaseImageHandle) -> Handles {
		Handles::Image(self.get_internal_base_image_handle(handle))
	}

	fn record_pipeline_bind(
		&mut self,
		bind_point: vk::PipelineBindPoint,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut Self {
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		unsafe {
			self.device
				.device
				.cmd_bind_pipeline(self.get_command_buffer().command_buffer, bind_point, pipeline.pipeline);
		}

		self.pipeline_bind_point = bind_point;
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(pipeline.layout);
		self.descriptor_materialization_dirty = true;
		self.descriptor_resources_initialized = false;
		self
	}

	/// Transitions the bound pipeline's descriptor-backed resources, then returns the command buffer to record into.
	/// Shader reads must observe earlier transfer writes even though the descriptor sets themselves are already bound.
	fn prepare_shader_work(&mut self) -> vk::CommandBuffer {
		self.consume_resources_current([]).apply(self);
		self.get_command_buffer().command_buffer
	}

	/// Like `prepare_shader_work`, but also begins the deferred render pass so the barriers stay outside rendering.
	fn prepare_draw(&mut self) -> vk::CommandBuffer {
		let command_buffer = self.prepare_shader_work();
		self.begin_rendering_if_needed();
		command_buffer
	}
}
