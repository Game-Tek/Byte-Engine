use serde::{Deserialize, Serialize};

use crate::{CreateResource, Resource, TypedResource};

// Audio

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Audio {
	pub bit_depth: BitDepths,
	pub channel_count: u16,
	pub sample_rate: u32,
	pub sample_count: u32,
}

impl Resource for Audio {
	fn get_class(&self) -> &'static str { "Audio" }
}

// Material

#[derive(Debug, Serialize, Deserialize)]
pub struct Model {
	/// The name of the model.
	pub name: String,
	/// The render pass of the model.
	pub pass: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Value {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Property {
	Factor(Value),
	Texture(String),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

#[derive(Debug,Serialize,Deserialize)]
pub struct Material {
	pub(crate) albedo: Property,
	pub(crate) normal: Property,
	pub(crate) roughness: Property,
	pub(crate) metallic: Property,
	pub(crate) emissive: Property,
	pub(crate) occlusion: Property,
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<TypedResource<Shader>>,

	/// The render model this material is for.
	pub model: Model,
}

impl Material {
	pub fn shaders(&self) -> &[TypedResource<Shader>] {
		&self.shaders
	}
}

impl Resource for Material {
	fn get_class(&self) -> &'static str { "Material" }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VariantVariable {
	pub name: String,
	pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Variant {
	pub material: TypedResource<Material>,
	pub variables: Vec<VariantVariable>,
}

impl Resource for Variant {
	fn get_class(&self) -> &'static str { "Variant" }
}

/// Enumerates the types of shaders that can be created.
#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shader {
	pub id: String,
	pub stage: ShaderTypes,
}

impl Shader {
	pub fn id(&self) -> &str {
		&self.id
	}
}

impl Resource for Shader {
	fn get_class(&self) -> &'static str { "Shader" }
}

// Mesh

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	Uv,
	Color,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum QuantizationSchemes {
	Quantization,
	Octahedral,
	OctahedralQuantization,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum IndexStreamTypes {
	Vertices,
	Meshlets,
	Triangles,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexStream {
	pub stream_type: IndexStreamTypes,
	pub offset: usize,
	pub count: u32,
	pub data_type: IntegralTypes,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeshletStream {
	pub offset: usize,
	pub count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Primitive {
	// pub material: Material,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_components: Vec<VertexComponent>,
	pub vertex_count: u32,
	pub index_streams: Vec<IndexStream>,
	pub meshlet_stream: Option<MeshletStream>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubMesh {
	pub primitives: Vec<Primitive>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Mesh {
	pub sub_meshes: Vec<SubMesh>,
}

impl Resource for Mesh {
	fn get_class(&self) -> &'static str { "Mesh" }
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
			VertexSemantics::Uv => 2 * 4,
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

#[derive(Debug, Serialize, Deserialize)]
pub enum CompressionSchemes {
	BC7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Formats {
	RGB8,
	RGBA8,
	RGB16,
	RGBA16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Image {
	pub compression: Option<CompressionSchemes>,
	pub format: Formats,
	pub extent: [u32; 3],
}

impl Resource for Image {
	fn get_class(&self) -> &'static str { "Image" }
}