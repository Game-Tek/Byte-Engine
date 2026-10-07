/// The `MeshPrimitiveProcessingError` enum preserves source-format failures alongside common processor failures.
#[derive(Debug, PartialEq, Eq)]
pub enum MeshPrimitiveProcessingError<E> {
	Source(E),
	Processing(MeshProcessingError),
}

impl<E: std::fmt::Display> std::fmt::Display for MeshPrimitiveProcessingError<E> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Source(error) => error.fmt(formatter),
			Self::Processing(error) => error.fmt(formatter),
		}
	}
}

impl<E> From<MeshProcessingError> for MeshPrimitiveProcessingError<E> {
	fn from(error: MeshProcessingError) -> Self {
		Self::Processing(error)
	}
}

/// The `MeshProcessorSession` struct runs the common mesh-processing pipeline after format-specific import.
///
/// It keeps reusable scratch and final stream writers alive across borrowed primitives. Start one with [`Self::new`].
pub struct MeshProcessorSession {
	vertex_layout: Vec<VertexComponent>,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	skeleton_nodes: Option<usize>,
	skins: Vec<SkinBinding>,
	blocks: Vec<PackedStreamBlock>,
	/// Processed primitives. Until the mesh finishes, each one's material holds its material slot.
	primitives: Vec<Primitive>,
	scratch: MeshProcessingScratch,
}

