use crate::{
	image, raster_pipeline, sampler, window, AllocationHandle, BaseBufferHandle, BindingConstructor,
	BottomLevelAccelerationStructure, BottomLevelAccelerationStructureHandle, BufferHandle, CommandBufferHandle,
	DescriptorSetBindingHandle, DescriptorSetBindingTemplate, DescriptorSetHandle, DescriptorSetTemplateHandle,
	DescriptorWrite, DeviceAccesses, DynamicBufferHandle, FilteringModes, Formats, ImageHandle, MeshHandle, PipelineHandle,
	PipelineLayoutHandle, PresentationModes, PushConstantRange, QueueHandle, SamplerAddressingModes, SamplerHandle,
	SamplingReductionModes, ShaderBindingDescriptor, ShaderHandle, ShaderParameter, ShaderSource, ShaderTypes, SwapchainHandle,
	SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, UseCases, Uses, VertexElement,
};
use std::num::NonZeroU32;
use utils::Extent;

pub struct Device {}

impl Device {
	pub fn new() -> Self {
		Device {}
	}

	#[cfg(debug_assertions)]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn set_frames_in_flight(&mut self, _frames: u8) {}

	pub fn create_allocation(
		&mut self,
		_size: usize,
		_resource_uses: Uses,
		_resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle {
		AllocationHandle(0)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		_vertex_count: u32,
		_index_count: u32,
		_vertices: &[u8],
		_indices: &[u8],
		_vertex_layout: &[VertexElement],
	) -> MeshHandle {
		MeshHandle(0)
	}

	pub fn create_shader(
		&mut self,
		_name: Option<&str>,
		_shader_source_type: ShaderSource,
		_stage: ShaderTypes,
		_shader_binding_descriptors: impl IntoIterator<Item = ShaderBindingDescriptor>,
	) -> Result<ShaderHandle, ()> {
		Ok(ShaderHandle(0))
	}

	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		_binding_templates: &[DescriptorSetBindingTemplate],
	) -> DescriptorSetTemplateHandle {
		DescriptorSetTemplateHandle(0)
	}

	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		_descriptor_set_template_handle: &DescriptorSetTemplateHandle,
	) -> DescriptorSetHandle {
		DescriptorSetHandle(0)
	}

	pub fn create_descriptor_binding(
		&mut self,
		_descriptor_set: DescriptorSetHandle,
		_binding_constructor: BindingConstructor,
	) -> DescriptorSetBindingHandle {
		DescriptorSetBindingHandle(0)
	}

	pub fn create_pipeline_layout(
		&mut self,
		_descriptor_set_template_handles: &[DescriptorSetTemplateHandle],
		_push_constant_ranges: &[PushConstantRange],
	) -> PipelineLayoutHandle {
		PipelineLayoutHandle(0)
	}

	pub fn create_raster_pipeline(&mut self, _builder: raster_pipeline::Builder) -> PipelineHandle {
		PipelineHandle(0)
	}

	pub fn create_compute_pipeline(
		&mut self,
		_pipeline_layout_handle: PipelineLayoutHandle,
		_shader_parameter: ShaderParameter,
	) -> PipelineHandle {
		PipelineHandle(0)
	}

	pub fn create_ray_tracing_pipeline(
		&mut self,
		_pipeline_layout_handle: PipelineLayoutHandle,
		_shaders: &[ShaderParameter],
	) -> PipelineHandle {
		PipelineHandle(0)
	}

	pub fn create_command_buffer(&mut self, _name: Option<&str>, _queue_handle: QueueHandle) -> CommandBufferHandle {
		CommandBufferHandle(0)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		super::CommandBufferRecording::new(self, command_buffer_handle, Vec::new(), Vec::new(), None)
	}

	pub fn create_buffer<T: Copy>(
		&mut self,
		_name: Option<&str>,
		_resource_uses: Uses,
		_device_accesses: DeviceAccesses,
	) -> BufferHandle<T> {
		BufferHandle(0, std::marker::PhantomData)
	}

	pub fn create_dynamic_buffer<T: Copy>(
		&mut self,
		_name: Option<&str>,
		_resource_uses: Uses,
		_device_accesses: DeviceAccesses,
	) -> DynamicBufferHandle<T> {
		DynamicBufferHandle(0, std::marker::PhantomData)
	}

	pub fn get_buffer_address(&self, _buffer_handle: BaseBufferHandle) -> u64 {
		0
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, _buffer_handle: BufferHandle<T>) -> &T {
		todo!("Handle true allocations");
	}

	pub fn get_mut_buffer_slice<'a, T: Copy>(&'a self, _buffer_handle: BufferHandle<T>) -> &'a mut T {
		todo!("Handle true allocations");
	}

	pub fn get_texture_slice_mut(&mut self, _texture_handle: ImageHandle) -> &'static mut [u8] {
		&mut []
	}

	pub fn write_texture(&mut self, _texture_handle: ImageHandle, _f: impl FnOnce(&mut [u8])) {}

	#[deprecated(note = "Use build_image instead.")]
	pub fn create_image(
		&mut self,
		name: Option<&str>,
		extent: Extent,
		format: Formats,
		resource_uses: Uses,
		device_accesses: DeviceAccesses,
		use_case: UseCases,
		array_layers: Option<NonZeroU32>,
	) -> ImageHandle {
		let builder = image::Builder::new(format, resource_uses)
			.extent(extent)
			.device_accesses(device_accesses)
			.use_case(use_case)
			.array_layers(array_layers);
		let builder = if let Some(name) = name { builder.name(name) } else { builder };

		self.build_image(builder)
	}

	pub fn build_image(&mut self, _builder: image::Builder) -> ImageHandle {
		ImageHandle(0)
	}

	pub fn create_sampler(
		&mut self,
		_filtering_mode: FilteringModes,
		_reduction_mode: SamplingReductionModes,
		_mip_map_mode: FilteringModes,
		_addressing_mode: SamplerAddressingModes,
		_anisotropy: Option<f32>,
		_min_lod: f32,
		_max_lod: f32,
	) -> SamplerHandle {
		SamplerHandle(0)
	}

	pub fn build_sampler(&mut self, _builder: sampler::Builder) -> SamplerHandle {
		SamplerHandle(0)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> BaseBufferHandle {
		BaseBufferHandle(0)
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		TopLevelAccelerationStructureHandle(0)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		_description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		BottomLevelAccelerationStructureHandle(0)
	}

	pub fn write(&mut self, _descriptor_set_writes: &[DescriptorWrite]) {}

	pub fn write_instance(
		&mut self,
		_instances_buffer_handle: BaseBufferHandle,
		_instance_index: usize,
		_transform: [[f32; 4]; 3],
		_custom_index: u16,
		_mask: u8,
		_sbt_record_offset: usize,
		_acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: PipelineHandle,
		_shader_handle: ShaderHandle,
	) {
	}

	pub fn bind_to_window(
		&mut self,
		_window_os_handles: &window::Handles,
		_presentation_mode: PresentationModes,
		_fallback_extent: Extent,
	) -> SwapchainHandle {
		SwapchainHandle(0)
	}

	pub fn get_image_data<'a>(&'a self, _texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
		&[]
	}

	pub fn create_synchronizer(&mut self, _name: Option<&str>, _signaled: bool) -> SynchronizerHandle {
		SynchronizerHandle(0)
	}

	pub fn start_frame<'a>(&'a mut self, index: u32, _synchronizer_handle: SynchronizerHandle) -> super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: 0,
		};
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer(&mut self, _buffer_handle: BaseBufferHandle, _size: usize) {}

	pub fn start_frame_capture(&self) {}

	pub fn end_frame_capture(&self) {}

	pub fn wait(&self) {}
}
