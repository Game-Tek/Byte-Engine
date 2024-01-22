//! The [`GraphicsHardwareInterface`] implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use crate::{window_system, Extent};

/// Possible types of a shader source
pub enum ShaderSource<'a> {
	/// GLSL code string
	GLSL(String),
	/// SPIR-V binary
	SPIRV(&'a [u8]),
}

/// Primitive GPU/shader data types.
#[derive(Hash, Clone, Copy)]
pub enum DataTypes {
	Float,
	Float2,
	Float3,
	Float4,
	U8,
	U16,
	U32,
	Int,
	Int2,
	Int3,
	Int4,
	UInt,
	UInt2,
	UInt3,
	UInt4,
}

#[derive(Hash)]
pub struct VertexElement {
	pub name: String,
	pub format: DataTypes,
	pub binding: u32,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct DeviceAccesses: u16 {
		const CpuRead = 1 << 0;
		const CpuWrite = 1 << 1;
		const GpuRead = 1 << 2;
		const GpuWrite = 1 << 3;
	}
}

// HANDLES

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BaseBufferHandle(pub(super) u64);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BufferHandle<T>(pub(super) u64, std::marker::PhantomData<T>);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandBufferHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ShaderHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeshHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SynchronizerHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetTemplateHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetBindingHandle(pub(super) u64);

/// Handle to a Pipeline Layout
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineLayoutHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SamplerHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AllocationHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureCopyHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handle {
	Buffer(BaseBufferHandle),
	// AccelerationStructure(AccelerationStructureHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	CommandBuffer(CommandBufferHandle),
	Shader(ShaderHandle),
	Pipeline(PipelineHandle),
	Image(ImageHandle),
	Mesh(MeshHandle),
	Synchronizer(SynchronizerHandle),
	DescriptorSetLayout(DescriptorSetTemplateHandle),
	DescriptorSet(DescriptorSetHandle),
	PipelineLayout(PipelineLayoutHandle),
	Sampler(SamplerHandle),
	Swapchain(SwapchainHandle),
	Allocation(AllocationHandle),
	TextureCopy(TextureCopyHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

// HANDLES

pub struct Consumption {
	pub handle: Handle,
	pub stages: Stages,
	pub access: AccessPolicies,
	pub layout: Layouts,
}

pub enum BottomLevelAccelerationStructureBuildDescriptions {
	Mesh {
		vertex_buffer: BufferStridedRange,
		vertex_count: u32,
		vertex_position_encoding: Encodings,
		index_buffer: BufferStridedRange,
		triangle_count: u32,
		index_format: DataTypes,
	},
	AABB {
		aabb_buffer: BaseBufferHandle,
		transform_buffer: BaseBufferHandle,
		transform_count: u32,
	},
}

pub enum TopLevelAccelerationStructureBuildDescriptions {
	Instance {
		instances_buffer: BaseBufferHandle,
		instance_count: u32,
	},
}

pub struct BottomLevelAccelerationStructureBuild {
	pub acceleration_structure: BottomLevelAccelerationStructureHandle,
	pub scratch_buffer: BufferDescriptor,
	pub description: BottomLevelAccelerationStructureBuildDescriptions,
}

pub struct TopLevelAccelerationStructureBuild {
	pub acceleration_structure: TopLevelAccelerationStructureHandle,
	pub scratch_buffer: BufferDescriptor,
	pub description: TopLevelAccelerationStructureBuildDescriptions,
}

pub struct BufferStridedRange {
	pub buffer: BaseBufferHandle,
	pub offset: usize,
	pub stride: usize,
	pub size: usize,
}

pub struct BindingTables {
	pub raygen: BufferStridedRange,
	pub hit: BufferStridedRange,
	pub miss: BufferStridedRange,
	pub callable: Option<BufferStridedRange>,
}

pub struct DispatchExtent {
	workgroup_extent: Extent,
	dispatch_extent: Extent,
}

impl DispatchExtent {
	pub fn new(dispatch_extent: Extent, workgroup_extent: Extent) -> Self {
		Self {
			workgroup_extent,
			dispatch_extent,
		}
	}

	pub fn get_extent(&self) -> Extent {
		Extent {
			width: self.dispatch_extent.width.div_ceil(self.workgroup_extent.width),
			height: self.dispatch_extent.height.div_ceil(self.workgroup_extent.height),
			depth: self.dispatch_extent.depth.div_ceil(self.workgroup_extent.depth),
		}
	}
}

pub enum BottomLevelAccelerationStructureDescriptions {
	Mesh {
		vertex_count: u32,
		vertex_position_encoding: Encodings,
		triangle_count: u32,
		index_format: DataTypes,
	},
	AABB {
		transform_count: u32,
	},
}

pub struct BottomLevelAccelerationStructure {
	pub description: BottomLevelAccelerationStructureDescriptions,
}

pub trait CommandBufferRecording {
	/// Enables recording on the command buffer.
	fn begin(&self);

	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &TopLevelAccelerationStructureBuild);
	fn build_bottom_level_acceleration_structures(&mut self, acceleration_structure_builds: &[BottomLevelAccelerationStructureBuild]);

	/// Starts a render pass on the GPU.
	/// A render pass is a particular configuration of render targets which will be used simultaneously to render certain imagery.
	fn start_render_pass(&mut self, extent: Extent, attachments: &[AttachmentInformation]) -> &mut dyn RasterizationRenderPassMode;

	/// Binds a shader to the GPU.
	fn bind_shader(&self, shader_handle: ShaderHandle);

	/// Writes to the push constant register.
	fn write_to_push_constant(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, offset: u32, data: &[u8]);

	unsafe fn consume_resources(&mut self, handles: &[Consumption]);

	fn clear_images(&mut self, textures: &[(ImageHandle, ClearValue)]);
	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]);

	fn transfer_textures(&mut self, texture_handles: &[ImageHandle]);

	/// Copies imaeg data from a CPU accessible buffer to a GPU accessible image.
	fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]);

	fn bind_compute_pipeline(&mut self, pipeline_handle: &PipelineHandle) -> &mut dyn BoundComputePipelineMode;

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: &PipelineHandle) -> &mut dyn BoundRayTracingPipelineMode;

	/// Ends recording on the command buffer.
	fn end(&mut self);

	/// Binds a decriptor set on the GPU.
	fn bind_descriptor_sets(&mut self, pipeline_layout: &PipelineLayoutHandle, sets: &[DescriptorSetHandle]) -> &mut dyn CommandBufferRecording;

	fn copy_to_swapchain(&mut self, source_texture_handle: ImageHandle, present_image_index: u32 ,swapchain_handle: SwapchainHandle);

	fn sync_textures(&mut self, texture_handles: &[ImageHandle]) -> Vec<TextureCopyHandle>;

	fn execute(&mut self, wait_for_synchronizer_handles: &[SynchronizerHandle], signal_synchronizer_handles: &[SynchronizerHandle], execution_synchronizer_handle: SynchronizerHandle);

	fn start_region(&self, name: &str);
	
	fn end_region(&self);
}

