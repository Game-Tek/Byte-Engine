use crate::{
	resources::{
		material::VariantModel,
		mesh::{MeshModel, PrimitiveModel},
	},
	types::{IndexStreamTypes, IntegralTypes, Stream, Streams, VertexComponent, VertexSemantics},
	ReferenceModel, StreamDescription,
};

/// The `TriangleFrontFaceWinding` enum describes which triangle winding should be treated as the mesh front face after processing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriangleFrontFaceWinding {
	Clockwise,
	CounterClockwise,
}

impl Default for TriangleFrontFaceWinding {
	fn default() -> Self {
		Self::Clockwise
	}
}

/// The `MeshAttributeData` enum describes borrowed attribute payloads that mesh sources expose to the mesh processor.
#[derive(Clone, Copy, Debug)]
pub enum MeshAttributeData<'a> {
	F32x2(&'a [[f32; 2]]),
	F32x3(&'a [[f32; 3]]),
	F32x4(&'a [[f32; 4]]),
	U16x4(&'a [[u16; 4]]),
}

impl MeshAttributeData<'_> {
	fn len(&self) -> usize {
		match self {
			MeshAttributeData::F32x2(values) => values.len(),
			MeshAttributeData::F32x3(values) => values.len(),
			MeshAttributeData::F32x4(values) => values.len(),
			MeshAttributeData::U16x4(values) => values.len(),
		}
	}

	fn element_size(&self) -> usize {
		match self {
			MeshAttributeData::F32x2(..) => 8,
			MeshAttributeData::F32x3(..) => 12,
			MeshAttributeData::F32x4(..) => 16,
			MeshAttributeData::U16x4(..) => 8,
		}
	}

	fn to_bytes(&self) -> Vec<u8> {
		match self {
			MeshAttributeData::F32x2(values) => values
				.iter()
				.flat_map(|value| value.iter().flat_map(|component| component.to_le_bytes()))
				.collect(),
			MeshAttributeData::F32x3(values) => values
				.iter()
				.flat_map(|value| value.iter().flat_map(|component| component.to_le_bytes()))
				.collect(),
			MeshAttributeData::F32x4(values) => values
				.iter()
				.flat_map(|value| value.iter().flat_map(|component| component.to_le_bytes()))
				.collect(),
			MeshAttributeData::U16x4(values) => values
				.iter()
				.flat_map(|value| value.iter().flat_map(|component| component.to_le_bytes()))
				.collect(),
		}
	}
}

/// The `MeshIndexData` enum describes borrowed index payloads that mesh sources expose to the mesh processor.
#[derive(Clone, Copy, Debug)]
pub enum MeshIndexData<'a> {
	U32(&'a [u32]),
}

impl MeshIndexData<'_> {
	fn to_u32_vec(&self) -> Vec<u32> {
		match self {
			MeshIndexData::U32(values) => values.to_vec(),
		}
	}
}

/// The `MeshPrimitiveSource` trait describes a primitive view that gives query-based access to mesh data.
pub trait MeshPrimitiveSource {
	fn material(&self) -> &ReferenceModel<VariantModel>;
	fn bounding_box(&self) -> [[f32; 3]; 2];
	fn vertex_count(&self) -> usize;
	fn attribute(&self, semantic: VertexSemantics, channel: u32) -> Option<MeshAttributeData<'_>>;
	fn indices(&self, stream_type: IndexStreamTypes) -> Option<MeshIndexData<'_>>;
}

/// The `MeshSource` trait describes a mesh input that the mesh processor can pack into resource streams.
pub trait MeshSource {
	type Primitive<'a>: MeshPrimitiveSource
	where
		Self: 'a;

	fn vertex_layout(&self) -> &[VertexComponent];
	fn primitive_count(&self) -> usize;
	fn primitive(&self, index: usize) -> Option<Self::Primitive<'_>>;

	fn primitives(&self) -> impl Iterator<Item = Self::Primitive<'_>> {
		(0..self.primitive_count()).filter_map(|index| self.primitive(index))
	}
}

/// The `OwnedMeshSource` struct stores normalized mesh data before the mesh processor packs it into resource streams.
#[derive(Debug, Default)]
pub struct OwnedMeshSource {
	vertex_layout: Vec<VertexComponent>,
	primitives: Vec<OwnedMeshPrimitive>,
}

impl OwnedMeshSource {
	pub fn new(vertex_layout: Vec<VertexComponent>, primitives: Vec<OwnedMeshPrimitive>) -> Self {
		Self {
			vertex_layout,
			primitives,
		}
	}

