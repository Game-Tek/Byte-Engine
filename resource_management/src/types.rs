use polodb_core::bson;
use serde::Deserialize;

use crate::{CreateResource, Resource, SolveErrors, Solver, StorageBackend, TypedResource, TypedResourceModel};

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

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Model {
	/// The name of the model.
	pub name: String,
	/// The render pass of the model.
	pub pass: String,
}

#[derive(Debug, serde::Serialize)]
pub enum Value {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Image(TypedResource<Image>),
}

#[derive(Debug, serde::Deserialize)]
pub enum ValueModel {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Image(TypedResourceModel<Image>),
}

#[derive(Debug, serde::Serialize)]
pub struct Parameter {
	pub r#type: String,
	pub name: String,
	pub value: Value,
}

#[derive(Debug, serde::Deserialize)]
pub struct ParameterModel {
	pub r#type: String,
	pub name: String,
	pub value: ValueModel,
}

impl <'de> Solver<'de, TypedResource<Image>> for TypedResourceModel<Image> {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Image>, SolveErrors> {
		let (gr, mut resource_reader) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let Image { compression, format, extent } = Image::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		let bx = {
			let mut vec = Vec::with_capacity(gr.size);
			unsafe {
				vec.set_len(gr.size);
			}
			smol::block_on(resource_reader.read_into(0, &mut vec));
			vec.into_boxed_slice()
		};

		Ok(TypedResource::new_with_buffer(&self.id, self.hash, Image {
			compression,
			format,
			extent,
		}, bx))
	}
}

impl <'de> Solver<'de, Parameter> for ParameterModel {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<Parameter, SolveErrors> {
		Ok(Parameter {
			r#type: self.r#type.clone(),
			name: self.name.clone(),
			value: match &self.value {
				ValueModel::Scalar(scalar) => Value::Scalar(*scalar),
				ValueModel::Vector3(vector) => Value::Vector3(*vector),
				ValueModel::Vector4(vector) => Value::Vector4(*vector),
				ValueModel::Image(image) => Value::Image(image.solve(storage_backend)?),
			},
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub enum Property {
	Factor(Value),
	Texture(String),
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

#[derive(Debug,serde::Serialize,)]
pub struct Material {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<TypedResource<Shader>>,

	/// The render model this material is for.
	pub model: Model,

	pub parameters: Vec<Parameter>,
}

#[derive(Debug,serde::Deserialize,)]
pub struct MaterialModel {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<TypedResourceModel<Shader>>,

	/// The render model this material is for.
	pub model: Model,

	pub parameters: Vec<ParameterModel>,
}

impl Material {
	pub fn shaders(&self) -> &[TypedResource<Shader>] {
		&self.shaders
	}
}

impl Resource for Material {
	fn get_class(&self) -> &'static str { "Material" }
}

impl Resource for MaterialModel {
	fn get_class(&self) -> &'static str { "Material" }
}

impl super::Model for MaterialModel {
	fn get_class() -> &'static str { "Material" }
}

impl <'de> Solver<'de, TypedResource<Material>> for TypedResourceModel<MaterialModel> {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Material>, SolveErrors> {
		let (gr, _) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let MaterialModel { double_sided, alpha_mode, shaders, model, parameters } = MaterialModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(TypedResource::new(&self.id, self.hash, Material {
			double_sided,
			alpha_mode,
			shaders: shaders.into_iter().map(|s| s.solve(storage_backend)).collect::<Result<_, _>>()?,
			model,
			parameters: parameters.into_iter().map(|p| p.solve(storage_backend)).collect::<Result<_, _>>()?,
		}))
	}
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VariantVariable {
	pub name: String,
	pub value: String,
}

#[derive(Debug, serde::Serialize)]
pub struct Variant {
	pub material: TypedResource<Material>,
	pub variables: Vec<VariantVariable>,
}

impl Resource for Variant {
	fn get_class(&self) -> &'static str { "Variant" }
}

#[derive(Debug, serde::Deserialize)]
pub struct VariantModel {
	pub material: TypedResourceModel<MaterialModel>,
	pub variables: Vec<VariantVariable>,
}

impl super::Model for VariantModel {
	fn get_class() -> &'static str { "Variant" }
}

impl <'de> Solver<'de, TypedResource<Variant>> for TypedResourceModel<VariantModel> {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Variant>, SolveErrors> {
		let (gr, _) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let VariantModel { material, variables } = VariantModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(TypedResource::new(&self.id, self.hash, Variant {
			material: material.solve(storage_backend)?,
			variables,
		}))
	}
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

impl super::Model for Shader {
	fn get_class() -> &'static str { "Shader" }
}

impl <'de> Solver<'de, TypedResource<Shader>> for TypedResourceModel<Shader> {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Shader>, SolveErrors> {
		let (gr, mut reader) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let Shader { id, stage } = Shader::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		let mut buffer = Vec::with_capacity(gr.size);

		unsafe {
			buffer.set_len(gr.size);
		}

		smol::block_on(reader.read_into(0, &mut buffer));

		Ok(TypedResource::new_with_buffer(&self.id, self.hash, Shader {
			id,
			stage,
		}, buffer.into()))
	}
}

// Mesh

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	Uv,
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum QuantizationSchemes {
	Quantization,
	Octahedral,
	OctahedralQuantization,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MeshletStream {
	pub offset: usize,
	pub count: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Primitive {
	// pub material: Material,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SubMesh {
	pub primitives: Vec<Primitive>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Mesh {
	pub index_streams: Vec<IndexStream>,
	pub meshlet_stream: Option<MeshletStream>,
	pub vertex_components: Vec<VertexComponent>,
	pub sub_meshes: Vec<SubMesh>,
	pub vertex_count: u32,
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

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum CompressionSchemes {
	BC7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Formats {
	RGB8,
	RGBA8,
	RGB16,
	RGBA16,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Image {
	pub compression: Option<CompressionSchemes>,
	pub format: Formats,
	pub extent: [u32; 3],
}

impl Resource for Image {
	fn get_class(&self) -> &'static str { "Image" }
}

impl super::Model for Image {
	fn get_class() -> &'static str { "Image" }
}