pub trait RasterizationRenderPassMode: CommandBufferRecording {
	/// Binds a pipeline to the GPU.
	fn bind_raster_pipeline(&mut self, pipeline_handle: &PipelineHandle) -> &mut dyn BoundRasterizationPipelineMode;

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]);

	fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor);

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self);
}

pub trait BoundRasterizationPipelineMode: RasterizationRenderPassMode {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &MeshHandle);

	fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32);

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32);
}

pub trait BoundComputePipelineMode: CommandBufferRecording {
	fn dispatch(&mut self, dispatch: DispatchExtent);

	fn indirect_dispatch(&mut self, buffer_descriptor: &BufferDescriptor);
}

pub trait BoundRayTracingPipelineMode: CommandBufferRecording {
	fn trace_rays(&mut self, binding_tables: BindingTables, x: u32, y: u32, z: u32);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ranges {
	Size(usize),
	Whole,
}

pub enum Descriptor {
	Buffer {
		handle: BaseBufferHandle,
		size: Ranges,
	},
	Image {
		handle: ImageHandle,
		layout: Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
	Swapchain(SwapchainHandle),
	Sampler(SamplerHandle),
}

pub enum UseCases {
	STATIC,
	DYNAMIC
}

#[derive(Clone,)]
pub struct ShaderBindingDescriptor {
	pub(crate) set: u32,
	pub(crate) binding: u32,
	pub(crate) access: AccessPolicies,
}

impl ShaderBindingDescriptor {
	pub fn new(set: u32, binding: u32, access: AccessPolicies) -> Self {
		Self {
			set,
			binding,
			access,
		}
	}
}

/// Configuration for which features to request from the underlying API.
pub struct Features {
	pub validation: bool,
	pub ray_tracing: bool,
}

pub struct BufferSplitter<'a> {
	buffer: &'a mut [u8],
	offset: usize,
}

impl<'a> BufferSplitter<'a> {
	pub fn new(buffer: &'a mut [u8], offset: usize) -> Self {
		Self {
			buffer,
			offset,
		}
	}

	pub fn take(&mut self, size: usize) -> &'a mut [u8] {
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		unsafe { std::mem::transmute(buffer) }
	}
}

pub trait GraphicsHardwareInterface {
	/// Returns whether the underlying API has encountered any errors. Used during tests to assert whether the validation layers have caught any errors.
	fn has_errors(&self) -> bool;

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: Uses, resource_device_accesses: DeviceAccesses) -> AllocationHandle;

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[VertexElement]) -> MeshHandle;

	/// Creates a shader.
	fn create_shader(&mut self, shader_source_type: ShaderSource, stage: ShaderTypes, shader_binding_descriptors: &[ShaderBindingDescriptor],) -> ShaderHandle;

	fn create_descriptor_set_template(&mut self, name: Option<&str>, binding_templates: &[DescriptorSetBindingTemplate]) -> DescriptorSetTemplateHandle;

	fn create_descriptor_set(&mut self, name: Option<&str>, descriptor_set_template_handle: &DescriptorSetTemplateHandle) -> DescriptorSetHandle;

	fn create_descriptor_binding(&mut self, descriptor_set: DescriptorSetHandle, binding_template: &DescriptorSetBindingTemplate) -> DescriptorSetBindingHandle;

	fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]);

	fn create_pipeline_layout(&mut self, descriptor_set_template_handles: &[DescriptorSetTemplateHandle], push_constant_ranges: &[PushConstantRange]) -> PipelineLayoutHandle;

	fn create_raster_pipeline(&mut self, pipeline_blocks: &[PipelineConfigurationBlocks]) -> PipelineHandle;

	fn create_compute_pipeline(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, shader_parameter: ShaderParameter) -> PipelineHandle;

	fn create_ray_tracing_pipeline(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, shaders: &[ShaderParameter]) -> PipelineHandle;

	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle;

	fn create_command_buffer_recording(&self, command_buffer_handle: CommandBufferHandle, frame_index: Option<u32>) -> Box<dyn CommandBufferRecording + '_>;

	/// Creates a new buffer.\
	/// If the access includes [`DeviceAccesses::CpuWrite`] and [`DeviceAccesses::GpuRead`] then multiple buffers will be created, one for each frame.\
	/// Staging buffers MAY be created if there's is not sufficient CPU writable, fast GPU readable memory.\
	/// 
	/// # Arguments
	/// 
	/// * `size` - The size of the buffer in bytes.
	/// * `resource_uses` - The uses of the buffer.
	/// * `device_accesses` - The accesses of the buffer.
	/// 
	/// # Returns
	/// 
	/// The handle of the buffer.
	fn create_buffer(&mut self, name: Option<&str>, size: usize, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases) -> BaseBufferHandle;

	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64;

	fn get_buffer_slice(&mut self, buffer_handle: BaseBufferHandle) -> &[u8];

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice(&self, buffer_handle: BaseBufferHandle) -> &mut [u8];

	fn get_texture_slice_mut(&self, texture_handle: ImageHandle) -> &mut [u8];

	/// Creates an image.
	fn create_image(&mut self, name: Option<&str>, extent: crate::Extent, format: Formats, compression: Option<CompressionSchemes>, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases) -> ImageHandle;

	fn create_sampler(&mut self, filtering_mode: FilteringModes, mip_map_mode: FilteringModes, addressing_mode: SamplerAddressingModes, anisotropy: Option<f32>, min_lod: f32, max_lod: f32) -> SamplerHandle;

	fn create_acceleration_structure_instance_buffer(&mut self, name: Option<&str>, max_instance_count: u32) -> BaseBufferHandle;

	fn create_top_level_acceleration_structure(&mut self, name: Option<&str>,) -> TopLevelAccelerationStructureHandle;
	fn create_bottom_level_acceleration_structure(&mut self, description: &BottomLevelAccelerationStructure) -> BottomLevelAccelerationStructureHandle;

	fn write_instance(&mut self, instances_buffer_handle: BaseBufferHandle, instance_index: usize, transform: [[f32; 4]; 3], custom_index: u16, mask: u8, sbt_record_offset: usize, acceleration_structure: BottomLevelAccelerationStructureHandle);

	fn write_sbt_entry(&mut self, sbt_buffer_handle: BaseBufferHandle, sbt_record_offset: usize, pipeline_handle: PipelineHandle, shader_handle: ShaderHandle);

	fn bind_to_window(&mut self, window_os_handles: &window_system::WindowOsHandles) -> SwapchainHandle;

	fn get_image_data(&self, texture_copy_handle: TextureCopyHandle) -> &[u8];

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle;

	/// Acquires an image from the swapchain as to have it ready for presentation.
	/// 
	/// # Arguments
	/// 
	/// * `frame_handle` - The frame to acquire the image for. If `None` is passed, the image will be acquired for the next frame.
	/// * `synchronizer_handle` - The synchronizer to wait for before acquiring the image. If `None` is passed, the image will be acquired immediately.
	///
	/// # Panics
	///
	/// Panics if .
	fn acquire_swapchain_image(&self, swapchain_handle: SwapchainHandle, synchronizer_handle: SynchronizerHandle) -> u32;

	fn present(&self, image_index: u32, swapchains: &[SwapchainHandle], synchronizer_handle: SynchronizerHandle);

	fn wait(&self, synchronizer_handle: SynchronizerHandle);

	fn start_frame_capture(&self);

	fn end_frame_capture(&self);

	fn get_splitter(&self, buffer_handle: BaseBufferHandle, offset: usize) -> BufferSplitter;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RGBAu8 {
	r: u8,
	g: u8,
	b: u8,
	a: u8,
}

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
#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub enum ShaderTypes {
	/// A vertex shader.
	Vertex,
	/// A fragment shader.
	Fragment,
	/// A compute shader.
	Compute,
	Task,
	Mesh,
	RayGen,
	ClosestHit,
	AnyHit,
	Intersection,
	Miss,
	Callable,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Encodings {
	IEEE754,
	UnsignedNormalized,
	SignedNormalized,
}

