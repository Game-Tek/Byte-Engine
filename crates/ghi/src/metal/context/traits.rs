use super::*;

impl crate::context::Context for Context {
	type Queue = crate::metal::queue::Queue;
	type QueueReference<'a> = crate::metal::queue::QueueReference<'a>;
	type CommandBuffer<'a> = crate::metal::CommandBuffer<'a>;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		Context::has_errors(self)
	}

	fn supports_bc_texture_compression(&self) -> bool {
		// self.device.supportsBCTextureCompression()
		true
	}

	fn queue(&mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue {
		Context::queue(self, queue_handle)
	}

	fn queue_reference<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::QueueReference<'a> {
		Context::queue_reference(self, queue_handle)
	}

	fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CommandBuffer<'a> {
		Context::command_buffer(self, command_buffer_handle)
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		Context::get_buffer_address(self, buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		Context::get_buffer_slice(self, buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		Context::get_mut_buffer_slice(self, buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		Context::sync_buffer(self, buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		Context::get_texture_slice_mut(self, texture_handle)
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		Context::sync_texture(self, image_handle);
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		Context::write_texture(self, texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		Context::write(self, descriptor_set_writes);
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		Context::write_instance(
			self,
			instances_buffer_handle,
			instance_index,
			transform,
			custom_index,
			mask,
			sbt_record_offset,
			acceleration_structure,
		);
	}

	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		Context::write_sbt_entry(self, sbt_buffer_handle, sbt_record_offset, pipeline_handle, shader_handle);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		Context::bind_to_window(self, window_os_handles, presentation_mode, fallback_extent, uses)
	}

	fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		Context::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>, size: usize) {
		Context::resize_buffer(self, buffer_handle, size);
	}

	fn start_frame_capture(&mut self) {
		Context::start_frame_capture(self);
	}

	fn end_frame_capture(&mut self) {
		Context::end_frame_capture(self);
	}

	fn wait_for_synchronizer(&mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		Context::wait_for_synchronizer(self, synchronizer);
	}

	fn wait(&self) {
		Context::wait(self);
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		Context::set_frames_in_flight(self, frames);
	}
}

impl crate::context::ContextCreate for Context {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		Context::create_allocation(self, size, resource_uses, resource_device_accesses)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		Context::add_mesh_from_vertices_and_indices(self, vertex_count, index_count, vertices, indices, vertex_layout)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		Context::create_shader(self, name, shader_source_type, stage, shader_resource_descriptors)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>) -> graphics_hardware_interface::DescriptorSetHandle {
		Context::create_descriptor_set(self, name)
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		Context::create_raster_pipeline(self, builder)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		Context::create_compute_pipeline(self, builder)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		Context::create_ray_tracing_pipeline(self, builder)
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		Context::build_buffer(self, builder)
	}

	fn build_dynamic_buffer<T: Copy>(
		&mut self,
		builder: crate::buffer::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		Context::build_dynamic_buffer(self, builder)
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		Context::build_dynamic_image(self, builder)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> graphics_hardware_interface::ImageHandle {
		Context::build_image(self, builder)
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> graphics_hardware_interface::SamplerHandle {
		Context::build_sampler(self, builder)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		Context::create_acceleration_structure_instance_buffer(self, name, max_instance_count)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		Context::create_top_level_acceleration_structure(self, name, max_instance_count)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		Context::create_bottom_level_acceleration_structure(self, description)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		Context::create_synchronizer(self, name, signaled)
	}
}
