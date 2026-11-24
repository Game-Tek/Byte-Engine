use crate::{BaseBufferHandle, BufferHandle};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// A resource layout required by GPU work.
pub enum Layouts {
	/// No specific layout is required.
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
	/// Indirect.
	Indirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A texture filtering mode used by samplers.
pub enum FilteringModes {
	/// Closest mode filtering. Rounds floating point coordinates to the nearest pixel.
	Closest,
	/// Linear mode filtering. Blends samples linearly across neighbouring pixels.
	Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A rule for combining neighboring texels during image sampling.
pub enum SamplingReductionModes {
	/// The average of the samples. Weighted by the proximity of the sample to the sample point.
	WeightedAverage,
	/// The minimum of the samples is taken.
	Min,
	/// The maximum of the samples is taken.
	Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A sampler rule for coordinates outside an image.
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
		/// A storage image that shaders update with image atomics, such as `image_atomic_max`. Combine it with
		/// [`Self::Storage`]; only unsigned 32-bit images support it.
		const Atomic = 1 << 16;
		/// Resource will be used as a depth stencil.
		const DepthStencil = 1 << 7;
		/// Resource will be used as a transfer source.
		const TransferSource = 1 << 9;
		/// Resource will be used as a transfer destination.
		const TransferDestination = 1 << 10;
		/// The buffer holds geometry that acceleration structures are built from.
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
		/// The tile stage, which runs a tile function inside a render pass. See [`ShaderTypes::Tile`].
		const TILE = 1 << 13;
		/// The acceleration structure build stage.
		const ACCELERATION_STRUCTURE_BUILD = 1 << 17;
		/// The last or bottom stage.
		const LAST = 1 << 63;
	}
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
/// A pixel format supported by GHI images.
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

	/// Packed unsigned floating point RGB with 11 bit R and G and 10 bit B (R11G11B10F).
	RGBu11u11u10,
	/// 8 bit unsigned per component normalized BGRA.
	BGRAu8,
	/// 8 bit sRGB RGBA.
	BGRAsRGB,
	/// 16 bit unsigned normalized depth.
	Depth16,
	/// 32 bit float depth.
	Depth32,
	/// 32 bit unsigned integer.
	U32,
	/// BC5 block compressed format (unsigned normalized).
	BC5,
	/// BC5 block compressed format (signed normalized) for normal maps.
	BC5SNORM,
	/// BC7 block compressed format.
	BC7,
	/// BC7 block compressed sRGB format.
	BC7SRGB,
}

/// The row-pitch alignment of buffer-to-texture and texture-to-buffer copies.
///
/// Upload preparation pads each row of a staged texture to it, and backends read and write copy buffers with it.
pub const TEXTURE_COPY_PITCH_ALIGNMENT: usize = 256;

/// Returns the row and image pitches of a copy whose compact rows are padded to [`TEXTURE_COPY_PITCH_ALIGNMENT`].
///
/// Take `bytes_per_row` and `row_count` from [`Formats::copy_layout`]. Returns `None` when a pitch overflows.
pub(crate) fn aligned_copy_pitches(bytes_per_row: usize, row_count: usize) -> Option<(usize, usize)> {
	let aligned_bytes_per_row = bytes_per_row.checked_next_multiple_of(TEXTURE_COPY_PITCH_ALIGNMENT)?;
	Some((aligned_bytes_per_row, aligned_bytes_per_row.checked_mul(row_count)?))
}

/// The `ImageCopyLayout` struct keeps the compact CPU rows of an image level and the padded rows a copy from a buffer
/// reads in agreement, so every caller that stages pixels derives the same pitches.
///
/// Size a staging buffer with [`Self::padded_size`], fill it with [`Self::write_rows`], or expand rows already read
/// into it with [`Self::pack_rows`]. Next, record [`Self::copy`] with
/// [`crate::command_buffer::CommonCommandBufferMode::copy_buffer_to_images`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageCopyLayout {
	/// The bytes of one row without padding.
	pub compact_bytes_per_row: usize,
	/// The rows of one image, which are block rows for a compressed format.
	pub row_count: usize,
	/// The bytes of one image without padding.
	pub compact_bytes_per_image: usize,
	/// The bytes of every image without padding.
	pub compact_size: usize,
	/// The bytes from one staged row to the next, a multiple of [`TEXTURE_COPY_PITCH_ALIGNMENT`].
	pub padded_bytes_per_row: usize,
	/// The bytes from one staged image to the next.
	pub padded_bytes_per_image: usize,
	/// The staging bytes of every image.
	pub padded_size: usize,
}

