use super::*;

impl crate::command_buffer::RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundRasterizationPipelineMode {
		self.record_pipeline_bind(vk::PipelineBindPoint::GRAPHICS, pipeline_handle)
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		self.vulkan_consume_resources(buffer_descriptors.iter().map(|buffer_descriptor| {
			vulkan_consumption(
				self.buffer_resource(buffer_descriptor.buffer),
				vk::PipelineStageFlags2::VERTEX_INPUT,
				vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
			)
		}))
		.apply(self);

		let buffers = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| {
				self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer))
					.buffer
			})
			.collect::<Vec<_>>();
		let offsets = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| buffer_descriptor.offset as vk::DeviceSize)
			.collect::<Vec<_>>();

		// TODO: implement slot splitting
		unsafe {
			self.device
				.device
				.cmd_bind_vertex_buffers(self.get_command_buffer().command_buffer, 0, &buffers, &offsets);
		}
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_descriptor.buffer);
		self.vulkan_consume_resources([vulkan_consumption(
			Handles::Buffer(buffer_handle),
			vk::PipelineStageFlags2::INDEX_INPUT,
			vk::AccessFlags2::INDEX_READ,
		)])
		.apply(self);

		let index_type = match buffer_descriptor.index_type {
			Some(crate::DataTypes::U16) => vk::IndexType::UINT16,
			Some(crate::DataTypes::U32) => vk::IndexType::UINT32,
			Some(_) => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
			None => panic!(
				"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32)."
			),
		};

		unsafe {
			self.device.device.cmd_bind_index_buffer(
				self.get_command_buffer().command_buffer,
				self.get_buffer(buffer_handle).buffer,
				buffer_descriptor.offset as _,
				index_type,
			);
		}
	}

	fn end_render_pass(&mut self) {
		// A pass with no draws must still begin so attachment clear/load/store operations execute.
		self.begin_rendering_if_needed();

		assert!(
			self.active_rendering,
			"No Vulkan render pass is active. The most likely cause is that end_render_pass was called without start_render_pass.",
		);
		unsafe {
			self.device.device.cmd_end_rendering(self.get_command_buffer().command_buffer);
		}
		self.active_rendering = false;
	}
}

impl crate::command_buffer::BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn write_push_constant<T: crate::Pod>(&mut self, offset: u32, data: T) {
		let layout_handle = self.bound_pipeline_layout.expect(
			"No Vulkan pipeline is bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let size = std::mem::size_of::<T>();
		let end = (offset as usize).checked_add(size).expect(
			"Invalid Vulkan push-data range. The most likely cause is that the offset and data size overflow addressable memory.",
		);
		let layout = &self.device.pipeline_layouts[layout_handle.0 as usize];

		assert!(
			offset.is_multiple_of(4) && size.is_multiple_of(4) && end <= layout.push_constant_size as usize,
			"Invalid Vulkan push-data write. The most likely cause is that the offset or data size is not four-byte aligned or exceeds the pipeline's declared push-constant ranges.",
		);
		let push_info = vk::PushDataInfoEXT::default()
			.offset(offset)
			.data(vk::HostAddressRangeConstEXT::default().address(bytemuck::bytes_of(&data)));
		unsafe {
			self.device
				.descriptor_heap
				.cmd_push_data(self.get_command_buffer().command_buffer, &push_info);
		}
	}

	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		self.bound_pipeline.expect(
			"No Vulkan pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		// Binding replaces the complete flat set union; no implicit set index or prior binding survives.
		self.bound_descriptor_set_handles.clear();
		self.bound_descriptor_set_handles.extend_from_slice(sets);
		self.current_descriptor_materialization = None;
		self.descriptor_materialization_dirty = true;
		self.descriptor_resources_initialized = false;
		self
	}
}

impl crate::command_buffer::BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		let command_buffer = self.prepare_draw();
		let mesh = &self.device.meshes[mesh_handle.0 as usize];
		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;
		unsafe {
			self.device
				.device
				.cmd_bind_vertex_buffers(command_buffer, 0, &[mesh.buffer], &[0]);
			self.device
				.device
				.cmd_bind_index_buffer(command_buffer, mesh.buffer, index_data_offset, vk::IndexType::UINT16);
			self.device
				.device
				.cmd_draw_indexed(command_buffer, mesh.index_count, 1, 0, 0, 0);
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		let command_buffer = self.prepare_draw();
		unsafe {
			self.device.mesh_shading.cmd_draw_mesh_tasks(command_buffer, x, y, z);
		}
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		let command_buffer = self.prepare_draw();
		unsafe {
			self.device
				.device
				.cmd_draw(command_buffer, vertex_count, instance_count, first_vertex, first_instance);
		}
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		let command_buffer = self.prepare_draw();
		unsafe {
			self.device.device.cmd_draw_indexed(
				command_buffer,
				index_count,
				instance_count,
				first_index,
				vertex_offset,
				first_instance,
			);
		}
	}
}
