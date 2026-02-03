//! The [`GraphicsHardwareInterface`] implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use utils::{Extent, RGBA};

/// Possible types of a shader source
pub enum ShaderSource<'a> {
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

#[derive(Clone, Hash)]
pub struct VertexElement<'a> {
	pub(crate) name: &'a str,
	pub(crate) format: DataTypes,
	pub(crate) binding: u32,
}

impl <'a> VertexElement<'a> {
	pub const fn new(name: &'a str, format: DataTypes, binding: u32) -> Self {
		Self {
			name,
			format,
			binding,
		}
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct DeviceAccesses: u16 {
		#[deprecated]
		const CpuRead = 1 << 0;
		#[deprecated]
		const CpuWrite = 1 << 1;
		#[deprecated]
		const GpuRead = 1 << 2;
		#[deprecated]
		const GpuWrite = 1 << 3;

		const DeviceOnly = 1 << 2 | 1 << 3;
		const HostOnly = 1 << 0 | 1 << 1;
		const HostToDevice = 1 << 1 | 1 << 2;
		const DeviceToHost = 1 << 0 | 1 << 3;
	}
}

// HANDLES

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueHandle(pub(crate) u64);

/// The `BaseBufferHandle` allows addressing any static buffer irregardless of it's underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
pub struct BaseBufferHandle(pub(super) u64);

/// The `BufferHandle` allows addressing a buffer static buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BufferHandle<T>(pub(super) u64, pub(super) std::marker::PhantomData<T>);

/// The `DynamicBufferHandle` allows addressing a dynamic buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicBufferHandle<T>(pub(super) u64, pub(super) std::marker::PhantomData<T>);

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

impl <T: Copy> Into<BaseBufferHandle> for BufferHandle<T> {
	fn into(self) -> BaseBufferHandle {
		BaseBufferHandle(self.0)
	}
}

impl <T: Copy> Into<BaseBufferHandle> for DynamicBufferHandle<T> {
	fn into(self) -> BaseBufferHandle {
		BaseBufferHandle(self.0)
	}
}


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

impl Into<Handle> for BaseBufferHandle {
	fn into(self) -> Handle {
		Handle::Buffer(self)
	}
}

impl Into<Handle> for ImageHandle {
	fn into(self) -> Handle {
		Handle::Image(self)
	}
}

impl Into<Handle> for SynchronizerHandle {
	fn into(self) -> Handle {
		Handle::Synchronizer(self)
	}
}

// HANDLES

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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

pub struct BufferOffset {
	pub(super) buffer: BaseBufferHandle,
	pub(super) offset: usize,
}

impl BufferOffset {
	pub fn new(buffer: BaseBufferHandle, offset: usize) -> Self {
		Self {
			buffer,
			offset,
		}
	}
}

pub struct BufferStridedRange {
	pub(super) buffer_offset: BufferOffset,
	pub(super) stride: usize,
	pub(super) size: usize,
}

impl BufferStridedRange {
	pub fn new(buffer: BaseBufferHandle, offset: usize, stride: usize, size: usize) -> Self {
		Self {
			buffer_offset: BufferOffset::new(buffer, offset),
			stride,
			size,
		}
	}
}

pub struct BindingTables {
	pub raygen: BufferStridedRange,
	pub hit: BufferStridedRange,
	pub miss: BufferStridedRange,
	pub callable: Option<BufferStridedRange>,
}

/// Describes the dimesions of a dispatch operation.
pub struct DispatchExtent {
	workgroup_extent: Extent,
	dispatch_extent: Extent,
}

impl DispatchExtent {
	/// Creates a new dispatch extent.
	/// # Arguments
	/// * `dispatch_extent` - The extent of the dispatch. (How many threads to have in each dimension).
	/// * `workgroup_extent` - The extent of the workgroup. (The workgroup extent defined in the shader).
	pub fn new(dispatch_extent: Extent, workgroup_extent: Extent) -> Self {
		Self {
			workgroup_extent,
			dispatch_extent,
		}
	}

