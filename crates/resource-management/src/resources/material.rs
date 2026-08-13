use serde::Serialize;

use crate::{
	resource,
	resources::image::Image,
	solver::SolveErrors,
	types::{AlphaMode, ShaderTypes},
	Reference, ReferenceModel, Solver,
};

#[derive(Debug, Serialize)]
pub struct Material {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub shaders: Vec<Reference<Shader>>,

	/// The render model that evaluates this material.
	pub model: RenderModel,

	pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MaterialModel {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,

	pub(crate) shaders: Vec<ReferenceModel<Shader>>,

	/// The render model that evaluates this material.
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

	pub fn alpha_mode(&self) -> &AlphaMode {
		&self.alpha_mode
	}
}

super::impl_resource_model!(Material, MaterialModel, "Material");

impl<'de> Solver<'de, Reference<Material>> for ReferenceModel<MaterialModel> {
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Material>, SolveErrors>> {
		crate::r#async::future(async move {
			let (gr, reader) = storage_backend.read(self.id()).await.ok_or(SolveErrors::StorageError)?;
			let MaterialModel {
				double_sided,
				alpha_mode,
				shaders,
				model,
				parameters,
			} = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

			let mut resolved_shaders = Vec::with_capacity(shaders.len());
			for shader in shaders {
				resolved_shaders.push(shader.solve(storage_backend).await?);
			}
			let mut resolved_parameters = Vec::with_capacity(parameters.len());
			for parameter in parameters {
				resolved_parameters.push(parameter.solve(storage_backend).await?);
			}

			Ok(Reference::from_model(
				self,
				Material {
					double_sided,
					alpha_mode,
					shaders: resolved_shaders,
					model,
					parameters: resolved_parameters,
				},
				reader,
			))
		})
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

#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct VariantVariableModel {
	pub name: String,
	pub r#type: String,
	pub value: ValueModel,
}

impl<'de> Solver<'de, VariantVariable> for VariantVariableModel {
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<VariantVariable, SolveErrors>> {
		crate::r#async::future(async move {
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
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub struct Variant {
	pub material: Reference<Material>,
	pub variables: Vec<VariantVariable>,
	pub alpha_mode: AlphaMode,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct VariantModel {
	pub material: ReferenceModel<MaterialModel>,
	pub variables: Vec<VariantVariableModel>,
	pub alpha_mode: AlphaMode,
}
super::impl_resource_model!(Variant, VariantModel, "Variant");

impl<'de> Solver<'de, Reference<Variant>> for ReferenceModel<VariantModel> {
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Variant>, SolveErrors>> {
		crate::r#async::future(async move {
			let (gr, reader) = storage_backend.read(self.id()).await.ok_or(SolveErrors::StorageError)?;
			let VariantModel {
				material,
				variables,
				alpha_mode,
			} = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

			let material = material.solve(storage_backend).await?;
			let mut resolved_variables = Vec::with_capacity(variables.len());
			for variable in variables {
				resolved_variables.push(variable.solve(storage_backend).await?);
			}

			Ok(Reference::from_model(
				self,
				Variant {
					material,
					variables: resolved_variables,
					alpha_mode,
				},
				reader,
			))
		})
	}
}

pub use crate::shader::besl::evaluation::{BindingKind, TextureView};

/// The `Binding` struct preserves one flat shader resource requirement in persisted material artifacts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Binding {
	pub name: String,
	pub slot: u32,
	pub kind: BindingKind,
	pub count: u32,
	pub buffer_stride: Option<u32>,
	pub read: bool,
	pub write: bool,
}

