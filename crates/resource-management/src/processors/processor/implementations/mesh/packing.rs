/// The `MeshProcessor` struct configures the common mesh-processing pipeline used after format-specific import.
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

	/// Starts a short-lived processing session that borrows one source primitive at a time.
	///
	/// Call [`MeshProcessorSession::push_primitive`] for each imported primitive, then call
	/// [`MeshProcessorSession::finish_into`] to write the payload directly, or
	/// [`MeshProcessorSession::finish`] when the caller needs an owned payload.
	pub fn begin(
		self,
		vertex_layout: Vec<VertexComponent>,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
	) -> Result<MeshProcessorSession, MeshProcessingError> {
		validate_vertex_layout(&vertex_layout)?;
		let skeleton_nodes = skeleton_node_count(skeleton.as_ref())?;
		for (skin_index, skin) in skins.iter().enumerate() {
			validate_skin_binding(skin_index, skin, skeleton_nodes)?;
		}

		let mut stream_order = vertex_layout
			.iter()
			.map(|component| Streams::Vertices(component.semantic))
			.collect::<Vec<_>>();
		stream_order.sort_by_key(|stream| match stream {
			Streams::Vertices(semantic) => vertex_semantic_order(*semantic),
			_ => usize::MAX,
		});
		stream_order.extend([
			Streams::Indices(IndexStreamTypes::Vertices),
			Streams::Indices(IndexStreamTypes::Triangles),
			Streams::Indices(IndexStreamTypes::Meshlets),
			Streams::Meshlets,
		]);

		Ok(MeshProcessorSession {
			triangle_front_face_winding: self.triangle_front_face_winding,
			vertex_layout,
			skeleton,
			skeleton_nodes,
			skins,
			blocks: stream_order.into_iter().map(PackedStreamBlock::new).collect(),
			primitives: Vec::new(),
			scratch: MeshProcessingScratch::default(),
		})
	}
}

/// The `MeshPrimitiveProcessingError` enum preserves source-format failures alongside common processor failures.
#[derive(Debug, PartialEq, Eq)]
pub enum MeshPrimitiveProcessingError<E> {
	Source(E),
	Processing(MeshProcessingError),
}

/// The `MeshProcessorSession` struct keeps reusable scratch and final stream writers alive across borrowed primitives.
pub struct MeshProcessorSession {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	vertex_layout: Vec<VertexComponent>,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	skeleton_nodes: Option<usize>,
	skins: Vec<SkinBinding>,
	blocks: Vec<PackedStreamBlock>,
	primitives: Vec<PrimitiveModel>,
	scratch: MeshProcessingScratch,
}

impl MeshProcessorSession {
	/// Adds a final skin binding and returns the palette index that a later source primitive should reference.
	pub fn add_skin(&mut self, skin: SkinBinding) -> Result<u32, MeshProcessingError> {
		let skin_index = self.skins.len();
		validate_skin_binding(skin_index, &skin, self.skeleton_nodes)?;
		let skin_index =
			u32::try_from(skin_index).map_err(|_| MeshProcessingError::TooManySkinBindings { skins: skin_index })?;
		self.skins.push(skin);
		Ok(skin_index)
	}

	/// Processes one borrowed primitive immediately so the handler can reuse its source-format scratch afterward.
	pub fn push_primitive<P: MeshPrimitiveSource>(
		&mut self,
		primitive: &P,
	) -> Result<(), MeshPrimitiveProcessingError<P::Error>> {
		self.scratch.block_lengths.clear();
		self.scratch
			.block_lengths
			.extend(self.blocks.iter().map(|block| block.bytes.len()));

		let result = self.push_primitive_inner(primitive);
		if result.is_err() {
			for (block, &length) in self.blocks.iter_mut().zip(&self.scratch.block_lengths) {
				block.bytes.truncate(length);
			}
		}
		result
	}

