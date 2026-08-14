use std::alloc::Allocator;

use super::{
	source::{MeshAttributeData, MeshPrimitiveSource, MeshSource, OwnedMeshSource},
	validation::{validate_skin_source, validate_vertex_layout, MeshProcessingError},
};
use crate::{
	resources::{
		mesh::{MeshModel, PrimitiveModel},
		skeleton::{SkeletonModel, SkinBinding},
	},
	types::{IndexStreamTypes, IntegralTypes, Size, Stream, Streams, VertexComponent, VertexSemantics},
	ReferenceModel, StreamDescription,
};

/// The `TriangleFrontFaceWinding` enum identifies the triangle winding used as the processed mesh front face.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum TriangleFrontFaceWinding {
	#[default]
	Clockwise,
	CounterClockwise,
}

const MESHLET_MAX_VERTICES: usize = 64;
const MESHLET_MAX_TRIANGLES: usize = 124;
const MESHLET_CONE_WEIGHT: f32 = 0.25;
pub(super) const MESHLET_STREAM_STRIDE: usize = 52;

/// The `MeshProcessor` struct packs normalized mesh data into the resource-management mesh stream format.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeshProcessor {
	triangle_front_face_winding: TriangleFrontFaceWinding,
}

impl MeshProcessor {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn triangle_front_face_winding(&self) -> TriangleFrontFaceWinding {
		self.triangle_front_face_winding
	}

	pub fn set_triangle_front_face_winding(&mut self, winding: TriangleFrontFaceWinding) {
		self.triangle_front_face_winding = winding;
	}

	pub fn with_triangle_front_face_winding(mut self, winding: TriangleFrontFaceWinding) -> Self {
		self.set_triangle_front_face_winding(winding);
		self
	}

	/// Packs source primitives and retains validated skeletal metadata alongside their vertex streams.
	pub fn process<T: MeshSource>(&self, source: &T) -> Result<ProcessedMesh, MeshProcessingError> {
		validate_vertex_layout(source.vertex_layout())?;
		validate_skin_source(source)?;
		self.process_validated(source, source.skeleton().cloned(), source.skins().to_vec())
	}

	/// Consumes processor-owned source data so large skeleton and skin metadata can move into the result without cloning.
	pub fn process_owned<A: Allocator>(&self, mut source: OwnedMeshSource<A>) -> Result<ProcessedMesh, MeshProcessingError> {
		validate_vertex_layout(source.vertex_layout())?;
		validate_skin_source(&source)?;
		let skeleton = source.skeleton.take();
		let skins = std::mem::take(&mut source.skins);
		self.process_validated(&source, skeleton, skins)
	}

	/// Packs a validated source while moving or cloning metadata according to the caller's ownership model.
	fn process_validated<T: MeshSource>(
		&self,
		source: &T,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
	) -> Result<ProcessedMesh, MeshProcessingError> {
		let active_vertex_layout = active_vertex_layout(source);
		let vertex_streams = ordered_vertex_streams(&active_vertex_layout);
		let stream_order = make_stream_order(&vertex_streams);
		let mut packed_blocks = stream_order
			.iter()
			.map(|stream_type| PackedStreamBlock::new(*stream_type))
			.collect::<Vec<_>>();
		let mut primitives = Vec::with_capacity(source.primitive_count());

		for primitive in source.primitives() {
			let packed_primitive = self.pack_primitive(primitive, &vertex_streams, &mut packed_blocks)?;
			primitives.push(packed_primitive);
		}

		let mut next_offset = 0usize;
		let mut mesh_streams = Vec::with_capacity(packed_blocks.len());
		let mut stream_descriptions = Vec::with_capacity(packed_blocks.len());
		let mut buffer = Vec::new();

		for block in packed_blocks {
			let size = block.total_size();
			mesh_streams.push(Stream {
				offset: next_offset,
				size,
				stream_type: block.stream_type,
				stride: stream_stride(block.stream_type),
			});
			stream_descriptions.push(StreamDescription::new(stream_name(block.stream_type), size, next_offset));
			next_offset += size;
			buffer.extend(block.into_bytes());
		}

		Ok(ProcessedMesh {
			mesh: MeshModel {
				skeleton,
				skins,
				vertex_components: active_vertex_layout,
				streams: mesh_streams,
				primitives,
			},
			stream_descriptions,
			buffer: buffer.into_boxed_slice(),
		})
	}

