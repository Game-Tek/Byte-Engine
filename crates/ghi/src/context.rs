use utils::Extent;

use crate::{
	buffer, descriptors, image,
	pipelines::VertexElement,
	sampler,
	shader::{self, Sources},
	window, AllocationHandle, BaseBufferHandle, BindingConstructor, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferHandle, CommandBufferHandle, DescriptorSetBindingHandle,
	DescriptorSetBindingTemplate, DescriptorSetHandle, DescriptorSetTemplateHandle, DeviceAccesses, DynamicBufferHandle,
	DynamicImageHandle, ImageHandle, MeshHandle, PipelineHandle, PresentationModes, QueueHandle, SamplerHandle, ShaderHandle,
	ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, Uses,
};

/// The `Context` trait identifies objects that own render resources created from a GPU device.
/// Its purpose is to be a "ownership context" that delineates the lifetime of GPU resources.
pub trait Context: ContextCreate {
	type Queue: crate::queue::Queue;
	type QueueReference<'a>: crate::queue::Queue
	where
		Self: 'a;
	type CommandBuffer<'a>: crate::command_buffer::CommandBuffer
	where
		Self: 'a;

	/// Returns whether the underlying API has encountered any errors.
	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool;

	/// Returns whether the GPU device supports BC block-compressed texture
	/// formats (BC5, BC7). On Apple Silicon this is always true; on Intel
	/// Macs and iOS Simulator it may be false. Callers should check this
	/// before creating any BC-compressed images or samplers.
	fn supports_bc_texture_compression(&self) -> bool;

	/// Returns an owned queue wrapper that exposes queue-local command submission.
	fn queue(&mut self, queue_handle: QueueHandle) -> Self::Queue;

	/// Returns a borrowed queue wrapper that exposes queue-local command submission.
	fn queue_reference<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::QueueReference<'a>;

	/// Returns a command-buffer wrapper that exposes command-buffer-local recording.
	fn command_buffer<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> Self::CommandBuffer<'a>;

	/// Updates the number of maximum frames in flight.
	/// This operation creates extra resources to support the new number of frames in flight.
	/// > THIS IS AN EXPENSIVE OPERATION
	fn set_frames_in_flight(&mut self, frames: u8);

	/// Returns a device accessible address for the provided buffer handle.
	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64;

	/// Returns a shared view into a typed buffer's contents.
	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T;

	/// Returns a mutable view into CPU-visible buffer contents.
	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T;

	/// Flushes or uploads pending writes for the provided buffer.
	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>);

	/// Returns mutable CPU access to an image's backing bytes.
	fn get_texture_slice_mut(&self, texture_handle: ImageHandle) -> &'static mut [u8];

	/// Flushes or uploads pending writes for the provided image.
	fn sync_texture(&mut self, image_handle: ImageHandle);

	/// Enables writing to a texture and queues a copy operation for it.
	/// Texture must still be synchronized by calling `sync` on a command buffer.
	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8]));

	/// Writes descriptor set updates.
	fn write(&mut self, descriptor_set_writes: &[descriptors::Write]);

	/// Writes one top-level acceleration-structure instance into an instance buffer.
	fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	);

	/// Writes one shader binding table entry for the provided pipeline shader.
	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	);

	/// Associates a swapchain with a window.
	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		uses: Uses,
	) -> SwapchainHandle;

	/// Returns CPU-visible bytes previously copied from an image.
	fn get_image_data(&self, texture_copy_handle: TextureCopyHandle) -> &[u8];

	/// Resizes a dynamic buffer to the specified size.
	fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize);

	/// Starts capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn start_frame_capture(&mut self);

	/// Ends capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn end_frame_capture(&mut self);

	/// Waits for all pending operations to complete.
	fn wait(&self);
}

/// The `ContextCreate` trait provides creation operations for resources owned by a GHI context.
pub trait ContextCreate {
	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: Uses,
		resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle;

	/// Uploads indexed mesh data and returns a reusable mesh handle.
	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle;

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
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = shader::BindingDescriptor>,
	) -> Result<ShaderHandle, ()>;

	/// Creates a reusable descriptor-set template from binding descriptions.
	fn create_descriptor_set_template(
		&mut self,
		name: Option<&str>,
		binding_templates: &[DescriptorSetBindingTemplate],
	) -> DescriptorSetTemplateHandle;

	/// Creates a descriptor set from a descriptor-set template.
	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		descriptor_set_template_handle: &DescriptorSetTemplateHandle,
	) -> DescriptorSetHandle;

	/// ```rust,ignore
	///	let views_data_binding = device.create_descriptor_binding(
	///		descriptor_set,
	///		ghi::BindingConstructor::buffer(&VIEWS_DATA_BINDING, views_data_buffer_handle.into()),
	/// );
	/// ```
	fn create_descriptor_binding(
		&mut self,
		descriptor_set: DescriptorSetHandle,
		binding_constructor: BindingConstructor,
	) -> DescriptorSetBindingHandle;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> PipelineHandle;

	/// Creates a compute pipeline.
	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> PipelineHandle;

	/// Creates a ray-tracing pipeline.
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> PipelineHandle;

	/// Creates a static fixed-size buffer from a builder.
	/// Static buffers are not resizable; use [`ContextCreate::build_dynamic_buffer`] when the allocation must grow.
	fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T>;

	/// Creates a dynamic buffer from a builder.
	/// Dynamic buffers can be resized with [`Context::resize_buffer`].
	fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T>;

	/// Creates a dynamic image from a builder.
	fn build_dynamic_image(&mut self, builder: image::Builder) -> DynamicImageHandle;

	/// Creates an image from a builder.
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle;

	/// Creates an image sampler from a builder.
	///
	/// Sampler builders are limited on multiple devices so you are encouraged to reuse them.
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle;

	/// Creates a buffer that stores top-level acceleration-structure instances.
	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle;

	/// Creates a top-level acceleration structure for ray tracing.
	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle;

	/// Creates a bottom-level acceleration structure from geometry descriptions.
	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle;

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle;
}
