use futures::future::try_join_all;
use utils::Extent;

use crate::{asset::ResourceId, image::Image, types::{AlphaMode, ShaderTypes}, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, resource};

#[derive(Debug, serde::Serialize)]
pub struct Material {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub shaders: Vec<Reference<Shader>>,

	/// The render model this material is for.
	pub model: RenderModel,

	pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MaterialModel {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<ReferenceModel<Shader>>,

	/// The render model this material is for.
	pub model: RenderModel,

	pub parameters: Vec<ParameterModel>,
}

impl Material {
	pub fn into_shaders(self) -> Vec<Reference<Shader>> {
		self.shaders
	}

	pub fn shaders(&self) -> &[Reference<Shader>] {
		&self.shaders
	}

	pub fn shaders_mut(&mut self) -> &mut [Reference<Shader>] {
		&mut self.shaders
	}
}

impl Resource for Material {
	fn get_class(&self) -> &'static str { "Material" }

	type Model = MaterialModel;
}

impl Model for MaterialModel {
	fn get_class() -> &'static str { "Material" }
}

impl <'de> Solver<'de, Reference<Material>> for ReferenceModel<MaterialModel> {
	async fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Material>, SolveErrors> {
		let (gr, reader) = storage_backend.read(ResourceId::new(&self.id)).await.ok_or_else(|| SolveErrors::StorageError)?;
		let MaterialModel { double_sided, alpha_mode, shaders, model, parameters } = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Material {
			double_sided,
			alpha_mode,
			shaders: try_join_all(shaders.into_iter().map(|s| s.solve(storage_backend))).await?,
			model,
			parameters: try_join_all(parameters.into_iter().map(|p| p.solve(storage_backend))).await?,
		}, reader))
	}
}

// impl <'a, 'de> Solver<'de, RequestReference<'a, Material<'a>>> for ReferenceModel<MaterialModel> {
// 	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<RequestReference<'a, Material<'a>>, SolveErrors> {
// 		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
// 		let MaterialModel { double_sided, alpha_mode, shaders, model, parameters } = MaterialModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

// 		Ok(RequestReference::new(Reference::new(&self.id, self.hash, gr.size, Material {
// 			double_sided,
// 			alpha_mode,
// 			shaders: try_join_all(shaders.into_iter().map(|s| s.solve(storage_backend))).await?,
// 			model,
// 			parameters: try_join_all(parameters.into_iter().map(|p| p.solve(storage_backend))).await?,
// 		}), reader))
// 	}
// }

#[derive(Debug, serde::Serialize)]
pub struct VariantVariable {
	pub name: String,
	pub r#type: String,
	pub value: Value,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VariantVariableModel {
	pub name: String,
	pub r#type: String,
	pub value: ValueModel,
}

impl <'de> Solver<'de, VariantVariable> for VariantVariableModel {
	async fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<VariantVariable, SolveErrors> {
		Ok(VariantVariable {
			name: self.name,
			r#type: self.r#type,
			value: match self.value {
				ValueModel::Scalar(scalar) => Value::Scalar(scalar),
				ValueModel::Vector3(vector) => Value::Vector3(vector),
				ValueModel::Vector4(vector) => Value::Vector4(vector),
				ValueModel::Image(image) => Value::Image(image.solve(storage_backend).await?),
			},
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub struct Variant {
	pub material: Reference<Material>,
	pub variables: Vec<VariantVariable>,
	pub alpha_mode: AlphaMode,
}

impl Resource for Variant {
	fn get_class(&self) -> &'static str { "Variant" }

	type Model = VariantModel;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VariantModel {
	pub material: ReferenceModel<MaterialModel>,
	pub variables: Vec<VariantVariableModel>,
	pub alpha_mode: AlphaMode,
}

impl Model for VariantModel {
	fn get_class() -> &'static str { "Variant" }
}

impl <'de> Solver<'de, Reference<Variant>> for ReferenceModel<VariantModel> {
	async fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Variant>, SolveErrors> {
		let (gr, reader) = storage_backend.read(ResourceId::new(&self.id)).await.ok_or_else(|| SolveErrors::StorageError)?;
		let VariantModel { material, variables, alpha_mode } = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Variant {
			material: material.solve(storage_backend).await?,
			variables: try_join_all(variables.into_iter().map(|v| v.solve(storage_backend))).await?,
			alpha_mode,
		}, reader))
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Binding {
	pub set: u32,
	pub binding: u32,
	pub read: bool,
	pub write: bool,
}

impl Binding {
	pub fn new(set: u32, binding: u32, read: bool, write: bool) -> Self {
		Self {
			set,
			binding,
			read,
			write,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShaderInterface {
	pub workgroup_size: Option<(u32, u32, u32)>,
	pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Shader {
	pub id: String,
	pub stage: ShaderTypes,
	pub interface: ShaderInterface,
}

impl Shader {
	pub fn id(&self) -> &str {
		&self.id
	}
}

impl Resource for Shader {
	fn get_class(&self) -> &'static str { "Shader" }

	type Model = Shader;
}

impl super::Model for Shader {
	fn get_class() -> &'static str { "Shader" }
}

impl <'de> Solver<'de, Reference<Shader>> for ReferenceModel<Shader> {
	async fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Shader>, SolveErrors> {
		let (gr, reader) = storage_backend.read(ResourceId::new(&self.id)).await.ok_or_else(|| SolveErrors::StorageError)?;
		let Shader { id, stage, interface } = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Shader {
			id,
			stage,
			interface,
		}, reader))
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderModel {
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
	Image(Reference<Image>),
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub enum ValueModel {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Image(ReferenceModel<Image>),
}

#[derive(Debug, serde::Serialize)]
pub struct Parameter {
	pub r#type: String,
	pub name: String,
	pub value: Value,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ParameterModel {
	pub r#type: String,
	pub name: String,
	pub value: ValueModel,
}

impl <'de> Solver<'de, Parameter> for ParameterModel {
	async fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Parameter, SolveErrors> {
		Ok(Parameter {
			r#type: self.r#type.clone(),
			name: self.name.clone(),
			value: match self.value {
				ValueModel::Scalar(scalar) => Value::Scalar(scalar),
				ValueModel::Vector3(vector) => Value::Vector3(vector),
				ValueModel::Vector4(vector) => Value::Vector4(vector),
				ValueModel::Image(image) => Value::Image(image.solve(storage_backend).await?),
			},
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub enum Property {
	Factor(Value),
	Texture(String),
}