	/// Packs one primitive into the shared stream blocks used by the resulting mesh resource.
	fn pack_primitive<T: MeshPrimitiveSource>(
		&self,
		primitive: T,
		vertex_streams: &[(VertexSemantics, u32)],
		packed_blocks: &mut [PackedStreamBlock],
	) -> Result<PrimitiveModel, MeshProcessingError> {
		let position_data = primitive
			.attribute(VertexSemantics::Position, 0)
			.ok_or(MeshProcessingError::MissingPositionAttribute)?;
		let position_count = position_data.len();

		if primitive.vertex_count() != position_count {
			return Err(MeshProcessingError::InconsistentVertexCount);
		}

		let position_bytes = match position_data {
			MeshAttributeData::F32x3(values) => values
				.iter()
				.map(|position| [position[0], position[1], position[2]])
				.collect::<Vec<_>>(),
			_ => return Err(MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Position)),
		};

		let triangle_indices = primitive
			.indices(IndexStreamTypes::Triangles)
			.ok_or(MeshProcessingError::MissingTriangleIndices)?;
		let triangle_indices =
			orient_triangle_indices_for_front_face(triangle_indices.to_u32_vec(), self.triangle_front_face_winding);

		if !triangle_indices.len().is_multiple_of(3) {
			return Err(MeshProcessingError::InvalidTriangleIndexCount);
		}

		let optimized_triangle_indices = meshopt::optimize_vertex_cache(&triangle_indices, position_count);
		let meshlet_source_bytes = position_bytes
			.iter()
			.flat_map(|position| position.iter().flat_map(|component| component.to_le_bytes()))
			.collect::<Vec<u8>>();
		let meshlet_vertex_adapter = meshopt::VertexDataAdapter::new(&meshlet_source_bytes, 12, 0)
			.map_err(|_| MeshProcessingError::FailedToBuildMeshlets)?;
		let meshlets = meshopt::clusterize::build_meshlets(
			&optimized_triangle_indices,
			&meshlet_vertex_adapter,
			MESHLET_MAX_VERTICES,
			MESHLET_MAX_TRIANGLES,
			MESHLET_CONE_WEIGHT,
		);

		let mut primitive_streams = Vec::with_capacity(vertex_streams.len() + 4);

		for &(semantic, channel) in vertex_streams {
			let Some(data) = primitive.attribute(semantic, channel) else {
				continue;
			};

			if data.len() != position_count {
				return Err(MeshProcessingError::AttributeLengthMismatch(semantic, channel));
			}

			let stream_type = Streams::Vertices(semantic);
			let block = packed_blocks
				.iter_mut()
				.find(|block| block.stream_type == stream_type)
				.expect("vertex stream block should exist");
			let stride = stream_stride(stream_type);

			if data.element_size() != stride {
				return Err(MeshProcessingError::InvalidAttributeFormat(semantic));
			}

			let offset = block.total_size();
			let bytes = data.to_bytes();
			let size = bytes.len();
			block.chunks.push(bytes);
			primitive_streams.push(Stream {
				offset,
				size,
				stream_type,
				stride,
			});
		}

		let vertex_indices_bytes = meshlets
			.iter()
			.flat_map(|meshlet| meshlet.vertices.iter().map(|index| *index as u16).flat_map(u16::to_le_bytes))
			.collect::<Vec<u8>>();
		append_stream(
			&mut primitive_streams,
			packed_blocks,
			Streams::Indices(IndexStreamTypes::Vertices),
			vertex_indices_bytes,
		);