	pub fn vertex_layout_mut(&mut self) -> &mut Vec<VertexComponent> {
		&mut self.vertex_layout
	}

	pub fn primitives_mut(&mut self) -> &mut Vec<OwnedMeshPrimitive> {
		&mut self.primitives
	}
}

impl MeshSource for OwnedMeshSource {
	type Primitive<'a>
		= &'a OwnedMeshPrimitive
	where
		Self: 'a;

	fn vertex_layout(&self) -> &[VertexComponent] {
		&self.vertex_layout
	}

	fn primitive_count(&self) -> usize {
		self.primitives.len()
	}

	fn primitive(&self, index: usize) -> Option<Self::Primitive<'_>> {
		self.primitives.get(index)
	}
}

/// The `OwnedMeshPrimitive` struct stores a primitive in a processor-friendly owned representation.
#[derive(Debug)]
pub struct OwnedMeshPrimitive {
	material: ReferenceModel<VariantModel>,
	bounding_box: [[f32; 3]; 2],
	attributes: Vec<OwnedMeshAttribute>,
	triangle_indices: Vec<u32>,
}

impl OwnedMeshPrimitive {
	pub fn new(material: ReferenceModel<VariantModel>, bounding_box: [[f32; 3]; 2], triangle_indices: Vec<u32>) -> Self {
		Self {
			material,
			bounding_box,
			attributes: Vec::new(),
			triangle_indices,
		}
	}

	pub fn with_attribute(mut self, attribute: OwnedMeshAttribute) -> Self {
		self.attributes.push(attribute);
		self
	}

	pub fn add_attribute(&mut self, attribute: OwnedMeshAttribute) {
		self.attributes.push(attribute);
	}
}

impl MeshPrimitiveSource for &OwnedMeshPrimitive {
	fn material(&self) -> &ReferenceModel<VariantModel> {
		&self.material
	}

	fn bounding_box(&self) -> [[f32; 3]; 2] {
		self.bounding_box
	}

	fn vertex_count(&self) -> usize {
		self.attributes
			.iter()
			.find(|attribute| attribute.semantic == VertexSemantics::Position && attribute.channel == 0)
			.map(|attribute| attribute.data.len())
			.unwrap_or(0)
	}

	fn attribute(&self, semantic: VertexSemantics, channel: u32) -> Option<MeshAttributeData<'_>> {
		self.attributes
			.iter()
			.find(|attribute| attribute.semantic == semantic && attribute.channel == channel)
			.map(OwnedMeshAttribute::borrow)
	}

	fn indices(&self, stream_type: IndexStreamTypes) -> Option<MeshIndexData<'_>> {
		match stream_type {
			IndexStreamTypes::Triangles => Some(MeshIndexData::U32(&self.triangle_indices)),
			IndexStreamTypes::Vertices | IndexStreamTypes::Meshlets => None,
		}
	}
}

/// The `OwnedMeshAttribute` struct stores owned attribute data for a single semantic and channel.
#[derive(Debug)]
pub struct OwnedMeshAttribute {
	semantic: VertexSemantics,
	channel: u32,
	data: OwnedMeshAttributeData,
}

impl OwnedMeshAttribute {
	pub fn new(semantic: VertexSemantics, channel: u32, data: OwnedMeshAttributeData) -> Self {
		Self { semantic, channel, data }
	}

	fn borrow(&self) -> MeshAttributeData<'_> {
		self.data.borrow()
	}
}

/// The `OwnedMeshAttributeData` enum stores owned attribute payloads for processor-owned meshes.
#[derive(Debug)]
pub enum OwnedMeshAttributeData {
	F32x2(Vec<[f32; 2]>),
	F32x3(Vec<[f32; 3]>),
	F32x4(Vec<[f32; 4]>),
	U16x4(Vec<[u16; 4]>),
}

impl OwnedMeshAttributeData {
	fn len(&self) -> usize {
		match self {
			OwnedMeshAttributeData::F32x2(values) => values.len(),
			OwnedMeshAttributeData::F32x3(values) => values.len(),
			OwnedMeshAttributeData::F32x4(values) => values.len(),
			OwnedMeshAttributeData::U16x4(values) => values.len(),
		}
	}

