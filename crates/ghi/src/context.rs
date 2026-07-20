use utils::Extent;

use crate::{
	buffer, descriptors, image,
	pipelines::VertexElement,
	sampler,
	shader::{self, Sources},
	window, AllocationHandle, BaseBufferHandle, BottomLevelAccelerationStructure, BottomLevelAccelerationStructureHandle,
	BufferHandle, CommandBufferHandle, DescriptorSetHandle, DeviceAccesses, DynamicBufferHandle, DynamicImageHandle,
	ImageHandle, MeshHandle, PipelineHandle, PresentationModes, QueueHandle, SamplerHandle, ShaderHandle, ShaderTypes,
	SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, Uses,
};

/// The `Context` trait identifies objects that own render resources created from a GPU device.
/// Implementations use the context lifetime to bound the lifetime of owned GPU resources.
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

	/// Returns whether the GPU supports BC5 and BC7 block-compressed textures.
	///
	/// Check this value before you create BC-compressed images or samplers.
	fn supports_bc_texture_compression(&self) -> bool;

	/// Returns an owned queue wrapper that exposes queue-local command submission.
	fn queue(&mut self, queue_handle: QueueHandle) -> Self::Queue;

	/// Returns a borrowed queue wrapper that exposes queue-local command submission.
	fn queue_reference<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::QueueReference<'a>;

	/// Returns a command-buffer wrapper that exposes command-buffer-local recording.
	fn command_buffer<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> Self::CommandBuffer<'a>;

	/// Changes the maximum number of frames in flight.
	///
	/// This expensive operation can create more frame resources.
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

	/// Enables writes to a texture and queues a copy operation.
	///
	/// Call `sync` on a command buffer before the GPU uses the texture.
	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8]));

	/// Updates retained descriptor-set state before command recording.
	///
	/// Rendering only binds complete retained sets; resource overrides are not recorded per draw.
	fn write(&mut self, descriptor_set_writes: &[descriptors::DescriptorWrite]);

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

	/// Returns CPU-visible bytes for an image synchronized by `transfer_textures`.
	fn get_image_data(&mut self, texture_copy_handle: TextureCopyHandle) -> &[u8];

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

	/// Creates a shader and returns its handle.
	///
	/// # Errors
	///
	/// Returns an error when GLSL compilation fails or SPIR-V input is not aligned
	/// to four bytes.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = shader::ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()>;

	/// Creates an empty retained descriptor set.
	///
	/// The set is a lifetime/update grouping only. Its shader-visible slots are established by
	/// [`Context::write`] calls and validated against the active pipeline when it is bound.
	fn create_descriptor_set(&mut self, name: Option<&str>) -> DescriptorSetHandle;

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
	/// Devices can limit their sampler count. Reuse samplers when possible.
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