		let triangle_indices_bytes = optimized_triangle_indices
			.iter()
			.map(|index| *index as u16)
			.flat_map(u16::to_le_bytes)
			.collect::<Vec<u8>>();
		append_stream(
			&mut primitive_streams,
			packed_blocks,
			Streams::Indices(IndexStreamTypes::Triangles),
			triangle_indices_bytes,
		);

		let meshlet_indices_bytes = meshlets
			.iter()
			.flat_map(|meshlet| meshlet.triangles.iter().copied())
			.collect::<Vec<u8>>();
		append_stream(
			&mut primitive_streams,
			packed_blocks,
			Streams::Indices(IndexStreamTypes::Meshlets),
			meshlet_indices_bytes,
		);

		let meshlet_bytes = meshlets
			.iter()
			.flat_map(|meshlet| {
				let bounds = meshopt::clusterize::compute_meshlet_bounds(meshlet, &meshlet_vertex_adapter);
				meshlet_stream_record_bytes(meshlet, &bounds)
			})
			.collect::<Vec<u8>>();
		append_stream(&mut primitive_streams, packed_blocks, Streams::Meshlets, meshlet_bytes);

		Ok(PrimitiveModel {
			material: primitive.material().clone(),
			transform_node: primitive.transform_node(),
			skin: primitive.skin(),
			streams: primitive_streams,
			quantization: None,
			bounding_box: primitive.bounding_box(),
			vertex_count: position_count as u32,
		})
	}
}

/// The `ProcessedMesh` struct stores the packed mesh resource and its stream payload.
#[derive(Debug)]
pub struct ProcessedMesh {
	pub mesh: MeshModel,
	pub stream_descriptions: Vec<StreamDescription>,
	pub buffer: Box<[u8]>,
}
#[derive(Debug)]
struct PackedStreamBlock {
	stream_type: Streams,
	chunks: Vec<Vec<u8>>,
}

impl PackedStreamBlock {
	fn new(stream_type: Streams) -> Self {
		Self {
			stream_type,
			chunks: Vec::new(),
		}
	}

	fn total_size(&self) -> usize {
		self.chunks.iter().map(Vec::len).sum()
	}

	fn into_bytes(self) -> Vec<u8> {
		self.chunks.into_iter().flatten().collect()
	}
}
/// Returns the subset of the declared vertex layout that is backed by primitive data.
fn active_vertex_layout<T: MeshSource>(source: &T) -> Vec<VertexComponent> {
	source
		.vertex_layout()
		.iter()
		.filter(|component| {
			source
				.primitives()
				.any(|primitive| primitive.attribute(component.semantic, component.channel).is_some())
		})
		.cloned()
		.collect()
}

fn ordered_vertex_streams(vertex_layout: &[VertexComponent]) -> Vec<(VertexSemantics, u32)> {
	let mut streams = vertex_layout
		.iter()
		.map(|component| (component.semantic, component.channel))
		.collect::<Vec<_>>();
	streams.sort_by_key(|(semantic, channel)| (vertex_semantic_order(*semantic), *channel));
	streams
}

fn make_stream_order(vertex_streams: &[(VertexSemantics, u32)]) -> Vec<Streams> {
	let mut streams = vertex_streams
		.iter()
		.map(|(semantic, _)| Streams::Vertices(*semantic))
		.collect::<Vec<_>>();
	streams.extend([
		Streams::Indices(IndexStreamTypes::Vertices),
		Streams::Indices(IndexStreamTypes::Triangles),
		Streams::Indices(IndexStreamTypes::Meshlets),
		Streams::Meshlets,
	]);
	streams
}

