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

/// The `AlphaMode` enum identifies how alpha affects surface visibility, from the material graph to the renderer.
#[derive(
	Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Copy,
)]
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

impl From<crate::shader::generator::Stages> for ShaderTypes {
	/// Returns the resource stage of shaders generated for `stage`.
	fn from(stage: crate::shader::generator::Stages) -> Self {
		use crate::shader::generator::Stages;

		match stage {
			Stages::Vertex => Self::Vertex,
			Stages::Fragment => Self::Fragment,
			Stages::Compute { .. } => Self::Compute,
			Stages::Task { .. } => Self::Task,
			Stages::Mesh { .. } => Self::Mesh,
		}
	}
}

// Mesh

/// The `VertexSemantics` enum names what one vertex stream holds.
///
/// The declaration order is the canonical interleaved vertex layout, which mesh processing orders streams by, and
/// stored meshes encode each variant by its index, so new semantics go at the end.
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

impl VertexComponent {
	/// Returns the stream every importer declares for `semantic`: its canonical format on channel 0.
	///
	/// Build importer vertex layouts from these, then pass them to
	/// [`MeshProcessorSession::new`](crate::processors::mesh::MeshProcessorSession::new).
	pub fn canonical(semantic: VertexSemantics) -> Self {
		let format = match semantic {
			VertexSemantics::Position | VertexSemantics::Normal | VertexSemantics::BiTangent => "vec3f",
			VertexSemantics::UV => "vec2f",
			VertexSemantics::Joints => "vec4u16",
			VertexSemantics::Tangent | VertexSemantics::Color | VertexSemantics::Weights => "vec4f",
		};
		Self {
			semantic,
			format: format.to_string(),
			channel: 0,
		}
	}
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
			VertexSemantics::Position | VertexSemantics::Normal | VertexSemantics::BiTangent => 3 * 4,
			VertexSemantics::Tangent | VertexSemantics::Color | VertexSemantics::Weights => 4 * 4,
			VertexSemantics::UV => 2 * 4,
			VertexSemantics::Joints => 4 * 2,
		}
	}
}

impl Size for Vec<VertexComponent> {
	fn size(&self) -> usize {
		self.iter().map(|component| component.semantic.size()).sum()
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
	/// Two eight-bit channels, used for packed metallic-roughness maps.
	RG8,
	/// Two sixteen-bit channels, used for packed metallic-roughness maps from sixteen-bit sources.
	RG16,
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
			Formats::RGBA8 | Formats::RGBA8SRGB | Formats::RG16 => Some(4),
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
