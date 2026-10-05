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

	fn start_counter(&mut self, counter: crate::CounterHandle) {
		let slot = self.device.counters.start(self.counter_sequence(), counter);
		// The start marks when the GPU reaches the command, before any later work has to complete.
		self.write_timestamp(vk::PipelineStageFlags2::TOP_OF_PIPE, slot);
	}

	fn end_counter(&mut self, counter: crate::CounterHandle) {
		let slot = self.device.counters.end(self.counter_sequence(), counter);
		// The end waits for every earlier command to finish, so the span covers the measured work's completion.
		self.write_timestamp(vk::PipelineStageFlags2::BOTTOM_OF_PIPE, slot);
	}
}

impl CommandBufferRecording<'_> {
	/// Returns the frame sequence whose counter slots this recording writes.
	fn counter_sequence(&self) -> u8 {
		self.frame_key
			.expect(
				"Counters need a frame. The most likely cause is that start_counter or end_counter was called in a recording created from a command buffer instead of a frame.",
			)
			.sequence_index
	}

	/// Writes one GPU timestamp into `slot` of the context's counter query pool once `stage` completes.
	///
	/// A device whose queues write no timestamp bits records nothing; its counters always read `None`.
	fn write_timestamp(&mut self, stage: vk::PipelineStageFlags2, slot: u32) {
		if self.device.timestamp_valid_bits == 0 {
			return;
		}
		let command_buffer = self.get_command_buffer().command_buffer;
		// SAFETY: The slot comes from the context's counters, which stay below the pool's query count, and the
		// frame start reset it.
		unsafe {
			self.device
				.device
				.cmd_write_timestamp2(command_buffer, stage, self.device.counter_query_pool, slot);
		}
	}
}

impl crate::command_buffer::BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let (x, y, z) = dispatch.get_extent().as_tuple();
		let command_buffer = self.prepare_shader_work();
		unsafe {
			self.device.cmd_dispatch(command_buffer, x, y, z);
		}
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: impl Into<crate::command_buffer::IndirectDispatchBuffer<N>>,
		entry_index: usize,
	) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into().0);
		let buffer = self.get_buffer(buffer_handle);
		let (vk_buffer, buffer_size) = (buffer.buffer, buffer.size);
		let entry = crate::command_buffer::indirect_entry_range::<[u32; 3], N>(entry_index);
		assert!(
			entry.end <= buffer_size,
			"Vulkan indirect dispatch entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={}, buffer_size={buffer_size}",
			entry.end,
		);

		self.consume_resources_current([Consumption {
			handle: Handles::Buffer(buffer_handle),
			stages: crate::Stages::COMPUTE,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Indirect,
		}])
		.apply(self);
		unsafe {
			self.device.cmd_dispatch_indirect(
				self.get_command_buffer().command_buffer,
				vk_buffer,
				entry.start as vk::DeviceSize,
			);
		}
	}
}