	/// Returns the extent for a dispatch operation.
	/// # Returns
	/// The extent for a dispatch operation, which is the result of dividing the dispatch extent by the workgroup extent, rounded up.
	pub fn get_extent(&self) -> Extent {
		Extent::new(self.dispatch_extent.width().div_ceil(self.workgroup_extent.width()), self.dispatch_extent.height().div_ceil(self.workgroup_extent.height()), self.dispatch_extent.depth().div_ceil(self.workgroup_extent.depth()),)
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ranges {
	Size(usize),
	Whole,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
		layer: Option<u32>,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
	Swapchain(SwapchainHandle),
	Sampler(SamplerHandle),
	StaticSamplers,
	CombinedImageSamplerArray,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UseCases {
	STATIC,
	DYNAMIC
}

#[derive(Clone,Copy)]
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

pub struct BufferSplitter<'a, T: Copy> {
	buffer: &'a mut [T],
	offset: usize,
}

impl<'a, T: Copy> BufferSplitter<'a, T> {
	pub fn new(buffer: &'a mut [T], offset: usize) -> Self {
		Self {
			buffer,
			offset,
		}
	}

	pub fn take(&mut self, size: usize) -> &'a mut [T] {
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		unsafe { std::mem::transmute(buffer) }
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FrameKey {
	/// The index of the frame.
	pub(crate) frame_index: u32,
	pub(crate) sequence_index: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PresentKey {
	/// The index of the acquired swapchain image.
	pub(crate) image_index: u8,
	/// The index corresponding to the frame index.
	pub(crate) sequence_index: u8,
	/// The swapchain handle corresponding to the presentation request that this key is associated with.
	pub(crate) swapchain: SwapchainHandle,
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
#[derive(Clone, Copy, Debug)]
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

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Encodings {
	FloatingPoint,
	UnsignedNormalized,
	SignedNormalized,
	#[allow(non_camel_case_types)]
	sRGB,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// Describes the bit layout of a format's channels.
pub enum ChannelLayout {
	/// Single channel (R).
	R,
	/// Two channels (RG).
	RG,
	/// Three channels (RGB).
	RGB,
	/// Four channels (RGBA).
	RGBA,
	/// Four channels in BGRA order.
	BGRA,
	/// Special packed format.
	Packed,
	/// Depth channel.
	Depth,
	/// Block compressed format.
	BC,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// Describes the bit size per channel.
pub enum ChannelBitSize {
	/// 8 bits per channel.
	Bits8,
	/// 16 bits per channel.
	Bits16,
	/// 32 bits per channel.
	Bits32,
	/// Special case: 11 bits for R and G, 10 bits for B.
	Bits11_11_10,
	/// Block compressed format (variable bit size).
	Compressed,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// Enumerates the formats that textures can have.
pub enum Formats {
	/// 8 bit unsigned per component floating point R.
	R8F,
	/// 8 bit unsigned normalized R.
	R8UNORM,
	/// 8 bit signed normalized R.
	R8SNORM,
	/// 8 bit sRGB R.
	R8sRGB,

	/// 16 bit unsigned per component floating point R.
	R16F,
	/// 16 bit unsigned normalized R.
	R16UNORM,
	/// 16 bit signed normalized R.
	R16SNORM,
	/// 16 bit sRGB R.
	R16sRGB,

	/// 32 bit unsigned per component floating point R.
	R32F,
	/// 32 bit unsigned normalized R.
	R32UNORM,
	/// 32 bit signed normalized R.
	R32SNORM,
	/// 32 bit sRGB R.
	R32sRGB,

	/// 8 bit unsigned per component floating point RG.
	RG8F,
	/// 8 bit unsigned normalized RG.
	RG8UNORM,
	/// 8 bit signed normalized RG.
	RG8SNORM,
	/// 8 bit sRGB RG.
	RG8sRGB,

	/// 16 bit unsigned per component floating point RG.
	RG16F,
	/// 16 bit unsigned normalized RG.
	RG16UNORM,
	/// 16 bit signed normalized RG.
	RG16SNORM,
	/// 16 bit sRGB RG.
	RG16sRGB,

	/// 8 bit unsigned per component floating point RGB.
	RGB8F,
	/// 8 bit unsigned normalized RGB.
	RGB8UNORM,
	/// 8 bit signed normalized RGB.
	RGB8SNORM,
	/// 8 bit sRGB RGB.
	RGB8sRGB,

	/// 16 bit unsigned per component floating point RGB.
	RGB16F,
	/// 16 bit unsigned normalized RGB.
	RGB16UNORM,
	/// 16 bit signed normalized RGB.
	RGB16SNORM,
	/// 16 bit sRGB RGB.
	RGB16sRGB,

	/// 8 bit unsigned per component floating point RGBA.
	RGBA8F,
	/// 8 bit unsigned normalized RGBA.
	RGBA8UNORM,
	/// 8 bit signed normalized RGBA.
	RGBA8SNORM,
	/// 8 bit sRGB RGBA.
	RGBA8sRGB,

	/// 16 bit unsigned per component floating point RGBA.
	RGBA16F,
	/// 16 bit unsigned normalized RGBA.
	RGBA16UNORM,
	/// 16 bit signed normalized RGBA.
	RGBA16SNORM,
	/// 16 bit sRGB RGBA.
	RGBA16sRGB,

	/// 11 bit unsigned for R, G and 10 bit unsigned for B normalized RGB.
	RGBu11u11u10,
	/// 8 bit unsigned per component normalized BGRA.
	BGRAu8,
	/// 32 bit float depth.
	Depth32,
	/// 32 bit unsigned integer.
	U32,
	/// BC5 block compressed format.
	BC5,
	/// BC7 block compressed format.
	BC7,
}

impl Formats {
	/// Returns the encoding of the format.
	pub fn encoding(&self) -> Option<Encodings> {
		match self {
			Formats::R8F | Formats::R16F | Formats::R32F |
			Formats::RG8F | Formats::RG16F |
			Formats::RGB8F | Formats::RGB16F |
			Formats::RGBA8F | Formats::RGBA16F |
			Formats::Depth32 => Some(Encodings::FloatingPoint),

			Formats::R8UNORM | Formats::R16UNORM | Formats::R32UNORM |
			Formats::RG8UNORM | Formats::RG16UNORM |
			Formats::RGB8UNORM | Formats::RGB16UNORM |
			Formats::RGBA8UNORM | Formats::RGBA16UNORM |
			Formats::RGBu11u11u10 | Formats::BGRAu8 => Some(Encodings::UnsignedNormalized),

			Formats::R8SNORM | Formats::R16SNORM | Formats::R32SNORM |
			Formats::RG8SNORM | Formats::RG16SNORM |
			Formats::RGB8SNORM | Formats::RGB16SNORM |
			Formats::RGBA8SNORM | Formats::RGBA16SNORM => Some(Encodings::SignedNormalized),

			Formats::R8sRGB | Formats::R16sRGB | Formats::R32sRGB |
			Formats::RG8sRGB | Formats::RG16sRGB |
			Formats::RGB8sRGB | Formats::RGB16sRGB |
			Formats::RGBA8sRGB | Formats::RGBA16sRGB => Some(Encodings::sRGB),

			Formats::U32 | Formats::BC5 | Formats::BC7 => None,
		}
	}

	/// Returns the channel bit size of the format.
	pub fn channel_bit_size(&self) -> ChannelBitSize {
		match self {
			Formats::R8F | Formats::R8UNORM | Formats::R8SNORM | Formats::R8sRGB |
			Formats::RG8F | Formats::RG8UNORM | Formats::RG8SNORM | Formats::RG8sRGB |
			Formats::RGB8F | Formats::RGB8UNORM | Formats::RGB8SNORM | Formats::RGB8sRGB |
			Formats::RGBA8F | Formats::RGBA8UNORM | Formats::RGBA8SNORM | Formats::RGBA8sRGB |
			Formats::BGRAu8 => ChannelBitSize::Bits8,

			Formats::R16F | Formats::R16UNORM | Formats::R16SNORM | Formats::R16sRGB |
			Formats::RG16F | Formats::RG16UNORM | Formats::RG16SNORM | Formats::RG16sRGB |
			Formats::RGB16F | Formats::RGB16UNORM | Formats::RGB16SNORM | Formats::RGB16sRGB |
			Formats::RGBA16F | Formats::RGBA16UNORM | Formats::RGBA16SNORM | Formats::RGBA16sRGB => ChannelBitSize::Bits16,

			Formats::R32F | Formats::R32UNORM | Formats::R32SNORM | Formats::R32sRGB |
			Formats::Depth32 | Formats::U32 => ChannelBitSize::Bits32,

			Formats::RGBu11u11u10 => ChannelBitSize::Bits11_11_10,

			Formats::BC5 | Formats::BC7 => ChannelBitSize::Compressed,
		}
	}

	/// Returns the channel layout of the format.
	pub fn channel_layout(&self) -> ChannelLayout {
		match self {
			Formats::R8F | Formats::R8UNORM | Formats::R8SNORM | Formats::R8sRGB |
			Formats::R16F | Formats::R16UNORM | Formats::R16SNORM | Formats::R16sRGB |
			Formats::R32F | Formats::R32UNORM | Formats::R32SNORM | Formats::R32sRGB => ChannelLayout::R,

			Formats::RG8F | Formats::RG8UNORM | Formats::RG8SNORM | Formats::RG8sRGB |
			Formats::RG16F | Formats::RG16UNORM | Formats::RG16SNORM | Formats::RG16sRGB => ChannelLayout::RG,

			Formats::RGB8F | Formats::RGB8UNORM | Formats::RGB8SNORM | Formats::RGB8sRGB |
			Formats::RGB16F | Formats::RGB16UNORM | Formats::RGB16SNORM | Formats::RGB16sRGB |
			Formats::RGBu11u11u10 => ChannelLayout::RGB,

			Formats::RGBA8F | Formats::RGBA8UNORM | Formats::RGBA8SNORM | Formats::RGBA8sRGB |
			Formats::RGBA16F | Formats::RGBA16UNORM | Formats::RGBA16SNORM | Formats::RGBA16sRGB => ChannelLayout::RGBA,

			Formats::BGRAu8 => ChannelLayout::BGRA,

			Formats::Depth32 => ChannelLayout::Depth,

			Formats::U32 => ChannelLayout::Packed,

			Formats::BC5 | Formats::BC7 => ChannelLayout::BC,
		}
	}
}

pub trait Size {
	fn size(&self) -> usize;
}

impl Size for Formats {
	fn size(&self) -> usize {
		match self {
			Formats::R8F | Formats::R8UNORM | Formats::R8SNORM | Formats::R8sRGB => 1,
			Formats::R16F | Formats::R16UNORM | Formats::R16SNORM | Formats::R16sRGB => 2,
			Formats::R32F | Formats::R32UNORM | Formats::R32SNORM | Formats::R32sRGB => 4,
			Formats::RG8F | Formats::RG8UNORM | Formats::RG8SNORM | Formats::RG8sRGB => 2,
			Formats::RG16F | Formats::RG16UNORM | Formats::RG16SNORM | Formats::RG16sRGB => 4,
			Formats::RGB8F | Formats::RGB8UNORM | Formats::RGB8SNORM | Formats::RGB8sRGB => 3,
			Formats::RGB16F | Formats::RGB16UNORM | Formats::RGB16SNORM | Formats::RGB16sRGB => 6,
			Formats::RGBA8F | Formats::RGBA8UNORM | Formats::RGBA8SNORM | Formats::RGBA8sRGB => 4,
			Formats::RGBA16F | Formats::RGBA16UNORM | Formats::RGBA16SNORM | Formats::RGBA16sRGB => 8,
			Formats::RGBu11u11u10 => 4,
			Formats::BGRAu8 => 4,
			Formats::Depth32 => 4,
			Formats::U32 => 4,
			Formats::BC5 => 1,
			Formats::BC7 => 1,
		}
	}
}

#[derive(Clone, Copy, Debug)]
pub enum CompressionSchemes {
	BC5,
	BC7,
}

#[derive(Clone, Copy, Debug)]
pub enum PresentationModes {
	Inmediate,
	FIFO,
	Mailbox,
}

impl Default for PresentationModes {
	fn default() -> Self {
		Self::FIFO
	}
}

#[derive(Clone, Copy)]
pub enum ClearValue {
	None,
	Color(RGBA),
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
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
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
			layer: None,
		}
	}

	pub fn layer(mut self, layer: u32) -> Self {
		self.layer = Some(layer);
		self
	}
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct PipelineAttachmentInformation {
	/// The format of the attachment.
	pub(crate) format: Formats,
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
}

impl PipelineAttachmentInformation {
	pub fn new(format: Formats,) -> Self {
		Self {
			format,
			layer: None,
		}
	}

	pub fn layer(mut self, layer: u32) -> Self {
		self.layer = Some(layer);
		self
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	/// Bit flags for the available access policies.
	pub struct AccessPolicies : u8 {
		/// Will perform no access.
		const NONE = 0b00000000;
		/// Will perform read access.
		const READ = 0b00000001;
		/// Will perform write access.
		const WRITE = 0b00000010;
		/// Will perform read and write access.
		const READ_WRITE = Self::READ.bits() | Self::WRITE.bits();
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
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	/// Bit flags for the available pipeline stages.
	pub struct Stages : u64 {
		/// No stage.
		const NONE = 0b0;
		/// The vertex stage.
		const VERTEX = 1 << 1;
		const INDEX = 1 << 2;
		/// The task stage.
		const TASK = 1 << 3;
		/// The mesh shader execution stage.
		const MESH = 1 << 4;
		/// The fragment stage.
		const FRAGMENT = 1 << 5;
		/// The compute stage.
		const COMPUTE = 1 << 6;
		/// The transfer stage.
		const TRANSFER = 1 << 7;
		/// The presentation stage.
		const PRESENTATION = 1 << 8;
		/// The host stage.
		const HOST = 1 << 9;
		/// The shader write stage.
		const SHADER_WRITE = 1 << 10;
		/// The ray generation stage.
		const RAYGEN = 1 << 11;
		/// The closest hit stage.
		const CLOSEST_HIT = 1 << 12;
		/// The any hit stage.
		const ANY_HIT = 1 << 13;
		/// The intersection stage.
		const INTERSECTION = 1 << 14;
		/// The miss stage.
		const MISS = 1 << 15;
		/// The callable stage.
		const CALLABLE = 1 << 16;
		/// The acceleration structure build stage.
		const ACCELERATION_STRUCTURE_BUILD = 1 << 17;
		/// The last or bottom stage.
		const LAST = 1 << 63;
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
		/// Resource will be used as an input attachment.
		const InputAttachment = 1 << 15;
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

		const Clear = 1 << 14;

		/// Resource will be used as a source for a blit operation.
		const BlitSource = 1 << 9;
		/// Resource will be used as a destination for a blit operation.
		const BlitDestination = 1 << 10;
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
	/// An input attachment.
	InputAttachment,
	/// A sampler.
	Sampler,
	/// An acceleration structure.
	AccelerationStructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available filtering modes, primarily used in samplers.
pub enum FilteringModes {
	/// Closest mode filtering. Rounds floating point coordinates to the nearest pixel.
	Closest,
	/// Linear mode filtering. Blends samples linearly across neighbouring pixels.
	Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available sampling reduction modes.
/// The sampling reduction mode is used to determine how to reduce/combine the samples of neighbouring texels when sampling an image.
pub enum SamplingReductionModes {
	/// The average of the samples. Weighted by the proximity of the sample to the sample point.
	WeightedAverage,
	/// The minimum of the samples is taken.
	Min,
	/// The maximum of the samples is taken.
	Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available sampler addressing modes.
pub enum SamplerAddressingModes {
	/// Repeat mode addressing.
	Repeat,
	/// Mirror mode addressing.
	Mirror,
	/// Clamp mode addressing.
	Clamp,
	/// Border mode addressing.
	Border {},
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
	pub const fn new(binding: u32, descriptor_type: DescriptorType, stages: Stages,) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: 1,
			stages,
			immutable_samplers: None,
		}
	}

	pub const fn new_array(binding: u32, descriptor_type: DescriptorType, stages: Stages, count: u32) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: count,
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

	pub fn into_shader_binding_descriptor(&self, set: u32, access_policies: AccessPolicies) -> ShaderBindingDescriptor {
		ShaderBindingDescriptor::new(set, self.binding, access_policies)
	}

	/// Returns the binding index of the descriptor set layout binding.
	pub fn binding(&self) -> u32 {
		self.binding
	}
}

pub struct BindingConstructor<'a> {
	pub(super) descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub(super) array_element: u32,
	/// Information describing the descriptor.
	pub(super) descriptor: Descriptor,
	pub(super) frame_offset: Option<i8>,
}

impl <'a> BindingConstructor<'a> {
	pub fn buffer(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, buffer_handle: BaseBufferHandle) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::Buffer {
				handle: buffer_handle,
				size: Ranges::Whole,
			},
			frame_offset: None,
		}
	}

	pub fn image(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, image_handle: ImageHandle, layout: Layouts) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::Image {
				handle: image_handle,
				layout,
			},
			frame_offset: None,
		}
	}

	pub fn sampler(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, sampler_handle: SamplerHandle) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::Sampler(sampler_handle),
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, image_handle: ImageHandle, sampler_handle: SamplerHandle, layout: Layouts) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_array(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::CombinedImageSamplerArray,
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_layer(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, image_handle: ImageHandle, sampler_handle: SamplerHandle, layout: Layouts, layer_index: u32) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer: Some(layer_index),
			},
			frame_offset: None,
		}
	}

	pub fn sampler_with_immutable_samplers(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: Descriptor::StaticSamplers,
			frame_offset: None,
		}
	}

	fn acceleration_structure(bindings: &'a DescriptorSetBindingTemplate, top_level_acceleration_structure: TopLevelAccelerationStructureHandle) -> Self {
		BindingConstructor {
			descriptor_set_binding_template: bindings,
			array_element: 0,
			descriptor: Descriptor::AccelerationStructure {
				handle: top_level_acceleration_structure,
			},
			frame_offset: None,
		}
	}

	pub fn frame(mut self, frame_offset: i8) -> Self {
		self.frame_offset = Some(frame_offset);
		self
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
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct DescriptorWrite {
	pub(super) binding_handle: DescriptorSetBindingHandle,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub(super) array_element: u32,
	/// Information describing the descriptor.
	pub(super) descriptor: Descriptor,
	pub(super) frame_offset: Option<i32>,
}

impl DescriptorWrite {
	pub fn new(binding_handle: DescriptorSetBindingHandle, descriptor: Descriptor) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor,
			frame_offset: None,
		}
	}

	pub fn buffer(binding_handle: DescriptorSetBindingHandle, buffer_handle: BaseBufferHandle) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::Buffer {
				handle: buffer_handle,
				size: Ranges::Whole,
			},
			frame_offset: None,
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
			frame_offset: None,
		}
	}

