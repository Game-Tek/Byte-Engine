use serde::Serialize;

use crate::{
	Reference, ReferenceModel, Solver, resource,
	resources::image::Image,
	solver::SolveError,
	types::{AlphaMode, ShaderTypes},
};

#[derive(Debug, Serialize)]
pub struct Material {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,
	pub coverage: MaterialCoverage,

	pub shaders: Vec<Reference<Shader>>,

	/// The render model that evaluates this material.
	pub model: RenderModel,

	pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MaterialModel {
	pub(crate) double_sided: bool,
	pub(crate) alpha_mode: AlphaMode,
	pub coverage: MaterialCoverage,

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

	/// Returns whether both faces of the surface are visible, as glTF `doubleSided` requires.
	pub fn double_sided(&self) -> bool {
		self.double_sided
	}
}

super::impl_resource_model!(Material, MaterialModel, "Material");

impl crate::StoredModel for MaterialModel {
	type Resource = Material;

	fn solve_stored<'de>(
		gr: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Material>, SolveError>> {
		crate::r#async::future(async move {
			let MaterialModel {
				double_sided,
				alpha_mode,
				coverage,
				shaders,
				model,
				parameters,
			} = crate::from_slice(&gr.resource).map_err(|e| SolveError::DeserializationFailed(e.to_string()))?;

			let resolved_shaders = super::solve_all(shaders, storage_backend).await?;
			let resolved_parameters = super::solve_all(parameters, storage_backend).await?;

			Ok(Reference::from_stored(
				gr,
				Material {
					double_sided,
					alpha_mode,
					coverage,
					shaders: resolved_shaders,
					model,
					parameters: resolved_parameters,
				},
				reader,
			))
		})
	}
}

/// The `MaterialCoverage` struct provides the minimal base-color alpha expression needed by masked raster passes.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MaterialCoverage {
	pub factor: f32,
	pub texture_slot: Option<u32>,
}

impl Default for MaterialCoverage {
	fn default() -> Self {
		Self {
			factor: 1.0,
			texture_slot: None,
		}
	}
}

// impl <'a, 'de> Solver<'de, RequestReference<'a, Material<'a>>> for ReferenceModel<MaterialModel> {
// 	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<RequestReference<'a, Material<'a>>, SolveError> {
// 		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveError::StorageError)?;
// 		let MaterialModel { double_sided, alpha_mode, shaders, model, parameters } = MaterialModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveError::DeserializationFailed(e.to_string()))?;

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
	) -> crate::r#async::BoxedFuture<'de, Result<VariantVariable, SolveError>> {
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

impl crate::Resource for Variant {
	type Model = VariantModel;
}

impl crate::Model for VariantModel {
	fn get_class() -> &'static str {
		"Variant"
	}

	fn queryable_properties(&self, id: &str) -> Vec<crate::QueryableProperty> {
		let material = crate::from_slice::<MaterialModel>(&self.material.resource).ok();
		variant_queryable_properties(id, material.as_ref().map(|material| &material.model))
	}
}

/// Indexes a variant by name and by the render model and pass of its material, so renderers can query their materials.
fn variant_queryable_properties(id: &str, model: Option<&RenderModel>) -> Vec<crate::QueryableProperty> {
	let property = |name: &str, value: &str| crate::QueryableProperty {
		name: name.to_string(),
		value: crate::QueryableValue::String(value.to_string()),
	};
	let mut properties = vec![property("name", id)];
	if let Some(model) = model {
		properties.push(property("render-model", &model.name));
		properties.push(property("render-pass", &model.pass));
	}
	properties
}

impl crate::StoredModel for VariantModel {
	type Resource = Variant;

