use crate::{BaseBufferHandle, BufferHandle};

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UseCases {
	STATIC,
	DYNAMIC,
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
	/// 8 bit sRGB RGBA.
	BGRAsRGB,
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
			Formats::R8F
			| Formats::R16F
			| Formats::R32F
			| Formats::RG8F
			| Formats::RG16F
			| Formats::RGB8F
			| Formats::RGB16F
			| Formats::RGBA8F
			| Formats::RGBA16F
			| Formats::Depth32 => Some(Encodings::FloatingPoint),

			Formats::R8UNORM
			| Formats::R16UNORM
			| Formats::R32UNORM
			| Formats::RG8UNORM
			| Formats::RG16UNORM
			| Formats::RGB8UNORM
			| Formats::RGB16UNORM
			| Formats::RGBA8UNORM
			| Formats::RGBA16UNORM
			| Formats::RGBu11u11u10
			| Formats::BGRAu8 => Some(Encodings::UnsignedNormalized),

			Formats::R8SNORM
			| Formats::R16SNORM
			| Formats::R32SNORM
			| Formats::RG8SNORM
			| Formats::RG16SNORM
			| Formats::RGB8SNORM
			| Formats::RGB16SNORM
			| Formats::RGBA8SNORM
			| Formats::RGBA16SNORM => Some(Encodings::SignedNormalized),

			Formats::R8sRGB
			| Formats::R16sRGB
			| Formats::R32sRGB
			| Formats::RG8sRGB
			| Formats::RG16sRGB
			| Formats::RGB8sRGB
			| Formats::RGB16sRGB
			| Formats::RGBA8sRGB
			| Formats::RGBA16sRGB
			| Formats::BGRAsRGB => Some(Encodings::sRGB),

			Formats::U32 | Formats::BC5 | Formats::BC7 => None,
		}
	}

	/// Returns the channel bit size of the format.
	pub fn channel_bit_size(&self) -> ChannelBitSize {
		match self {
			Formats::R8F
			| Formats::R8UNORM
			| Formats::R8SNORM
			| Formats::R8sRGB
			| Formats::RG8F
			| Formats::RG8UNORM
			| Formats::RG8SNORM
			| Formats::RG8sRGB
			| Formats::RGB8F
			| Formats::RGB8UNORM
			| Formats::RGB8SNORM
			| Formats::RGB8sRGB
			| Formats::RGBA8F
			| Formats::RGBA8UNORM
			| Formats::RGBA8SNORM
			| Formats::RGBA8sRGB
			| Formats::BGRAu8
			| Formats::BGRAsRGB => ChannelBitSize::Bits8,

			Formats::R16F
			| Formats::R16UNORM
			| Formats::R16SNORM
			| Formats::R16sRGB
			| Formats::RG16F
			| Formats::RG16UNORM
			| Formats::RG16SNORM
			| Formats::RG16sRGB
			| Formats::RGB16F
			| Formats::RGB16UNORM
			| Formats::RGB16SNORM
			| Formats::RGB16sRGB
			| Formats::RGBA16F
			| Formats::RGBA16UNORM
			| Formats::RGBA16SNORM
			| Formats::RGBA16sRGB => ChannelBitSize::Bits16,

			Formats::R32F | Formats::R32UNORM | Formats::R32SNORM | Formats::R32sRGB | Formats::Depth32 | Formats::U32 => {
				ChannelBitSize::Bits32
			}

			Formats::RGBu11u11u10 => ChannelBitSize::Bits11_11_10,

			Formats::BC5 | Formats::BC7 => ChannelBitSize::Compressed,
		}
	}

	/// Returns the channel layout of the format.
	pub fn channel_layout(&self) -> ChannelLayout {
		match self {
			Formats::R8F
			| Formats::R8UNORM
			| Formats::R8SNORM
			| Formats::R8sRGB
			| Formats::R16F
			| Formats::R16UNORM
			| Formats::R16SNORM
			| Formats::R16sRGB
			| Formats::R32F
			| Formats::R32UNORM
			| Formats::R32SNORM
			| Formats::R32sRGB => ChannelLayout::R,

			Formats::RG8F
			| Formats::RG8UNORM
			| Formats::RG8SNORM
			| Formats::RG8sRGB
			| Formats::RG16F
			| Formats::RG16UNORM
			| Formats::RG16SNORM
			| Formats::RG16sRGB => ChannelLayout::RG,

			Formats::RGB8F
			| Formats::RGB8UNORM
			| Formats::RGB8SNORM
			| Formats::RGB8sRGB
			| Formats::RGB16F
			| Formats::RGB16UNORM
			| Formats::RGB16SNORM
			| Formats::RGB16sRGB
			| Formats::RGBu11u11u10 => ChannelLayout::RGB,

			Formats::RGBA8F
			| Formats::RGBA8UNORM
			| Formats::RGBA8SNORM
			| Formats::RGBA8sRGB
			| Formats::RGBA16F
			| Formats::RGBA16UNORM
			| Formats::RGBA16SNORM
			| Formats::RGBA16sRGB => ChannelLayout::RGBA,

			Formats::BGRAu8 | Formats::BGRAsRGB => ChannelLayout::BGRA,

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
			Formats::BGRAu8 | Formats::BGRAsRGB => 4,
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

/// Primitive GPU/shader data types.
#[derive(Hash, Clone, Copy, PartialEq, Eq)]
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

impl DataTypes {
	pub fn size(self) -> usize {
		match self {
			DataTypes::Float => std::mem::size_of::<f32>(),
			DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			DataTypes::U8 => std::mem::size_of::<u8>(),
			DataTypes::U16 => std::mem::size_of::<u16>(),
			DataTypes::U32 => std::mem::size_of::<u32>(),
			DataTypes::Int => std::mem::size_of::<i32>(),
			DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			DataTypes::UInt => std::mem::size_of::<u32>(),
			DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct DeviceAccesses: u16 {
		const CpuRead = 1 << 0;
		const CpuWrite = 1 << 1;
		const GpuRead = 1 << 2;
		const GpuWrite = 1 << 3;

		const DeviceOnly = 1 << 2 | 1 << 3;
		const HostOnly = 1 << 0 | 1 << 1;
		const HostToDevice = 1 << 1 | 1 << 2;
		const DeviceToHost = 1 << 0 | 1 << 3;
	}
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

impl From<ShaderTypes> for Stages {
	fn from(ty: ShaderTypes) -> Self {
		match ty {
			ShaderTypes::Vertex => Self::VERTEX,
			ShaderTypes::Fragment => Self::FRAGMENT,
			ShaderTypes::Compute => Self::COMPUTE,
			ShaderTypes::Task => Self::TASK,
			ShaderTypes::Mesh => Self::MESH,
			ShaderTypes::RayGen => Self::RAYGEN,
			ShaderTypes::ClosestHit => Self::CLOSEST_HIT,
			ShaderTypes::AnyHit => Self::ANY_HIT,
			ShaderTypes::Intersection => Self::INTERSECTION,
			ShaderTypes::Miss => Self::MISS,
			ShaderTypes::Callable => Self::CALLABLE,
		}
	}
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

pub struct BufferDescriptor {
	pub(super) buffer: BaseBufferHandle,
	pub(super) offset: usize,
	pub(super) index_type: Option<DataTypes>,
}

impl BufferDescriptor {
	pub fn new<T: Copy, const N: usize>(buffer: BufferHandle<[T; N]>) -> Self {
		Self {
			buffer: buffer.into(),
			offset: 0,
			index_type: None,
		}
	}

	pub fn offset(mut self, offset: usize) -> Self {
		self.offset = offset;
		self
	}

	pub fn index_type(mut self, index_type: DataTypes) -> Self {
		self.index_type = Some(index_type);
		self
	}
}

impl<T: Copy> Into<BufferDescriptor> for BufferHandle<T> {
	fn into(self) -> BufferDescriptor {
		BufferDescriptor {
			buffer: self.into(),
			offset: 0,
			index_type: None,
		}
	}
}

pub struct BufferStridedRange {
	pub(super) buffer_offset: BufferDescriptor,
	pub(super) stride: usize,
	pub(super) size: usize,
}

impl BufferStridedRange {
	pub fn new(buffer: BaseBufferHandle, offset: usize, stride: usize, size: usize) -> Self {
		Self {
			buffer_offset: BufferDescriptor {
				buffer,
				offset,
				index_type: None,
			},
			stride,
			size,
		}
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct WorkloadTypes: u16 {
		const RASTER = 1 << 0;
		const RAY_TRACING = 1 << 1;
		const COMPUTE = 1 << 2;
		const TRANSFER = 1 << 3;
		const VIDEO = 1 << 4;
	}
}
