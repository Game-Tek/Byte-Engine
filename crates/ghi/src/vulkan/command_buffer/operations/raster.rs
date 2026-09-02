use super::*;

impl crate::command_buffer::RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundRasterizationPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		unsafe {
			self.device.device.cmd_bind_pipeline(
				command_buffer.command_buffer,
				vk::PipelineBindPoint::GRAPHICS,
				pipeline.pipeline,
			);
		}

		self.pipeline_bind_point = vk::PipelineBindPoint::GRAPHICS;
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(pipeline.layout);
		self.descriptor_materialization_dirty = true;
		self.descriptor_resources_initialized = false;

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		let consumptions = buffer_descriptors.iter().map(|buffer_descriptor| VulkanConsumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
			stages: vk::PipelineStageFlags2::VERTEX_INPUT,
			access: vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
			layout: vk::ImageLayout::UNDEFINED,
			range: None,
		});

		self.vulkan_consume_resources(consumptions).apply(self);

		let command_buffer = self.get_command_buffer();

		let buffers = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| {
				self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer))
					.buffer
			})
			.collect::<Vec<_>>();
		let offsets = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| buffer_descriptor.offset)
			.collect::<Vec<_>>();

		// TODO: implent slot splitting
		unsafe {
			self.device.device.cmd_bind_vertex_buffers(
				command_buffer.command_buffer,
				0,
				&buffers,
				&offsets.iter().map(|&e| e as _).collect::<Vec<_>>(),
			);
		}
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		self.vulkan_consume_resources([VulkanConsumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
			stages: vk::PipelineStageFlags2::INDEX_INPUT,
			access: vk::AccessFlags2::INDEX_READ,
			layout: vk::ImageLayout::UNDEFINED,
			range: None,
		}])
		.apply(self);

		let command_buffer = self.get_command_buffer();

		let buffer = self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer));
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
				command_buffer.command_buffer,
				buffer.buffer,
				buffer_descriptor.offset as _,
				index_type,
			);
		}
	}

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self) {
		// A pass with no draws must still begin so attachment clear/load/store operations execute.
		self.begin_rendering_if_needed();

		assert!(
			self.active_rendering,
			"No Vulkan render pass is active. The most likely cause is that end_render_pass was called without start_render_pass.",
		);
		let command_buffer = self.get_command_buffer();
		unsafe {
			self.device.device.cmd_end_rendering(command_buffer.command_buffer);
		}
		self.active_rendering = false;
	}
}

impl crate::command_buffer::BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn write_push_constant<T: crate::Pod>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let layout_handle = self.bound_pipeline_layout.expect(
			"No Vulkan pipeline is bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let size = std::mem::size_of::<T>();
		let end = (offset as usize).checked_add(size).expect(
			"Invalid Vulkan push-data range. The most likely cause is that the offset and data size overflow addressable memory.",
		);
		let layout = &self.device.pipeline_layouts[layout_handle.0 as usize];

		assert!(
			offset % 4 == 0 && size % 4 == 0 && end <= layout.push_constant_size as usize,
			"Invalid Vulkan push-data write. The most likely cause is that the offset or data size is not four-byte aligned or exceeds the pipeline's declared push-constant ranges.",
		);
		let bytes = bytemuck::bytes_of(&data);
		let push_info = vk::PushDataInfoEXT::default()
			.offset(offset)
			.data(vk::HostAddressRangeConstEXT::default().address(bytes));
		let command_buffer = self.get_command_buffer().command_buffer;
		unsafe {
			self.device.descriptor_heap.cmd_push_data(command_buffer, &push_info);
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
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		// Raster pipelines can read descriptor-backed resources in vertex, mesh, and fragment stages.
		// Transition them before issuing the draw so transfer uploads are visible to shader reads.
		self.consume_resources_current([]).apply(self);
		self.begin_rendering_if_needed();

		let command_buffer = self.get_command_buffer();

		let mesh = &self.device.meshes[mesh_handle.0 as usize];

		let buffers = [mesh.buffer];
		let offsets = [0];

		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device
				.device
				.cmd_bind_vertex_buffers(command_buffer_handle, 0, &buffers, &offsets);
		}
		unsafe {
			self.device.device.cmd_bind_index_buffer(
				command_buffer_handle,
				mesh.buffer,
				index_data_offset,
				vk::IndexType::UINT16,
			);
		}

		unsafe {
			self.device
				.device
				.cmd_draw_indexed(command_buffer_handle, mesh.index_count, 1, 0, 0, 0);
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		// Mesh shaders in the visibility pipeline read descriptor-backed storage buffers populated by
		// transfer uploads. Without this transition, Vulkan can execute the mesh read before those
		// transfer writes are available even though the descriptor set itself is correctly bound.
		self.consume_resources_current([]).apply(self);
		self.begin_rendering_if_needed();

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.mesh_shading.cmd_draw_mesh_tasks(command_buffer_handle, x, y, z);
		}
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		// Draw calls use the currently bound pipeline descriptors just like compute dispatches do.
		self.consume_resources_current([]).apply(self);
		self.begin_rendering_if_needed();

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.device.cmd_draw(
				command_buffer_handle,
				vertex_count,
				instance_count,
				first_vertex,
				first_instance,
			);
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
		// Draw calls use the currently bound pipeline descriptors just like compute dispatches do.
		self.consume_resources_current([]).apply(self);
		self.begin_rendering_if_needed();

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.device.cmd_draw_indexed(
				command_buffer_handle,
				index_count,
				instance_count,
				first_index,
				vertex_offset,
				first_instance,
			);
		}
	}
}