	fn solve_stored<'de>(
		gr: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Variant>, SolveError>> {
		crate::r#async::future(async move {
			let VariantModel {
				material,
				variables,
				alpha_mode,
			} = crate::from_slice(&gr.resource).map_err(|e| SolveError::DeserializationFailed(e.to_string()))?;

			let material = material.solve(storage_backend).await?;
			let resolved_variables = super::solve_all(variables, storage_backend).await?;

			Ok(Reference::from_stored(
				gr,
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

impl From<crate::shader::besl::evaluation::BindingUsage> for Binding {
	/// Persists one reflected BESL binding, keeping its authored name for runtime lookups.
	fn from(binding: crate::shader::besl::evaluation::BindingUsage) -> Self {
		Self::named(
			binding.name,
			binding.slot,
			binding.kind,
			binding.count,
			binding.buffer_stride,
			binding.read,
			binding.write,
		)
	}
}

/// The `SetInterface` struct preserves one descriptor set layout a shader declares, so the renderer can create a
/// descriptor set for it, bind resources to it by name, and share it between every shader that lists the same
/// layout layer.
///
/// `bindings` holds the whole layout, because the native set must look the same to every shader that shares it.
/// `used` records which bindings this shader can reach; only those are its resource accesses, and only those need a
/// resource before the shader runs. Compare layouts with [`Self::identity`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SetInterface {
	/// The layout layer that declares the set, or the shader's own name for the descriptors the shader declares
	/// itself. Diagnostics name the set with it.
	pub layout: String,
	/// Every binding of the layout, ordered by slot.
	pub bindings: Vec<Binding>,
	/// Bit `i` is set when the shader uses `bindings[i]`.
	pub used: u64,
}

impl SetInterface {
	/// Persists one reflected set. `own_layout` names the layout of the descriptors the shader declares itself.
	pub fn new(set: crate::shader::besl::evaluation::SetUsage, own_layout: &str) -> Self {
		Self {
			layout: set.layout.unwrap_or_else(|| own_layout.to_string()),
			bindings: set.bindings.into_iter().map(Binding::from).collect(),
			used: set.used,
		}
	}

	/// Builds a layout named `layout` whose every binding the shader uses.
	pub fn with_bindings(layout: impl Into<String>, bindings: Vec<Binding>) -> Self {
		assert!(
			bindings.len() <= u64::BITS as usize,
			"Too many bindings in one descriptor set layout. The most likely cause is that one layout groups resources that belong to several descriptor sets."
		);
		// One bit per binding, without shifting a whole word out when the layout is full.
		let used = match bindings.len() {
			0 => 0,
			count => u64::MAX >> (u64::BITS as usize - count),
		};
		Self {
			layout: layout.into(),
			bindings,
			used,
		}
	}

	/// Returns whether the shader uses the binding at `index`.
	pub fn is_used(&self, index: usize) -> bool {
		self.used & (1 << index) != 0
	}

	/// Returns the binding named `name`.
	pub fn binding(&self, name: &str) -> Option<&Binding> {
		self.bindings.iter().find(|binding| binding.name == name)
	}

	/// Returns a value that is equal for two layouts exactly when their bindings are equal: names, slots, kinds,
	/// counts, strides, and access.
	///
	/// A descriptor set created for one layout can be bound wherever a shader declares a layout of the same identity.
	/// The layout's name and the bindings a shader uses are not part of it, so a set built from a layer's source matches
	/// every shader baked from that source, and a shader baked from an older layer does not.
	pub fn identity(&self) -> u64 {
		layout_identity(&self.bindings)
	}
}

/// Hashes a layout's bindings with FNV-1a, which is stable across processes and builds.
fn layout_identity(bindings: &[Binding]) -> u64 {
	const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
	const PRIME: u64 = 0x0000_0100_0000_01b3;

	let mut hash = OFFSET_BASIS;
	let mut write = |bytes: &[u8]| {
		for byte in bytes {
			hash = (hash ^ u64::from(*byte)).wrapping_mul(PRIME);
		}
	};

	for binding in bindings {
		let (kind, view) = match binding.kind {
			BindingKind::StorageBuffer => (0u8, 0u8),
			BindingKind::StorageImage => (1, 0),
			BindingKind::CombinedImageSampler { view } => (
				2,
				match view {
					TextureView::Texture2D => 0,
					TextureView::Texture2DArray => 1,
					TextureView::TextureCube => 2,
					TextureView::TextureCubeArray => 3,
					TextureView::Texture3D => 4,
				},
			),
		};

		write(binding.name.as_bytes());
		// The separator keeps a name from running into the fields after it.
		write(&[0, kind, view, u8::from(binding.read), u8::from(binding.write)]);
		write(&binding.slot.to_le_bytes());
		write(&binding.count.to_le_bytes());
		write(&binding.buffer_stride.unwrap_or(0).to_le_bytes());
	}

	hash
}

/// The `ShaderInterface` struct preserves what a baked shader needs from the renderer: its workgroup and the
/// descriptor set layouts it declares.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ShaderInterface {
	pub workgroup_size: Option<(u32, u32, u32)>,
	/// The descriptor set layouts the shader declares: one per layout layer it lists that declares descriptors, in
	/// listing order, then its own.
	pub sets: Vec<SetInterface>,
}

impl ShaderInterface {
	/// Returns every binding of every set, the flat interface a backend without native per-layout sets consumes.
	pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
		self.sets.iter().flat_map(|set| &set.bindings)
	}
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
	) -> crate::r#async::BoxedFuture<'de, Result<Parameter, SolveError>> {
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
