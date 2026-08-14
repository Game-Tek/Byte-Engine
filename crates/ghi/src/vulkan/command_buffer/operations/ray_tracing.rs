use super::*;

impl crate::command_buffer::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: crate::rt::BindingTables, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();
		let comamand_buffer_handle = command_buffer.command_buffer;

		let make_strided_range = |range: crate::BufferStridedRange| -> vk::StridedDeviceAddressRegionKHR {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(
					self.device.get_buffer_address(range.buffer_offset.buffer) as vk::DeviceSize
						+ range.buffer_offset.offset as vk::DeviceSize,
				)
				.stride(range.stride as vk::DeviceSize)
				.size(range.size as vk::DeviceSize)
		};

		let raygen_shader_binding_tables = make_strided_range(binding_tables.raygen);
		let miss_shader_binding_tables = make_strided_range(binding_tables.miss);
		let hit_shader_binding_tables = make_strided_range(binding_tables.hit);
		let callable_shader_binding_tables = if let Some(binding_table) = binding_tables.callable {
			make_strided_range(binding_table)
		} else {
			vk::StridedDeviceAddressRegionKHR::default()
		};

		self.consume_resources_current([]).apply(self);

		unsafe {
			self.device.ray_tracing_pipeline.cmd_trace_rays(
				comamand_buffer_handle,
				&raygen_shader_binding_tables,
				&miss_shader_binding_tables,
				&hit_shader_binding_tables,
				&callable_shader_binding_tables,
				x,
				y,
				z,
			)
		}
	}
}