	pub fn image_with_frame(binding_handle: DescriptorSetBindingHandle, image_handle: ImageHandle, layout: Layouts, frame_offset: i32) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::Image {
				handle: image_handle,
				layout,
			},
			frame_offset: Some(frame_offset),
		}
	}

	pub fn sampler(binding_handle: DescriptorSetBindingHandle, sampler_handle: SamplerHandle) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::Sampler(sampler_handle),
			frame_offset: None,
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
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_array(binding_handle: DescriptorSetBindingHandle, image_handle: ImageHandle, sampler_handle: SamplerHandle, layout: Layouts, index: u32) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: index,
			descriptor: Descriptor::CombinedImageSampler {
				image_handle: image_handle,
				sampler_handle,
				layout,
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn acceleration_structure(binding_handle: DescriptorSetBindingHandle, acceleration_structure_handle: TopLevelAccelerationStructureHandle) -> DescriptorWrite {
		DescriptorWrite {
			binding_handle,
			array_element: 0,
			descriptor: Descriptor::AccelerationStructure {
				handle: acceleration_structure_handle,
			},
			frame_offset: None,
		}
	}
}

/// Describes the details of the memory layout of a particular image.
pub struct ImageSubresourceLayout {
	/// The offset inside a memory region where the texture will read it's first texel from.
	pub(super) offset: usize,
	/// The size of the texture in bytes.
	pub(super) size: usize,
	/// The row pitch of the texture.
	pub(super) row_pitch: usize,
	/// The array pitch of the texture.
	pub(super) array_pitch: usize,
	/// The depth pitch of the texture.
	pub(super) depth_pitch: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
	pub(super) buffer: BaseBufferHandle,
	pub(super) offset: usize,
}