impl ImageCopyLayout {
	/// Returns the layout of `image_count` images of `extent` in `format`, or `None` when a size overflows.
	///
	/// `image_count` counts every 2D image the copy writes: the array layers times the depth slices of a volume.
	pub fn new(format: Formats, extent: utils::Extent, image_count: usize) -> Option<Self> {
		let (compact_bytes_per_row, row_count, compact_bytes_per_image) = format.copy_layout(extent)?;
		let (padded_bytes_per_row, padded_bytes_per_image) = aligned_copy_pitches(compact_bytes_per_row, row_count)?;
		Some(Self {
			compact_bytes_per_row,
			row_count,
			compact_bytes_per_image,
			compact_size: compact_bytes_per_image.checked_mul(image_count)?,
			padded_bytes_per_row,
			padded_bytes_per_image,
			padded_size: padded_bytes_per_image.checked_mul(image_count)?,
		})
	}

	/// Copies every row of every image from `source` into the padded rows of `destination`.
	///
	/// Source rows start `source_bytes_per_row` apart, so `source` can be a rectangle inside a larger CPU image; pass
	/// [`Self::compact_bytes_per_row`] for compact pixels. `destination` holds at least [`Self::padded_size`] bytes.
	pub fn write_rows(&self, destination: &mut [u8], source: &[u8], source_bytes_per_row: usize) {
		let rows = destination[..self.padded_size].chunks_exact_mut(self.padded_bytes_per_row);
		// Images follow each other without padding between them, so every staged row follows the previous one.
		for (destination, source) in rows.zip(source.chunks(source_bytes_per_row)) {
			destination[..self.compact_bytes_per_row].copy_from_slice(&source[..self.compact_bytes_per_row]);
		}
	}

	/// Expands compact rows at the start of `bytes` into their padded places in the same range.
	///
	/// Use it when pixels are read straight into staging. `bytes` holds exactly [`Self::padded_size`] bytes.
	pub fn pack_rows(&self, bytes: &mut [u8]) {
		assert_eq!(bytes.len(), self.padded_size);
		// Compact rows already on the copy pitch sit where the GPU reads them, so every move would be onto itself.
		if self.padded_bytes_per_row == self.compact_bytes_per_row {
			return;
		}
		// Rows move toward the end, so walking backward never overwrites a row before it moved.
		for row in (0..self.compact_size / self.compact_bytes_per_row).rev() {
			let source = row * self.compact_bytes_per_row;
			bytes.copy_within(source..source + self.compact_bytes_per_row, row * self.padded_bytes_per_row);
		}
	}

	/// Returns the copy of the staged images at `source_offset` of `source_buffer` into a mip of `destination_image`.
	pub fn copy(
		&self,
		source_buffer: BaseBufferHandle,
		source_offset: usize,
		destination_image: crate::BaseImageHandle,
		destination_mip_level: u32,
	) -> BufferImageCopyDescriptor {
		BufferImageCopyDescriptor::new(
			source_buffer,
			source_offset,
			self.padded_bytes_per_row,
			self.padded_bytes_per_image,
			destination_image,
			destination_mip_level,
		)
	}
}

