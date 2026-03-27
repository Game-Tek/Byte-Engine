use super::*;
use objc2_metal::MTLBlitCommandEncoder;
use objc2_metal::MTLCommandBuffer;
use objc2_metal::MTLCommandEncoder;

pub struct Frame<'a> {
	frame_key: graphics_hardware_interface::FrameKey,
	device: &'a mut device::Device,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut device::Device, frame_key: graphics_hardware_interface::FrameKey) -> Self {
		Self { frame_key, device }
	}

	fn get_current_image_handle(&self, image_handle: impl graphics_hardware_interface::ImageHandleLike) -> image::ImageHandle {
		let image_handle = image_handle.into_image_handle();
		let handles = image::ImageHandle(image_handle.0).get_all(&self.device.images);
		handles[(self.frame_key.sequence_index as usize).rem_euclid(handles.len())]
	}
}

impl Frame<'_> {
	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	pub fn get_mut_dynamic_buffer_slice<'a, T: Copy>(
		&'a self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &'a mut T {
		let handles = buffer::BufferHandle(buffer_handle.0).get_all(&self.device.buffers);
		let handle = handles[self.frame_key.sequence_index as usize];
		let buffer = &self.device.buffers[handle.0 as usize];

		unsafe { &mut *(buffer.pointer as *mut T) }
	}

	pub fn get_texture_slice_mut(
		&mut self,
		texture_handle: impl graphics_hardware_interface::ImageHandleLike,
	) -> &'static mut [u8] {
		self.device.get_texture_slice_mut(graphics_hardware_interface::ImageHandle(
			self.get_current_image_handle(texture_handle).0,
		))
	}

	pub fn sync_texture(&mut self, image_handle: impl graphics_hardware_interface::ImageHandleLike) {
		self.device.sync_texture(graphics_hardware_interface::ImageHandle(
			self.get_current_image_handle(image_handle).0,
		));
	}

	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		self.device.write(descriptor_set_writes);
	}

	pub fn resize_image(&mut self, image_handle: impl graphics_hardware_interface::ImageHandleLike, extent: Extent) {
		let handle = self.get_current_image_handle(image_handle);
		let image = &self.device.images[handle.0 as usize];

		if image.extent == extent {
			return;
		}

		let replacement = self.device.create_image_resource(
			image.next,
			None,
			extent,
			image.format,
			image.uses,
			image.access,
			image.array_layers,
		);
		self.device.images[handle.0 as usize] = replacement;
		self.device.rewrite_descriptors_for_handle(Handle::Image(handle));
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.device
			.create_command_buffer_recording_with_frame_key(command_buffer_handle, Some(self.frame_key))
	}

	pub fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		let swapchain = &mut self.device.swapchains[swapchain_handle.0 as usize];

		swapchain.extent = update_layer_extent(&swapchain.layer, &swapchain.view);

		let drawable = swapchain
			.layer
			.nextDrawable()
			.expect("Failed to acquire Metal drawable. The most likely cause is that the layer has no available drawables.");

		let index = swapchain.store_drawable(drawable);

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: index,
			sequence_index: self.frame_key.sequence_index,
			swapchain: swapchain_handle,
		};

		swapchain.acquired_image_indices[self.frame_key.sequence_index as usize] = index;

		(present_key, swapchain.extent)
	}

	pub fn device(&mut self) -> &mut device::Device {
		self.device
	}

	pub fn execute(
		&mut self,
		cbr: super::FinishedCommandBuffer<'_>,
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let super::FinishedCommandBuffer {
			command_buffer_handle: _command_buffer_handle,
			command_buffer,
			present_drawables,
			states,
			present_keys,
		} = cbr;

		if !present_keys.is_empty() {
			let blit_encoder = command_buffer.blitCommandEncoder().expect(
				"Metal blit command encoder creation failed. The most likely cause is that the command buffer could not start the swapchain resolve pass.",
			);

			for (present_key, drawable) in present_keys.iter().zip(present_drawables.iter()) {
				let swapchain = &self.device.swapchains[present_key.swapchain.0 as usize];
				let Some(proxy_image) = swapchain.images[present_key.image_index as usize] else {
					continue;
				};
				let source_texture = self.device.images[proxy_image.0 as usize].texture.clone();
				let destination_texture = drawable.texture();

				unsafe {
					blit_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
				}
			}

			blit_encoder.endEncoding();
		}

		for drawable in &present_drawables {
			let drawable_ref: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			command_buffer.presentDrawable(drawable_ref);
		}

		command_buffer.commit();
		command_buffer.waitUntilCompleted();

		if let Some(synchronizer) = self.device.synchronizers.get_mut(synchronizer.0 as usize) {
			synchronizer.signaled = true;
		}

		self.device.states = states;
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'f>
		= super::CommandBufferRecording<'f>
	where
		Self: 'f;

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: crate::BufferHandle<T>) -> &'static mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: impl graphics_hardware_interface::ImageHandleLike) -> &'static mut [u8] {
		self.device.get_texture_slice_mut(graphics_hardware_interface::ImageHandle(
			self.get_current_image_handle(texture_handle).0,
		))
	}

	fn sync_texture(&mut self, image_handle: impl graphics_hardware_interface::ImageHandleLike) {
		self.device.sync_texture(graphics_hardware_interface::ImageHandle(
			self.get_current_image_handle(image_handle).0,
		));
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		self.device.write(descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		let handles = buffer::BufferHandle(buffer_handle.0).get_all(&self.device.buffers);
		let handle = handles[self.frame_key.sequence_index as usize];
		let buffer = &self.device.buffers[handle.0 as usize];

		unsafe { &mut *(buffer.pointer as *mut T) }
	}

	fn resize_image(&mut self, image_handle: impl graphics_hardware_interface::ImageHandleLike, extent: Extent) {
		Frame::resize_image(self, image_handle, extent);
	}

	fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CBR<'_> {
		self.device
			.create_command_buffer_recording_with_frame_key(command_buffer_handle, Some(self.frame_key))
	}

	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		Frame::acquire_swapchain_image(self, swapchain_handle)
	}

	fn execute<'s, 'f>(
		&mut self,
		cbr: <Self::CBR<'f> as crate::command_buffer::CommandBufferRecording>::Result<'s>,
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) where
		Self: 'f,
	{
		Frame::execute(self, cbr, synchronizer);
	}
}