	fn borrow(&self) -> MeshAttributeData<'_> {
		match self {
			OwnedMeshAttributeData::F32x2(values) => MeshAttributeData::F32x2(values),
			OwnedMeshAttributeData::F32x3(values) => MeshAttributeData::F32x3(values),
			OwnedMeshAttributeData::F32x4(values) => MeshAttributeData::F32x4(values),
			OwnedMeshAttributeData::U16x4(values) => MeshAttributeData::U16x4(values),
		}
	}
}

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

	pub fn process<T: MeshSource>(&self, source: &T) -> Result<ProcessedMesh, MeshProcessingError> {
		validate_vertex_layout(source.vertex_layout())?;

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
				vertex_components: active_vertex_layout,
				streams: mesh_streams,
				primitives,
			},
			stream_descriptions,
			buffer: buffer.into_boxed_slice(),
		})
	}

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

		if triangle_indices.len() % 3 != 0 {
			return Err(MeshProcessingError::InvalidTriangleIndexCount);
		}

		let optimized_triangle_indices = meshopt::optimize_vertex_cache(&triangle_indices, position_count);
		let meshlet_source_bytes = position_bytes
			.iter()
			.flat_map(|position| position.iter().flat_map(|component| component.to_le_bytes()))
			.collect::<Vec<u8>>();
		let meshlets = meshopt::clusterize::build_meshlets(
			&optimized_triangle_indices,
			&meshopt::VertexDataAdapter::new(&meshlet_source_bytes, 12, 0)
				.map_err(|_| MeshProcessingError::FailedToBuildMeshlets)?,
			64,
			124,
			0.0f32,
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
			.flat_map(|meshlet| [meshlet.vertices.len() as u8, (meshlet.triangles.len() / 3) as u8])
			.collect::<Vec<u8>>();
		append_stream(&mut primitive_streams, packed_blocks, Streams::Meshlets, meshlet_bytes);

		Ok(PrimitiveModel {
			material: duplicate_reference_model(primitive.material()),
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

#[derive(Debug, PartialEq, Eq)]
pub enum MeshProcessingError {
	MissingPositionAttribute,
	MissingTriangleIndices,
	MissingAttribute(VertexSemantics, u32),
	DuplicateVertexSemantic(VertexSemantics),
	InvalidAttributeFormat(VertexSemantics),
	AttributeLengthMismatch(VertexSemantics, u32),
	InconsistentVertexCount,
	InvalidTriangleIndexCount,
	FailedToBuildMeshlets,
}

impl std::fmt::Display for MeshProcessingError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			MeshProcessingError::MissingPositionAttribute => {
				write!(
					f,
					"Mesh is missing the position attribute. The most likely cause is that the mesh source did not expose `Position` on channel 0."
				)
			}
			MeshProcessingError::MissingTriangleIndices => {
				write!(
					f,
					"Mesh is missing triangle indices. The most likely cause is that the mesh source did not expose a triangle index stream."
				)
			}
			MeshProcessingError::MissingAttribute(semantic, channel) => {
				write!(
					f,
					"Mesh is missing a required vertex attribute. The most likely cause is that {:?} on channel {} is present in the vertex layout but not in the primitive data.",
					semantic, channel
				)
			}
			MeshProcessingError::DuplicateVertexSemantic(semantic) => {
				write!(
					f,
					"Mesh uses the same vertex semantic more than once. The most likely cause is that the current stream metadata cannot represent multiple channels for {:?}.",
					semantic
				)
			}
			MeshProcessingError::InvalidAttributeFormat(semantic) => {
				write!(
					f,
					"Mesh uses an unsupported vertex attribute format. The most likely cause is that {:?} was exposed with a shape that does not match the engine stream layout.",
					semantic
				)
			}
			MeshProcessingError::AttributeLengthMismatch(semantic, channel) => {
				write!(
					f,
					"Mesh attribute length does not match the position stream. The most likely cause is that {:?} on channel {} does not contain one value per vertex.",
					semantic, channel
				)
			}
			MeshProcessingError::InconsistentVertexCount => {
				write!(
					f,
					"Mesh primitive reported an inconsistent vertex count. The most likely cause is that the primitive metadata does not match its position attribute length."
				)
			}
			MeshProcessingError::InvalidTriangleIndexCount => {
				write!(
					f,
					"Triangle index count is invalid. The most likely cause is that the index stream length is not divisible by three."
				)
			}
			MeshProcessingError::FailedToBuildMeshlets => {
				write!(
					f,
					"Meshlet generation failed. The most likely cause is that the packed position stream could not be adapted for meshopt."
				)
			}
		}
	}
}