impl Binding {
	/// Builds one validated persisted shader resource requirement.
	pub fn new(slot: u32, kind: BindingKind, count: u32, buffer_stride: Option<u32>, read: bool, write: bool) -> Self {
		assert!(
			count > 0,
			"Invalid resource count. The most likely cause is that a shader interface resource was declared with an empty array."
		);
		assert!(
			slot.checked_add(count).is_some(),
			"Invalid resource slot range. The most likely cause is that a persisted shader resource array extends beyond the flat slot space."
		);
		match (kind, buffer_stride) {
			(BindingKind::StorageBuffer, Some(stride)) => assert!(
				stride > 0,
				"Invalid storage-buffer stride. The most likely cause is that shader reflection produced a zero-byte element."
			),
			(BindingKind::StorageBuffer, None) => panic!(
				"Missing storage-buffer stride. The most likely cause is that shader reflection was not retained in the persisted interface."
			),
			(_, Some(_)) => panic!(
				"Unexpected buffer stride. The most likely cause is that buffer metadata was attached to a non-buffer resource."
			),
			(_, None) => {}
		}
		Self {
			name: String::new(),
			slot,
			kind,
			count,
			buffer_stride,
			read,
			write,
		}
	}

	/// Associates the authored BESL descriptor name with a validated persisted binding.
	pub fn named(
		name: impl Into<String>,
		slot: u32,
		kind: BindingKind,
		count: u32,
		buffer_stride: Option<u32>,
		read: bool,
		write: bool,
	) -> Self {
		Self {
			name: name.into(),
			..Self::new(slot, kind, count, buffer_stride, read, write)
		}
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ShaderInterface {
	pub workgroup_size: Option<(u32, u32, u32)>,
	pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ShaderArtifact {
	Spirv,
	Hlsl { entry_point: String },
	Msl { entry_point: String },
	Mtlb { entry_point: String },
	Dxil,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Shader {
	pub id: String,
	pub stage: ShaderTypes,
	pub interface: ShaderInterface,
	pub artifact: ShaderArtifact,
	pub source_hash: u64,
}

impl Shader {
	pub fn id(&self) -> &str {
		&self.id
	}
}

super::impl_direct_resource!(Shader, "Shader");

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ParameterModel {
	pub r#type: String,
	pub name: String,
	pub value: ValueModel,
}

impl<'de> Solver<'de, Parameter> for ParameterModel {
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Parameter, SolveErrors>> {
		crate::r#async::future(async move {
			Ok(Parameter {
				r#type: self.r#type,
				name: self.name,
				value: match self.value {
					ValueModel::Scalar(scalar) => Value::Scalar(scalar),
					ValueModel::Vector3(vector) => Value::Vector3(vector),
					ValueModel::Vector4(vector) => Value::Vector4(vector),
					ValueModel::Image(image) => Value::Image(image.solve(storage_backend).await?),
				},
			})
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub enum Property {
	Factor(Value),
	Texture(String),
}

#[cfg(test)]
mod tests {
	use super::{Binding, BindingKind, ShaderArtifact};

	#[test]
	fn persisted_bindings_keep_named_and_unnamed_construction_distinct() {
		let unnamed = Binding::new(0, BindingKind::StorageBuffer, 1, Some(64), true, false);
		let named = Binding::named("scene", 1, BindingKind::StorageBuffer, 1, Some(80), true, false);

		assert!(unnamed.name.is_empty());
		assert_eq!(named.name, "scene");
		assert_eq!(unnamed.buffer_stride, Some(64));
		assert_eq!(named.buffer_stride, Some(80));
	}

	#[test]
	#[should_panic(expected = "Invalid resource slot range")]
	fn persisted_binding_rejects_flat_slot_overflow() {
		Binding::new(u32::MAX, BindingKind::StorageBuffer, 1, Some(4), true, false);
	}

	#[test]
	fn dxil_shader_artifact_round_trips_through_resource_archiving() {
		let bytes = crate::to_vec(&ShaderArtifact::Dxil).expect("DXIL artifact should serialize");
		let artifact: ShaderArtifact = crate::from_slice(&bytes).expect("DXIL artifact should deserialize");

		assert!(matches!(artifact, ShaderArtifact::Dxil));
	}

	#[test]
	fn storage_buffer_stride_round_trips_through_resource_archiving() {
		let binding = Binding::named("meshlets", 8, BindingKind::StorageBuffer, 1, Some(64), true, false);
		let bytes = crate::to_vec(&binding).expect("Storage-buffer binding should serialize");
		let archived: Binding = crate::from_slice(&bytes).expect("Storage-buffer binding should deserialize");

		assert_eq!(archived.name, "meshlets");
		assert_eq!(archived.buffer_stride, Some(64));
	}
}