impl BufferDescriptor {
	pub fn new<T: Copy, const N: usize>(buffer: BufferHandle<[T; N]>) -> Self {
		Self {
			buffer: buffer.into(),
			offset: 0,
		}
	}
}

impl <T: Copy> Into<BufferDescriptor> for BufferHandle<T> {
	fn into(self) -> BufferDescriptor {
		BufferDescriptor {
			buffer: self.into(),
			offset: 0,
		}
	}
}

pub struct SpecializationMapEntry {
	pub(super) r#type: String,
	pub(super) constant_id: u32,
	pub(super) value: Box<[u8]>,
}

impl SpecializationMapEntry {
	pub fn new<T: Copy + 'static>(constant_id: u32, r#type: String, value: T) -> Self where [(); std::mem::size_of::<T>()]: {
		if r#type == "vec4f".to_owned() {
			assert_eq!(std::mem::size_of::<T>(), 16);
		}

		let mut data = [0 as u8; std::mem::size_of::<T>()];

		// SAFETY: We know that the data is valid for the lifetime of the specialization map entry.
		unsafe { std::ptr::copy_nonoverlapping((&value) as *const T as *const u8, data.as_mut_ptr(), std::mem::size_of::<T>()) };

		Self {
			r#type,
			constant_id,
			value: Box::new(data),
		}
	}

	pub fn get_constant_id(&self) -> u32 {
		self.constant_id
	}

	pub fn get_type(&self) -> String {
		self.r#type.clone()
	}

	pub fn get_size(&self) -> usize {
		std::mem::size_of_val(&self.value)
	}

	pub fn get_data(&self) -> &[u8] {
		// SAFETY: We know that the data is valid for the lifetime of the specialization map entry.
		self.value.as_ref()
	}
}

#[derive(Clone, Copy)]
pub struct ShaderParameter<'a> {
	pub(crate) handle: &'a ShaderHandle,
	pub(crate) stage: ShaderTypes,
	pub(crate) specialization_map: &'a [SpecializationMapEntry],
}

