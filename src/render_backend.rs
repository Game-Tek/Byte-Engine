//! The RenderBackend trait is the interface between the engine and the graphics API.
//! It provides an abstraction layer for the graphics API(Vulkan, DX12, etc).
//! It is used by the engine to create and manage resources, and to submit commands to the GPU.\
//! It is the lowest level abstraction for performing graphics operations.

use crate::{vulkan_render_backend, render_system::VertexElement};

/// Enumerates the types of command buffers that can be created.
pub enum CommandBufferType {
	/// A command buffer that can perform graphics operations. Draws, blits, presentations, etc.
	GRAPHICS,
	/// A command buffer that can perform compute operations. Dispatches, etc.
	COMPUTE,
	/// A command buffer that is optimized for transfer operations. Copies, etc.
	TRANSFER
}

/// Enumerates the types of buffers that can be created.
pub enum BufferType {
	/// A buffer that can be used as a vertex buffer.
	VERTEX,
	/// A buffer that can be used as an index buffer.
	INDEX,
	/// A buffer that can be used as a uniform buffer.
	UNIFORM,
	/// A buffer that can be used as a storage buffer.
	STORAGE,
	/// A buffer that can be used as an indirect buffer.
	INDIRECT
}

/// Enumerates the types of shaders that can be created.
pub enum ShaderTypes {
	/// A vertex shader.
	Vertex,
	/// A fragment shader.
	Fragment,
	/// A compute shader.
	Compute
}

#[derive(PartialEq, Eq, Clone, Copy)]
/// Enumerates the formats that textures can have.
pub enum TextureFormats {
	/// 8 bit unsigned per component normalized RGBA.
	RGBAu8,
	/// 16 bit unsigned per component normalized RGBA.
	RGBAu16,
	/// 32 bit unsigned per component normalized RGBA.
	RGBAu32,
	/// 16 bit float per component RGBA.
	RGBAf16,
	/// 32 bit float per component RGBA.
	RGBAf32,
	/// 10 bit unsigned for R, G and 11 bit unsigned for B normalized RGB.
	RGBu10u10u11,
	/// 8 bit unsigned per component normalized BGRA.
	BGRAu8,
	/// 32 bit float depth.
	Depth32,
}

#[derive(Clone, Copy)]
/// Stores the information of a buffer.
pub union Buffer {
	/// The information of a buffer.
	pub(crate) vulkan_buffer: vulkan_render_backend::Buffer,
}

#[derive(Clone, Copy)]
/// Stores the information of a descriptor set layout.
pub union DescriptorSetLayout {
	/// The information of a descriptor set layout.
	pub(crate) vulkan_descriptor_set_layout: vulkan_render_backend::DescriptorSetLayout,
}

#[derive(Clone, Copy)]
/// Stores the information of a texture.
pub union Texture {
	pub(crate) vulkan_texture: vulkan_render_backend::Texture,
}

#[derive(Clone, Copy)]
/// Stores the information of a texture view.
pub union TextureView {
	pub(crate) vulkan_texture_view: vulkan_render_backend::TextureView,
}

#[derive(Clone, Copy)]
/// Stores the information of a shader.
pub union Shader {
	/// The information of a shader.
	pub(crate) vulkan_shader: vulkan_render_backend::Shader,
}

#[derive(Clone, Copy)]
/// Stores the information of a swapchain.
pub union Swapchain {
	/// The information of a swapchain.
	pub(crate) vulkan_swapchain: vulkan_render_backend::Swapchain,
}

#[derive(Clone, Copy)]
/// Stores the information of a surface.
pub union Surface {
	/// The information of a surface.
	pub(crate) vulkan_surface: vulkan_render_backend::Surface,
}

#[derive(Clone, Copy)]
/// Stores the information of a synchronizer.
pub union Synchronizer {
	/// The information of a synchronizer.
	pub(crate) vulkan_synchronizer: vulkan_render_backend::Synchronizer,
}

#[derive(Clone, Copy)]
/// Stores the information of a pipeline layout.
pub union PipelineLayout {
	/// The information of a pipeline layout.
	pub(crate) vulkan_pipeline_layout: vulkan_render_backend::PipelineLayout,
}

#[derive(Clone, Copy)]
/// Stores the information of a pipeline.
pub union Pipeline {
	/// The information of a pipeline.
	pub(crate) vulkan_pipeline: vulkan_render_backend::Pipeline,
}

