use super::*;

impl crate::command_buffer::CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundComputePipelineMode {
		self.record_pipeline_bind(vk::PipelineBindPoint::COMPUTE, pipeline_handle)
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundRayTracingPipelineMode {
		self.record_pipeline_bind(vk::PipelineBindPoint::RAY_TRACING_KHR, pipeline_handle)
	}

	fn start_region(&mut self, _write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		#[cfg(debug_assertions)]
		{
			let mut label = crate::command_buffer::DebugLabelWriter::new();
			_write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");

			// Vulkan requires a null-terminated label that remains alive for the duration of the call.
			label.null_terminate();
			let name = std::ffi::CStr::from_bytes_with_nul(label.as_bytes())
				.expect("Invalid debug label. The label most likely contains an interior null byte.");
			if let Some(debug_utils) = &self.device.debug_utils {
				unsafe {
					debug_utils.cmd_begin_debug_utils_label(
						self.get_command_buffer().command_buffer,
						&vk::DebugUtilsLabelEXT::default().label_name(name),
					);
				}
			}
		}
	}

	fn end_region(&mut self) {
		#[cfg(debug_assertions)]
		if let Some(debug_utils) = &self.device.debug_utils {
			unsafe {
				debug_utils.cmd_end_debug_utils_label(self.get_command_buffer().command_buffer);
			}
		}
	}
}

impl crate::command_buffer::BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let (x, y, z) = dispatch.get_extent().as_tuple();
		let command_buffer = self.prepare_shader_work();
		unsafe {
			self.device.device.cmd_dispatch(command_buffer, x, y, z);
		}
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: impl Into<crate::command_buffer::IndirectDispatchBuffer<N>>,
		entry_index: usize,
	) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into().handle());
		let buffer = self.get_buffer(buffer_handle);
		let (vk_buffer, buffer_size) = (buffer.buffer, buffer.size);
		assert!(
			entry_index < N,
			"Vulkan indirect dispatch entry is out of bounds. The most likely cause is that entry_index exceeds the typed indirect buffer length. entry_index={entry_index}, entry_count={N}",
		);
		let argument_size = std::mem::size_of::<[u32; 3]>();
		let argument_offset = entry_index.checked_mul(argument_size).expect(
			"Vulkan indirect dispatch offset overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		let argument_end = argument_offset.checked_add(argument_size).expect(
			"Vulkan indirect dispatch range overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		assert!(
			argument_end <= buffer_size,
			"Vulkan indirect dispatch entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={argument_end}, buffer_size={buffer_size}",
		);
		let argument_offset = u64::try_from(argument_offset).expect(
			"Vulkan indirect dispatch offset exceeds the native address range. The most likely cause is that the host address space is wider than Vulkan device offsets.",
		);

		self.consume_resources_current([Consumption {
			handle: Handles::Buffer(buffer_handle),
			stages: crate::Stages::COMPUTE,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Indirect,
		}])
		.apply(self);
		unsafe {
			self.device
				.device
				.cmd_dispatch_indirect(self.get_command_buffer().command_buffer, vk_buffer, argument_offset);
		}
	}
}