	/// Packs one primitive into aggregate stream writers while using scratch only for meshopt's random-access inputs.
	fn push_primitive_inner<P: MeshPrimitiveSource>(
		&mut self,
		primitive: &P,
	) -> Result<(), MeshPrimitiveProcessingError<P::Error>> {
		let primitive_index = self.primitives.len();
		let positions = primitive.positions().map_err(MeshPrimitiveProcessingError::Source)?;
		let position_count = positions.len();
		self.scratch.positions.clear();
		self.scratch.positions.reserve(position_count);
		for position in positions {
			self.scratch
				.positions
				.push(position.map_err(MeshPrimitiveProcessingError::Source)?);
		}
		let bounds = bounding_box_from_positions(&self.scratch.positions).ok_or(MeshPrimitiveProcessingError::Processing(
			MeshProcessingError::InvalidPositionData,
		))?;

		let indices = primitive.indices().map_err(MeshPrimitiveProcessingError::Source)?;
		self.scratch.indices.clear();
		self.scratch.indices.reserve(indices.len());
		for index in indices {
			self.scratch
				.indices
				.push(index.map_err(MeshPrimitiveProcessingError::Source)?);
		}
		if !self.scratch.indices.len().is_multiple_of(3) {
			return Err(MeshPrimitiveProcessingError::Processing(
				MeshProcessingError::InvalidTriangleIndexCount,
			));
		}
		orient_triangle_indices_in_place(&mut self.scratch.indices, self.triangle_front_face_winding);
		meshopt::optimize_vertex_cache_in_place(&mut self.scratch.indices, position_count);

		self.scratch.position_bytes.clear();
		self.scratch.position_bytes.reserve(position_count.saturating_mul(12));
		for position in &self.scratch.positions {
			write_f32_components(&mut self.scratch.position_bytes, position);
		}
		let meshlet_vertex_adapter = meshopt::VertexDataAdapter::new(&self.scratch.position_bytes, 12, 0)
			.map_err(|_| MeshPrimitiveProcessingError::Processing(MeshProcessingError::FailedToBuildMeshlets))?;
		let meshlets = meshopt::clusterize::build_meshlets(
			&self.scratch.indices,
			&meshlet_vertex_adapter,
			MESHLET_MAX_VERTICES,
			MESHLET_MAX_TRIANGLES,
			MESHLET_CONE_WEIGHT,
		);

		let vertex_skin = primitive.vertex_skin().map_err(MeshPrimitiveProcessingError::Source)?;
		validate_primitive_metadata(
			primitive_index,
			primitive.transform_node(),
			primitive.skin(),
			vertex_skin.is_some(),
			&self.vertex_layout,
			self.skeleton_nodes,
			&self.skins,
		)
		.map_err(MeshPrimitiveProcessingError::Processing)?;

		let mut primitive_streams = Vec::with_capacity(self.vertex_layout.len() + 4);
		primitive_streams.push(append_f32_slice(
			&mut self.blocks,
			Streams::Vertices(VertexSemantics::Position),
			&self.scratch.positions,
		));
		append_optional_f32(
			&mut primitive_streams,
			&mut self.blocks,
			VertexSemantics::Normal,
			position_count,
			primitive.normals().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut primitive_streams,
			&mut self.blocks,
			VertexSemantics::Tangent,
			position_count,
			primitive.tangents().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut primitive_streams,
			&mut self.blocks,
			VertexSemantics::BiTangent,
			position_count,
			primitive.bitangents().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut primitive_streams,
			&mut self.blocks,
			VertexSemantics::UV,
			position_count,
			primitive.uvs().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut primitive_streams,
			&mut self.blocks,
			VertexSemantics::Color,
			position_count,
			primitive.colors().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		if let Some(vertex_skin) = vertex_skin {
			append_vertex_skin(
				&mut primitive_streams,
				&mut self.blocks,
				primitive_index,
				position_count,
				vertex_skin,
				&self.skins[primitive.skin().expect("validated skinned primitive") as usize],
			)?;
		}

		primitive_streams.push(append_generated_stream(
			&mut self.blocks,
			Streams::Indices(IndexStreamTypes::Vertices),
			|bytes| {
				for meshlet in meshlets.iter() {
					for &index in meshlet.vertices {
						bytes.extend((index as u16).to_le_bytes());
					}
				}
			},
		));
		primitive_streams.push(append_generated_stream(
			&mut self.blocks,
			Streams::Indices(IndexStreamTypes::Triangles),
			|bytes| {
				for &index in &self.scratch.indices {
					bytes.extend((index as u16).to_le_bytes());
				}
			},
		));
		primitive_streams.push(append_generated_stream(
			&mut self.blocks,
			Streams::Indices(IndexStreamTypes::Meshlets),
			|bytes| {
				for meshlet in meshlets.iter() {
					bytes.extend_from_slice(meshlet.triangles);
				}
			},
		));
		primitive_streams.push(append_generated_stream(&mut self.blocks, Streams::Meshlets, |bytes| {
			for meshlet in meshlets.iter() {
				let bounds = meshopt::clusterize::compute_meshlet_bounds(meshlet, &meshlet_vertex_adapter);
				write_meshlet_record(bytes, meshlet, &bounds);
			}
		}));

		self.primitives.push(PrimitiveModel {
			material: primitive.material().clone(),
			transform_node: primitive.transform_node(),
			skin: primitive.skin(),
			streams: primitive_streams,
			quantization: None,
			bounding_box: bounds,
			vertex_count: position_count as u32,
		});
		Ok(())
	}

	/// Returns the exact number of bytes that [`Self::finish_into`] will write.
	pub fn payload_size(&self) -> usize {
		self.blocks.iter().map(|block| block.bytes.len()).sum()
	}

	/// Finishes stream offsets and writes each completed stream directly to `writer`.
	///
	/// `W` remains generic so stream writes do not use dynamic dispatch. Reserve
	/// [`Self::payload_size`] bytes before calling this method.
	pub fn finish_into<W: std::io::Write>(self, writer: &mut W) -> std::io::Result<(MeshModel, Vec<StreamDescription>)> {
		let (mesh, stream_descriptions, blocks) = self.finish_parts();
		for block in blocks {
			std::io::Write::write_all(writer, &block.bytes)?;
		}
		Ok((mesh, stream_descriptions))
	}

	/// Finishes stream offsets and asynchronously writes each completed stream into resource storage.
	///
	/// Reserve [`Self::payload_size`] bytes before calling this method. Use
	/// [`Self::finish_into`] for a synchronous non-resource sink.
	pub async fn finish_into_resource(
		self,
		writer: &mut crate::resource::ResourceTransaction<'_>,
	) -> std::io::Result<(MeshModel, Vec<StreamDescription>)> {
		let (mesh, stream_descriptions, blocks) = self.finish_parts();
		for block in blocks {
			let compio::buf::BufResult(result, _) = compio::io::AsyncWriteExt::write_all(&mut *writer, block.bytes).await;
			result?;
		}
		Ok((mesh, stream_descriptions))
	}

	/// Finishes aggregate stream offsets and moves final metadata into the stored mesh resource.
	///
	/// Use [`Self::finish_into`] when the payload can go directly to resource storage.
	pub fn finish(self) -> ProcessedMesh {
		let mut buffer = Vec::with_capacity(self.payload_size());
		let (mesh, stream_descriptions, blocks) = self.finish_parts();
		for block in blocks {
			buffer.extend_from_slice(&block.bytes);
		}

		ProcessedMesh {
			mesh,
			stream_descriptions,
			buffer: buffer.into_boxed_slice(),
		}
	}

	/// Builds final stream metadata once before a caller moves each completed block to its selected sink.
	fn finish_parts(mut self) -> (MeshModel, Vec<StreamDescription>, Vec<PackedStreamBlock>) {
		let active_vertex_components = self
			.vertex_layout
			.into_iter()
			.filter(|component| {
				self.blocks
					.iter()
					.find(|block| block.stream_type == Streams::Vertices(component.semantic))
					.is_some_and(|block| !block.bytes.is_empty())
			})
			.collect::<Vec<_>>();
		self.blocks.retain(|block| !block.bytes.is_empty());
		let mut streams = Vec::with_capacity(self.blocks.len());
		let mut stream_descriptions = Vec::with_capacity(self.blocks.len());
		let mut offset = 0;
		for block in &self.blocks {
			let size = block.bytes.len();
			streams.push(Stream {
				offset,
				size,
				stream_type: block.stream_type,
				stride: stream_stride(block.stream_type),
			});
			stream_descriptions.push(StreamDescription::new(stream_name(block.stream_type), size, offset));
			offset += size;
		}
		(
			MeshModel {
				skeleton: self.skeleton,
				skins: self.skins,
				vertex_components: active_vertex_components,
				streams,
				primitives: self.primitives,
			},
			stream_descriptions,
			self.blocks,
		)
	}
}

/// The `ProcessedMesh` struct stores the packed mesh resource and its stream payload.
#[derive(Debug)]
pub struct ProcessedMesh {
	pub mesh: MeshModel,
	pub stream_descriptions: Vec<StreamDescription>,
	pub buffer: Box<[u8]>,
}

#[derive(Default)]
struct MeshProcessingScratch {
	positions: Vec<[f32; 3]>,
	indices: Vec<u32>,
	position_bytes: Vec<u8>,
	block_lengths: Vec<usize>,
}

struct PackedStreamBlock {
	stream_type: Streams,
	bytes: Vec<u8>,
}

impl PackedStreamBlock {
	fn new(stream_type: Streams) -> Self {
		Self {
			stream_type,
			bytes: Vec::new(),
		}
	}
}

fn append_f32_slice<const N: usize>(blocks: &mut [PackedStreamBlock], stream_type: Streams, values: &[[f32; N]]) -> Stream {
	append_generated_stream(blocks, stream_type, |bytes| {
		for value in values {
			write_f32_components(bytes, value);
		}
	})
}

fn append_optional_f32<const N: usize, I, E>(
	primitive_streams: &mut Vec<Stream>,
	blocks: &mut [PackedStreamBlock],
	semantic: VertexSemantics,
	position_count: usize,
	values: Option<I>,
) -> Result<(), MeshPrimitiveProcessingError<E>>
where
	I: ExactSizeIterator<Item = Result<[f32; N], E>>,
{
	let Some(values) = values else {
		return Ok(());
	};
	if values.len() != position_count {
		return Err(MeshPrimitiveProcessingError::Processing(
			MeshProcessingError::AttributeLengthMismatch(semantic, 0),
		));
	}
	let stream_type = Streams::Vertices(semantic);
	let block =
		blocks
			.iter_mut()
			.find(|block| block.stream_type == stream_type)
			.ok_or(MeshPrimitiveProcessingError::Processing(
				MeshProcessingError::MissingAttribute(semantic, 0),
			))?;
	let offset = block.bytes.len();
	for value in values {
		write_f32_components(&mut block.bytes, &value.map_err(MeshPrimitiveProcessingError::Source)?);
	}
	primitive_streams.push(Stream {
		offset,
		size: block.bytes.len() - offset,
		stream_type,
		stride: stream_stride(stream_type),
	});
	Ok(())
}

fn append_vertex_skin<I, E>(
	primitive_streams: &mut Vec<Stream>,
	blocks: &mut [PackedStreamBlock],
	primitive: usize,
	position_count: usize,
	values: I,
	skin: &SkinBinding,
) -> Result<(), MeshPrimitiveProcessingError<E>>
where
	I: ExactSizeIterator<Item = Result<VertexSkin, E>>,
{
	if values.len() != position_count {
		return Err(MeshPrimitiveProcessingError::Processing(
			MeshProcessingError::SkinVertexCountMismatch {
				primitive,
				values: values.len(),
				positions: position_count,
			},
		));
	}
	let joints_index = blocks
		.iter()
		.position(|block| block.stream_type == Streams::Vertices(VertexSemantics::Joints))
		.ok_or(MeshPrimitiveProcessingError::Processing(
			MeshProcessingError::MissingSkinVertexComponent(VertexSemantics::Joints),
		))?;
	let weights_index = blocks
		.iter()
		.position(|block| block.stream_type == Streams::Vertices(VertexSemantics::Weights))
		.ok_or(MeshPrimitiveProcessingError::Processing(
			MeshProcessingError::MissingSkinVertexComponent(VertexSemantics::Weights),
		))?;
	let joints_offset = blocks[joints_index].bytes.len();
	let weights_offset = blocks[weights_index].bytes.len();
	for (vertex, value) in values.enumerate() {
		let value = value.map_err(MeshPrimitiveProcessingError::Source)?;
		validate_vertex_skin(primitive, vertex, value, skin).map_err(MeshPrimitiveProcessingError::Processing)?;
		for joint in value.joints {
			blocks[joints_index].bytes.extend(joint.to_le_bytes());
		}
		write_f32_components(&mut blocks[weights_index].bytes, &value.weights);
	}
	for (index, offset, semantic) in [
		(joints_index, joints_offset, VertexSemantics::Joints),
		(weights_index, weights_offset, VertexSemantics::Weights),
	] {
		let stream_type = Streams::Vertices(semantic);
		primitive_streams.push(Stream {
			offset,
			size: blocks[index].bytes.len() - offset,
			stream_type,
			stride: stream_stride(stream_type),
		});
	}
	Ok(())
}

fn append_generated_stream(blocks: &mut [PackedStreamBlock], stream_type: Streams, write: impl FnOnce(&mut Vec<u8>)) -> Stream {
	let block = blocks
		.iter_mut()
		.find(|block| block.stream_type == stream_type)
		.expect("processor stream order should contain every generated stream");
	let offset = block.bytes.len();
	write(&mut block.bytes);
	Stream {
		offset,
		size: block.bytes.len() - offset,
		stream_type,
		stride: stream_stride(stream_type),
	}
}

fn write_f32_components<const N: usize>(bytes: &mut Vec<u8>, value: &[f32; N]) {
	for component in value {
		bytes.extend(component.to_le_bytes());
	}
}

fn bounding_box_from_positions(positions: &[[f32; 3]]) -> Option<[[f32; 3]; 2]> {
	let first = *positions.first()?;
	if first.iter().any(|component| !component.is_finite()) {
		return None;
	}
	let mut minimum = first;
	let mut maximum = first;
	for position in &positions[1..] {
		if position.iter().any(|component| !component.is_finite()) {
			return None;
		}
		for axis in 0..3 {
			minimum[axis] = minimum[axis].min(position[axis]);
			maximum[axis] = maximum[axis].max(position[axis]);
		}
	}
	Some([minimum, maximum])
}

fn orient_triangle_indices_in_place(indices: &mut [u32], winding: TriangleFrontFaceWinding) {
	if winding == TriangleFrontFaceWinding::Clockwise {
		for triangle in indices.chunks_exact_mut(3) {
			triangle.swap(1, 2);
		}
	}
}

pub fn orient_triangle_indices_for_front_face(mut indices: Vec<u32>, winding: TriangleFrontFaceWinding) -> Vec<u32> {
	debug_assert_eq!(
		indices.len() % 3,
		0,
		"Triangle index streams must be emitted in groups of three"
	);
	orient_triangle_indices_in_place(&mut indices, winding);
	indices
}

fn write_meshlet_record(bytes: &mut Vec<u8>, meshlet: meshopt::clusterize::Meshlet<'_>, bounds: &meshopt::clusterize::Bounds) {
	let offset = bytes.len();
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
	debug_assert_eq!(bytes.len() - offset, MESHLET_STREAM_STRIDE);
}

fn stream_stride(stream_type: Streams) -> usize {
	match stream_type {
		Streams::Vertices(semantic) => semantic.size(),
		Streams::Indices(IndexStreamTypes::Vertices | IndexStreamTypes::Triangles) => IntegralTypes::U16.size(),
		Streams::Indices(IndexStreamTypes::Meshlets) => IntegralTypes::U8.size(),
		Streams::Meshlets => MESHLET_STREAM_STRIDE,
	}
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

const fn vertex_semantic_order(semantic: VertexSemantics) -> usize {
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

use super::{
	source::{MeshPrimitiveSource, VertexSkin},
	validation::{
		MeshProcessingError, skeleton_node_count, validate_primitive_metadata, validate_skin_binding, validate_vertex_layout,
		validate_vertex_skin,
	},
};
use crate::{
	ReferenceModel, StreamDescription,
	resources::{
		mesh::{MeshModel, PrimitiveModel},
		skeleton::{SkeletonModel, SkinBinding},
	},
	types::{IndexStreamTypes, IntegralTypes, Size, Stream, Streams, VertexComponent, VertexSemantics},
};