impl <'a> ShaderParameter<'a> {
	pub fn new(handle: &'a ShaderHandle, stage: ShaderTypes,) -> Self {
		Self {
			handle,
			stage,
			specialization_map: &[],
		}
	}

	pub fn with_specialization_map(mut self, specialization_map: &'a [SpecializationMapEntry]) -> Self {
		self.specialization_map = specialization_map;
		self
	}
}

#[derive(Clone, Copy)]
pub struct PushConstantRange {
	pub(crate) offset: u32,
	pub(crate) size: u32,
}

impl PushConstantRange {
	pub fn new(offset: u32, size: u32) -> Self {
		Self {
			offset,
			size,
		}
	}
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

pub struct QueueSelection {
	pub(crate) r#type: CommandBufferType,
}

impl QueueSelection {
	pub fn new(r#type: CommandBufferType) -> Self {
		Self {
			r#type,
		}
	}
}

#[cfg(test)]
pub(super) mod tests {
	use std::borrow::Borrow as _;

use crate::{command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, BoundRayTracingPipelineMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _}, device::Device, frame::Frame as _, raster_pipeline, window::Window};

	use resource_management::glsl;

	use super::*;

	#[test]
	fn test_formats_encoding() {
		// Test floating point formats
		assert_eq!(Formats::R8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R32F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::Depth32.encoding(), Some(Encodings::FloatingPoint));

		// Test unsigned normalized formats
		assert_eq!(Formats::R8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R32UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBu11u11u10.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::BGRAu8.encoding(), Some(Encodings::UnsignedNormalized));

		// Test signed normalized formats
		assert_eq!(Formats::R8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R32SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA16SNORM.encoding(), Some(Encodings::SignedNormalized));

		// Test sRGB formats
		assert_eq!(Formats::R8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R32sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA16sRGB.encoding(), Some(Encodings::sRGB));

		// Test formats without encoding
		assert_eq!(Formats::U32.encoding(), None);
		assert_eq!(Formats::BC5.encoding(), None);
		assert_eq!(Formats::BC7.encoding(), None);
	}

	#[test]
	fn test_formats_channel_bit_size() {
		// Test 8-bit formats
		assert_eq!(Formats::R8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8sRGB.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RG8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGB8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGBA8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAu8.channel_bit_size(), ChannelBitSize::Bits8);

		// Test 16-bit formats
		assert_eq!(Formats::R16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::R16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RG16SNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGB16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGBA16UNORM.channel_bit_size(), ChannelBitSize::Bits16);

		// Test 32-bit formats
		assert_eq!(Formats::R32F.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::R32UNORM.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::Depth32.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::U32.channel_bit_size(), ChannelBitSize::Bits32);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(Formats::BC5.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7.channel_bit_size(), ChannelBitSize::Compressed);
	}