#[derive(Clone, Copy)]
/// Stores the information of a command buffer.
pub union CommandBuffer {
	/// The information of a command buffer.
	pub(crate) vulkan_command_buffer: vulkan_render_backend::CommandBuffer,
}

#[derive(Clone, Copy)]
/// Stores the information of an allocation.
pub struct Allocation {
	pub(crate) vulkan_allocation: vulkan_render_backend::Allocation,
	pub(crate) pointer: *mut u8,
}

unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

#[derive(Clone, Copy)]
/// Stores the information of a memory region.
pub struct Memory<'a> {
	/// The allocation that the memory region is associated with.
	pub allocation: &'a Allocation,
	/// The offset of the memory region.
	pub offset: usize,
	/// The size of the memory region.
	pub size: usize,
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentInformation {
	/// The texture view of the attachment.
	pub texture_view: TextureView,
	/// The format of the attachment.
	pub format: TextureFormats,
	/// The layout of the attachment.
	pub layout: Layouts,
	/// The clear color of the attachment.
	pub clear: Option<crate::RGBA>,
	/// The resource uses of the attachment.
	pub resource_use: Layouts,
	/// Whether to load the contents of the attchment when starting a render pass.
	pub load: bool,
	/// Whether to store the contents of the attachment when ending a render pass.
	pub store: bool,
}

#[derive(Clone, Copy)]
/// Stores the information of a texture copy.
pub struct TextureCopy {
	/// The source texture.
	pub source: Texture,
	pub source_format: TextureFormats,
	/// The destination texture.
	pub destination: Texture,
	pub destination_format: TextureFormats,
	/// The images extent.
	pub extent: crate::Extent,
}

#[derive(Clone, Copy)]
/// Stores the information of a memory backed resource.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	pub resource: T,
	/// The final size of the resource.
	pub size: usize,
	/// Tha alignment the resources needs when bound to a memory region.
	pub alignment: usize,
}

use bitflags::bitflags;

bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq)]
	/// Bit flags for the available access policies.
	pub struct AccessPolicies : u8 {
		/// Will perform read access.
		const READ = 0b00000001;
		/// Will perform write access.
		const WRITE = 0b00000010;
	}
}

#[derive(Clone, Copy)]
/// Stores the information of a barrier.
pub enum Barrier {
	/// A texture barrier.
	Texture(Texture),
}

bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq)]
	/// Bit flags for the available pipeline stages.
	pub struct Stages : u64 {
		/// No stage.
		const NONE = 0b00000000;
		/// The vertex stage.
		const VERTEX = 0b00000001;
		/// The fragment stage.
		const FRAGMENT = 0b00000010;
		/// The compute stage.
		const COMPUTE = 0b00000100;
		/// The transfer stage.
		const TRANSFER = 0b00001000;
	}
}

#[derive(Clone, Copy)]
/// Stores the information of a transition state.
pub struct TransitionState {
	/// The stages this transition will either wait or block on.
	pub stage: Stages,
	/// The type of access that will be done on the resource by the process the operation that requires this transition.
	pub access: AccessPolicies,
	/// The layout of the resource.
	pub layout: Layouts,
}

/// Stores the information of a barrier descriptor.
pub struct BarrierDescriptor {
	/// The barrier.
	pub barrier: Barrier,
	/// The state of the resource previous to the barrier. If None, the resource state will be discarded.
	pub source: Option<TransitionState>,
	/// The state of the resource after the barrier.
	pub destination: TransitionState,
}

bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	/// Bit flags for the available resource uses.
	pub struct Uses : u32 {
		/// Resource will be used as a vertex buffer.
		const Vertex = 1 << 0;
		/// Resource will be used as an index buffer.
		const Index = 1 << 1;
		/// Resource will be used as a uniform buffer.
		const Uniform = 1 << 2;
		/// Resource will be used as a storage buffer.
		const Storage = 1 << 3;
		/// Resource will be used as an indirect buffer.
		const Indirect = 1 << 4;
		/// Resource will be used as a texture.
		const Texture = 1 << 5;
		/// Resource will be used as a render target.
		const RenderTarget = 1 << 6;
		/// Resource will be used as a depth stencil.
		const DepthStencil = 1 << 7;
		/// Resource will be used as an acceleration structure.
		const AccelerationStructure = 1 << 8;
		/// Resource will be used as a transfer source.
		const TransferSource = 1 << 9;
		/// Resource will be used as a transfer destination.
		const TransferDestination = 1 << 10;
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enumerates the available layouts.
pub enum Layouts {
	/// The layout is undefined. We don't mind what the layout is.
	Undefined,
	/// The texture will be used as render target.
	RenderTarget,
	/// The texture will be used in a transfer operation.
	Transfer,
	/// The texture will be used as a presentation source.
	Present,
	/// The texture will be used as a read only sample source.
	Texture,
}

#[derive(Clone, Copy)]
/// Enumerates the available descriptor types.
pub enum DescriptorType {
	/// A uniform buffer.
	UniformBuffer,
	/// A storage buffer.
	StorageBuffer,
	/// A combined image sampler.
	SampledImage,
	/// A storage image.
	StorageImage,
	/// A sampler.
	Sampler
}

#[derive(Clone, Copy)]
/// Stores the information of a sampler.
pub struct Sampler {
	/// The Vulkan backend implementation object for a sampler.
	pub(crate) vulkan_sampler: vulkan_render_backend::Sampler,
}

/// Stores the information of a descriptor set layout binding.
pub struct DescriptorSetLayoutBinding {
	/// The binding of the descriptor set layout binding.
	pub binding: u32,
	/// The descriptor type of the descriptor set layout binding.
	pub descriptor_type: DescriptorType,
	/// The number of descriptors in the descriptor set layout binding.
	pub descriptor_count: u32,
	/// The stages the descriptor set layout binding will be used in.
	pub stage_flags: Stages,
	/// The immutable samplers of the descriptor set layout binding.
	pub immutable_samplers: Option<Vec<Sampler>>,
}

#[derive(Clone, Copy)]
/// Stores the information of a descriptor set.
pub union DescriptorSet {
	/// The Vulkan backend implementation object for a descriptor set.
	pub(crate) vulkan_descriptor_set: vulkan_render_backend::DescriptorSet,
}

/// Stores the information of a descriptor.
pub enum DescriptorInfo {
	/// A buffer descriptor.
	Buffer {
		/// The buffer of the descriptor.
		buffer: Buffer,
		/// The offset to start reading from inside the buffer.
		offset: u64,
		/// How much to read from the buffer after `offset`.
		range: u64,
	},
	/// A texture descriptor.
	Texture {
		/// The texture of the descriptor.
		texture: TextureView,
		/// The format of the texture.
		format: TextureFormats,
		/// The layout of the texture.
		layout: Layouts,
	},
	/// A sampler descriptor.
	Sampler {
		/// The sampler of the descriptor.
		sampler: u32,
	}
}

/// Stores the information of a descriptor set write.
pub struct DescriptorSetWrite {
	/// The descriptor set to write to.
	pub descriptor_set: DescriptorSet,
	/// The binding to write to.
	pub binding: u32,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub array_element: u32,
	/// The type of the descriptor.
	pub descriptor_type: DescriptorType,
	/// Information describing the descriptor.
	pub descriptor_info: DescriptorInfo,
}

/// Describes the details of the memory layout of a particular texture.
pub struct ImageSubresourceLayout {
	/// The offset inside a memory region where the texture will read it's first texel from.
	pub offset: u64,
	/// The size of the texture in bytes.
	pub size: u64,
	/// The row pitch of the texture.
	pub row_pitch: u64,
	/// The array pitch of the texture.
	pub array_pitch: u64,
	/// The depth pitch of the texture.
	pub depth_pitch: u64,
}

/// Describes the properties of a particular surface.
pub struct SurfaceProperties {
	/// The current extent of the surface.
	pub extent: crate::Extent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enumerates the states of a swapchain's validity for presentation.
pub enum SwapchainStates {
	/// The swapchain is valid for presentation.
	Ok,
	/// The swapchain is suboptimal for presentation.
	Suboptimal,
	/// The swapchain can't be used for presentation.
	Invalid,
}

pub struct BufferDescriptor {
	pub buffer: Buffer,
	pub offset: u64,
	pub range: u64,
	pub slot: u32,
}

pub enum PipelineConfigurationBlocks<'a> {
	VertexInput {
		vertex_elements: &'a [VertexElement]
	},
	InputAssembly {

	},
	RenderTargets {
		targets: &'a [TextureView],
	},
	Shaders {
		shaders: &'a [Shader],
	},
	Layout {
		layout: &'a PipelineLayout,
	}
}

/// The RenderBackend trait is the interface between the engine and the graphics API.
/// It provides an abstraction layer for the graphics API(Vulkan, DX12, etc).
/// It is used by the engine to create and manage resources, and to submit commands to the GPU.\
/// It is the lowest level abstraction for performing graphics operations.
pub trait RenderBackend {

	/// Creates a descriptor set layout.
	/// Descriptor set layouts are used to describe the layout of a descriptor set.
	fn create_descriptor_set_layout(&self, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetLayout;

	/// Creates a pipeline layout.
	/// Pipeline layouts are used to describe the layout of a pipeline.
	/// Pipeline layouts are composed of descriptor set layouts and push constants.
	fn create_pipeline_layout(&self, descriptor_set_layouts: &[DescriptorSetLayout]) -> PipelineLayout;

	/// Creates a descriptor set.
	/// Descriptor sets are used to describe the resources used by a shader.
	/// Descriptor sets are a particular instance of a descriptor set layout.
	fn create_descriptor_set(&self, descriptor_set_layout: &DescriptorSetLayout, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSet;

	/// Writes to a descriptor set.
	fn write_descriptors(&self, descriptor_set_writes: &[DescriptorSetWrite]);

	/// Creates a shader.
	/// Shaders are used to describe the operations performed by the GPU.
	/// 
	/// # Arguments
	/// * `stage` - The type of shader to create.
	/// * `shader` - The shader code.
	fn create_shader(&self, stage: ShaderTypes, shader: &[u8]) -> Shader;

	/// Creates a pipeline.
	/// Pipelines are used to describe the operations performed by the GPU.
	/// 
	/// # Arguments
	/// * `pipeline_layout` - The pipeline layout to use.
	/// * `shaders` - The shaders to use.
	fn create_pipeline(&self, blocks: &[PipelineConfigurationBlocks]) -> Pipeline;

	/// Allocates a region of memory.
	/// 
	/// # Arguments
	/// * `size` - The size of the memory region to allocate.
	/// * `device_accesses` - The device accesses that will be performed on the memory region.
	/// 
	/// # Returns
	/// * `Allocation` - The allocated memory region.
	fn allocate_memory(&self, size: usize, device_accesses: crate::render_system::DeviceAccesses) -> Allocation;

	/// Returns a pointer to the start of the memory region.
	/// 
	/// # Arguments
	/// * `allocation` - The allocation to get the pointer for.
	/// 
	/// # Returns
	/// * `*mut u8` - A pointer to the start of the memory region. Null if the allocation is not mapped.
	fn get_allocation_pointer(&self, allocation: &Allocation) -> *mut u8;

	/// Creates a buffer.
	/// Buffers are used to store data on the GPU.
	/// 
	/// # Arguments
	/// * `size` - The size of the buffer.
	/// * `resource_uses` - The resource uses of the buffer.
	/// 
	/// # Returns
	/// * `MemoryBackedResourceCreationResult<Buffer>` - The created buffer.
	fn create_buffer(&self, size: usize, resource_uses: Uses) -> MemoryBackedResourceCreationResult<Buffer>;

	fn get_buffer_address(&self, buffer: &Buffer) -> u64;

	/// Creates a texture.
	/// Textures are used to store images on the GPU.
	/// 
	/// # Arguments
	/// * `extent` - The extent of the texture.
	/// * `format` - The format of the texture.
	/// * `resource_uses` - The resource uses of the texture.
	/// * `device_accesses` - The device accesses that will be performed on the texture.
	/// * `access_policies` - The access policies of the texture.
	/// * `mip_levels` - The number of mip levels of the texture.
	/// 
	/// # Returns
	/// * `MemoryBackedResourceCreationResult<Texture>` - The created texture.
	fn create_texture(&self, extent: crate::Extent, format: TextureFormats, resource_uses: Uses, device_accesses: crate::render_system::DeviceAccesses, access_policies: AccessPolicies, mip_levels: u32) -> MemoryBackedResourceCreationResult<Texture>;

	/// Creates a sampler.
	/// Samplers are used to describe how textures are sampled on the GPU.
	/// 
	/// # Returns
	/// * `Sampler` - The created sampler.
	fn create_sampler(&self) -> Sampler;

	/// Gets tne memory layout of a texture.
	/// 
	/// # Arguments
	/// * `texture` - The texture to get the memory layout of.
	/// * `mip_level` - The mip level to get the memory layout of.
	/// 
	/// # Returns
	/// * `ImageSubresourceLayout` - The memory layout of the texture.
	fn get_image_subresource_layout(&self, texture: &Texture, mip_level: u32) -> ImageSubresourceLayout;

	/// Associates a buffer with a region of memory.
	/// 
	/// # Arguments
	/// * `memory` - The memory to associate the buffer with.
	/// * `resource_creation_info` - The buffer to associate with the memory.
	fn bind_buffer_memory(&self, memory: Memory, resource_creation_info: &MemoryBackedResourceCreationResult<Buffer>);

	/// Associates a texture with a region of memory.
	/// 
	/// # Arguments
	/// * `memory` - The memory to associate the texture with.
	/// * `resource_creation_info` - The texture to associate with the memory.
	fn bind_texture_memory(&self, memory: Memory, resource_creation_info: &MemoryBackedResourceCreationResult<Texture>);

	/// Creates a synchronizer.
	/// Synchronizers are used to synchronize operations between the CPU and GPU and GPU to GPU.
	/// 
	/// # Arguments
	/// * `signaled` - Whether the synchronizer should be intialized as signaled.
	fn create_synchronizer(&self, signaled: bool) -> Synchronizer;

	/// Creates a texture view.
	/// Texture views are used to describe how textures are accessed on the GPU.
	/// 
	/// # Arguments
	/// * `texture` - The texture to create the texture view for.
	/// * `format` - The format of the texture view.
	/// * `mip_levels` - The number of mip levels of the texture view.
	/// 
	/// # Returns
	/// * `TextureView` - The created texture view.
	fn create_texture_view(&self, texture: &Texture, format: TextureFormats, mip_levels: u32) -> TextureView;

	/// Creates a surface.
	/// Surfaces are used to describe the surface of a window.
	/// 
	/// # Arguments
	/// * `window_os_handles` - The OS handles of the window.
	/// 
	/// # Returns
	/// * `Surface` - The created surface.
	fn create_surface(&self, window_os_handles: crate::window_system::WindowOsHandles) -> Surface;

	/// Gets the properties of a surface.
	/// 
	/// # Arguments
	/// * `surface` - The surface to get the properties of.
	/// 
	/// # Returns
	/// * `SurfaceProperties` - The properties of the surface.
	fn get_surface_properties(&self, surface: &Surface) -> SurfaceProperties;

	/// Creates a swapchain.
	/// Swapchains are used to describe the swapchain of a window.
	/// 
	/// # Arguments
	/// * `surface` - The surface to create the swapchain for.
	/// * `extent` - The extent of the swapchain.
	/// * `buffer_count` - The number of buffers in the swapchain.
	/// 
	/// # Returns
	/// * `Swapchain` - The created swapchain.
	fn create_swapchain(&self, surface: &Surface, extent: crate::Extent, buffer_count: u32) -> Swapchain;

	/// Gets the images of a swapchain.
	/// 
	/// # Arguments
	/// * `swapchain` - The swapchain to get the images of.
	/// 
	/// # Returns
	/// * `Vec<Texture>` - The images of the swapchain.
	fn get_swapchain_images(&self, swapchain: &Swapchain) -> Vec<Texture>;

	/// Creates a command buffer.
	/// Command buffers are used to record commands to be executed on the GPU.
	/// 
	/// # Arguments
	/// 
	/// # Returns
	/// * `CommandBuffer` - The created command buffer.
	fn create_command_buffer(&self) -> CommandBuffer;

	/// Begins recording commands to a command buffer.
	/// A command buffer must be set to recording mode before commands can be recorded to it.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to begin recording commands to.
	fn begin_command_buffer_recording(&self, command_buffer: &CommandBuffer);

	/// Ends recording commands to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to end recording commands to.
	fn end_command_buffer_recording(&self, command_buffer: &CommandBuffer);

	/// Begins a render pass.
	/// Render passes are used to describe the operations performed by the GPU.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to begin the render pass on.
	/// * `extent` - The extent of the render pass.
	/// * `attachments` - The attachments of the render pass.
	fn start_render_pass(&self, command_buffer: &CommandBuffer, extent: crate::Extent, attachments: &[AttachmentInformation]);

	/// Ends a render pass.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to end the render pass on.
	fn end_render_pass(&self, command_buffer: &CommandBuffer);

	/// Binds a shader to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the shader to.
	/// * `shader` - The shader to bind.
	fn bind_shader(&self, command_buffer: &CommandBuffer, shader: &Shader);

	/// Binds a pipeline to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the pipeline to.
	/// * `pipeline` - The pipeline to bind.
	fn bind_pipeline(&self, command_buffer: &CommandBuffer, pipeline: &Pipeline);

	/// Binds a push constant to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the push constant to.
	/// * `pipeline_layout` - The pipeline layout to bind the push constant to.
	/// * `offset` - The offset of the push constant.
	/// * `data` - The data of the push constant.
	fn write_to_push_constant(&self, command_buffer: &CommandBuffer, pipeline_layout: &PipelineLayout, offset: u32, data: &[u8]);

	/// Binds a descriptor set to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the descriptor set to.
	/// * `pipeline_layout` - The pipeline layout to bind the descriptor set to.
	/// * `descriptor_set` - The descriptor set to bind.
	/// * `index` - The index of the descriptor set.
	fn bind_descriptor_set(&self, command_buffer: &CommandBuffer, pipeline_layout: &PipelineLayout, descriptor_set: &DescriptorSet, index: u32);

	/// Binds a vertex buffer to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the vertex buffer to.
	/// * `buffer` - The buffer to bind.
	fn bind_vertex_buffers(&self, command_buffer: &CommandBuffer, buffer_descriptors: &[BufferDescriptor]);

	/// Binds an index buffer to a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to bind the index buffer to.
	/// * `buffer` - The buffer to bind.
	fn bind_index_buffer(&self, command_buffer: &CommandBuffer, buffer_descriptor: &BufferDescriptor);

	/// Performs a draw call.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to perform the draw call on.
	/// * `index_count` - The number of indices to draw.
	/// * `instance_count` - The number of instances to draw.
	/// * `first_index` - The first index to draw.
	/// * `vertex_offset` - The vertex offset.
	/// * `first_instance` - The first instance to draw.
	fn draw_indexed(&self, command_buffer: &CommandBuffer, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32);

	/// Performs a barrier.
	/// Pipeline barriers synchronize memory accesses between commands.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to perform the barrier on.
	/// * `barriers` - The barriers to perform.
	fn execute_barriers(&self, command_buffer: &CommandBuffer, barriers: &[crate::render_backend::BarrierDescriptor]);

	/// Copies textures to other textures.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to perform the copy on.
	/// * `copies` - The copies to perform.
	fn copy_textures(&self, command_buffer: &CommandBuffer, copies: &[TextureCopy]);

	/// Executes a command buffer.
	/// 
	/// # Arguments
	/// * `command_buffer` - The command buffer to execute.
	/// * `wait_for` - The synchronizer to wait for.
	/// * `signal` - The synchronizer to signal.
	fn execute(&self, command_buffer: &CommandBuffer, wait_for: Option<&crate::render_backend::Synchronizer>, signal: Option<&crate::render_backend::Synchronizer>, execution_completion: &crate::render_backend::Synchronizer);

	/// Acquires an image from a swapchain.
	/// 
	/// # Arguments
	/// * `swapchain` - The swapchain to acquire the image from.
	/// * `image_available` - The synchronizer to signal when the image is available.
	/// 
	/// # Returns
	/// * `u32` - The index of the acquired image.
	fn acquire_swapchain_image(&self, swapchain: &Swapchain, image_available: &Synchronizer) -> (u32, SwapchainStates);

	/// Presents an image to a swapchain.
	/// 
	/// # Arguments
	/// * `swapchain` - The swapchain to present the image to.
	/// * `wait_for` - The synchronizer to wait for.
	/// * `image_index` - The index of the image to present.
	fn present(&self, swapchain: &Swapchain, wait_for: &Synchronizer, image_index: u32);

	/// Waits for a synchronizer.
	/// 
	/// # Arguments
	/// * `synchronizer` - The synchronizer to wait for.
	fn wait(&self, synchronizer: &Synchronizer);

	/// Return the number of logs that have been created.
	fn get_log_count(&self) -> u32;
}