use std::alloc::{Allocator, Global};

use crate::{
	ReferenceModel, StreamDescription,
	resources::{
		material::VariantModel,
		mesh::{MeshModel, PrimitiveModel},
		skeleton::{SkeletonModel, SkinBinding, SkinJoint},
	},
	types::{IndexStreamTypes, IntegralTypes, Size, Stream, Streams, VertexComponent, VertexSemantics},
};

/// The `MeshAttributeData` enum provides borrowed attribute payloads to the mesh processor.
#[derive(Clone, Copy, Debug)]
pub enum MeshAttributeData<'a> {
	F32x2(&'a [[f32; 2]]),
	F32x3(&'a [[f32; 3]]),
	F32x4(&'a [[f32; 4]]),
	U16x4(&'a [[u16; 4]]),
}

impl MeshAttributeData<'_> {
	pub(super) fn len(&self) -> usize {
		match self {
			MeshAttributeData::F32x2(values) => values.len(),
			MeshAttributeData::F32x3(values) => values.len(),
			MeshAttributeData::F32x4(values) => values.len(),
			MeshAttributeData::U16x4(values) => values.len(),
		}
	}

	pub(super) fn element_size(&self) -> usize {
		match self {
			MeshAttributeData::F32x2(..) => 8,
			MeshAttributeData::F32x3(..) => 12,
			MeshAttributeData::F32x4(..) => 16,
			MeshAttributeData::U16x4(..) => 8,
		}
	}

	pub(super) fn to_bytes(&self) -> Vec<u8> {
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

/// The `MeshIndexData` enum provides borrowed index payloads to the mesh processor.
#[derive(Clone, Copy, Debug)]
pub enum MeshIndexData<'a> {
	U32(&'a [u32]),
}

impl MeshIndexData<'_> {
	pub(super) fn to_u32_vec(&self) -> Vec<u32> {
		match self {
			MeshIndexData::U32(values) => values.to_vec(),
		}
	}
}

/// The `MeshPrimitiveSource` trait provides query-based access to one mesh primitive.
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

/// The `MeshSource` trait provides mesh input that the mesh processor can pack into resource streams.
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
	pub(super) skeleton: Option<ReferenceModel<SkeletonModel>>,
	pub(super) skins: Vec<SkinBinding>,
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
	pub(super) attributes: Vec<OwnedMeshAttribute<A>, A>,
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
	pub(super) semantic: VertexSemantics,
	channel: u32,
	pub(super) data: OwnedMeshAttributeData<A>,
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
		self.borrow().len()
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