#[derive(PartialEq, Eq, Clone, Copy)]
/// Enumerates the formats that textures can have.
pub enum Formats {
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
	U32,
	R8(Encodings),
	R16(Encodings),
	R32(Encodings),
	RG16(Encodings),
	RGB16(Encodings),
	RGBA16(Encodings),
}

#[derive(Clone, Copy)]
pub enum CompressionSchemes {
	BC7,
}

#[derive(Clone, Copy)]
/// Stores the information of a memory region.
pub struct Memory<'a> {
	/// The allocation that the memory region is associated with.
	allocation: &'a AllocationHandle,
	/// The offset of the memory region.
	offset: usize,
	/// The size of the memory region.
	size: usize,
}

#[derive(Clone, Copy)]
pub enum ClearValue {
	None,
	Color(crate::RGBA),
	Integer(u32, u32, u32, u32),
	Depth(f32),
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentInformation {
	/// The image view of the attachment.
	pub(crate) image: ImageHandle,
	/// The format of the attachment.
	pub(crate) format: Formats,
	/// The layout of the attachment.
	pub(crate) layout: Layouts,
	/// The clear color of the attachment.
	pub(crate) clear: ClearValue,
	/// Whether to load the contents of the attchment when starting a render pass.
	pub(crate) load: bool,
	/// Whether to store the contents of the attachment when ending a render pass.
	pub(crate) store: bool,
}

impl AttachmentInformation {
	pub fn new(image: ImageHandle, format: Formats, layout: Layouts, clear: ClearValue, load: bool, store: bool) -> Self {
		Self {
			image,
			format,
			layout,
			clear,
			load,
			store,
		}
	}

}

#[derive(Clone, Copy)]
/// Stores the information of a image copy.
pub struct ImageCopy {
	/// The source image.
	pub(super) source: ImageHandle,
	pub(super) source_format: Formats,
	/// The destination image.
	pub(super) destination: ImageHandle,
	pub(super) destination_format: Formats,
	/// The images extent.
	pub(super) extent: crate::Extent,
}

#[derive(Clone, Copy)]
/// Stores the information of a buffer copy.
pub struct BufferCopy {
	/// The source buffer.
	pub(super)	source: BaseBufferHandle,
	/// The destination buffer.
	pub(super)	destination: BaseBufferHandle,
	/// The size of the copy.
	pub(super) size: usize,
}

use serde::{Serialize, Deserialize};

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, Debug)]
	/// Bit flags for the available access policies.
	pub struct AccessPolicies : u8 {
		/// Will perform read access.
		const READ = 0b00000001;
		/// Will perform write access.
		const WRITE = 0b00000010;
	}
}

#[derive(Clone, Copy)]
pub struct TextureState {
	/// The layout of the resource.
	pub layout: Layouts,
}

#[derive(Clone, Copy)]
/// Stores the information of a barrier.
pub enum Barrier {
	/// An image barrier.
	Image(ImageHandle),
	/// A buffer barrier.
	Buffer(BaseBufferHandle),
	/// A memory barrier.
	Memory,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, Debug)]
	/// Bit flags for the available pipeline stages.
	pub struct Stages : u64 {
		/// No stage.
		const NONE = 0b0;
		/// The vertex stage.
		const VERTEX = 0b1;
		/// The mesh shader execution stage.
		const MESH = 0b10;
		/// The fragment stage.
		const FRAGMENT = 0b100;
		/// The compute stage.
		const COMPUTE = 0b1000;
		/// The transfer stage.
		const TRANSFER = 0b10000;
		/// The presentation stage.
		const PRESENTATION = 0b1000000;
		/// The host stage.
		const HOST = 0b10000000;
		/// The shader write stage.
		const SHADER_WRITE = 0b1000000000;
		/// The task stage.
		const TASK = 0b100000000000;
		/// The ray generation stage.
		const RAYGEN = 0b100000000000;
		/// The closest hit stage.
		const CLOSEST_HIT = 0b1000000000000;
		/// The any hit stage.
		const ANY_HIT = 0b10000000000000;
		/// The intersection stage.
		const INTERSECTION = 0b100000000000000;
		/// The miss stage.
		const MISS = 0b1000000000000000;
		/// The callable stage.
		const CALLABLE = 0b10000000000000000;
		/// The acceleration structure build stage.
		const ACCELERATION_STRUCTURE_BUILD = 0b100000000000000000;
	}
}

#[derive(Clone, Copy)]
/// Stores the information of a transition state.
pub struct TransitionState {
	/// The stages this transition will either wait or block on.
	pub stage: Stages,
	/// The type of access that will be done on the resource by the process the operation that requires this transition.
	pub access: AccessPolicies,
	pub layout: Layouts,
}

/// Stores the information of a barrier descriptor.
#[derive(Clone, Copy)]
pub struct BarrierDescriptor {
	/// The barrier.
	pub barrier: Barrier,
	/// The state of the resource previous to the barrier. If None, the resource state will be discarded.
	pub source: Option<TransitionState>,
	/// The state of the resource after the barrier.
	pub destination: TransitionState,
}