	#[test]
	fn test_formats_channel_layout() {
		// Test single channel formats
		assert_eq!(Formats::R8F.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R16UNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R32SNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R8sRGB.channel_layout(), ChannelLayout::R);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG16UNORM.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG8SNORM.channel_layout(), ChannelLayout::RG);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB16UNORM.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB8sRGB.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGBu11u11u10.channel_layout(), ChannelLayout::RGB);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA16UNORM.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA8SNORM.channel_layout(), ChannelLayout::RGBA);

		// Test BGRA format
		assert_eq!(Formats::BGRAu8.channel_layout(), ChannelLayout::BGRA);

		// Test depth format
		assert_eq!(Formats::Depth32.channel_layout(), ChannelLayout::Depth);

		// Test packed format
		assert_eq!(Formats::U32.channel_layout(), ChannelLayout::Packed);

		// Test block compressed formats
		assert_eq!(Formats::BC5.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7.channel_layout(), ChannelLayout::BC);
	}

	#[test]
	fn test_formats_size() {
		// Test single channel formats
		assert_eq!(Formats::R8F.size(), 1);
		assert_eq!(Formats::R8UNORM.size(), 1);
		assert_eq!(Formats::R16F.size(), 2);
		assert_eq!(Formats::R16UNORM.size(), 2);
		assert_eq!(Formats::R32F.size(), 4);
		assert_eq!(Formats::R32SNORM.size(), 4);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.size(), 2);
		assert_eq!(Formats::RG8UNORM.size(), 2);
		assert_eq!(Formats::RG16F.size(), 4);
		assert_eq!(Formats::RG16SNORM.size(), 4);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.size(), 3);
		assert_eq!(Formats::RGB8UNORM.size(), 3);
		assert_eq!(Formats::RGB16F.size(), 6);
		assert_eq!(Formats::RGB16SNORM.size(), 6);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.size(), 4);
		assert_eq!(Formats::RGBA8UNORM.size(), 4);
		assert_eq!(Formats::RGBA16F.size(), 8);
		assert_eq!(Formats::RGBA16UNORM.size(), 8);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.size(), 4);
		assert_eq!(Formats::BGRAu8.size(), 4);
		assert_eq!(Formats::Depth32.size(), 4);
		assert_eq!(Formats::U32.size(), 4);
		assert_eq!(Formats::BC5.size(), 1);
		assert_eq!(Formats::BC7.size(), 1);
	}

	#[test]
	fn test_formats_comprehensive() {
		// Test that encoding, channel_bit_size, and channel_layout are consistent
		// For R8FloatingPoint
		let format = Formats::R8F;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::R);
		assert_eq!(format.size(), 1);

		// For RGBA16UnsignedNormalized
		let format = Formats::RGBA16UNORM;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(format.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(format.size(), 8);

		// For RGB8sRGB
		let format = Formats::RGB8sRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 3);

		// For special format RGBu11u11u10
		let format = Formats::RGBu11u11u10;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 4);

		// For BGRAu8
		let format = Formats::BGRAu8;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(format.size(), 4);

		// For Depth32
		let format = Formats::Depth32;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(format.channel_layout(), ChannelLayout::Depth);
		assert_eq!(format.size(), 4);

		// For BC7
		let format = Formats::BC7;
		assert_eq!(format.encoding(), None);
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(format.channel_layout(), ChannelLayout::BC);
		assert_eq!(format.size(), 1);
	}

	fn compile_shaders() -> (glsl::CompiledShader, glsl::CompiledShader) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
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

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	fn compile_shaders_with_model_matrix() -> (glsl::CompiledShader, glsl::CompiledShader) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(push_constant) uniform ModelMatrix {
				mat4 model_matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.model_matrix * vec4(in_position, 1.0);
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

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	#[test]
	fn dispatch_extent() {
		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(5, 5, 5));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(2, 2, 2));

		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(3, 3, 3));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(4, 4, 4));
	}

	fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
		assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });

		if extent.width() % 2 != 0 {
			let pixel = pixels[(extent.width() / 2) as usize]; // middle top center
			assert_eq!(pixel, RGBAu8 { r: 255, g: 0, b: 0, a: 255 });
		}

		let pixel = pixels[(extent.width() - 1) as usize]; // top right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });

		let pixel = pixels[(extent.width()  * (extent.height() - 1)) as usize]; // bottom left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 255, a: 255 });

		let pixel = pixels[(extent.width() * extent.height() - (extent.width() / 2)) as usize]; // middle bottom center
		assert!(pixel == RGBAu8 { r: 0, g: 127, b: 127, a: 255 } || pixel == RGBAu8 { r: 0, g: 128, b: 127, a: 255 }); // different implementations render slightly differently

		let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
	}

	pub(crate) fn render_triangle(device: &mut impl Device, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::STATIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		device.start_frame_capture();

		let mut command_buffer_recording = device.create_command_buffer_recording(command_buffer_handle);

		let attachments = [
			AttachmentInformation::new(render_target,Formats::RGBA8UNORM,Layouts::RenderTarget,ClearValue::Color(RGBA::black()), false, true,)
		];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

		let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		// Get image data and cast u8 slice to rgbau8
		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

		check_triangle(pixels, extent);
	}

	pub(crate) fn present(renderer: &mut impl Device, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let mut window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = renderer.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		let render_target = renderer.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceOnly, UseCases::STATIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = renderer.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		window.poll();

		renderer.start_frame_capture();

		let mut frame = renderer.start_frame(0, render_finished_synchronizer);

		let (present_key, _) = frame.acquire_swapchain_image(swapchain);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

		let attachments = [
			AttachmentInformation::new(render_target, Formats::RGBA8UNORM, Layouts::RenderTarget, ClearValue::Color(RGBA::black()), false, true,)
		];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

		let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		command_buffer_recording.copy_to_swapchain(render_target, present_key, swapchain);

		command_buffer_recording.execute(&[], &[render_finished_synchronizer], &[present_key], render_finished_synchronizer);

		renderer.end_frame_capture();

		window.poll();

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut impl Device, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = renderer.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = renderer.create_pipeline_layout(&[], &[]);

		let render_target = renderer.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = renderer.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for i in 0..2*64 {
			renderer.start_frame_capture();

			let mut frame = renderer.start_frame(i, render_finished_synchronizer);

			let (present_key, _) = frame.acquire_swapchain_image(swapchain,);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [
				AttachmentInformation::new(render_target,Formats::RGBA8UNORM,Layouts::RenderTarget,ClearValue::Color(RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),false,true,)
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

			let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			command_buffer_recording.copy_to_swapchain(render_target, present_key, swapchain);

			command_buffer_recording.execute(&[], &[render_finished_synchronizer], &[present_key], render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());
		}
	}

	pub(crate) fn multiframe_rendering(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = Extent::rectangle(1920, 1080);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [
				AttachmentInformation::new(render_target, Formats::RGBA8UNORM, Layouts::RenderTarget, ClearValue::Color(RGBA::black()), false, true,)
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

			let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], &[], render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn change_frames(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while changing the amount of frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			if i == 2 {
				device.set_frames_in_flight(3); // Change from default 2 to 3
			}

			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [
				AttachmentInformation::new(render_target, Formats::RGBA8UNORM, Layouts::RenderTarget, ClearValue::Color(RGBA::black()), false, true,)
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

			let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], &[], render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn resize(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while resize the render targets.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		let mut extent = Extent::rectangle(1280, 720);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			if i == 2 {
				extent = Extent::rectangle(1920, 1080);
				frame.resize_image(render_target, extent);
			}

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [
				AttachmentInformation::new(render_target, Formats::RGBA8UNORM, Layouts::RenderTarget, ClearValue::Color(RGBA::black()), false, true,)
			];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let pipeline_layout_command = render_pass_command.bind_pipeline_layout(pipeline_layout);

			let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], &[], render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn dynamic_data(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
			std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
			std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
			&vertex_layout
		) };

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders_with_model_matrix();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, []).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, []).expect("Failed to create fragment shader");

		let pipeline_layout = device.create_pipeline_layout(&[], &[PushConstantRange{ offset: 0, size: 16 * 4 }]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let _buffer = device.create_buffer::<u8>(None, Uses::Storage, DeviceAccesses::HostToDevice);

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut cb = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [
				AttachmentInformation::new(render_target, Formats::RGBA8UNORM, Layouts::RenderTarget, ClearValue::Color(RGBA::black()), false, true,)
			];

			let c = cb.start_render_pass(extent, &attachments);

			let c = c.bind_pipeline_layout(pipeline_layout);

			let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);

			let matrix: [f32; 16] =
				[
					angle.cos(), -angle.sin(), 0f32, 0f32,
					angle.sin(), angle.cos(), 0f32, 0f32,
					0f32, 0f32, 1f32, 0f32,
					0f32, 0f32, 0f32, 1f32,
				];

			c.write_push_constant(0, matrix);

			let c = c.bind_raster_pipeline(pipeline);

			c.draw_mesh(&mesh);

			c.end_render_pass();

			let copy_texture_handles = cb.transfer_textures(&[render_target]);

			cb.execute(&[], &[], &[], render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_texture_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			// Track green corner as it should move through screen

			if i % 4 == 0 {
				let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 }, "Pixel at bottom right corner did not match expected green color in frame: {i}");
			} else if i % 4 == 1 {
				let pixel = pixels[(extent.width()  * (extent.height() - 1)) as usize]; // bottom left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 }, "Pixel at bottom left corner did not match expected green color in frame: {i}");
			} else if i % 4 == 2 {
				let pixel = pixels[0]; // top left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 }, "Pixel at top left corner did not match expected green color in frame: {i}");
			} else if i % 4 == 3 {
				let pixel = pixels[(extent.width() - 1) as usize]; // top right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 }, "Pixel at top right corner did not match expected green color in frame: {i}");
			}
		}

		assert!(!device.has_errors())
	}

	pub(crate) fn multiframe_resources(device: &mut impl Device, queue_handle: QueueHandle) { // TODO: test multiframe resources for combined image samplers
		let compute_shader_string = "
			#version 450
			#pragma shader_stage(compute)

			layout(set=0,binding=0, rgba8) uniform image2D img;
			layout(set=0,binding=1, rgba8) uniform readonly image2D last_frame_img;

			layout(push_constant) uniform PushConstants {
				float value;
			} push_constants;

			layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
			void main() {
				imageStore(img, ivec2(0, 0), vec4(vec3(push_constants.value), 1));
				imageStore(img, ivec2(1, 0), imageLoad(last_frame_img, ivec2(0, 0)));
			}
		";

		let compute_shader_artifact = glsl::compile(compute_shader_string, "compute").unwrap();

		let compute_shader = device.create_shader(None, ShaderSource::SPIRV(compute_shader_artifact.borrow().into()), ShaderTypes::Compute, [ShaderBindingDescriptor::new(0, 0, AccessPolicies::WRITE), ShaderBindingDescriptor::new(0, 1, AccessPolicies::READ)]).expect("Failed to create compute shader");

		let image_binding_template = DescriptorSetBindingTemplate::new(0, DescriptorType::StorageImage, Stages::COMPUTE);
		let last_frame_image_binding_template = DescriptorSetBindingTemplate::new(1, DescriptorType::StorageImage, Stages::COMPUTE);

		let descriptor_set_template = device.create_descriptor_set_template(None, &[image_binding_template.clone(),last_frame_image_binding_template.clone()]);

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_template], &[PushConstantRange{ offset: 0, size: 4 }]);

		let pipeline = device.create_compute_pipeline(pipeline_layout, ShaderParameter::new(&compute_shader, ShaderTypes::Compute,));

		let image = device.create_image(Some("Image"), Extent::square(2), Formats::RGBA8UNORM, Uses::Storage, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_template);

		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::image(&image_binding_template, image, Layouts::General));
		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::image(&last_frame_image_binding_template, image, Layouts::General).frame(-1));

		let command_buffer = device.create_command_buffer(None, queue_handle);

		let signal = device.create_synchronizer(None, true);

		let mut frame = device.start_frame(0, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let data = [0.5f32];

		let pipeline_layout_command = command_buffer_recording.bind_pipeline_layout(pipeline_layout);

		pipeline_layout_command.write_push_constant(0, data);
		pipeline_layout_command.bind_descriptor_sets(&[descriptor_set]).bind_compute_pipeline(pipeline).dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert!(pixels[0] == RGBAu8 { r: 127, g: 127, b: 127, a: 255 } || pixels[0] == RGBAu8 { r: 128, g: 128, b: 128, a: 255 }); // Current frame image
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 }); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let mut frame = device.start_frame(1, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let data = [1.0f32];

		let pipeline_layout_command = command_buffer_recording.bind_pipeline_layout(pipeline_layout);

		pipeline_layout_command.write_push_constant(0, data);
		pipeline_layout_command.bind_descriptor_sets(&[descriptor_set]).bind_compute_pipeline(pipeline).dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert_eq!(pixels[0], RGBAu8 { r: 255, g: 255, b: 255, a: 255 });
		assert!(pixels[1] == RGBAu8 { r: 127, g: 127, b: 127, a: 255 } || pixels[1] == RGBAu8 { r: 128, g: 128, b: 128, a: 255 }); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let mut frame = device.start_frame(2, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert!(pixels[0] == RGBAu8 { r: 127, g: 127, b: 127, a: 255 } || pixels[0] == RGBAu8 { r: 128, g: 128, b: 128, a: 255 });
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });

		assert!(!device.has_errors());

		let mut frame = device.start_frame(3, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert_eq!(pixels[0], RGBAu8 { r: 255, g: 255, b: 255, a: 255 });
		assert!(pixels[1] == RGBAu8 { r: 127, g: 127, b: 127, a: 255 } || pixels[1] == RGBAu8 { r: 128, g: 128, b: 128, a: 255 });

		assert!(!device.has_errors());
	}

	pub(crate) fn descriptor_sets(device: &mut impl Device, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, true);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe { device.add_mesh_from_vertices_and_indices(3, 3,
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

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		let vertex_shader = device.create_shader(None, ShaderSource::SPIRV(vertex_shader_artifact.borrow().into()), ShaderTypes::Vertex, [ShaderBindingDescriptor::new(0, 1, AccessPolicies::READ)]).expect("Failed to create vertex shader");
		let fragment_shader = device.create_shader(None, ShaderSource::SPIRV(fragment_shader_artifact.borrow().into()), ShaderTypes::Fragment, [ShaderBindingDescriptor::new(0, 0, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 2, AccessPolicies::READ)]).expect("Failed to create fragment shader");

		let buffer = device.create_dynamic_buffer::<[u8; 64]>(None, Uses::Uniform | Uses::Storage, DeviceAccesses::HostToDevice);

		let sampled_texture = device.create_image(Some("sampled texture"), Extent::square(2,), Formats::RGBA8UNORM, Uses::Image, DeviceAccesses::HostToDevice, UseCases::STATIC, None);

		let pixels = vec![
			RGBAu8 { r: 255, g: 0, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 255, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 0, b: 255, a: 255 },
			RGBAu8 { r: 255, g: 255, b: 0, a: 255 },
		];

		let sampler =  device.create_sampler(FilteringModes::Closest, SamplingReductionModes::WeightedAverage, FilteringModes::Closest, SamplerAddressingModes::Repeat, None, 0.0f32, 0.0f32);

		let descriptor_set_layout_handle = device.create_descriptor_set_template(None, &[
			DescriptorSetBindingTemplate::new_with_immutable_samplers(0, Stages::FRAGMENT, Some(vec![sampler])),
			DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer,Stages::VERTEX),
			DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT),
		]);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_layout_handle,);

		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::sampler(&DescriptorSetBindingTemplate::new(0, DescriptorType::Sampler, Stages::FRAGMENT,), sampler));
		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::buffer(&DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer,Stages::VERTEX), buffer.into()));
		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::image(&DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT), sampled_texture, Layouts::Read));

		assert!(!device.has_errors());

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_layout_handle], &[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.create_image(None, extent, Formats::RGBA8UNORM, Uses::RenderTarget, DeviceAccesses::DeviceToHost, UseCases::STATIC, None);

		let attachments = [
			PipelineAttachmentInformation::new(Formats::RGBA8UNORM,)
		];

		let pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(pipeline_layout, &vertex_layout, &[ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex,), ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment,)], &attachments));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		device.start_frame_capture();

		let mut frame = device.start_frame(0, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

		command_buffer_recording.write_image_data(sampled_texture, &pixels);

		let attachments = [
			AttachmentInformation::new(render_target,Formats::RGBA8UNORM,Layouts::RenderTarget,ClearValue::Color(RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),false,true,)
		];

		let raster_render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let pipeline_layout_command = raster_render_pass_command.bind_pipeline_layout(pipeline_layout);

		pipeline_layout_command.bind_descriptor_sets(&[descriptor_set]);

		let raster_pipeline_command = pipeline_layout_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		raster_render_pass_command.end_render_pass();

		let texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], &[], signal);

		device.end_frame_capture();

		device.wait();

		// assert colored triangle was drawn to texture
		let _pixels = device.get_image_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!device.has_errors());
	}

	pub(crate) fn ray_tracing(renderer: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

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

		let vertex_positions_buffer = renderer.create_buffer::<[f32; 3 * 3]>(None, Uses::Storage | Uses::AccelerationStructureBuild, DeviceAccesses::HostToDevice,);
		let vertex_colors_buffer = renderer.create_buffer::<[f32; 4 * 3]>(None, Uses::Storage  | Uses::AccelerationStructureBuild, DeviceAccesses::HostToDevice,);
		let index_buffer = renderer.create_buffer::<[u16; 3]>(None, Uses::Storage  | Uses::AccelerationStructureBuild, DeviceAccesses::HostToDevice,);

		renderer.get_mut_buffer_slice(vertex_positions_buffer).copy_from_slice(&positions);
		renderer.get_mut_buffer_slice(vertex_colors_buffer).copy_from_slice(&colors);
		renderer.get_mut_buffer_slice(index_buffer).copy_from_slice(&[0u16, 1u16, 2u16]);

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

		let raygen_shader_artifact = glsl::compile(raygen_shader_code, "raygen").unwrap();
		let closest_hit_shader_artifact = glsl::compile(closest_hit_shader_code, "closest_hit").unwrap();
		let miss_shader_artifact = glsl::compile(miss_shader_code, "miss").unwrap();

		let raygen_shader = renderer.create_shader(None, ShaderSource::SPIRV(raygen_shader_artifact.borrow().into()), ShaderTypes::RayGen, [ShaderBindingDescriptor::new(0, 0, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 1, AccessPolicies::WRITE)]).expect("Failed to create raygen shader");
		let closest_hit_shader = renderer.create_shader(None, ShaderSource::SPIRV(closest_hit_shader_artifact.borrow().into()), ShaderTypes::ClosestHit, [ShaderBindingDescriptor::new(0, 2, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 3, AccessPolicies::READ), ShaderBindingDescriptor::new(0, 4, AccessPolicies::READ)]).expect("Failed to create closest hit shader");
		let miss_shader = renderer.create_shader(None, ShaderSource::SPIRV(miss_shader_artifact.borrow().into()), ShaderTypes::Miss, []).expect("Failed to create miss shader");

		let top_level_acceleration_structure = renderer.create_top_level_acceleration_structure(Some("Top Level"), 1);
		let bottom_level_acceleration_structure = renderer.create_bottom_level_acceleration_structure(&BottomLevelAccelerationStructure{
			description: BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count: 3,
				vertex_position_encoding: Encodings::FloatingPoint,
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

		let render_target = renderer.create_image(None, extent, Formats::RGBA8UNORM, Uses::Storage, DeviceAccesses::DeviceToHost, UseCases::DYNAMIC, None);

		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::acceleration_structure(&bindings[0], top_level_acceleration_structure));
		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::image(&bindings[1], render_target, Layouts::General));
		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::buffer(&bindings[2], vertex_positions_buffer.into()));
		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::buffer(&bindings[3], vertex_colors_buffer.into()));
		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::buffer(&bindings[4], index_buffer.into()));

		let pipeline_layout = renderer.create_pipeline_layout(&[descriptor_set_layout_handle], &[]);

		let pipeline = renderer.create_ray_tracing_pipeline(
			pipeline_layout,
			&[ShaderParameter::new(&raygen_shader, ShaderTypes::RayGen,), ShaderParameter::new(&closest_hit_shader, ShaderTypes::ClosestHit,), ShaderParameter::new(&miss_shader, ShaderTypes::Miss,)],
		);

		let rendering_command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		let instances_buffer = renderer.create_acceleration_structure_instance_buffer(None, 1);

		renderer.write_instance(instances_buffer, 0, [[1f32, 0f32,  0f32, 0f32], [0f32, 1f32,  0f32, 0f32], [0f32, 0f32,  1f32, 0f32]], 0, 0xFF, 0, bottom_level_acceleration_structure);

		let scratch_buffer = renderer.create_buffer::<[u8; 1024 * 1024]>(None, Uses::AccelerationStructureBuildScratch, DeviceAccesses::DeviceOnly,);

		let raygen_sbt_buffer = renderer.create_buffer::<[u8; 64]>(None, Uses::ShaderBindingTable, DeviceAccesses::HostToDevice);
		let miss_sbt_buffer = renderer.create_buffer::<[u8; 64]>(None, Uses::ShaderBindingTable, DeviceAccesses::HostToDevice);
		let hit_sbt_buffer = renderer.create_buffer::<[u8; 64]>(None, Uses::ShaderBindingTable, DeviceAccesses::HostToDevice);

		renderer.write_sbt_entry(raygen_sbt_buffer.into(), 0, pipeline, raygen_shader);
		renderer.write_sbt_entry(miss_sbt_buffer.into(), 0, pipeline, miss_shader);
		renderer.write_sbt_entry(hit_sbt_buffer.into(), 0, pipeline, closest_hit_shader);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			renderer.start_frame_capture();

			let mut frame = renderer.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(rendering_command_buffer_handle);

			{
				command_buffer_recording.build_bottom_level_acceleration_structures(&[BottomLevelAccelerationStructureBuild {
					acceleration_structure: bottom_level_acceleration_structure,
					description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer: BufferStridedRange::new(vertex_positions_buffer.into(), 0, 12, 12 * 3),
						vertex_count: 3,
						index_buffer: BufferStridedRange::new(index_buffer.into(), 0, 2, 2 * 3),
						vertex_position_encoding: Encodings::FloatingPoint,
						index_format: DataTypes::U16,
						triangle_count: 1,
					},
					scratch_buffer: BufferDescriptor::new(scratch_buffer),
				}]);

				command_buffer_recording.build_top_level_acceleration_structure(&TopLevelAccelerationStructureBuild {
					acceleration_structure: top_level_acceleration_structure,
					description: TopLevelAccelerationStructureBuildDescriptions::Instance {
						instances_buffer,
						instance_count: 1,
					},
					scratch_buffer: BufferDescriptor::new(scratch_buffer),
				});
			}

			let pipeline_layout_command = command_buffer_recording.bind_pipeline_layout(pipeline_layout);

			let ray_tracing_pipeline_command = pipeline_layout_command.bind_ray_tracing_pipeline(pipeline);

			ray_tracing_pipeline_command.bind_descriptor_sets(&[descriptor_set]);

			ray_tracing_pipeline_command.trace_rays(BindingTables {
				raygen: BufferStridedRange::new(raygen_sbt_buffer.into(), 0, 64, 64),
				hit: BufferStridedRange::new(hit_sbt_buffer.into(), 0, 64, 64),
				miss: BufferStridedRange::new(miss_sbt_buffer.into(), 0, 64, 64),
				callable: None,
			}, 1920, 1080, 1);

			let texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_image_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width() * extent.height()) as usize) };

			check_triangle(pixels, extent);
		}
	}
}