impl<'a> crate::device::DeviceCreate for Frame<'a> {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> crate::AllocationHandle {
		self.device.create_allocation(size, resource_uses, resource_device_accesses)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> crate::MeshHandle {
		self.device
			.add_mesh_from_vertices_and_indices(vertex_count, index_count, vertices, indices, vertex_layout)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<crate::ShaderHandle, ()> {
		self.device
			.create_shader(name, shader_source_type, stage, shader_binding_descriptors)
	}

	fn create_descriptor_set_template(
		&mut self,
		name: Option<&str>,
		binding_templates: &[crate::DescriptorSetBindingTemplate],
	) -> crate::DescriptorSetTemplateHandle {
		self.device.create_descriptor_set_template(name, binding_templates)
	}

	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		descriptor_set_template_handle: &crate::DescriptorSetTemplateHandle,
	) -> crate::DescriptorSetHandle {
		self.device.create_descriptor_set(name, descriptor_set_template_handle)
	}

	fn create_descriptor_binding(
		&mut self,
		descriptor_set: crate::DescriptorSetHandle,
		binding_constructor: crate::BindingConstructor,
	) -> crate::DescriptorSetBindingHandle {
		self.device.create_descriptor_binding(descriptor_set, binding_constructor)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> crate::PipelineHandle {
		self.device.create_raster_pipeline(builder)
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> crate::PipelineHandle {
		self.device.create_compute_pipeline(builder)
	}

	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> crate::PipelineHandle {
		self.device.create_ray_tracing_pipeline(builder)
	}

	fn create_command_buffer(&mut self, name: Option<&str>, queue_handle: crate::QueueHandle) -> crate::CommandBufferHandle {
		self.device.create_command_buffer(name, queue_handle)
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::BufferHandle<T> {
		self.device.build_buffer(builder)
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		self.device.build_dynamic_buffer(builder)
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		self.device.build_dynamic_image(builder)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> crate::ImageHandle {
		self.device.build_image(builder)
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> crate::SamplerHandle {
		self.device.build_sampler(builder)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::BaseBufferHandle {
		self.device
			.create_acceleration_structure_instance_buffer(name, max_instance_count)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.device.create_top_level_acceleration_structure(name, max_instance_count)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &crate::BottomLevelAccelerationStructure,
	) -> crate::BottomLevelAccelerationStructureHandle {
		self.device.create_bottom_level_acceleration_structure(description)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> crate::SynchronizerHandle {
		self.device.create_synchronizer(name, signaled)
	}
}
