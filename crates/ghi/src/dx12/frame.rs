use utils::Extent;

use crate::{
	BaseBufferHandle, BaseImageHandle, BufferHandle, CommandBufferHandle, DynamicBufferHandle, FrameKey, ImageHandle,
	PresentKey, SwapchainHandle,
};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut super::Device,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut super::Device, frame_key: FrameKey) -> Self {
		Self { frame_key, device }
	}
}

impl Frame<'_> {
	pub fn intern_raster_pipeline(&mut self, pipeline: crate::implementation::RasterPipeline) -> crate::PipelineHandle {
		let shader_handles = self.intern_factory_shaders(&pipeline.factory_shaders);
		let vertex_elements = pipeline
			.vertex_elements
			.iter()
			.map(|element| crate::pipelines::VertexElement::new(element.name.as_str(), element.format, element.binding))
			.collect::<Vec<_>>();
		let shaders = pipeline
			.shaders
			.iter()
			.filter_map(|shader| {
				shader_handles.get(shader.handle.0 as usize).map(|handle| {
					crate::pipelines::ShaderParameter::new(handle, shader.stage)
						.with_specialization_map(&shader.specialization_map)
				})
			})
			.collect::<Vec<_>>();
		let builder = crate::pipelines::raster::Builder::new(
			&pipeline.descriptor_set_templates,
			&pipeline.push_constant_ranges,
			&vertex_elements,
			&shaders,
			&pipeline.render_targets,
		)
		.face_winding(pipeline.face_winding)
		.cull_mode(pipeline.cull_mode);

		self.device.create_raster_pipeline(builder)
	}

	pub fn intern_compute_pipeline(&mut self, pipeline: crate::implementation::ComputePipeline) -> crate::PipelineHandle {
		let shader_handles = self.intern_factory_shaders(&pipeline.factory_shaders);
		let shader_handle = shader_handles
			.get(pipeline.shader.handle.0 as usize)
			.expect("Missing DX12 factory compute shader. The most likely cause is that the pipeline references a shader not created by the factory.");
		let shader = crate::pipelines::ShaderParameter::new(shader_handle, pipeline.shader.stage)
			.with_specialization_map(&pipeline.shader.specialization_map);

		self.device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&pipeline.descriptor_set_templates,
			&pipeline.push_constant_ranges,
			shader,
		))
	}

	pub fn intern_image(&mut self, image: crate::implementation::FactoryImage) -> crate::ImageHandle {
		let mut builder = crate::image::Builder::new(image.format, image.resource_uses)
			.extent(image.extent)
			.device_accesses(image.device_accesses)
			.use_case(image.use_case)
			.mip_levels(image.mip_levels)
			.array_layers(image.array_layers);
		if let Some(name) = image.name.as_deref() {
			builder = builder.name(name);
		}
		self.device.build_image(builder)
	}

	pub fn intern_sampler(&mut self, sampler: crate::implementation::FactorySampler) -> crate::SamplerHandle {
		let mut builder = crate::sampler::Builder::new()
			.filtering_mode(sampler.filtering_mode)
			.reduction_mode(sampler.reduction_mode)
			.mip_map_mode(sampler.mip_map_mode)
			.addressing_mode(sampler.addressing_mode)
			.min_lod(sampler.min_lod)
			.max_lod(sampler.max_lod);
		if let Some(anisotropy) = sampler.anisotropy {
			builder = builder.anisotropy(anisotropy);
		}
		self.device.build_sampler(builder)
	}

	fn intern_factory_shaders(&mut self, shaders: &[crate::dx12::factory::Shader]) -> Vec<crate::ShaderHandle> {
		shaders
			.iter()
			.map(|shader| {
				let source = match &shader.source {
					crate::dx12::factory::ShaderSource::Spirv(bytes) => crate::shader::Sources::SPIRV(bytes),
					crate::dx12::factory::ShaderSource::Dxil(bytes) => crate::shader::Sources::DXIL(bytes),
					crate::dx12::factory::ShaderSource::Hlsl { source, entry_point } => crate::shader::Sources::HLSL {
						source,
						entry_point,
					},
				};
				self.device
					.create_shader(shader.name.as_deref(), source, shader.stage, shader.bindings.iter().copied())
					.expect("Failed to intern DX12 factory shader. The most likely cause is that the factory stored an unsupported shader source.")
			})
			.collect()
	}

	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T {
		unsafe { std::mem::transmute::<&mut T, &'static mut T>(self.device.get_mut_buffer_slice(buffer_handle)) }
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		self.device
			.sync_buffer_for_sequence(buffer_handle, self.frame_key.sequence_index);
	}

	pub fn get_texture_slice_mut(&self, texture_handle: BaseImageHandle) -> &'static mut [u8] {
		self.device
			.texture_slice_mut_for_sequence(texture_handle, self.frame_key.sequence_index)
	}

	pub fn sync_texture(&mut self, image_handle: BaseImageHandle) {
		self.device
			.queue_texture_sync_for_sequence(image_handle, self.frame_key.sequence_index);
	}

	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		self.device.write(descriptor_set_writes);
	}

	pub fn get_mut_dynamic_buffer_slice<'a, T: Copy>(&'a mut self, buffer_handle: DynamicBufferHandle<T>) -> &'a mut T {
		self.device
			.dynamic_buffer_slice_mut(buffer_handle, self.frame_key.sequence_index)
	}

	pub fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent) {
		self.device.resize_image_internal(ImageHandle(image_handle), extent);
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.device
			.begin_command_buffer(command_buffer_handle, self.frame_key.sequence_index);
		self.device
			.flush_pending_texture_syncs_for_sequence(command_buffer_handle, self.frame_key.sequence_index);
		super::CommandBufferRecording::new(self.device, command_buffer_handle, Some(self.frame_key))
	}

	pub fn create_command_buffer_recording_without_implicit_sync<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.device
			.begin_command_buffer(command_buffer_handle, self.frame_key.sequence_index);
		super::CommandBufferRecording::new(self.device, command_buffer_handle, Some(self.frame_key))
	}

	pub fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> (PresentKey, Extent) {
		let extent = self.device.swapchain_extent(swapchain_handle);
		let image_index = self.device.next_swapchain_image_index(swapchain_handle);
		let present_key = PresentKey {
			image_index,
			sequence_index: self.frame_key.sequence_index,
			swapchain: swapchain_handle,
		};
		self.device.swapchains[swapchain_handle.0 as usize].acquired_image_indices[self.frame_key.sequence_index as usize] =
			image_index;
		(present_key, extent)
	}

	pub fn device(&mut self) -> &mut super::Device {
		self.device
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= super::CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> FrameKey {
		self.frame_key
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T {
		Frame::get_mut_buffer_slice(self, buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		Frame::sync_buffer(self, buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: BaseImageHandle) -> &'static mut [u8] {
		Frame::get_texture_slice_mut(self, texture_handle)
	}

	fn sync_texture(&mut self, image_handle: BaseImageHandle) {
		Frame::sync_texture(self, image_handle);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		Frame::write(self, descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>) -> &mut T {
		Frame::get_mut_dynamic_buffer_slice(self, buffer_handle)
	}

	fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent) {
		Frame::resize_image(self, image_handle, extent);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record> {
		Frame::create_command_buffer_recording(self, command_buffer_handle)
	}

	fn create_command_buffer_recording_without_implicit_sync<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record> {
		Frame::create_command_buffer_recording_without_implicit_sync(self, command_buffer_handle)
	}

	fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> (PresentKey, Extent) {
		Frame::acquire_swapchain_image(self, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
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