impl Formats {
	/// Every format, for backends that prepare per-format state when a device is created, such as the DX12 format
	/// support table. Add a new variant here too.
	pub const ALL: [Self; 46] = [
		Self::R8F,
		Self::R8UNORM,
		Self::R8SNORM,
		Self::R8sRGB,
		Self::R16F,
		Self::R16UNORM,
		Self::R16SNORM,
		Self::R16sRGB,
		Self::R32F,
		Self::R32UNORM,
		Self::R32SNORM,
		Self::R32sRGB,
		Self::RG8F,
		Self::RG8UNORM,
		Self::RG8SNORM,
		Self::RG8sRGB,
		Self::RG16F,
		Self::RG16UNORM,
		Self::RG16SNORM,
		Self::RG16sRGB,
		Self::RGB8F,
		Self::RGB8UNORM,
		Self::RGB8SNORM,
		Self::RGB8sRGB,
		Self::RGB16F,
		Self::RGB16UNORM,
		Self::RGB16SNORM,
		Self::RGB16sRGB,
		Self::RGBA8F,
		Self::RGBA8UNORM,
		Self::RGBA8SNORM,
		Self::RGBA8sRGB,
		Self::RGBA16F,
		Self::RGBA16UNORM,
		Self::RGBA16SNORM,
		Self::RGBA16sRGB,
		Self::RGBu11u11u10,
		Self::BGRAu8,
		Self::BGRAsRGB,
		Self::Depth16,
		Self::Depth32,
		Self::U32,
		Self::BC5,
		Self::BC5SNORM,
		Self::BC7,
		Self::BC7SRGB,
	];

	/// Returns whether this format can be used as a depth attachment.
	pub const fn is_depth(self) -> bool {
		matches!(self, Self::Depth16 | Self::Depth32)
	}

	/// Returns the byte size of one compressed block for BC formats.
	pub fn bc_bytes_per_block(&self) -> Option<u32> {
		match self {
			Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => Some(16),
			_ => None,
		}
	}

	/// Returns compact row bytes, row count, and image bytes for one texture level.
	///
	/// A BC format has one row per 4x4 block row and at least one block. An uncompressed empty level has no bytes.
	/// Returns `None` when a size overflows.
	fn checked_compact_copy_layout(&self, width: u32, height: u32) -> Option<(usize, usize, usize)> {
		let (bytes_per_row, row_count) = match self.bc_bytes_per_block() {
			Some(bytes_per_block) => (
				usize::try_from(width.max(1).div_ceil(4))
					.ok()?
					.checked_mul(bytes_per_block as usize)?,
				usize::try_from(height.max(1).div_ceil(4)).ok()?,
			),
			None => (
				usize::try_from(width).ok()?.checked_mul(self.size())?,
				usize::try_from(height).ok()?,
			),
		};
		Some((bytes_per_row, row_count, bytes_per_row.checked_mul(row_count)?))
	}

	/// Returns compact row bytes, row count, and image bytes for one texture level.
	///
	/// An uncompressed empty level has no bytes. Use [`Self::copy_layout`] for an extent that may be empty.
	pub fn compact_copy_layout(&self, width: u32, height: u32) -> (usize, usize, usize) {
		self.checked_compact_copy_layout(width, height).expect(
			"Texture copy layout overflowed. The most likely cause is an image extent too large for the host address space.",
		)
	}

	/// Returns compact row bytes, row count, and image bytes for copying one 2D level of `extent`.
	///
	/// An empty width or height counts as one texel, so every copy moves at least one row. Returns `None` when a size
	/// overflows. Pad the result with [`aligned_copy_pitches`] for GPU copy buffers.
	pub fn copy_layout(&self, extent: utils::Extent) -> Option<(usize, usize, usize)> {
		self.checked_compact_copy_layout(extent.width().max(1), extent.height().max(1))
	}

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
			| Formats::RGBu11u11u10
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
			| Formats::BGRAu8 => Some(Encodings::UnsignedNormalized),

			Formats::Depth16 => Some(Encodings::UnsignedNormalized),

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

			Formats::BC7SRGB => Some(Encodings::sRGB),

			Formats::BC5SNORM => Some(Encodings::SignedNormalized),

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
			| Formats::RGBA16sRGB
			| Formats::Depth16 => ChannelBitSize::Bits16,

			Formats::R32F | Formats::R32UNORM | Formats::R32SNORM | Formats::R32sRGB | Formats::Depth32 | Formats::U32 => {
				ChannelBitSize::Bits32
			}

			Formats::RGBu11u11u10 => ChannelBitSize::Bits11_11_10,

			Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => ChannelBitSize::Compressed,
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

			Formats::Depth16 | Formats::Depth32 => ChannelLayout::Depth,

