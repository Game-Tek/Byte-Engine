// Audio

#[derive(
	Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Clone, Copy,
)]
pub enum BitDepths {
	Eight,
	Sixteen,
	TwentyFour,
	ThirtyTwo,
}

impl From<BitDepths> for usize {
	fn from(bit_depth: BitDepths) -> Self {
		match bit_depth {
			BitDepths::Eight => 8,
			BitDepths::Sixteen => 16,
			BitDepths::TwentyFour => 24,
			BitDepths::ThirtyTwo => 32,
		}
	}
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone)]
pub enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

/// The `ShaderTypes` enum identifies the stage for a shader resource.
#[derive(
	Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Debug, PartialEq, Eq,
)]
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

// Mesh

#[derive(
	Clone, Copy, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Eq,
)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	UV,
	Color,
	Joints,
	Weights,
}

#[derive(
	Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Eq,
)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(
	Clone, Copy, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Eq,
)]
pub enum IndexStreamTypes {
	Vertices,
	Meshlets,
	Triangles,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Stream {
	pub stream_type: Streams,
	pub offset: usize,
	pub size: usize,
	pub stride: usize,
}

impl Stream {
	/// Returns the number of logical elements (not bytes) in the stream.
	pub fn count(&self) -> usize {
		assert!(
			self.stride > 0,
			"Stream stride is zero. The most likely cause is malformed resource metadata for a typed stream."
		);
		self.size / self.stride
	}
}

#[derive(
	Clone, Copy, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Eq,
)]
pub enum Streams {
	Vertices(VertexSemantics),
	Indices(IndexStreamTypes),
	Meshlets,
}

pub trait Size {
	fn size(&self) -> usize;
}

impl Size for VertexSemantics {
	fn size(&self) -> usize {
		match self {
			VertexSemantics::Position => 3 * 4,
			VertexSemantics::Normal => 3 * 4,
			VertexSemantics::Tangent => 4 * 4,
			VertexSemantics::BiTangent => 3 * 4,
			VertexSemantics::UV => 2 * 4,
			VertexSemantics::Color => 4 * 4,
			VertexSemantics::Joints => 4 * 2,
			VertexSemantics::Weights => 4 * 4,
		}
	}
}

impl Size for Vec<VertexComponent> {
	fn size(&self) -> usize {
		let mut size = 0;

		for component in self {
			size += component.semantic.size();
		}

		size
	}
}

// Image

#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum Gamma {
	Linear,
	SRGB,
}

#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum Formats {
	BC5,
	BC5SNORM,
	RG8,
	/// 16-bit floating-point luminous intensity per texel.
	R16F,
	RGB8,
	RGBA8,
	BC7,
	RGB16,
	RGBA16,
	BC7SRGB,
	RGBA16F,
	/// Eight-bit RGBA color encoded with the sRGB transfer function.
	RGBA8SRGB,
}

impl Formats {
	/// Returns the bytes one texel occupies, or `None` for block-compressed formats.
	///
	/// Image processors and mip generators size their buffers with this, then apply their own supported-format guard.
	pub const fn texel_bytes(self) -> Option<usize> {
		match self {
			Formats::RG8 | Formats::R16F => Some(2),
			Formats::RGB8 => Some(3),
			Formats::RGBA8 | Formats::RGBA8SRGB => Some(4),
			Formats::RGB16 => Some(6),
			Formats::RGBA16 | Formats::RGBA16F => Some(8),
			Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => None,
		}
	}

	/// Returns the bytes one level of `extent` occupies, counting whole 4x4 blocks for block-compressed formats.
	pub fn level_size(self, extent: utils::Extent) -> Option<usize> {
		let (width, height) = (extent.width() as usize, extent.height() as usize);
		match self.texel_bytes() {
			Some(texel_bytes) => width.checked_mul(height)?.checked_mul(texel_bytes),
			// Every supported block format stores one 4x4 texel block in 16 bytes.
			None => width.div_ceil(4).checked_mul(height.div_ceil(4))?.checked_mul(16),
		}
	}
}
