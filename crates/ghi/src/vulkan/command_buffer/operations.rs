use super::*;

mod acceleration_structures;
mod compute;
mod raster;
mod ray_tracing;
mod tile;
mod transfer;

/// Consumes a whole buffer, which has no layout to transition.
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
		let pipeline = &self.device.pipelines[pipeline_handle.index() as usize];
		unsafe {
			self.device
				.cmd_bind_pipeline(self.get_command_buffer().command_buffer, bind_point, pipeline.pipeline);
		}

		self.pipeline_bind_point = bind_point;
		self.bound_pipeline = Some(pipeline_handle);
		self.descriptor_bindings_dirty = true;
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

	/// Like `prepare_shader_work`, but also readies record `entry_index` of an indirect argument buffer for the indirect
	/// stage, and begins the deferred render pass when `draw` is set. Returns the command buffer, and the argument
	/// buffer with the record's byte offset and size.
	fn prepare_indirect<R, const N: usize>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<[R; N]>,
		entry_index: usize,
		draw: bool,
	) -> (vk::CommandBuffer, vk::Buffer, vk::DeviceSize, u32) {
		let entry = crate::command_buffer::indirect_entry_range::<R, N>(entry_index);
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into());
		// The record must be visible to the indirect stage before the deferred render pass begins.
		self.consume_resources_current([Consumption {
			handle: Handles::Buffer(buffer_handle),
			stages: crate::Stages::COMPUTE,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Indirect,
		}])
		.apply(self);
		if draw {
			self.begin_rendering_if_needed();
		}
		let buffer = self.get_buffer(buffer_handle).buffer;
		let command_buffer = self.get_command_buffer().command_buffer;
		(command_buffer, buffer, entry.start as vk::DeviceSize, entry.len() as u32)
	}
}
