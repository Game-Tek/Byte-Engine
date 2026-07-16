use std::alloc::{Allocator, Global};

use crate::{
	resources::{
		material::VariantModel,
		mesh::{MeshModel, PrimitiveModel},
		skeleton::{SkeletonModel, SkinBinding, SkinJoint},
	},
	types::{IndexStreamTypes, IntegralTypes, Size, Stream, Streams, VertexComponent, VertexSemantics},
	ReferenceModel, StreamDescription,
};

const MESHLET_MAX_VERTICES: usize = 64;
const MESHLET_MAX_TRIANGLES: usize = 124;
const MESHLET_CONE_WEIGHT: f32 = 0.25;
const MESHLET_STREAM_STRIDE: usize = 52;
// Four normalized f32 influences can accumulate a few rounding ULPs across importer conversions.
const SKIN_WEIGHT_SUM_TOLERANCE: f32 = 1.0e-4;

/// The `TriangleFrontFaceWinding` enum describes which triangle winding should be treated as the mesh front face after processing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum TriangleFrontFaceWinding {
	#[default]
	Clockwise,
	CounterClockwise,
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
	fn transform_node(&self) -> Option<u32> {
		None
	}
	fn skin(&self) -> Option<u32> {
		None
	}
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
	fn skeleton(&self) -> Option<&ReferenceModel<SkeletonModel>> {
		None
	}
	fn skins(&self) -> &[SkinBinding] {
		&[]
	}
	fn primitive_count(&self) -> usize;
	fn primitive(&self, index: usize) -> Option<Self::Primitive<'_>>;

	fn primitives(&self) -> impl Iterator<Item = Self::Primitive<'_>> {
		(0..self.primitive_count()).filter_map(|index| self.primitive(index))
	}
}

/// The `OwnedMeshSource` struct stores normalized mesh data before the mesh processor packs it into resource streams.
#[derive(Debug)]
pub struct OwnedMeshSource<A: Allocator = Global> {
	vertex_layout: Vec<VertexComponent, A>,
	primitives: Vec<OwnedMeshPrimitive<A>, A>,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	skins: Vec<SkinBinding>,
}

impl<A: Allocator> OwnedMeshSource<A> {
	pub fn new(vertex_layout: Vec<VertexComponent, A>, primitives: Vec<OwnedMeshPrimitive<A>, A>) -> Self {
		Self {
			vertex_layout,
			primitives,
			skeleton: None,
			skins: Vec::new(),
		}
	}

	pub fn with_skeleton(mut self, skeleton: ReferenceModel<SkeletonModel>) -> Self {
		self.set_skeleton(Some(skeleton));
		self
	}

	pub fn set_skeleton(&mut self, skeleton: Option<ReferenceModel<SkeletonModel>>) {
		self.skeleton = skeleton;
	}

	pub fn with_skins(mut self, skins: Vec<SkinBinding>) -> Self {
		self.set_skins(skins);
		self
	}

	pub fn set_skins(&mut self, skins: Vec<SkinBinding>) {
		self.skins = skins;
	}

	pub fn vertex_layout_mut(&mut self) -> &mut Vec<VertexComponent, A> {
		&mut self.vertex_layout
	}

	pub fn primitives_mut(&mut self) -> &mut Vec<OwnedMeshPrimitive<A>, A> {
		&mut self.primitives
	}
}

impl Default for OwnedMeshSource {
	fn default() -> Self {
		Self::new(Vec::new(), Vec::new())
	}
}

impl<A: Allocator> MeshSource for OwnedMeshSource<A> {
	type Primitive<'a>
		= &'a OwnedMeshPrimitive<A>
	where
		Self: 'a;

	fn vertex_layout(&self) -> &[VertexComponent] {
		&self.vertex_layout
	}

	fn skeleton(&self) -> Option<&ReferenceModel<SkeletonModel>> {
		self.skeleton.as_ref()
	}

