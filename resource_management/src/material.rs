use futures::future::try_join_all;
use polodb_core::bson;
use serde::Deserialize;

use crate::{image::Image, resource::resource_handler::ReadTargets, types::{AlphaMode, ShaderTypes}, LoadResults, Loader, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

#[derive(Debug, serde::Serialize)]
pub struct Material<'a> {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<Reference<'a, Shader>>,

	/// The render model this material is for.
	pub model: RenderModel,

	pub parameters: Vec<Parameter<'a>>,
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

impl <'a> Material<'a> {
	pub fn shaders(&self) -> &[Reference<Shader>] {
		&self.shaders
	}
}

impl <'a> Resource for Material<'a> {
	fn get_class(&self) -> &'static str { "Material" }

	type Model = MaterialModel;
}

impl Model for MaterialModel {
	fn get_class() -> &'static str { "Material" }
}

impl <'a, 'de> Solver<'de, Reference<'a, Material<'a>>> for ReferenceModel<MaterialModel> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Material<'a>>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let MaterialModel { double_sided, alpha_mode, shaders, model, parameters } = MaterialModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, gr.size, Material {
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
pub struct VariantVariable<'a> {
	pub name: String,
	pub r#type: String,
	pub value: Value<'a>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VariantVariableModel {
	pub name: String,
	pub r#type: String,
	pub value: ValueModel,
}

impl <'a, 'de> Solver<'de, VariantVariable<'a>> for VariantVariableModel {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<VariantVariable<'a>, SolveErrors> {
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
pub struct Variant<'a> {
	pub material: Reference<'a, Material<'a>>,
	pub variables: Vec<VariantVariable<'a>>,
}

impl <'a> Resource for Variant<'a> {
	fn get_class(&self) -> &'static str { "Variant" }

	type Model = VariantModel;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VariantModel {
	pub material: ReferenceModel<MaterialModel>,
	pub variables: Vec<VariantVariableModel>,
}

impl Model for VariantModel {
	fn get_class() -> &'static str { "Variant" }
}

impl <'a, 'de> Solver<'de, Reference<'a, Variant<'a>>> for ReferenceModel<VariantModel> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Variant<'a>>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let VariantModel { material, variables } = VariantModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, gr.size, Variant {
			material: material.solve(storage_backend).await?,
			variables: try_join_all(variables.into_iter().map(|v| v.solve(storage_backend))).await?,
		}, reader))
	}
}

// impl <'a, 'de> Solver<'de, RequestReference<'a, Variant<'a>>> for ReferenceModel<VariantModel> {
// 	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<RequestReference<'a, Variant<'a>>, SolveErrors> {
// 		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
// 		let VariantModel { material, variables } = VariantModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

// 		Ok(RequestReference::new(Reference::new(&self.id, self.hash, gr.size, Variant {
// 			material: material.solve(storage_backend).await?,
// 			variables: try_join_all(variables.into_iter().map(|v| v.solve(storage_backend))).await?,
// 		}), reader))
// 	}
// }

impl <'a> Loader for Reference<'a, Variant<'a>> {
	async fn load(self,) -> Result<Self, LoadResults> {
		Ok(self) // No need to load anything
	}
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

	type Model = Shader;
}

impl super::Model for Shader {
	fn get_class() -> &'static str { "Shader" }
}

impl <'a, 'de> Solver<'de, Reference<'a, Shader>> for ReferenceModel<Shader> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Shader>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let Shader { id, stage } = Shader::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, gr.size, Shader {
			id,
			stage,
		}, reader))
	}
}

// impl <'a, 'de> Solver<'de, RequestReference<'a, Shader>> for ReferenceModel<Shader> {
// 	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<RequestReference<'a, Shader>, SolveErrors> {
// 		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
// 		let Shader { id, stage } = Shader::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

// 		Ok(RequestReference::new(Reference::new(&self.id, self.hash, gr.size, Shader {
// 			id,
// 			stage,
// 		}), reader))
// 	}
// }

impl <'a> Loader for Reference<'a, Shader> {
	async fn load(mut self,) -> Result<Self, LoadResults> {
		let reader = &mut self.reader;

		if let Some(read_target) = &mut self.read_target {
			match read_target {
				ReadTargets::Buffer(buffer) => {
					reader.read_into(0, buffer).await.ok_or(LoadResults::LoadFailed)?;
				},
				ReadTargets::Box(buffer) => {
					reader.read_into(0, buffer).await.ok_or(LoadResults::LoadFailed)?;
				},
				_ => {
					return Err(LoadResults::NoReadTarget);
				}
				
			}
		} else {
			// log::warn!("No read target found for shader resource: {}", self.id);
		}

		Ok(self)
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
pub enum Value<'a> {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Image(Reference<'a, Image>),
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub enum ValueModel {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Image(ReferenceModel<Image>),
}

#[derive(Debug, serde::Serialize)]
pub struct Parameter<'a> {
	pub r#type: String,
	pub name: String,
	pub value: Value<'a>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ParameterModel {
	pub r#type: String,
	pub name: String,
	pub value: ValueModel,
}

impl <'a, 'de> Solver<'de, Parameter<'a>> for ParameterModel {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Parameter<'a>, SolveErrors> {
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
pub enum Property<'a> {
	Factor(Value<'a>),
	Texture(String),
}