impl std::error::Error for MeshProcessingError {}

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

fn validate_vertex_layout(vertex_layout: &[VertexComponent]) -> Result<(), MeshProcessingError> {
	let mut seen = Vec::with_capacity(vertex_layout.len());

	for component in vertex_layout {
		if seen.contains(&component.semantic) {
			return Err(MeshProcessingError::DuplicateVertexSemantic(component.semantic));
		}

		seen.push(component.semantic);
	}

	Ok(())
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
		Streams::Vertices(VertexSemantics::Position) => 12,
		Streams::Vertices(VertexSemantics::Normal) => 12,
		Streams::Vertices(VertexSemantics::Tangent) => 16,
		Streams::Vertices(VertexSemantics::BiTangent) => 12,
		Streams::Vertices(VertexSemantics::UV) => 8,
		Streams::Vertices(VertexSemantics::Color) => 16,
		Streams::Vertices(VertexSemantics::Joints) => 8,
		Streams::Vertices(VertexSemantics::Weights) => 16,
		Streams::Indices(IndexStreamTypes::Vertices) => IntegralTypes::U16.size(),
		Streams::Indices(IndexStreamTypes::Triangles) => IntegralTypes::U16.size(),
		Streams::Indices(IndexStreamTypes::Meshlets) => IntegralTypes::U8.size(),
		Streams::Meshlets => 2,
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

fn duplicate_reference_model<T: crate::Model>(reference: &ReferenceModel<T>) -> ReferenceModel<T> {
	pot::from_slice(&pot::to_vec(reference).expect("Reference model should serialize"))
		.expect("Reference model should deserialize")
}

trait IntegralTypeSize {
	fn size(self) -> usize;
}

impl IntegralTypeSize for IntegralTypes {
	fn size(self) -> usize {
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

#[cfg(test)]
mod tests {
	use crate::{
		resources::material::VariantModel,
		types::{AlphaMode, VertexComponent},
		ReferenceModel,
	};

	use super::{
		MeshProcessingError, MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource,
		TriangleFrontFaceWinding,
	};
	use crate::types::VertexSemantics;

	#[test]
	fn rewinds_triangle_order_for_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = super::orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::Clockwise);

		assert_eq!(oriented, vec![0, 2, 1, 3, 5, 4]);
	}

	#[test]
	fn preserves_triangle_order_for_counter_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = super::orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::CounterClockwise);

		assert_eq!(oriented, vec![0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn packs_mesh_streams_from_a_query_based_source() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::Normal,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 0,
				},
			],
			vec![
				OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2])
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
					))
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Normal,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 1.0]; 3]),
					))
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::UV,
						0,
						OwnedMeshAttributeData::F32x2(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
					)),
			],
		);

		let processed = MeshProcessor::new().process(&source).expect("Mesh processing should succeed");

		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.streams.len(), 7);
		assert_eq!(
			processed.mesh.streams[0].stream_type,
			crate::types::Streams::Vertices(VertexSemantics::Position)
		);
		assert_eq!(
			processed.mesh.streams[4].stream_type,
			crate::types::Streams::Indices(crate::types::IndexStreamTypes::Triangles)
		);
		assert!(!processed.buffer.is_empty());
	}

	#[test]
	fn rejects_duplicate_vertex_semantics_in_the_layout() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 1,
				},
			],
			Vec::new(),
		);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Mesh processing should reject duplicate semantics");

		assert_eq!(error, MeshProcessingError::DuplicateVertexSemantic(VertexSemantics::UV));
	}

	#[test]
	fn skips_disabled_vertex_streams_from_the_layout() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::BiTangent,
					format: "vec3f".to_string(),
					channel: 0,
				},
			],
			vec![
				OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2]).with_attribute(
					OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
					),
				),
			],
		);

		let processed = MeshProcessor::new().process(&source).expect("Mesh processing should succeed");

		assert_eq!(processed.mesh.vertex_components.len(), 1);
		assert_eq!(processed.mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert!(processed
			.mesh
			.streams
			.iter()
			.all(|stream| stream.stream_type != crate::types::Streams::Vertices(VertexSemantics::BiTangent)));
	}

	fn test_material() -> ReferenceModel<VariantModel> {
		ReferenceModel::new_serialized(
			"materials/test.variant",
			0,
			0,
			pot::to_vec(&VariantModel {
				material: ReferenceModel::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
			})
			.expect("Variant model should serialize"),
			None,
		)
	}
}