			Formats::U32 => ChannelLayout::Packed,

			Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => ChannelLayout::BC,
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
			Formats::Depth16 => 2,
			Formats::Depth32 => 4,
			Formats::U32 => 4,
			Formats::BC5 | Formats::BC5SNORM => 1,
			Formats::BC7 | Formats::BC7SRGB => 1,
		}
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	/// Bit flags for the available access policies.
	pub struct AccessPolicies : u8 {
		/// No access.
		const NONE = 0b00000000;
		/// Read access.
		const READ = 0b00000001;
		/// Write access.
		const WRITE = 0b00000010;
		/// Read and write access.
		const READ_WRITE = Self::READ.bits() | Self::WRITE.bits();
	}
}

/// A primitive data type shared by GPU resources and shaders.
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

impl Size for DataTypes {
	fn size(&self) -> usize {
		(*self).size()
	}
}

/// The `HostAccess` enum declares how the CPU accesses a buffer, so the backend can place its memory without the
/// application knowing the GPU's memory architecture.
///
/// Pass it to [`crate::buffer::Builder::host_access`]. Next, choose the buffer's [`Copies`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum HostAccess {
	/// Only GPU commands read and write the buffer.
	#[default]
	None,
	/// The CPU writes the buffer and never reads it back. Its memory may be write-combined or across the bus on the
	/// GPU, where CPU reads are orders of magnitude slower than writes. With [`Copies::PerFrame`], map each frame's copy
	/// with [`crate::frame::Frame::map`]; with one copy, use [`crate::context::Context::map_unsynchronized`].
	Write,
	/// The CPU reads the buffer, either what GPU commands wrote or what the CPU wrote itself, such as a decoder that
	/// copies from its own output. Its memory is cached for the CPU. With [`Copies::PerFrame`], read a completed
	/// frame's copy with [`crate::frame::Frame::read`]; with one copy, use
	/// [`crate::context::Context::map_unsynchronized`].
	Read,
}

/// The `Copies` enum declares how many copies of a buffer or image exist, so frames in flight never share data the
/// CPU or a later frame rewrites.
///
/// Copy `i` belongs to the frames whose sequence index is `i`. Pass it to [`crate::buffer::Builder::copies`] or
/// [`crate::image::Builder::copies`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Copies {
	/// One copy that every frame uses.
	#[default]
	One,
	/// One copy per frame in flight. A frame's commands use its copy, and a descriptor written with a frame offset
	/// names another frame's copy.
	PerFrame,
}

impl Copies {
	/// Returns how many copies a buffer or image has when `frames` frames are in flight.
	pub(crate) fn count(self, frames: u8) -> u8 {
		match self {
			Copies::One => 1,
			Copies::PerFrame => frames,
		}
	}
}

/// A programmable shader stage.
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
	/// The shader a ray tracing pipeline launches.
	RayGen,
	/// The shader a ray tracing pipeline's hit group runs on a hit.
	ClosestHit,
	/// A tile function: a compute-like shader a render pass runs once per tile, which writes the pass's color
	/// attachments. Only backends whose [`crate::context::Context::supports_tile_functions`] is `true` create them.
	Tile,
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
			ShaderTypes::Tile => Self::TILE,
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
/// The channel order in a pixel format.
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
/// The bit width of each channel in a pixel format.
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

/// The `BufferCopyDescriptor` struct configures one byte-range copy between buffers.
#[derive(Clone, Copy, Debug)]
pub struct BufferCopyDescriptor {
	pub source_buffer: BaseBufferHandle,
	pub source_offset: usize,
	pub destination_buffer: BaseBufferHandle,
	pub destination_offset: usize,
	pub size: usize,
}

impl BufferCopyDescriptor {
	/// Creates a buffer copy descriptor from source and destination byte ranges.
	pub fn new(
		source_buffer: BaseBufferHandle,
		source_offset: usize,
		destination_buffer: BaseBufferHandle,
		destination_offset: usize,
		size: usize,
	) -> Self {
		Self {
			source_buffer,
			source_offset,
			destination_buffer,
			destination_offset,
			size,
		}
	}
}