bitflags::bitflags! {
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
		/// Resource will be used as an image.
		const Image = 1 << 5;
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
		/// Resource will be used as a shader binding table.
		const ShaderBindingTable = 1 << 11;
		/// Resource will be used as a acceleration structure build scratch buffer.
		const AccelerationStructureBuildScratch = 1 << 12;
		
		const AccelerationStructureBuild = 1 << 13;
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enumerates the available layouts.
pub enum Layouts {
	/// The layout is undefined. We don't mind what the layout is.
	Undefined,
	/// The image will be used as render target.
	RenderTarget,
	/// The resource will be used in a transfer operation.
	Transfer,
	/// The resource will be used as a presentation source.
	Present,
	/// The resource will be used as a read only sample source.
	Read,
	/// The resource will be used as a read/write storage.
	General,
	/// The resource will be used as a shader binding table.
	ShaderBindingTable,
	/// Indirect.
	Indirect,
}

#[derive(Clone, Copy)]
/// Enumerates the available descriptor types.
pub enum DescriptorType {
	/// A uniform buffer.
	UniformBuffer,
	/// A storage buffer.
	StorageBuffer,
	/// An image.
	SampledImage,
	/// A combined image sampler.
	CombinedImageSampler,
	/// A storage image.
	StorageImage,
	/// A sampler.
	Sampler,
	/// An acceleration structure.
	AccelerationStructure,
}

#[derive(Clone, Copy)]
/// Enumerates the available filtering modes, primarily used in samplers.
pub enum FilteringModes {
	/// Closest mode filtering. Rounds floating point coordinates to the nearest pixel.
	Closest,
	/// Linear mode filtering. Blends samples linearly across neighbouring pixels.
	Linear,
}

#[derive(Clone, Copy)]
/// Enumerates the available sampler addressing modes.
pub enum SamplerAddressingModes {
	/// Repeat mode addressing.
	Repeat,
	/// Mirror mode addressing.
	Mirror,
	/// Clamp mode addressing.
	Clamp,
}

/// Stores the information of a descriptor set layout binding.
#[derive(Clone)]
pub struct DescriptorSetBindingTemplate {
	/// The binding of the descriptor set layout binding.
	pub(crate) binding: u32,
	/// The descriptor type of the descriptor set layout binding.
	pub(crate) descriptor_type: DescriptorType,
	/// The number of descriptors in the descriptor set layout binding.
	pub(crate) descriptor_count: u32,
	/// The stages the descriptor set layout binding will be used in.
	pub(crate) stages: Stages,
	/// The immutable samplers of the descriptor set layout binding.
	pub(crate) immutable_samplers: Option<Vec<SamplerHandle>>,
}

impl DescriptorSetBindingTemplate {
	pub fn new(binding: u32, descriptor_type: DescriptorType, stages: Stages,) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: 1,
			stages,
			immutable_samplers: None,
		}
	}

	pub fn new_with_immutable_samplers(binding: u32, stages: Stages, samplers: Option<Vec<SamplerHandle>>) -> Self {
		Self {
			binding,
			descriptor_type: DescriptorType::Sampler,
			descriptor_count: 1,
			stages,
			immutable_samplers: samplers,
		}
	}
}

/// Stores the information of a descriptor.
pub enum DescriptorInfo {
	/// A buffer descriptor.
	Buffer {
		/// The buffer of the descriptor.
		buffer: BaseBufferHandle,
		/// The offset to start reading from inside the buffer.
		offset: usize,
		/// How much to read from the buffer after `offset`.
		range: usize,
	},
	/// An image descriptor.
	Image {
		/// The image of the descriptor.
		image: ImageHandle,
		/// The format of the texture.
		format: Formats,
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
pub struct DescriptorWrite {
	pub(super) binding_handle: DescriptorSetBindingHandle,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub(super) array_element: u32,
	/// Information describing the descriptor.
	pub(super) descriptor: Descriptor,
}

impl DescriptorWrite {
	pub fn buffer(binding_handle: DescriptorSetBindingHandle, buffer_handle: BaseBufferHandle) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::Buffer {
				handle: buffer_handle,
				size: Ranges::Whole,
			},
		}
	}

	pub fn image(binding_handle: DescriptorSetBindingHandle, image_handle: ImageHandle, layout: Layouts) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::Image {
				handle: image_handle,
				layout,
			},
		}
	}

	pub fn combined_image_sampler(binding_handle: DescriptorSetBindingHandle, image_handle: ImageHandle, sampler_handle: SamplerHandle, layout: Layouts) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::CombinedImageSampler {
				image_handle: image_handle,
				sampler_handle,
				layout,
			},
		}
	}
}

/// Describes the details of the memory layout of a particular image.
pub struct ImageSubresourceLayout {
	/// The offset inside a memory region where the texture will read it's first texel from.
	pub(super) offset: u64,
	/// The size of the texture in bytes.
	pub(super) size: u64,
	/// The row pitch of the texture.
	pub(super) row_pitch: u64,
	/// The array pitch of the texture.
	pub(super) array_pitch: u64,
	/// The depth pitch of the texture.
	pub(super) depth_pitch: u64,
}

/// Describes the properties of a particular surface.
pub struct SurfaceProperties {
	/// The current extent of the surface.
	pub(super) extent: crate::Extent,
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
	pub buffer: BaseBufferHandle,
	pub offset: u64,
	pub range: u64,
	pub slot: u32,
}

pub trait SpecializationMapEntry {
	fn get_constant_id(&self) -> u32;
	fn get_size(&self) -> usize;
	fn get_data(&self) -> &[u8];
	fn get_type(&self) -> String;
}

pub struct GenericSpecializationMapEntry<T> {
	pub r#type: String,
	pub constant_id: u32,
	pub value: T,
}

impl <T> SpecializationMapEntry for GenericSpecializationMapEntry<T> {
	fn get_constant_id(&self) -> u32 {
		self.constant_id
	}

	fn get_type(&self) -> String {
		self.r#type.clone()
	}

	fn get_size(&self) -> usize {
		std::mem::size_of::<T>()
	}

	fn get_data(&self) -> &[u8] {
		unsafe { std::slice::from_raw_parts(&self.value as *const T as *const u8, std::mem::size_of::<T>()) }
	}
}

pub type ShaderParameter<'a> = (&'a ShaderHandle, ShaderTypes, Vec<Box<dyn SpecializationMapEntry>>);