impl MeshProcessorSession {
	/// Starts a short-lived processing session that borrows one source primitive at a time.
	///
	/// Call [`Self::push_primitive`] for each imported primitive, then call [`Self::finish_into`] to write the payload
	/// directly, or [`Self::finish`] when the caller needs an owned payload. Each primitive names its material by its
	/// slot in the list the finish call receives.
	pub fn new(
		vertex_layout: Vec<VertexComponent>,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
	) -> Result<Self, MeshProcessingError> {
		validate_vertex_layout(&vertex_layout)?;
		let skeleton_nodes = skeleton_node_count(skeleton.as_ref())?;
		for (skin_index, skin) in skins.iter().enumerate() {
			validate_skin_binding(skin_index, skin, skeleton_nodes)?;
		}

		// Vertex streams follow the declaration order of `VertexSemantics`, then the generated streams.
		let mut semantics = vertex_layout.iter().map(|component| component.semantic).collect::<Vec<_>>();
		semantics.sort_by_key(|semantic| *semantic as usize);
		let stream_order = semantics.into_iter().map(Streams::Vertices).chain([
			Streams::Indices(IndexStreamTypes::Vertices),
			Streams::Indices(IndexStreamTypes::Triangles),
			Streams::Indices(IndexStreamTypes::Meshlets),
			Streams::Meshlets,
		]);

		let blocks = stream_order
			.map(|stream_type| PackedStreamBlock {
				stream_type,
				stride: stream_stride(&vertex_layout, stream_type),
				bytes: Vec::new(),
			})
			.collect();
		Ok(Self {
			vertex_layout,
			skeleton,
			skeleton_nodes,
			skins,
			blocks,
			primitives: Vec::new(),
			scratch: MeshProcessingScratch::default(),
		})
	}

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
				.push(position.map_err(MeshPrimitiveProcessingError::Source)?.to_array());
		}
		let bounds = bounding_box_from_positions(&self.scratch.positions).ok_or(MeshProcessingError::InvalidPositionData)?;

		let indices = primitive.indices().map_err(MeshPrimitiveProcessingError::Source)?;
		self.scratch.indices.clear();
		self.scratch.indices.reserve(indices.len());
		for index in indices {
			self.scratch
				.indices
				.push(index.map_err(MeshPrimitiveProcessingError::Source)?);
		}
		if !self.scratch.indices.len().is_multiple_of(3) {
			return Err(MeshProcessingError::InvalidTriangleIndexCount.into());
		}
		rewind_triangles_to_clockwise(&mut self.scratch.indices);
		meshopt::optimize_vertex_cache_in_place(&mut self.scratch.indices, position_count);
		let mut primitive_streams = self.append_primitive_vertex_streams(primitive, primitive_index, position_count)?;

		// meshopt reads native-endian positions, which is what casting the position scratch gives it.
		let meshlet_vertex_adapter = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(&self.scratch.positions), 12, 0)
			.map_err(|_| MeshProcessingError::FailedToBuildMeshlets)?;
		let meshlets = meshopt::clusterize::build_meshlets(
			&self.scratch.indices,
			&meshlet_vertex_adapter,
			MESHLET_MAX_VERTICES,
			MESHLET_MAX_TRIANGLES,
			MESHLET_CONE_WEIGHT,
		);

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

		self.primitives.push(Primitive {
			material: primitive.material_slot() as u32,
			transform_node: primitive.transform_node(),
			skin: primitive.skin(),
			streams: primitive_streams,
			bounding_box: bounds,
			vertex_count: position_count as u32,
		});
		Ok(())
	}

	/// Validates and appends the authored vertex streams for one primitive.
	fn append_primitive_vertex_streams<P: MeshPrimitiveSource>(
		&mut self,
		primitive: &P,
		primitive_index: usize,
		position_count: usize,
	) -> Result<Vec<Stream>, MeshPrimitiveProcessingError<P::Error>> {
		let vertex_skin = primitive.vertex_skin().map_err(MeshPrimitiveProcessingError::Source)?;
		validate_primitive_metadata(
			primitive_index,
			primitive.transform_node(),
			primitive.skin(),
			vertex_skin.is_some(),
			&self.vertex_layout,
			self.skeleton_nodes,
			&self.skins,
		)?;

		let mut streams = Vec::with_capacity(self.vertex_layout.len() + 4);
		streams.push(append_generated_stream(
			&mut self.blocks,
			Streams::Vertices(VertexSemantics::Position),
			|bytes| {
				for position in &self.scratch.positions {
					write_f32_components(bytes, position);
				}
			},
		));
		append_optional_f32(
			&mut streams,
			&mut self.blocks,
			VertexSemantics::Normal,
			position_count,
			primitive
				.normals()
				.map_err(MeshPrimitiveProcessingError::Source)?
				.map(|normals| normals.map(|normal| normal.map(Vector::to_array))),
		)?;
		append_optional_f32(
			&mut streams,
			&mut self.blocks,
			VertexSemantics::Tangent,
			position_count,
			primitive.tangents().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut streams,
			&mut self.blocks,
			VertexSemantics::BiTangent,
			position_count,
			primitive
				.bitangents()
				.map_err(MeshPrimitiveProcessingError::Source)?
				.map(|bitangents| bitangents.map(|bitangent| bitangent.map(Vector::to_array))),
		)?;
		append_optional_f32(
			&mut streams,
			&mut self.blocks,
			VertexSemantics::UV,
			position_count,
			primitive.uvs().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		append_optional_f32(
			&mut streams,
			&mut self.blocks,
			VertexSemantics::Color,
			position_count,
			primitive.colors().map_err(MeshPrimitiveProcessingError::Source)?,
		)?;
		if let Some(vertex_skin) = vertex_skin {
			append_vertex_skin(
				&mut streams,
				&mut self.blocks,
				primitive_index,
				position_count,
				vertex_skin,
				&self.skins[primitive.skin().expect("validated skinned primitive") as usize],
			)?;
		}

		Ok(streams)
	}

	/// Returns the exact number of bytes that [`Self::finish_into`] will write.
	pub fn payload_size(&self) -> usize {
		self.blocks.iter().map(|block| block.bytes.len()).sum()
	}

	/// Finishes stream offsets and writes each completed stream directly to `writer`.
	///
	/// `materials` holds the material of each slot primitives referenced. `W` remains generic so stream writes do not
	/// use dynamic dispatch. Reserve [`Self::payload_size`] bytes before calling this method.
	pub fn finish_into<W: std::io::Write>(
		self,
		materials: &[ReferenceModel<VariantModel>],
		writer: &mut W,
	) -> std::io::Result<(MeshModel, Vec<StreamDescription>)> {
		let (mesh, stream_descriptions, blocks) = self.finish_parts(materials);
		for block in blocks {
			std::io::Write::write_all(writer, &block.bytes)?;
		}
		Ok((mesh, stream_descriptions))
	}

	/// Finishes stream offsets and asynchronously writes each completed stream into resource storage.
	///
	/// `materials` holds the material of each slot primitives referenced. Reserve [`Self::payload_size`] bytes before
	/// calling this method. Use [`Self::finish_into`] for a synchronous non-resource sink.
	pub async fn finish_into_resource(
		self,
		materials: &[ReferenceModel<VariantModel>],
		writer: &mut crate::resource::ResourceTransaction<'_>,
	) -> std::io::Result<(MeshModel, Vec<StreamDescription>)> {
		let (mesh, stream_descriptions, blocks) = self.finish_parts(materials);
		for block in blocks {
			let compio::buf::BufResult(result, _) = compio::io::AsyncWriteExt::write_all(&mut *writer, block.bytes).await;
			result?;
		}
		Ok((mesh, stream_descriptions))
	}

	/// Finishes aggregate stream offsets and moves final metadata into the stored mesh resource.
	///
	/// `materials` holds the material of each slot primitives referenced. Use [`Self::finish_into`] when the payload
	/// can go directly to resource storage.
	pub fn finish(self, materials: &[ReferenceModel<VariantModel>]) -> ProcessedMesh {
		let mut buffer = Vec::with_capacity(self.payload_size());
		let (mesh, stream_descriptions) = self
			.finish_into(materials, &mut buffer)
			.expect("Writing to a Vec never fails");

		ProcessedMesh {
			mesh,
			stream_descriptions,
			buffer: buffer.into_boxed_slice(),
		}
	}

	/// Builds final stream metadata once before a caller moves each completed block to its selected sink.
	fn finish_parts(
		mut self,
		slot_materials: &[ReferenceModel<VariantModel>],
	) -> (MeshModel, Vec<StreamDescription>, Vec<PackedStreamBlock>) {
		// The stored mesh lists each distinct variant once, in first-use order, and primitives index that list.
		// Meshes use few distinct materials, so a linear search beats hashing each variant ID.
		let mut materials: Vec<ReferenceModel<VariantModel>> = Vec::with_capacity(slot_materials.len());
		for primitive in &mut self.primitives {
			let slot = primitive.material as usize;
			let material = slot_materials.get(slot).unwrap_or_else(|| {
				panic!(
					"Mesh material slot {slot} is out of range for {} materials. The most likely cause is an importer that numbered its primitives' materials differently from the list it passed to finish the mesh.",
					slot_materials.len()
				)
			});
			primitive.material = match materials
				.iter()
				.position(|existing| existing.id().as_ref() == material.id().as_ref())
			{
				Some(index) => index as u32,
				None => {
					materials.push(material.clone());
					(materials.len() - 1) as u32
				}
			};
		}

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
				stride: block.stride,
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
				materials,
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
	block_lengths: Vec<usize>,
}

struct PackedStreamBlock {
	stream_type: Streams,
	stride: usize,
	bytes: Vec<u8>,
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
		return Err(MeshProcessingError::AttributeLengthMismatch(semantic, 0).into());
	}
	let stream_type = Streams::Vertices(semantic);
	let block = blocks
		.iter_mut()
		.find(|block| block.stream_type == stream_type)
		.ok_or(MeshProcessingError::MissingAttribute(semantic, 0))?;
	let offset = block.bytes.len();
	for value in values {
		write_f32_components(&mut block.bytes, &value.map_err(MeshPrimitiveProcessingError::Source)?);
	}
	primitive_streams.push(Stream {
		offset,
		size: block.bytes.len() - offset,
		stream_type,
		stride: block.stride,
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
		return Err(MeshProcessingError::SkinVertexCountMismatch {
			primitive,
			values: values.len(),
			positions: position_count,
		}
		.into());
	}
	let block_index = |semantic| {
		(blocks
			.iter()
			.position(|block| block.stream_type == Streams::Vertices(semantic)))
		.ok_or(MeshProcessingError::MissingSkinVertexComponent(semantic))
	};
	let joints_index = block_index(VertexSemantics::Joints)?;
	let weights_index = block_index(VertexSemantics::Weights)?;
	let joints_offset = blocks[joints_index].bytes.len();
	let weights_offset = blocks[weights_index].bytes.len();
	for (vertex, value) in values.enumerate() {
		let value = value.map_err(MeshPrimitiveProcessingError::Source)?;
		validate_vertex_skin(primitive, vertex, value, skin)?;
		blocks[joints_index]
			.bytes
			.extend_from_slice(value.joints.map(u16::to_le_bytes).as_flattened());
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
			stride: blocks[index].stride,
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
		stride: block.stride,
	}
}

fn write_f32_components<const N: usize>(bytes: &mut Vec<u8>, value: &[f32; N]) {
	bytes.extend_from_slice(value.map(f32::to_le_bytes).as_flattened());
}

fn bounding_box_from_positions(positions: &[[f32; 3]]) -> Option<AABB<ModelSpace>> {
	let first = *positions.first()?;
	let (mut minimum, mut maximum) = (first, first);
	for position in positions {
		if position.iter().any(|component| !component.is_finite()) {
			return None;
		}
		for axis in 0..3 {
			minimum[axis] = minimum[axis].min(position[axis]);
			maximum[axis] = maximum[axis].max(position[axis]);
		}
	}
	Some(AABB::new(Point::from_array(minimum), Point::from_array(maximum)))
}

/// Rewinds counter-clockwise source triangles so processed meshes always use clockwise front faces.
pub(super) fn rewind_triangles_to_clockwise(indices: &mut [u32]) {
	debug_assert!(
		indices.len().is_multiple_of(3),
		"Triangle index streams must be emitted in groups of three"
	);
	for triangle in indices.as_chunks_mut::<3>().0 {
		triangle.swap(1, 2);
	}
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

/// Returns the element stride of `stream_type`. Vertex streams take it from their declared format.
fn stream_stride(vertex_layout: &[VertexComponent], stream_type: Streams) -> usize {
	match stream_type {
		Streams::Vertices(semantic) => vertex_layout
			.iter()
			.find(|component| component.semantic == semantic)
			.and_then(VertexComponent::size)
			.expect("validated vertex layouts declare a sized format for every stream"),
		Streams::Indices(IndexStreamTypes::Vertices | IndexStreamTypes::Triangles) => size_of::<u16>(),
		Streams::Indices(IndexStreamTypes::Meshlets) => size_of::<u8>(),
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

const MESHLET_MAX_VERTICES: usize = 64;
const MESHLET_MAX_TRIANGLES: usize = 124;
const MESHLET_CONE_WEIGHT: f32 = 0.25;
pub(super) const MESHLET_STREAM_STRIDE: usize = 52;

use math::{AABB, Point, Vector};

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
		ModelSpace,
		material::VariantModel,
		mesh::{MeshModel, Primitive},
		skeleton::{SkeletonModel, SkinBinding},
	},
	types::{IndexStreamTypes, Stream, Streams, VertexComponent, VertexSemantics},
};
