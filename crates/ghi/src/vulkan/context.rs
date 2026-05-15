use utils::Extent;

use crate::{graphics_hardware_interface, vulkan::Device, window};

/// The `Context` struct owns Vulkan device state while presenting the GHI context API.
pub struct Context {
	device: Device,
}

impl Context {
	pub(super) fn new(device: Device) -> Self {
		Self { device }
	}

	/// Returns no detached-resource factory because the Vulkan backend does not implement this path yet.
	pub fn create_factory(&self) -> Option<crate::implementation::Factory> {
		self.device.create_factory()
	}

	/// Returns no detached pipeline factory for compatibility with the previous pipeline factory API.
	pub fn create_pipeline_factory(&self) -> Option<crate::implementation::Factory> {
		self.device.create_pipeline_factory()
	}
}

impl crate::context::Context for Context {
	type Queue = <Device as crate::context::Context>::Queue;
	type QueueReference<'a>
		= <Device as crate::context::Context>::QueueReference<'a>
	where
		Self: 'a;
	type CommandBuffer<'a>
		= <Device as crate::context::Context>::CommandBuffer<'a>
	where
		Self: 'a;

	fn queue(&mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue {
		crate::context::Context::queue(&mut self.device, queue_handle)
	}

	fn queue_reference<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::QueueReference<'a> {
		crate::context::Context::queue_reference(&mut self.device, queue_handle)
	}

	fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CommandBuffer<'a> {
		crate::context::Context::command_buffer(&mut self.device, command_buffer_handle)
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		crate::context::Context::set_frames_in_flight(&mut self.device, frames);
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		crate::context::Context::get_buffer_address(&self.device, buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		crate::context::Context::get_buffer_slice(&mut self.device, buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		crate::context::Context::get_mut_buffer_slice(&self.device, buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		crate::context::Context::sync_buffer(&mut self.device, buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		crate::context::Context::get_texture_slice_mut(&self.device, texture_handle)
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		crate::context::Context::sync_texture(&mut self.device, image_handle);
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		crate::context::Context::write_texture(&mut self.device, texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		crate::context::Context::write(&mut self.device, descriptor_set_writes);
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
		crate::context::Context::write_instance(
			&mut self.device,
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
		crate::context::Context::write_sbt_entry(
			&mut self.device,
			sbt_buffer_handle,
			sbt_record_offset,
			pipeline_handle,
			shader_handle,
		);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		crate::context::Context::bind_to_window(&mut self.device, window_os_handles, presentation_mode, fallback_extent, uses)
	}

	fn get_image_data<'a>(&'a self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		crate::context::Context::get_image_data(&self.device, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>, size: usize) {
		crate::context::Context::resize_buffer(&mut self.device, buffer_handle, size);
	}

	fn start_frame_capture(&mut self) {
		crate::context::Context::start_frame_capture(&mut self.device);
	}

	fn end_frame_capture(&mut self) {
		crate::context::Context::end_frame_capture(&mut self.device);
	}

	fn wait(&self) {
		crate::context::Context::wait(&self.device);
	}
}

impl crate::context::ContextCreate for Context {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		crate::context::ContextCreate::create_allocation(&mut self.device, size, resource_uses, resource_device_accesses)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		crate::context::ContextCreate::add_mesh_from_vertices_and_indices(
			&mut self.device,
			vertex_count,
			index_count,
			vertices,
			indices,
			vertex_layout,
		)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		crate::context::ContextCreate::create_shader(
			&mut self.device,
			name,
			shader_source_type,
			stage,
			shader_binding_descriptors,
		)
	}

	fn create_descriptor_set_template(
		&mut self,
		name: Option<&str>,
		binding_templates: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
	) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		crate::context::ContextCreate::create_descriptor_set_template(&mut self.device, name, binding_templates)
	}

	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		descriptor_set_template_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		crate::context::ContextCreate::create_descriptor_set(&mut self.device, name, descriptor_set_template_handle)
	}

	fn create_descriptor_binding(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		binding_constructor: graphics_hardware_interface::BindingConstructor,
	) -> graphics_hardware_interface::DescriptorSetBindingHandle {
		crate::context::ContextCreate::create_descriptor_binding(&mut self.device, descriptor_set, binding_constructor)
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		crate::context::ContextCreate::create_raster_pipeline(&mut self.device, builder)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		crate::context::ContextCreate::create_compute_pipeline(&mut self.device, builder)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		crate::context::ContextCreate::create_ray_tracing_pipeline(&mut self.device, builder)
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		crate::context::ContextCreate::build_buffer(&mut self.device, builder)
	}

	fn build_dynamic_buffer<T: Copy>(
		&mut self,
		builder: crate::buffer::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		crate::context::ContextCreate::build_dynamic_buffer(&mut self.device, builder)
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		crate::context::ContextCreate::build_dynamic_image(&mut self.device, builder)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> graphics_hardware_interface::ImageHandle {
		crate::context::ContextCreate::build_image(&mut self.device, builder)
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> graphics_hardware_interface::SamplerHandle {
		crate::context::ContextCreate::build_sampler(&mut self.device, builder)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		crate::context::ContextCreate::create_acceleration_structure_instance_buffer(&mut self.device, name, max_instance_count)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		crate::context::ContextCreate::create_top_level_acceleration_structure(&mut self.device, name, max_instance_count)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		crate::context::ContextCreate::create_bottom_level_acceleration_structure(&mut self.device, description)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		crate::context::ContextCreate::create_synchronizer(&mut self.device, name, signaled)
	}
}