pub enum PipelineConfigurationBlocks<'a> {
	VertexInput {
		vertex_elements: &'a [VertexElement]
	},
	InputAssembly {
	
	},
	RenderTargets {
		targets: &'a [AttachmentInformation],
	},
	Shaders {
		shaders: &'a [(&'a ShaderHandle, ShaderTypes, Vec<Box<dyn SpecializationMapEntry>>)],
	},
	Layout {
		layout: &'a PipelineLayoutHandle,
	}
}

pub struct PushConstantRange {
	pub offset: u32,
	pub size: u32,
}

pub enum AccelerationStructureTypes {
	TopLevel {
		instance_count: u32,
	},
	BottomLevel {
		vertex_count: u32,
		triangle_count: u32,
		vertex_position_format: DataTypes,
		index_format: DataTypes,
	},
}

#[cfg(test)]
pub(super) mod tests {
	use super::*;

	fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
		assert_eq!(pixels.len(), (extent.width * extent.height) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });

		if extent.width % 2 != 0 {
			let pixel = pixels[(extent.width / 2) as usize]; // middle top center
			assert_eq!(pixel, RGBAu8 { r: 255, g: 0, b: 0, a: 255 });
		}
		
		let pixel = pixels[(extent.width - 1) as usize]; // top right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });
		
		let pixel = pixels[(extent.width  * (extent.height - 1)) as usize]; // bottom left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 255, a: 255 });
		
		let pixel = pixels[(extent.width * extent.height - (extent.width / 2)) as usize]; // middle bottom center
		assert!(pixel == RGBAu8 { r: 0, g: 127, b: 127, a: 255 } || pixel == RGBAu8 { r: 0, g: 128, b: 127, a: 255 }); // FIX: workaround for CI, TODO: make near equal function
		
		let pixel = pixels[(extent.width * extent.height - 1) as usize]; // bottom right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
	}

	pub(crate) fn render_triangle(renderer: &mut dyn GraphicsHardwareInterface) {
		let signal = renderer.create_synchronizer(None, false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[]);

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let command_buffer_handle = renderer.create_command_buffer(None);

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = render_pass_command.bind_raster_pipeline(&pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		let texture_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], signal);

		renderer.end_frame_capture();

		renderer.wait(signal); // Wait for the render to finish before accessing the image data

		assert!(!renderer.has_errors());

		// Get image data and cast u8 slice to rgbau8
		let pixels = unsafe { std::slice::from_raw_parts(renderer.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

		check_triangle(pixels, extent);

		// let mut file = std::fs::File::create("test.png").unwrap();

		// let mut encoder = png::Encoder::new(&mut file, extent.width, extent.height);

		// encoder.set_color(png::ColorType::Rgba);
		// encoder.set_depth(png::BitDepth::Eight);

		// let mut writer = encoder.write_header().unwrap();
		// writer.write_image_data(unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4) }).unwrap();
	}

	use crate::core;

	pub(crate) fn present(renderer: &mut dyn GraphicsHardwareInterface) {
		let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let window_handle = core::spawn(window_system::Window::new("Renderer Test", extent));

		window_system.create_window(window_handle.clone(), "Renderer Test", extent, "present");

		let swapchain = renderer.bind_to_window(&window_system.get_os_handles(&window_handle));

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[]);

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::None,
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let command_buffer_handle = renderer.create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, false);
		let image_ready = renderer.create_synchronizer(None, false);

		let image_index = renderer.acquire_swapchain_image(swapchain, image_ready);

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = render_pass_command.bind_raster_pipeline(&pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		command_buffer_recording.copy_to_swapchain(render_target, image_index, swapchain);

		command_buffer_recording.execute(&[image_ready], &[render_finished_synchronizer], render_finished_synchronizer);

		renderer.present(image_index, &[swapchain], render_finished_synchronizer);

		renderer.end_frame_capture();

		renderer.wait(render_finished_synchronizer);

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut dyn GraphicsHardwareInterface) {
		let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let window_handle = core::spawn(window_system::Window::new("Renderer Test", extent));

		window_system.create_window(window_handle.clone(), "Renderer Test", extent, "present");

		let swapchain = renderer.bind_to_window(&window_system.get_os_handles(&window_handle));

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[]);

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::GpuWrite | DeviceAccesses::CpuRead, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::None,
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let command_buffer_handle = renderer.create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);
		let image_ready = renderer.create_synchronizer(None, true);

		for i in 0..2*64 {
			renderer.wait(render_finished_synchronizer);

			let image_index = renderer.acquire_swapchain_image(swapchain, image_ready);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					image: render_target,
					layout: Layouts::RenderTarget,
					format: Formats::RGBAu8,
					clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(&pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			command_buffer_recording.copy_to_swapchain(render_target, image_index, swapchain);

			let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[image_ready], &[render_finished_synchronizer], render_finished_synchronizer);

			renderer.present(image_index, &[swapchain], render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());
		}
	}

	pub(crate) fn multiframe_rendering(renderer: &mut dyn GraphicsHardwareInterface) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[]);

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let command_buffer_handle = renderer.create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, false);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			// renderer.wait(render_finished_synchronizer);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					image: render_target,
					layout: Layouts::RenderTarget,
					format: Formats::RGBAu8,
					clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(&pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			renderer.wait(render_finished_synchronizer);

			assert!(!renderer.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

			check_triangle(pixels, extent);
		}
	}

	// TODO: Test changing frames in flight count during rendering

	pub(crate) fn dynamic_data(renderer: &mut dyn GraphicsHardwareInterface) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(row_major) uniform;

			layout(push_constant) uniform PushConstants {
				mat4 matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.matrix * vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[]);

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[PushConstantRange{ offset: 0, size: 16 * 4 }]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let _buffer = renderer.create_buffer(None, 64, Uses::Storage, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::DYNAMIC);

		let command_buffer_handle = renderer.create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, false);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			// renderer.wait(render_finished_synchronizer);

			//let pointer = renderer.get_buffer_pointer(Some(frames[i % FRAMES_IN_FLIGHT]), buffer);

			//unsafe { std::ptr::copy_nonoverlapping(matrix.as_ptr(), pointer as *mut f32, 16); }

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					image: render_target,
					layout: Layouts::RenderTarget,
					format: Formats::RGBAu8,
					clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			let raster_render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = raster_render_pass_command.bind_raster_pipeline(&pipeline);
			
			let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);

			let matrix: [f32; 16] = 
				[
					angle.cos(), -angle.sin(), 0f32, 0f32,
					angle.sin(), angle.cos(), 0f32, 0f32,
					0f32, 0f32, 1f32, 0f32,
					0f32, 0f32, 0f32, 1f32,
				];

			raster_pipeline_command.write_to_push_constant(&pipeline_layout, 0, unsafe { std::slice::from_raw_parts(matrix.as_ptr() as *const u8, 16 * 4) });

			raster_pipeline_command.draw_mesh(&mesh);

			raster_render_pass_command.end_render_pass();

			let copy_texture_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			renderer.wait(render_finished_synchronizer);

			assert!(!renderer.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_image_data(copy_texture_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

			assert_eq!(pixels.len(), (extent.width * extent.height) as usize);
			
			// Track green corner as it should move through screen

			if i % 4 == 0 {
				let pixel = pixels[(extent.width * extent.height - 1) as usize]; // bottom right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 1 {
				let pixel = pixels[(extent.width - 1) as usize]; // top right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 2 {
				let pixel = pixels[0]; // top left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 3 {
				let pixel = pixels[(extent.width  * (extent.height - 1)) as usize]; // bottom left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			}
		}

		assert!(!renderer.has_errors())
	}

	pub(crate) fn descriptor_sets(renderer: &mut dyn GraphicsHardwareInterface) {
		let signal = renderer.create_synchronizer(None, false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0, binding=1) uniform UniformBufferObject {
				mat4 matrix;
			} ubo;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
				gl_Position.y *= -1.0;
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0,binding=0) uniform sampler smpl;
			layout(set=0,binding=2) uniform texture2D tex;

			void main() {
				out_color = texture(sampler2D(tex, smpl), vec2(0, 0));
			}
		";

		let vertex_shader = renderer.create_shader(ShaderSource::GLSL(vertex_shader_code.to_string()), ShaderTypes::Vertex, &[ShaderBindingDescriptor::new(0, 1, AccessPolicies::READ)]);
		let fragment_shader = renderer.create_shader(ShaderSource::GLSL(fragment_shader_code.to_string()), ShaderTypes::Fragment, &[ShaderBindingDescriptor::new(0, 0, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 2, AccessPolicies::READ)]);

		let buffer = renderer.create_buffer(None, 64, Uses::Uniform | Uses::Storage, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::DYNAMIC);

		let sampled_texture = renderer.create_image(Some("sampled texture"), crate::Extent { width: 2, height: 2, depth: 1 }, Formats::RGBAu8, None, Uses::Image, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);

		let pixels = vec![
			RGBAu8 { r: 255, g: 0, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 255, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 0, b: 255, a: 255 },
			RGBAu8 { r: 255, g: 255, b: 0, a: 255 },
		];

		let sampler =  renderer.create_sampler(FilteringModes::Closest, FilteringModes::Closest, SamplerAddressingModes::Repeat, None, 0.0f32, 0.0f32);

		let descriptor_set_layout_handle = renderer.create_descriptor_set_template(None, &[
			DescriptorSetBindingTemplate::new_with_immutable_samplers(0, Stages::FRAGMENT, Some(vec![sampler])),
			DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer,Stages::VERTEX),
			DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT),
		]);

		let descriptor_set = renderer.create_descriptor_set(None, &descriptor_set_layout_handle,);

		let sampler_binding = renderer.create_descriptor_binding(descriptor_set, &DescriptorSetBindingTemplate::new_with_immutable_samplers(0, Stages::FRAGMENT, Some(vec![sampler])));
		let ubo_binding = renderer.create_descriptor_binding(descriptor_set, &DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer,Stages::VERTEX));
		let tex_binding = renderer.create_descriptor_binding(descriptor_set, &DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT));

		renderer.write(&[
			DescriptorWrite { binding_handle: sampler_binding, array_element: 0, descriptor: Descriptor::Sampler(sampler) },
			DescriptorWrite { binding_handle: ubo_binding, array_element: 0, descriptor: Descriptor::Buffer{ handle: buffer, size: Ranges::Size(64) } },
			DescriptorWrite { binding_handle: tex_binding, array_element: 0, descriptor: Descriptor::Image{ handle: sampled_texture, layout: Layouts::Read } },
		]);

		assert!(!renderer.has_errors());

		let pipeline_layout = renderer.create_pipeline_layout(&[descriptor_set_layout_handle], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_raster_pipeline(&[
			PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			PipelineConfigurationBlocks::Shaders { shaders: &[(&vertex_shader, ShaderTypes::Vertex, vec![]), (&fragment_shader, ShaderTypes::Fragment, vec![])], },
			PipelineConfigurationBlocks::VertexInput { vertex_elements: &vertex_layout, },
			PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		let command_buffer_handle = renderer.create_command_buffer(None);

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		command_buffer_recording.write_image_data(sampled_texture, &pixels);

		let attachments = [
			AttachmentInformation {
				image: render_target,
				layout: Layouts::RenderTarget,
				format: Formats::RGBAu8,
				clear: ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let raster_render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = raster_render_pass_command.bind_raster_pipeline(&pipeline);

		raster_pipeline_command.bind_descriptor_sets(&pipeline_layout, &[descriptor_set]);

		raster_pipeline_command.draw_mesh(&mesh);

		raster_render_pass_command.end_render_pass();

		let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], signal);

		renderer.end_frame_capture();

		renderer.wait(signal); // Wait for the render to finish before accessing the texture data

		// assert colored triangle was drawn to texture
		let _pixels = renderer.get_image_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!renderer.has_errors());
	}

	pub(crate) fn ray_tracing(renderer: &mut dyn GraphicsHardwareInterface) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		// let window_handle = window_system.create_window("Renderer Test", extent, "test");
		// let swapchain = renderer.bind_to_window(&window_system.get_os_handles_2(&window_handle));

		let positions: [f32; 3 * 3] = [
			0.0, 1.0, 0.0,
			1.0, -1.0, 0.0,
			-1.0, -1.0, 0.0,
		];

		let colors: [f32; 4 * 3] = [
			1.0, 0.0, 0.0, 1.0,
			0.0, 1.0, 0.0, 1.0, 
			0.0, 0.0, 1.0, 1.0,
		];

		let vertex_positions_buffer = renderer.create_buffer(None, positions.len() * 4, Uses::Storage | Uses::AccelerationStructureBuild, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);
		let vertex_colors_buffer = renderer.create_buffer(None, colors.len() * 4, Uses::Storage  | Uses::AccelerationStructureBuild, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);
		let index_buffer = renderer.create_buffer(None, 3 * 2, Uses::Storage  | Uses::AccelerationStructureBuild, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);

		renderer.get_mut_buffer_slice(vertex_positions_buffer).copy_from_slice(unsafe { std::slice::from_raw_parts(positions.as_ptr() as *const u8, positions.len() * 4) });
		renderer.get_mut_buffer_slice(vertex_colors_buffer).copy_from_slice(unsafe { std::slice::from_raw_parts(colors.as_ptr() as *const u8, colors.len() * 4) });
		renderer.get_mut_buffer_slice(index_buffer).copy_from_slice(unsafe { std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2) });

		let raygen_shader_code = "