	fn skins(&self) -> &[SkinBinding] {
		&self.skins
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
pub struct OwnedMeshPrimitive<A: Allocator = Global> {
	material: ReferenceModel<VariantModel>,
	transform_node: Option<u32>,
	skin: Option<u32>,
	bounding_box: [[f32; 3]; 2],
	attributes: Vec<OwnedMeshAttribute<A>, A>,
	triangle_indices: Vec<u32, A>,
}

impl OwnedMeshPrimitive {
	pub fn new(material: ReferenceModel<VariantModel>, bounding_box: [[f32; 3]; 2], triangle_indices: Vec<u32>) -> Self {
		Self::new_in(material, bounding_box, triangle_indices, Global)
	}
}

impl<A: Allocator + Clone> OwnedMeshPrimitive<A> {
	/// Creates processor staging storage with allocator-backed index and attribute buffers.
	pub fn new_in(
		material: ReferenceModel<VariantModel>,
		bounding_box: [[f32; 3]; 2],
		triangle_indices: Vec<u32, A>,
		allocator: A,
	) -> Self {
		Self {
			material,
			transform_node: None,
			skin: None,
			bounding_box,
			attributes: Vec::with_capacity_in(8, allocator),
			triangle_indices,
		}
	}
}

impl<A: Allocator> OwnedMeshPrimitive<A> {
	pub fn with_transform_node(mut self, transform_node: u32) -> Self {
		self.set_transform_node(Some(transform_node));
		self
	}

	pub fn set_transform_node(&mut self, transform_node: Option<u32>) {
		self.transform_node = transform_node;
	}

	pub fn transform_node(&self) -> Option<u32> {
		self.transform_node
	}

	pub fn with_skin(mut self, skin: u32) -> Self {
		self.set_skin(Some(skin));
		self
	}

	pub fn set_skin(&mut self, skin: Option<u32>) {
		self.skin = skin;
	}

	pub fn skin(&self) -> Option<u32> {
		self.skin
	}

	pub fn with_attribute(mut self, attribute: OwnedMeshAttribute<A>) -> Self {
		self.attributes.push(attribute);
		self
	}

	pub fn add_attribute(&mut self, attribute: OwnedMeshAttribute<A>) {
		self.attributes.push(attribute);
	}
}

impl<A: Allocator> MeshPrimitiveSource for &OwnedMeshPrimitive<A> {
	fn material(&self) -> &ReferenceModel<VariantModel> {
		&self.material
	}

	fn transform_node(&self) -> Option<u32> {
		self.transform_node
	}

	fn skin(&self) -> Option<u32> {
		self.skin
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
pub struct OwnedMeshAttribute<A: Allocator = Global> {
	semantic: VertexSemantics,
	channel: u32,
	data: OwnedMeshAttributeData<A>,
}

impl<A: Allocator> OwnedMeshAttribute<A> {
	pub fn new(semantic: VertexSemantics, channel: u32, data: OwnedMeshAttributeData<A>) -> Self {
		Self { semantic, channel, data }
	}

	fn borrow(&self) -> MeshAttributeData<'_> {
		self.data.borrow()
	}
}

/// The `OwnedMeshAttributeData` enum stores owned attribute payloads for processor-owned meshes.
#[derive(Debug)]
pub enum OwnedMeshAttributeData<A: Allocator = Global> {
	F32x2(Vec<[f32; 2], A>),
	F32x3(Vec<[f32; 3], A>),
	F32x4(Vec<[f32; 4], A>),
	U16x4(Vec<[u16; 4], A>),
}

impl<A: Allocator> OwnedMeshAttributeData<A> {
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
	InvalidSkeletonModel,
	SkinWithoutSkeleton,
	TransformNodeWithoutSkeleton {
		primitive: usize,
		node: u32,
	},
	TransformNodeOutOfRange {
		primitive: usize,
		node: u32,
		nodes: usize,
	},
	SkinPaletteTooLarge {
		skin: usize,
		joints: usize,
	},
	SkinJointOutOfRange {
		skin: usize,
		joint: usize,
		node: u32,
		nodes: usize,
	},
	NonFiniteInverseBind {
		skin: usize,
	},
	SkinIndexOutOfRange {
		primitive: usize,
		skin: u32,
		skins: usize,
	},
	IncompleteSkinAttributes {
		primitive: usize,
	},
	UnboundSkinAttributes {
		primitive: usize,
	},
	MissingSkinVertexComponent(VertexSemantics),
	InvalidSkinVertexComponentFormat {
		semantic: VertexSemantics,
		expected: &'static str,
		actual: String,
	},
	SkinAttributeLengthMismatch {
		primitive: usize,
		joints: usize,
		weights: usize,
	},
	VertexJointOutOfRange {
		primitive: usize,
		vertex: usize,
		lane: usize,
		joint: u16,
		palette_len: usize,
	},
	NonFiniteSkinWeight {
		primitive: usize,
		vertex: usize,
		lane: usize,
	},
	NegativeSkinWeight {
		primitive: usize,
		vertex: usize,
		lane: usize,
	},
	NonPositiveSkinWeightTotal {
		primitive: usize,
		vertex: usize,
	},
	NonNormalizedSkinWeights {
		primitive: usize,
		vertex: usize,
	},
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
			MeshProcessingError::InvalidSkeletonModel => write!(
				f,
				"Skeleton metadata is invalid. The most likely cause is that the mesh source contains an incompatible serialized skeleton model."
			),
			MeshProcessingError::SkinWithoutSkeleton => write!(
				f,
				"Mesh skin bindings have no skeleton. The most likely cause is that the importer omitted the skeleton reference while retaining skin palettes."
			),
			MeshProcessingError::TransformNodeWithoutSkeleton { primitive, node } => write!(
				f,
				"Primitive transform node has no skeleton. The most likely cause is that primitive {primitive} targets node {node} without retaining its hierarchy."
			),
			MeshProcessingError::TransformNodeOutOfRange {
				primitive,
				node,
				nodes,
			} => write!(
				f,
				"Primitive transform node is outside the skeleton. The most likely cause is that primitive {primitive} targets node {node} in a {nodes}-node hierarchy."
			),
			MeshProcessingError::SkinPaletteTooLarge { skin, joints } => write!(
				f,
				"Skin palette is too large. The most likely cause is that skin {skin} contains {joints} entries, which cannot be addressed by u16 vertex joints."
			),
			MeshProcessingError::SkinJointOutOfRange {
				skin,
				joint,
				node,
				nodes,
			} => write!(
				f,
				"Skin joint is outside the skeleton. The most likely cause is that skin {skin} palette entry {joint} targets node {node} in a {nodes}-node skeleton."
			),
			MeshProcessingError::NonFiniteInverseBind { skin } => write!(
				f,
				"Skin inverse bind is not finite. The most likely cause is malformed transform data in skin {skin}."
			),
			MeshProcessingError::SkinIndexOutOfRange {
				primitive,
				skin,
				skins,
			} => write!(
				f,
				"Primitive skin index is invalid. The most likely cause is that primitive {primitive} targets skin {skin} in a mesh with {skins} skins."
			),
			MeshProcessingError::IncompleteSkinAttributes { primitive } => write!(
				f,
				"Skinned primitive attributes are incomplete. The most likely cause is that primitive {primitive} does not provide both joint and weight values."
			),
			MeshProcessingError::UnboundSkinAttributes { primitive } => write!(
				f,
				"Primitive skin attributes have no binding. The most likely cause is that primitive {primitive} provides joint or weight values without selecting a skin."
			),
			MeshProcessingError::MissingSkinVertexComponent(semantic) => write!(
				f,
				"Skin vertex layout is incomplete. The most likely cause is that {semantic:?} channel 0 was omitted from the declared mesh layout."
			),
			MeshProcessingError::InvalidSkinVertexComponentFormat {
				semantic,
				expected,
				actual,
			} => write!(
				f,
				"Skin vertex layout has an invalid format. The most likely cause is that {semantic:?} was declared as '{actual}' instead of '{expected}'."
			),
			MeshProcessingError::SkinAttributeLengthMismatch {
				primitive,
				joints,
				weights,
			} => write!(
				f,
				"Skin attribute lengths do not match. The most likely cause is that primitive {primitive} contains {joints} joint values but {weights} weight values."
			),
			MeshProcessingError::VertexJointOutOfRange {
				primitive,
				vertex,
				lane,
				joint,
				palette_len,
			} => write!(
				f,
				"Vertex joint index is outside the skin palette. The most likely cause is that primitive {primitive} vertex {vertex} lane {lane} targets joint {joint} in a {palette_len}-entry palette."
			),
			MeshProcessingError::NonFiniteSkinWeight {
				primitive,
				vertex,
				lane,
			} => write!(
				f,
				"Vertex skin weight is not finite. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			MeshProcessingError::NegativeSkinWeight {
				primitive,
				vertex,
				lane,
			} => write!(
				f,
				"Vertex skin weight is negative. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			MeshProcessingError::NonPositiveSkinWeightTotal { primitive, vertex } => write!(
				f,
				"Vertex skin weight total is not positive. The most likely cause is that primitive {primitive} vertex {vertex} has no usable joint influence."
			),
			MeshProcessingError::NonNormalizedSkinWeights { primitive, vertex } => write!(
				f,
				"Vertex skin weights are not normalized. The most likely cause is that primitive {primitive} vertex {vertex} was not normalized after influence reduction."
			),
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

/// Validates skin references before packing so invalid vertex palettes never enter stored mesh resources.
fn validate_skin_source<T: MeshSource>(source: &T) -> Result<(), MeshProcessingError> {
	let skeleton_nodes = source
		.skeleton()
		.map(|skeleton| {
			crate::archived_from_slice::<SkeletonModel>(&skeleton.resource)
				.map_err(|_| MeshProcessingError::InvalidSkeletonModel)
				.and_then(|skeleton| {
					crate::resources::skeleton::validate_archived_nodes(skeleton.nodes.as_slice())
						.map_err(|_| MeshProcessingError::InvalidSkeletonModel)?;
					Ok(skeleton.nodes.len())
				})
		})
		.transpose()?;

	if !source.skins().is_empty() && skeleton_nodes.is_none() {
		return Err(MeshProcessingError::SkinWithoutSkeleton);
	}

	for (skin_index, skin) in source.skins().iter().enumerate() {
		if skin.len() > u16::MAX as usize + 1 {
			return Err(MeshProcessingError::SkinPaletteTooLarge {
				skin: skin_index,
				joints: skin.len(),
			});
		}

		let node_count = skeleton_nodes.unwrap_or(0);
		for (joint_index, entry) in skin.entries.iter().enumerate() {
			if let SkinJoint::Node(node) = entry.joint {
				if node as usize >= node_count {
					return Err(MeshProcessingError::SkinJointOutOfRange {
						skin: skin_index,
						joint: joint_index,
						node,
						nodes: node_count,
					});
				}
			}
			if !entry
				.adjusted_inverse_bind_matrix
				.iter()
				.flatten()
				.all(|value| value.is_finite())
			{
				return Err(MeshProcessingError::NonFiniteInverseBind { skin: skin_index });
			}
		}
	}

	let mut validated_skin_layout = false;
	for (primitive_index, primitive) in source.primitives().enumerate() {
		if let Some(node) = primitive.transform_node() {
			let Some(node_count) = skeleton_nodes else {
				return Err(MeshProcessingError::TransformNodeWithoutSkeleton {
					primitive: primitive_index,
					node,
				});
			};
			if node as usize >= node_count {
				return Err(MeshProcessingError::TransformNodeOutOfRange {
					primitive: primitive_index,
					node,
					nodes: node_count,
				});
			}
		}
		let joints = primitive.attribute(VertexSemantics::Joints, 0);
		let weights = primitive.attribute(VertexSemantics::Weights, 0);

		match primitive.skin() {
			Some(skin) => {
				if skin as usize >= source.skins().len() {
					return Err(MeshProcessingError::SkinIndexOutOfRange {
						primitive: primitive_index,
						skin,
						skins: source.skins().len(),
					});
				}
				if !validated_skin_layout {
					validate_skin_vertex_layout(source.vertex_layout())?;
					validated_skin_layout = true;
				}
				let (Some(joints), Some(weights)) = (joints, weights) else {
					return Err(MeshProcessingError::IncompleteSkinAttributes {
						primitive: primitive_index,
					});
				};
				validate_skin_vertex_data(primitive_index, joints, weights, &source.skins()[skin as usize])?;
			}
			None if joints.is_some() || weights.is_some() => {
				return Err(MeshProcessingError::UnboundSkinAttributes {
					primitive: primitive_index,
				});
			}
			None => {}
		}
	}

	Ok(())
}

/// Validates the declared shader types required to pack fixed-width skin attributes.
fn validate_skin_vertex_layout(vertex_layout: &[VertexComponent]) -> Result<(), MeshProcessingError> {
	for (semantic, expected) in [(VertexSemantics::Joints, "vec4u16"), (VertexSemantics::Weights, "vec4f")] {
		let Some(component) = vertex_layout
			.iter()
			.find(|component| component.semantic == semantic && component.channel == 0)
		else {
			return Err(MeshProcessingError::MissingSkinVertexComponent(semantic));
		};
		if component.format != expected {
			return Err(MeshProcessingError::InvalidSkinVertexComponentFormat {
				semantic,
				expected,
				actual: component.format.clone(),
			});
		}
	}
	Ok(())
}

/// Validates palette-local joint indices and normalized weights before they are copied into GPU-facing streams.
fn validate_skin_vertex_data(
	primitive: usize,
	joints: MeshAttributeData<'_>,
	weights: MeshAttributeData<'_>,
	skin: &SkinBinding,
) -> Result<(), MeshProcessingError> {
	let MeshAttributeData::U16x4(joints) = joints else {
		return Err(MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Joints));
	};
	let MeshAttributeData::F32x4(weights) = weights else {
		return Err(MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Weights));
	};
	if joints.len() != weights.len() {
		return Err(MeshProcessingError::SkinAttributeLengthMismatch {
			primitive,
			joints: joints.len(),
			weights: weights.len(),
		});
	}

	for (vertex, (vertex_joints, vertex_weights)) in joints.iter().zip(weights).enumerate() {
		let mut total = 0.0;
		for lane in 0..4 {
			let joint = vertex_joints[lane];
			if joint as usize >= skin.len() {
				return Err(MeshProcessingError::VertexJointOutOfRange {
					primitive,
					vertex,
					lane,
					joint,
					palette_len: skin.len(),
				});
			}

			let weight = vertex_weights[lane];
			if !weight.is_finite() {
				return Err(MeshProcessingError::NonFiniteSkinWeight { primitive, vertex, lane });
			}
			if weight < 0.0 {
				return Err(MeshProcessingError::NegativeSkinWeight { primitive, vertex, lane });
			}
			total += weight;
		}

		if total <= 0.0 {
			return Err(MeshProcessingError::NonPositiveSkinWeightTotal { primitive, vertex });
		}
		if (total - 1.0).abs() > SKIN_WEIGHT_SUM_TOLERANCE {
			return Err(MeshProcessingError::NonNormalizedSkinWeights { primitive, vertex });
		}
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

#[cfg(test)]
mod tests {
	use super::{
		MeshProcessingError, MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource,
		TriangleFrontFaceWinding,
	};
	use crate::types::VertexSemantics;
	use crate::{
		resources::{
			material::VariantModel,
			skeleton::{
				identity_matrix4_columns, LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry,
			},
		},
		types::{AlphaMode, VertexComponent},
		ReferenceModel,
	};

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
		let meshlet_stream = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == crate::types::Streams::Meshlets)
			.expect("Processed mesh should include a packed meshlet stream");
		assert_eq!(meshlet_stream.stride, super::MESHLET_STREAM_STRIDE);
		assert_eq!(meshlet_stream.size, super::MESHLET_STREAM_STRIDE);
		assert!(!processed.buffer.is_empty());
	}

	#[test]
	fn preserves_processable_skin_metadata_and_joint_streams() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true).with_transform_node(0)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))]);

		let processed = MeshProcessor::new()
			.process_owned(source)
			.expect("Skinned mesh processing should succeed");

		assert!(processed.mesh.skeleton.is_some());
		assert_eq!(processed.mesh.skins.len(), 1);
		assert_eq!(processed.mesh.primitives[0].transform_node, Some(0));
		assert_eq!(processed.mesh.primitives[0].skin, Some(0));
		assert!(processed.mesh.primitives[0]
			.streams
			.iter()
			.any(|stream| stream.stream_type == crate::types::Streams::Vertices(VertexSemantics::Joints)));
		assert!(processed.mesh.primitives[0]
			.streams
			.iter()
			.any(|stream| stream.stream_type == crate::types::Streams::Vertices(VertexSemantics::Weights)));
	}

	#[test]
	fn rejects_skin_nodes_outside_the_source_skeleton() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(1))]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Out-of-range palette nodes should be rejected before packing");

		assert_eq!(
			error,
			MeshProcessingError::SkinJointOutOfRange {
				skin: 0,
				joint: 0,
				node: 1,
				nodes: 1,
			}
		);
	}

	#[test]
	fn rejects_skinned_primitives_without_paired_joint_and_weight_attributes() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(false)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Skinned primitives should require both joint and weight attributes");

		assert_eq!(error, MeshProcessingError::IncompleteSkinAttributes { primitive: 0 });
	}

	#[test]
	fn rejects_skin_bindings_without_a_skeleton() {
		let source = OwnedMeshSource::new(Vec::new(), Vec::new()).with_skins(vec![test_skin(SkinJoint::Identity)]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Skin bindings should require a skeleton reference");

		assert_eq!(error, MeshProcessingError::SkinWithoutSkeleton);
	}

	#[test]
	fn rejects_skinned_primitives_when_required_layout_components_are_missing_or_mistyped() {
		for semantic in [VertexSemantics::Joints, VertexSemantics::Weights] {
			let mut source = valid_skinned_source();
			source.vertex_layout_mut().retain(|component| component.semantic != semantic);
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("A missing skin layout component should be rejected");
			assert_eq!(error, MeshProcessingError::MissingSkinVertexComponent(semantic));
		}

		for (semantic, expected) in [(VertexSemantics::Joints, "vec4u16"), (VertexSemantics::Weights, "vec4f")] {
			let mut source = valid_skinned_source();
			source
				.vertex_layout_mut()
				.iter_mut()
				.find(|component| component.semantic == semantic)
				.expect("Skin component should exist")
				.format = "wrong".into();
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("A mistyped skin layout component should be rejected");
			assert_eq!(
				error,
				MeshProcessingError::InvalidSkinVertexComponentFormat {
					semantic,
					expected,
					actual: "wrong".into(),
				}
			);
		}
	}

	#[test]
	fn rejects_skin_attributes_with_the_wrong_typed_payload() {
		let mut source = valid_skinned_source();
		let attribute = source.primitives_mut()[0]
			.attributes
			.iter_mut()
			.find(|attribute| attribute.semantic == VertexSemantics::Joints)
			.expect("Joint attribute should exist");
		attribute.data = OwnedMeshAttributeData::F32x2(vec![[0.0; 2]; 3]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("A mistyped joint payload should be rejected");
		assert_eq!(error, MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Joints));
	}

	#[test]
	fn rejects_vertex_joint_indices_outside_the_selected_palette() {
		let mut source = valid_skinned_source();
		let OwnedMeshAttributeData::U16x4(joints) = skin_attribute_data_mut(&mut source, VertexSemantics::Joints) else {
			panic!("Joint test data should use U16x4")
		};
		joints[0][2] = 1;

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("An out-of-range vertex joint should be rejected");
		assert_eq!(
			error,
			MeshProcessingError::VertexJointOutOfRange {
				primitive: 0,
				vertex: 0,
				lane: 2,
				joint: 1,
				palette_len: 1,
			}
		);
	}

	#[test]
	fn rejects_non_finite_negative_zero_total_and_non_normalized_skin_weights() {
		let cases = [
			(
				[f32::NAN, 0.0, 0.0, 0.0],
				MeshProcessingError::NonFiniteSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				[-0.25, 1.25, 0.0, 0.0],
				MeshProcessingError::NegativeSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				[0.0; 4],
				MeshProcessingError::NonPositiveSkinWeightTotal { primitive: 0, vertex: 0 },
			),
			(
				[0.4, 0.4, 0.0, 0.0],
				MeshProcessingError::NonNormalizedSkinWeights { primitive: 0, vertex: 0 },
			),
		];

		for (weights, expected) in cases {
			let mut source = valid_skinned_source();
			let OwnedMeshAttributeData::F32x4(values) = skin_attribute_data_mut(&mut source, VertexSemantics::Weights) else {
				panic!("Weight test data should use F32x4")
			};
			values[0] = weights;
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("Invalid skin weights should be rejected");
			assert_eq!(error, expected);
		}
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
			crate::to_vec(&VariantModel {
				material: ReferenceModel::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
			})
			.expect("Variant model should serialize"),
			None,
		)
	}

	fn skinned_layout() -> Vec<VertexComponent> {
		vec![
			VertexComponent {
				semantic: VertexSemantics::Position,
				format: "vec3f".to_string(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Joints,
				format: "vec4u16".to_string(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Weights,
				format: "vec4f".to_string(),
				channel: 0,
			},
		]
	}

	fn skinned_primitive(include_weights: bool) -> OwnedMeshPrimitive {
		let mut primitive = OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2])
			.with_skin(0)
			.with_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Position,
				0,
				OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
			))
			.with_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Joints,
				0,
				OwnedMeshAttributeData::U16x4(vec![[0, 0, 0, 0]; 3]),
			));
		if include_weights {
			primitive.add_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Weights,
				0,
				OwnedMeshAttributeData::F32x4(vec![[1.0, 0.0, 0.0, 0.0]; 3]),
			));
		}
		primitive
	}

	fn test_skeleton(node_count: usize) -> ReferenceModel<SkeletonModel> {
		ReferenceModel::new(
			"skeletons/test.skeleton",
			0,
			0,
			&SkeletonModel {
				nodes: (0..node_count)
					.map(|index| SkeletonNode {
						name: None,
						parent: index.checked_sub(1).map(|parent| parent as u32),
						rest_local: LocalTransform::identity(),
					})
					.collect(),
			},
			None,
		)
	}

	fn test_skin(joint: SkinJoint) -> SkinBinding {
		SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint,
				adjusted_inverse_bind_matrix: identity_matrix4_columns(),
			}],
		}
	}

	fn valid_skinned_source() -> OwnedMeshSource {
		OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))])
	}

	fn skin_attribute_data_mut(source: &mut OwnedMeshSource, semantic: VertexSemantics) -> &mut OwnedMeshAttributeData {
		&mut source.primitives_mut()[0]
			.attributes
			.iter_mut()
			.find(|attribute| attribute.semantic == semantic)
			.expect("Skin test attribute should exist")
			.data
	}
}
