use super::*;

impl crate::command_buffer::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: crate::rt::BindingTables, x: u32, y: u32, z: u32) {
		let make_strided_range = |range: crate::BufferStridedRange| {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(
					self.device.get_buffer_address(range.buffer_offset.buffer) + range.buffer_offset.offset as vk::DeviceSize,
				)
				.stride(range.stride as vk::DeviceSize)
				.size(range.size as vk::DeviceSize)
		};
		let raygen = make_strided_range(binding_tables.raygen);
		let miss = make_strided_range(binding_tables.miss);
		let hit = make_strided_range(binding_tables.hit);
		let callable = binding_tables.callable.map_or_else(Default::default, make_strided_range);

		let command_buffer = self.prepare_shader_work();
		unsafe {
			self.device
				.ray_tracing_pipeline
				.cmd_trace_rays(command_buffer, &raygen, &miss, &hit, &callable, x, y, z)
		}
	}
}