#version 460 core
#pragma shader_stage(raygen)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(binding = 0, set = 0) uniform accelerationStructureEXT topLevelAS;
layout(binding = 1, set = 0, rgba8) uniform image2D image;

layout(location = 0) rayPayloadEXT vec3 hitValue;

void main() {
	const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
	const vec2 inUV = pixelCenter/vec2(gl_LaunchSizeEXT.xy);
	vec2 d = inUV * 2.0 - 1.0;
	d.y *= -1.0;

	uint rayFlags = gl_RayFlagsOpaqueEXT;
	uint cullMask = 0xff;
	float tmin = 0.001;
	float tmax = 10.0;

	vec3 origin = vec3(d, -1.0);
	vec3 direction = vec3(0.0, 0.0, 1.0);

	traceRayEXT(topLevelAS, rayFlags, cullMask, 0, 0, 0, origin, tmin, direction, tmax, 0);

	imageStore(image, ivec2(gl_LaunchIDEXT.xy), vec4(hitValue, 1.0));
}
		";

		let closest_hit_shader_code = "
#version 460 core
#pragma shader_stage(closest)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;
hitAttributeEXT vec2 attribs;

layout(binding = 2, set = 0) buffer VertexPositions { vec3 positions[3]; };
layout(binding = 3, set = 0) buffer VertexColors { vec4 colors[3]; };
layout(binding = 4, set = 0) buffer Indices { uint16_t indices[3]; };