fn append_stream(
	primitive_streams: &mut Vec<Stream>,
	packed_blocks: &mut [PackedStreamBlock],
	stream_type: Streams,
	bytes: Vec<u8>,
) {
	let block = packed_blocks
		.iter_mut()
		.find(|block| block.stream_type == stream_type)
		.expect("packed stream block should exist");
	let offset = block.total_size();
	let size = bytes.len();
	block.chunks.push(bytes);
	primitive_streams.push(Stream {
		offset,
		size,
		stream_type,
		stride: stream_stride(stream_type),
	});
}

pub fn orient_triangle_indices_for_front_face(mut indices: Vec<u32>, winding: TriangleFrontFaceWinding) -> Vec<u32> {
	debug_assert_eq!(
		indices.len() % 3,
		0,
		"Triangle index streams must be emitted in groups of three"
	);

	if winding == TriangleFrontFaceWinding::Clockwise {
		for triangle in indices.chunks_exact_mut(3) {
			triangle.swap(1, 2);
		}
	}

	indices
}

fn stream_stride(stream_type: Streams) -> usize {
	match stream_type {
		Streams::Vertices(semantic) => semantic.size(),
		Streams::Indices(IndexStreamTypes::Vertices) => IntegralTypes::U16.size(),
		Streams::Indices(IndexStreamTypes::Triangles) => IntegralTypes::U16.size(),
		Streams::Indices(IndexStreamTypes::Meshlets) => IntegralTypes::U8.size(),
		Streams::Meshlets => MESHLET_STREAM_STRIDE,
	}
}

/// Packs a meshopt meshlet and its object-space bounds into the meshlet resource stream.
fn meshlet_stream_record_bytes(meshlet: meshopt::clusterize::Meshlet<'_>, bounds: &meshopt::clusterize::Bounds) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(MESHLET_STREAM_STRIDE);
	bytes.push(meshlet.vertices.len() as u8);
	bytes.push((meshlet.triangles.len() / 3) as u8);
	bytes.extend([0u8; 2]);
	for value in bounds.center.iter().copied().chain([bounds.radius]) {
		bytes.extend(value.to_le_bytes());
	}
	for value in bounds.cone_apex.iter().copied().chain([bounds.cone_cutoff]) {
		bytes.extend(value.to_le_bytes());
	}
	for value in bounds.cone_axis.iter().copied().chain([0.0]) {
		bytes.extend(value.to_le_bytes());
	}

	debug_assert_eq!(bytes.len(), MESHLET_STREAM_STRIDE);
	bytes
}

fn stream_name(stream_type: Streams) -> &'static str {
	match stream_type {
		Streams::Vertices(VertexSemantics::Position) => "Vertex.Position",
		Streams::Vertices(VertexSemantics::Normal) => "Vertex.Normal",
		Streams::Vertices(VertexSemantics::Tangent) => "Vertex.Tangent",
		Streams::Vertices(VertexSemantics::BiTangent) => "Vertex.BiTangent",
		Streams::Vertices(VertexSemantics::UV) => "Vertex.UV",
		Streams::Vertices(VertexSemantics::Color) => "Vertex.Color",
		Streams::Vertices(VertexSemantics::Joints) => "Vertex.Joints",
		Streams::Vertices(VertexSemantics::Weights) => "Vertex.Weights",
		Streams::Indices(IndexStreamTypes::Vertices) => "VertexIndices",
		Streams::Indices(IndexStreamTypes::Triangles) => "TriangleIndices",
		Streams::Indices(IndexStreamTypes::Meshlets) => "MeshletIndices",
		Streams::Meshlets => "Meshlets",
	}
}

fn vertex_semantic_order(semantic: VertexSemantics) -> usize {
	match semantic {
		VertexSemantics::Position => 0,
		VertexSemantics::Normal => 1,
		VertexSemantics::Tangent => 2,
		VertexSemantics::BiTangent => 3,
		VertexSemantics::UV => 4,
		VertexSemantics::Color => 5,
		VertexSemantics::Joints => 6,
		VertexSemantics::Weights => 7,
	}
}
