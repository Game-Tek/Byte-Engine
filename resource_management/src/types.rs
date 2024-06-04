use futures::future::try_join_all;
use polodb_core::bson;
use serde::Deserialize;

use crate::{resource::resource_handler::ReadTargets, CreateResource, LoadResults, Loader, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

// Audio

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Clone, Copy)]
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

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize, Clone)]
pub enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

/// Enumerates the types of shaders that can be created.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	UV,
	Color,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum QuantizationSchemes {
	Quantization,
	Octahedral,
	OctahedralQuantization,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum IndexStreamTypes {
	Vertices,
	Meshlets,
	Triangles,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct IndexStream {
	pub stream_type: IndexStreamTypes,
	pub offset: usize,
	pub count: u32,
	pub data_type: IntegralTypes,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Stream {
	pub stream_type: Streams,
	pub offset: usize,
	pub size: usize,
	pub stride: usize,
}

impl Stream {
	/// Returns the number of logical elements (not bytes) in the stream.
	pub fn count(&self) -> usize {
		self.size / self.stride
	}	
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum Streams {
	Vertices(VertexSemantics),
	Indices(IndexStreamTypes),
	Meshlets,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
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

pub struct CreateImage {
	pub format: Formats,
	pub extent: [u32; 3],
}

impl CreateResource for CreateImage {}

// #[derive(Debug, serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
// pub enum CompressionSchemes {
// 	BC7,
// 	BC5,
// }

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Gamma {
	Linear,
	SRGB,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Formats {
	BC5,
	RG8,
	RGB8,
	RGBA8,
	BC7,
	RGB16,
	RGBA16,
}