void main() {
	const vec3 barycentricCoords = vec3(1.0f - attribs.x - attribs.y, attribs.x, attribs.y);
	ivec3 index = ivec3(indices[3 * gl_PrimitiveID], indices[3 * gl_PrimitiveID + 1], indices[3 * gl_PrimitiveID + 2]);

	vec3[3] vertex_positions = vec3[3](positions[index.x], positions[index.y], positions[index.z]);
	vec4[3] vertex_colors = vec4[3](colors[index.x], colors[index.y], colors[index.z]);

	vec3 position = vertex_positions[0] * barycentricCoords.x + vertex_positions[1] * barycentricCoords.y + vertex_positions[2] * barycentricCoords.z;
	vec4 color = vertex_colors[0] * barycentricCoords.x + vertex_colors[1] * barycentricCoords.y + vertex_colors[2] * barycentricCoords.z;

	hitValue = color.xyz;
}
		";

		let miss_shader_code = "
#version 460 core
#pragma shader_stage(miss)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;

void main() {
    hitValue = vec3(0.0, 0.0, 0.0);
}
		";

		let raygen_shader = renderer.create_shader(ShaderSource::GLSL(raygen_shader_code.to_string()), ShaderTypes::RayGen, &[ShaderBindingDescriptor::new(0, 0, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 1, AccessPolicies::WRITE)]);
		let closest_hit_shader = renderer.create_shader(ShaderSource::GLSL(closest_hit_shader_code.to_string()), ShaderTypes::ClosestHit, &[ShaderBindingDescriptor::new(0, 2, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 3, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 4, AccessPolicies::READ)]);
		let miss_shader = renderer.create_shader(ShaderSource::GLSL(miss_shader_code.to_string()), ShaderTypes::Miss, &[]);

		let top_level_acceleration_structure = renderer.create_top_level_acceleration_structure(Some("Top Level"));
		let bottom_level_acceleration_structure = renderer.create_bottom_level_acceleration_structure(&BottomLevelAccelerationStructure{
			description: BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count: 3,
				vertex_position_encoding: Encodings::IEEE754,
				triangle_count: 1,
				index_format: DataTypes::U16,
			}
		});

		let bindings = [
			DescriptorSetBindingTemplate::new(0, DescriptorType::AccelerationStructure, Stages::RAYGEN),
			DescriptorSetBindingTemplate::new(1, DescriptorType::StorageImage, Stages::RAYGEN),
			DescriptorSetBindingTemplate::new(2, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
			DescriptorSetBindingTemplate::new(3, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
			DescriptorSetBindingTemplate::new(4, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
		];

		let descriptor_set_layout_handle = renderer.create_descriptor_set_template(None, &bindings);

		let descriptor_set = renderer.create_descriptor_set(None, &descriptor_set_layout_handle);

		let acceleration_structure_binding = renderer.create_descriptor_binding(descriptor_set, &bindings[0],);
		let render_target_binding = renderer.create_descriptor_binding(descriptor_set, &bindings[1],);
		let vertex_positions_binding = renderer.create_descriptor_binding(descriptor_set, &bindings[2],);
		let vertex_colors_binding = renderer.create_descriptor_binding(descriptor_set, &bindings[3],);
		let indices_binding = renderer.create_descriptor_binding(descriptor_set, &bindings[4],);

		let render_target = renderer.create_image(None, extent, Formats::RGBAu8, None, Uses::Storage, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::DYNAMIC);

		renderer.write(&[
			DescriptorWrite { binding_handle: acceleration_structure_binding, array_element: 0, descriptor: Descriptor::AccelerationStructure{ handle: top_level_acceleration_structure } },
			DescriptorWrite { binding_handle: render_target_binding, array_element: 0, descriptor: Descriptor::Image { handle: render_target, layout: Layouts::General } },
			DescriptorWrite { binding_handle: vertex_positions_binding, array_element: 0, descriptor: Descriptor::Buffer { handle: vertex_positions_buffer, size: Ranges::Size(12 * 3) } },
			DescriptorWrite { binding_handle: vertex_colors_binding, array_element: 0, descriptor: Descriptor::Buffer { handle: vertex_colors_buffer, size: Ranges::Size(16 * 3) } },
			DescriptorWrite { binding_handle: indices_binding, array_element: 0, descriptor: Descriptor::Buffer { handle: index_buffer, size: Ranges::Size(2 * 3) } },
		]);

		let pipeline_layout = renderer.create_pipeline_layout(&[descriptor_set_layout_handle], &[]);

		let pipeline = renderer.create_ray_tracing_pipeline(
			&pipeline_layout,
			&[(&raygen_shader, ShaderTypes::RayGen, vec![]), (&closest_hit_shader, ShaderTypes::ClosestHit, vec![]), (&miss_shader, ShaderTypes::Miss, vec![])],
		);

		let building_command_buffer_handle = renderer.create_command_buffer(None);
		let rendering_command_buffer_handle = renderer.create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, false);
		// let image_ready = renderer.create_synchronizer(true);

		let instances_buffer = renderer.create_acceleration_structure_instance_buffer(None, 1);

		renderer.write_instance(instances_buffer, 0, [[1f32, 0f32,  0f32, 0f32], [0f32, 1f32,  0f32, 0f32], [0f32, 0f32,  1f32, 0f32]], 0, 0xFF, 0, bottom_level_acceleration_structure);

		let build_sync = renderer.create_synchronizer(None, true);

		let scratch_buffer = renderer.create_buffer(None, 1024 * 1024, Uses::AccelerationStructureBuildScratch, DeviceAccesses::GpuWrite, UseCases::STATIC);

		let raygen_sbt_buffer = renderer.create_buffer(None, 64, Uses::ShaderBindingTable, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);
		let miss_sbt_buffer = renderer.create_buffer(None, 64, Uses::ShaderBindingTable, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);
		let hit_sbt_buffer = renderer.create_buffer(None, 64, Uses::ShaderBindingTable, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);

		renderer.write_sbt_entry(raygen_sbt_buffer, 0, pipeline, raygen_shader);
		renderer.write_sbt_entry(miss_sbt_buffer, 0, pipeline, miss_shader);
		renderer.write_sbt_entry(hit_sbt_buffer, 0, pipeline, closest_hit_shader);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			// {
			// 	renderer.wait(build_sync);

			// 	let mut command_buffer_recording = renderer.create_command_buffer_recording(building_command_buffer_handle, Some(i as u32));

			// 	command_buffer_recording.build_bottom_level_acceleration_structures(&[BottomLevelAccelerationStructureBuild {
			// 		acceleration_structure: bottom_level_acceleration_structure,
			// 		description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
			// 			vertex_buffer: BufferStridedRange { buffer: vertex_positions_buffer, offset: 0, stride: 12, size: 12 * 3 },
			// 			vertex_count: 3,
			// 			index_buffer: BufferStridedRange { buffer: index_buffer, offset: 0, stride: 2, size: 2 * 3 },
			// 			vertex_position_encoding: Encodings::IEEE754,
			// 			index_format: DataTypes::U16,
			// 			triangle_count: 1,
			// 		},
			// 		scratch_buffer: BufferDescriptor { buffer: scratch_buffer, offset: 0, range: 1024 * 512, slot: 0 },
			// 	}]);

			// 	command_buffer_recording.build_top_level_acceleration_structure(&TopLevelAccelerationStructureBuild {
			// 		acceleration_structure: top_level_acceleration_structure,
			// 		description: TopLevelAccelerationStructureBuildDescriptions::Instance {
			// 			instances_buffer,
			// 			instance_count: 1,
			// 		},
			// 		scratch_buffer: BufferDescriptor { buffer: scratch_buffer, offset: 1024 * 512, range: 1024 * 512, slot: 0 },
			// 	});

			// 	command_buffer_recording.execute(&[], &[build_sync], build_sync);
			// }

			// renderer.wait(render_finished_synchronizer);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(rendering_command_buffer_handle, Some(i as u32));

			{
				// renderer.wait(build_sync);

				// let mut command_buffer_recording = renderer.create_command_buffer_recording(building_command_buffer_handle, Some(i as u32));

				command_buffer_recording.build_bottom_level_acceleration_structures(&[BottomLevelAccelerationStructureBuild {
					acceleration_structure: bottom_level_acceleration_structure,
					description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer: BufferStridedRange { buffer: vertex_positions_buffer, offset: 0, stride: 12, size: 12 * 3 },
						vertex_count: 3,
						index_buffer: BufferStridedRange { buffer: index_buffer, offset: 0, stride: 2, size: 2 * 3 },
						vertex_position_encoding: Encodings::IEEE754,
						index_format: DataTypes::U16,
						triangle_count: 1,
					},
					scratch_buffer: BufferDescriptor { buffer: scratch_buffer, offset: 0, range: 1024 * 512, slot: 0 },
				}]);

				unsafe { command_buffer_recording.consume_resources(&[
					Consumption {
						handle: Handle::BottomLevelAccelerationStructure(bottom_level_acceleration_structure),
						stages: Stages::ACCELERATION_STRUCTURE_BUILD,
						access: AccessPolicies::READ,
						layout: Layouts::General,
					}
				]) };

				command_buffer_recording.build_top_level_acceleration_structure(&TopLevelAccelerationStructureBuild {
					acceleration_structure: top_level_acceleration_structure,
					description: TopLevelAccelerationStructureBuildDescriptions::Instance {
						instances_buffer,
						instance_count: 1,
					},
					scratch_buffer: BufferDescriptor { buffer: scratch_buffer, offset: 1024 * 512, range: 1024 * 512, slot: 0 },
				});

				// command_buffer_recording.execute(&[], &[build_sync], build_sync);
			}

			let ray_tracing_pipeline_command = command_buffer_recording.bind_ray_tracing_pipeline(&pipeline);

			ray_tracing_pipeline_command.bind_descriptor_sets(&pipeline_layout, &[descriptor_set]);

			unsafe { ray_tracing_pipeline_command.consume_resources(&[
				Consumption {
					handle: Handle::TopLevelAccelerationStructure(top_level_acceleration_structure),
					stages: Stages::RAYGEN,
					access: AccessPolicies::READ,
					layout: Layouts::General,
				},
				Consumption {
					handle: Handle::BottomLevelAccelerationStructure(bottom_level_acceleration_structure),
					stages: Stages::RAYGEN,
					access: AccessPolicies::READ,
					layout: Layouts::General,
				},
				Consumption {
					handle: Handle::Buffer(raygen_sbt_buffer),
					stages: Stages::RAYGEN,
					access: AccessPolicies::READ,
					layout: Layouts::General,
				},
				Consumption {
					handle: Handle::Buffer(miss_sbt_buffer),
					stages: Stages::RAYGEN,
					access: AccessPolicies::READ,
					layout: Layouts::General,
				},
				Consumption {
					handle: Handle::Buffer(hit_sbt_buffer),
					stages: Stages::RAYGEN,
					access: AccessPolicies::READ,
					layout: Layouts::General,
				},
			]) };

			ray_tracing_pipeline_command.trace_rays(BindingTables {
				raygen: BufferStridedRange { buffer: raygen_sbt_buffer, offset: 0, stride: 64, size: 64 },
				hit: BufferStridedRange { buffer: hit_sbt_buffer, offset: 0, stride: 64, size: 64 },
				miss: BufferStridedRange { buffer: miss_sbt_buffer, offset: 0, stride: 64, size: 64 },
				callable: None,
			}, 1920, 1080, 1);

			let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[/*build_sync,*/], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());

			renderer.wait(render_finished_synchronizer);

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_image_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };
			
			check_triangle(pixels, extent);
		}
	}
}