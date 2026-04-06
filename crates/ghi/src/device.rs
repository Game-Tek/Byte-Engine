use utils::Extent;

use crate::{
	buffer, descriptors, image,
	implementation::{CommandBufferRecording, Frame},
	pipelines::VertexElement,
	sampler,
	shader::{self, Sources},
	window, AllocationHandle, BaseBufferHandle, BindingConstructor, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferHandle, CommandBufferHandle, DescriptorSetBindingHandle,
	DescriptorSetBindingTemplate, DescriptorSetHandle, DescriptorSetTemplateHandle, DeviceAccesses, DynamicBufferHandle,
	DynamicImageHandle, ImageHandle, MeshHandle, PipelineHandle, PresentationModes, QueueHandle, SamplerHandle, ShaderHandle,
	ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, Uses,
};

/// The `Device` trait centralizes ownership of GPU resources and backend state for the graphics hardware interface.
pub trait Device
where
	Self: Sized + DeviceCreate,
{
	/// Returns whether the underlying API has encountered any errors. Used during tests to assert whether the validation layers have caught any errors.
	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool;

	/// Updates the number of maximum frames in flight.
	/// This operation creates extra resources to support the new number of frames in flight.
	/// > THIS IS AN EXPENSIVE OPERATION
	fn set_frames_in_flight(&mut self, frames: u8);

	/// Starts recording commands into an existing command buffer.
	fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> CommandBufferRecording<'a>;

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
	fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8];

	/// Starts a new frame by waiting for these sequence frame's synchronizers.
	/// The returned frame allows safe access to the frame's resources and it's operations.
	fn start_frame<'a>(&'a mut self, index: u32, synchronizer_handle: SynchronizerHandle) -> Frame<'a>;

	/// Resizes a buffer to the specified size.
	/// Does nothing if the buffer is already the specified size.
	/// May not reallocate if a smaller size is requested.
	fn resize_buffer(&mut self, buffer_handle: BaseBufferHandle, size: usize);

	/// Starts capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn start_frame_capture(&mut self);

	/// Ends capturing the underlying's API calls if the application is attached to a graphics debugger.
	/// Must only be called after start_frame_capture.
	fn end_frame_capture(&mut self);

	/// Waits for all pending operations to complete.
	/// Usually called before destroying the device or before doing a complex operation.
	/// Should be rarely called.
	fn wait(&self);
}

pub trait DeviceCreate {
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

	/// ```rust
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

	/// Creates a command buffer which will execute commands on the provided queue.
	///
	/// Commands can be recorded onto it by starting a recording from a `Frame` or by calling `Device::create_command_buffer_recording` if the command buffer is not for performing per frame workloads.
	fn create_command_buffer(&mut self, name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle;

	/// Creates a static buffer from a builder.
	fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T>;

	/// Creates a dynamic buffer from a builder.
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

/// Configuration for which features to request from the underlying API when creating a device/instance.
/// This uses a builder pattern to allow for easy configuration of the features.
///
/// # Features
/// - `validation`: Whether to enable validation layers for API use. This can provide insight into potential issues with the API usage at the expense of performance. Default is `false`.
/// - `gpu_validation`: Whether to enable on GPU validation. This can provide more extensive validation at the expense of performance. Default is `false`.
/// - `api_dump`: Whether to enable API dump. This will print all API calls to the console. Default is `false`.
/// - `ray_tracing`: Whether to enable ray tracing. This will enable ray tracing features in the API. Default is `false`.
/// - `debug_log_function`: A function to log debug messages. If none is provided, `println!` will be used. Default is `None`.
/// - `gpu`: The GPU to use. If `None`, the most appropriate(as defined during device creation) available GPU will be used. Default is `None`.
/// - `sparse`: Whether to enable sparse resources. This can provide more efficient memory usage. Default is `false`.
/// - `geometry_shader`: Whether to enable geometry shaders. This can provide more advanced rendering techniques. Default is `false`.
/// - `mesh_shading`: Whether to enable mesh shaders. This can provide more advanced rendering techniques. Default is `true`.
#[derive(Debug, Clone, Copy)]
pub struct Features {
	pub(crate) validation: bool,
	pub(crate) gpu_validation: bool,
	pub(crate) api_dump: bool,
	pub(crate) ray_tracing: bool,
	pub(crate) debug_log_function: Option<fn(&str)>,
	pub(crate) gpu: Option<&'static str>,
	pub(crate) sparse: bool,
	pub(crate) geometry_shader: bool,
	pub(crate) mesh_shading: bool,
}

impl Features {
	pub fn new() -> Self {
		Self {
			validation: false,
			gpu_validation: false,
			api_dump: false,
			ray_tracing: false,
			debug_log_function: None,
			gpu: None,
			sparse: false,
			geometry_shader: false,
			mesh_shading: true,
		}
	}

	pub fn validation(mut self, value: bool) -> Self {
		self.validation = value;
		self
	}

	pub fn gpu_validation(mut self, value: bool) -> Self {
		self.gpu_validation = value;
		self
	}

	pub fn api_dump(mut self, value: bool) -> Self {
		self.api_dump = value;
		self
	}

	pub fn ray_tracing(mut self, value: bool) -> Self {
		self.ray_tracing = value;
		self
	}

	pub fn debug_log_function(mut self, value: fn(&str)) -> Self {
		self.debug_log_function = Some(value);
		self
	}

	pub fn gpu(mut self, value: &'static str) -> Self {
		self.gpu = Some(value);
		self
	}

	pub fn sparse(mut self, value: bool) -> Self {
		self.sparse = value;
		self
	}

	pub fn geometry_shader(mut self, value: bool) -> Self {
		self.geometry_shader = value;
		self
	}

	pub fn mesh_shading(mut self, value: bool) -> Self {
		self.mesh_shading = value;
		self
	}
}
