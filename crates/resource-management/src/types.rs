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

/// Enumerates the types of shaders that can be created.
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

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum IntegralTypes {
	U8,
	I8,
	U16,
	I16,
	U32,
	I32,
	F16,
	F32,
	F64,
}

#[derive(
	Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, PartialEq, Eq,
)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum QuantizationSchemes {
	Quantization,
	Octahedral,
	OctahedralQuantization,
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
pub struct IndexStream {
	pub stream_type: IndexStreamTypes,
	pub offset: usize,
	pub count: u32,
	pub data_type: IntegralTypes,
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

#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MeshletStream {
	pub offset: usize,
	pub count: u32,
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

impl Size for IntegralTypes {
	fn size(&self) -> usize {
		match self {
			IntegralTypes::U8 => 1,
			IntegralTypes::I8 => 1,
			IntegralTypes::U16 => 2,
			IntegralTypes::I16 => 2,
			IntegralTypes::U32 => 4,
			IntegralTypes::I32 => 4,
			IntegralTypes::F16 => 2,
			IntegralTypes::F32 => 4,
			IntegralTypes::F64 => 8,
		}
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
	RGB8,
	RGBA8,
	BC7,
	RGB16,
	RGBA16,
	BC7SRGB,
	RGBA16F,
}

#[cfg(test)]
mod tests {
	use super::{BitDepths, IntegralTypes, Size, Stream, Streams, VertexComponent, VertexSemantics};

	#[test]
	fn bit_depths_convert_to_their_exact_number_of_bits() {
		assert_eq!(usize::from(BitDepths::Eight), 8);
		assert_eq!(usize::from(BitDepths::Sixteen), 16);
		assert_eq!(usize::from(BitDepths::TwentyFour), 24);
		assert_eq!(usize::from(BitDepths::ThirtyTwo), 32);
	}

	#[test]
	fn integral_and_vertex_sizes_match_the_binary_contract() {
		let integral_sizes = [
			(IntegralTypes::U8, 1),
			(IntegralTypes::I8, 1),
			(IntegralTypes::U16, 2),
			(IntegralTypes::I16, 2),
			(IntegralTypes::U32, 4),
			(IntegralTypes::I32, 4),
			(IntegralTypes::F16, 2),
			(IntegralTypes::F32, 4),
			(IntegralTypes::F64, 8),
		];
		for (kind, expected) in integral_sizes {
			assert_eq!(kind.size(), expected);
		}

		let components = vec![
			VertexComponent {
				semantic: VertexSemantics::Position,
				format: "float3".into(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::UV,
				format: "float2".into(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Joints,
				format: "ushort4".into(),
				channel: 0,
			},
		];
		assert_eq!(components.size(), 12 + 8 + 8);
	}

	#[test]
	fn stream_count_uses_byte_size_and_stride() {
		let stream = Stream {
			stream_type: Streams::Vertices(VertexSemantics::Position),
			offset: 64,
			size: 120,
			stride: 12,
		};
		assert_eq!(stream.count(), 10);
	}

	#[test]
	#[should_panic(expected = "Stream stride is zero")]
	fn stream_count_rejects_zero_stride_metadata() {
		Stream {
			stream_type: Streams::Meshlets,
			offset: 0,
			size: 10,
			stride: 0,
		}
		.count();
	}
}
