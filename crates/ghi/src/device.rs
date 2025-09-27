use std::num::NonZeroU32;

use utils::Extent;

use crate::{image, raster_pipeline, sampler, window, AllocationHandle, BaseBufferHandle, BindingConstructor, BottomLevelAccelerationStructure, BottomLevelAccelerationStructureHandle, BufferHandle, CommandBufferHandle, CommandBufferRecording, DescriptorSetBindingHandle, DescriptorSetBindingTemplate, DescriptorSetHandle, DescriptorSetTemplateHandle, DescriptorWrite, DeviceAccesses, DynamicBufferHandle, FilteringModes, Formats, Frame, ImageHandle, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentationModes, PushConstantRange, QueueHandle, SamplerAddressingModes, SamplerHandle, SamplingReductionModes, ShaderBindingDescriptor, ShaderHandle, ShaderParameter, ShaderSource, ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, UseCases, Uses, VertexElement};

/// The `Device` trait represents a graphics device that can be used to create and manage resources such as buffers, images, pipelines, and descriptor sets.
pub trait Device where Self: Sized {
	/// Returns whether the underlying API has encountered any errors. Used during tests to assert whether the validation layers have caught any errors.
	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool;

	/// Updates the number of maximum frames in flight.
	/// This operation creates extra resources to support the new number of frames in flight.
	/// > THIS IS AN EXPENSIVE OPERATION
	fn set_frames_in_flight(&mut self, frames: u8);

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: Uses, resource_device_accesses: DeviceAccesses) -> AllocationHandle;

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[VertexElement]) -> MeshHandle;

	/// Creates a shader.
	/// # Arguments
	/// * `name` - The name of the shader.
	/// * `shader_source_type` - The type of the shader source.
	/// * `stage` - The stage of the shader.
	/// * `shader_binding_descriptors` - The binding descriptors of the shader.
	/// # Returns
	/// The handle of the shader.
	/// # Errors
	/// Returns an error if the shader source was GLSL source code and could not be compiled.
	/// Returns an error if the shader source was SPIR-V binary and could not aligned to 4 bytes.
	fn create_shader(&mut self, name: Option<&str>, shader_source_type: ShaderSource, stage: ShaderTypes, shader_binding_descriptors: impl IntoIterator<Item = ShaderBindingDescriptor>) -> Result<ShaderHandle, ()>;

	fn create_descriptor_set_template(&mut self, name: Option<&str>, binding_templates: &[DescriptorSetBindingTemplate]) -> DescriptorSetTemplateHandle;

	fn create_descriptor_set(&mut self, name: Option<&str>, descriptor_set_template_handle: &DescriptorSetTemplateHandle) -> DescriptorSetHandle;

	fn create_descriptor_binding(&mut self, descriptor_set: DescriptorSetHandle, binding_constructor: BindingConstructor) -> DescriptorSetBindingHandle;

	fn create_pipeline_layout(&mut self, descriptor_set_template_handles: &[DescriptorSetTemplateHandle], push_constant_ranges: &[PushConstantRange]) -> PipelineLayoutHandle;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> PipelineHandle;

	/// Creates a compute pipeline.
	fn create_compute_pipeline(&mut self, pipeline_layout_handle: PipelineLayoutHandle, shader_parameter: ShaderParameter) -> PipelineHandle;

	/// Creates a ray-tracing pipeline.
	fn create_ray_tracing_pipeline(&mut self, pipeline_layout_handle: PipelineLayoutHandle, shaders: &[ShaderParameter]) -> PipelineHandle;

	/// Creates a command buffer which will execute commands on the provided queue.
	///
	/// Commands can be recorded onto it by starting a recording from a `Frame` or by calling `Device::create_command_buffer_recording` if the command buffer is not for performing per frame workloads.
	fn create_command_buffer(&mut self, name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle;

	fn create_command_buffer_recording<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> CommandBufferRecording<'a>;

	/// Creates a new static buffer.\
	/// If the access includes specifies both device and host access, staging buffers MAY be created.\
	///
	/// # Arguments
	///
	/// * `resource_uses` - The uses of the buffer.
	/// * `device_accesses` - The accesses of the buffer.
	///
	/// # Returns
	///
	/// The handle of the buffer.
	fn create_buffer<T: Copy>(&mut self, name: Option<&str>, resource_uses: Uses, device_accesses: DeviceAccesses) -> BufferHandle<T>;

	/// Creates a new dynamic buffer. Which can be updated every frame.\
	/// If the access specifies both device and host access, staging buffers MAY be created.\
	///
	/// # Arguments
	///
	/// * `resource_uses` - The uses of the buffer.
	/// * `device_accesses` - The accesses of the buffer.
	///
	/// # Returns
	///
	/// The handle of the buffer.
	fn create_dynamic_buffer<T: Copy>(&mut self, name: Option<&str>, resource_uses: Uses, device_accesses: DeviceAccesses) -> DynamicBufferHandle<T>;

	/// Returns a device accessible address for the provided buffer handle.
	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64;

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T;

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: BufferHandle<T>) -> &'a mut T;

	fn get_texture_slice_mut(&mut self, texture_handle: ImageHandle) -> &'static mut [u8];

	/// Enables writing to a texture and queues a copy operation for it.
	/// Texture must still be synchronized by calling `sync` on a command buffer.
	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8]));

	/// Creates an image.
	///
	/// # Arguments
	///
	/// * `extent` - The size of the image. Can be 0 to skip eager allocation, such as for framebuffers.
	/// * `format` - The format of the image.
	fn create_image(&mut self, name: Option<&str>, extent: Extent, format: Formats, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases, array_layers: Option<NonZeroU32>) -> ImageHandle;

	/// Creates an image from a builder.
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle;

	/// Creates an image sampler.
	///
	/// Samplers are limited on multiple devices so you are encouraged to reuse them.
	fn create_sampler(&mut self, filtering_mode: FilteringModes, reduction_mode: SamplingReductionModes, mip_map_mode: FilteringModes, addressing_mode: SamplerAddressingModes, anisotropy: Option<f32>, min_lod: f32, max_lod: f32) -> SamplerHandle;

	/// Creates an image sampler from a builder.
	///
	/// Sampler builders are limited on multiple devices so you are encouraged to reuse them.
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle;

	fn create_acceleration_structure_instance_buffer(&mut self, name: Option<&str>, max_instance_count: u32) -> BaseBufferHandle;

	fn create_top_level_acceleration_structure(&mut self, name: Option<&str>, max_instance_count: u32) -> TopLevelAccelerationStructureHandle;
	fn create_bottom_level_acceleration_structure(&mut self, description: &BottomLevelAccelerationStructure) -> BottomLevelAccelerationStructureHandle;

	/// Writes descriptor set updates.
	fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]);

	fn write_instance(&mut self, instances_buffer_handle: BaseBufferHandle, instance_index: usize, transform: [[f32; 4]; 3], custom_index: u16, mask: u8, sbt_record_offset: usize, acceleration_structure: BottomLevelAccelerationStructureHandle);

	fn write_sbt_entry(&mut self, sbt_buffer_handle: BaseBufferHandle, sbt_record_offset: usize, pipeline_handle: PipelineHandle, shader_handle: ShaderHandle);

	/// Associates a swapchain with a window.
	fn bind_to_window(&mut self, window_os_handles: &window::OSHandles, presentation_mode: PresentationModes, fallback_extent: Extent) -> SwapchainHandle;

	fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8];

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle;

	/// Starts a new frame by waiting for these sequence frame's synchronizers.
	/// The returned frame allows safe access to the frame's resources and it's operations.
	fn start_frame<'a>(&'a mut self, index: u32, synchronizer_handle: SynchronizerHandle) -> Frame<'a>;

	/// Resizes a buffer to the specified size.
	/// Does nothing if the buffer is already the specified size.
	/// May not reallocate if a smaller size is requested.
	fn resize_buffer(&mut self, buffer_handle: BaseBufferHandle, size: usize);

	/// Starts capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn start_frame_capture(&self);

	/// Ends capturing the underlying's API calls if the application is attached to a graphics debugger.
	/// Must only be called after start_frame_capture.
	fn end_frame_capture(&self);

	/// Waits for all pending operations to complete.
	/// Usually called before destroying the device or before doing a complex operation.
	/// Should be rarely called.
	fn wait(&self);
}