/// The `BufferImageCopyDescriptor` struct configures one copy from a buffer into a mip of an image, or into a region
/// of its base level.
///
/// Pass it to [`crate::command_buffer::CommonCommandBufferMode::copy_buffer_to_images`]. Build a whole-mip copy with
/// [`ImageCopyLayout::copy`], which also pads each source row to [`TEXTURE_COPY_PITCH_ALIGNMENT`].
#[derive(Clone, Copy, Debug)]
pub struct BufferImageCopyDescriptor {
	pub source_buffer: BaseBufferHandle,
	pub source_offset: usize,
	pub source_bytes_per_row: usize,
	pub source_bytes_per_image: usize,
	pub destination_image: crate::BaseImageHandle,
	pub destination_mip_level: u32,
	/// The rectangle of the base level the copy writes, or `None` for the whole mip.
	pub destination_region: Option<crate::image::Region>,
}

impl BufferImageCopyDescriptor {
	/// Creates a buffer-to-image copy descriptor from a source byte layout and destination image.
	pub fn new(
		source_buffer: BaseBufferHandle,
		source_offset: usize,
		source_bytes_per_row: usize,
		source_bytes_per_image: usize,
		destination_image: crate::BaseImageHandle,
		destination_mip_level: u32,
	) -> Self {
		Self {
			source_buffer,
			source_offset,
			source_bytes_per_row,
			source_bytes_per_image,
			destination_image,
			destination_mip_level,
			destination_region: None,
		}
	}

	/// Restricts the copy to `region` of the base level, so it updates part of an image and keeps the rest.
	///
	/// The source then holds only the region's rows, and its pitches follow the region's width and height.
	pub fn region(mut self, region: crate::image::Region) -> Self {
		assert_eq!(
			self.destination_mip_level, 0,
			"Image region copy names mip {}. The most likely cause is a region copy into a mip other than the base level. Copy into mip 0, or copy the whole mip.",
			self.destination_mip_level
		);
		self.destination_region = Some(region);
		self
	}
}

#[derive(Clone, Copy)]
pub struct BufferDescriptor {
	pub(super) buffer: BaseBufferHandle,
	pub(super) offset: usize,
	pub(super) index_type: Option<DataTypes>,
}

impl BufferDescriptor {
	pub fn new<T: bytemuck::Pod, const N: usize>(buffer: BufferHandle<[T; N]>) -> Self {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn aligned_copy_pitches_pad_rows_to_the_copy_alignment() {
		assert_eq!(aligned_copy_pitches(20, 7), Some((256, 1792)));
		assert_eq!(aligned_copy_pitches(256, 2), Some((256, 512)));
		assert_eq!(aligned_copy_pitches(usize::MAX, 1), None);
	}

	#[test]
	fn compact_copy_layout_preserves_texel_rows_and_bc_block_rows() {
		let cases = [
			(Formats::RGBA8UNORM, 5, 7, (20, 7, 140)),
			(Formats::RGB16UNORM, 5, 7, (30, 7, 210)),
			(Formats::RGBu11u11u10, 5, 7, (20, 7, 140)),
			(Formats::Depth16, 5, 7, (10, 7, 70)),
			(Formats::Depth32, 5, 7, (20, 7, 140)),
			(Formats::BC7, 5, 7, (32, 2, 64)),
			(Formats::BC5, 8, 4, (32, 1, 32)),
			(Formats::RGBA8UNORM, 0, 0, (0, 0, 0)),
			(Formats::BC7, 0, 0, (16, 1, 16)),
		];

		for (format, width, height, expected) in cases {
			assert_eq!(format.compact_copy_layout(width, height), expected);
		}
	}
}

impl<T: ?Sized> From<BufferHandle<T>> for BufferDescriptor {
	fn from(val: BufferHandle<T>) -> Self {
		BufferDescriptor {
			buffer: val.into(),
			offset: 0,
			index_type: None,
		}
	}
}

impl From<BaseBufferHandle> for BufferDescriptor {
	/// Describes a whole buffer whose element type was erased, such as one chosen at runtime among differently typed buffers.
	fn from(buffer: BaseBufferHandle) -> Self {
		BufferDescriptor {
			buffer,
			offset: 0,
			index_type: None,
		}
	}
}

#[derive(Clone, Copy)]
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
		const IO = 1 << 5;
	}
}
