/// The `ProgramGenerator` trait provides renderer-specific shader adaptation before platform compilation.
pub trait ProgramGenerator: Send + Sync {
	/// Adapts a parsed material program to the bindings and entry-point contract used by its renderer.
	fn transform<'a>(&self, node: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a>;
}

impl<T: ProgramGenerator + ?Sized> ProgramGenerator for Arc<T> {
	fn transform<'a>(&self, node: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
		self.as_ref().transform(node, material)
	}
}

/// The `ShaderCompiler` trait isolates BEMA resource orchestration from platform shader toolchains.
trait ShaderCompiler: Send + Sync {
	fn compile<'a>(
		&'a self,
		generator: &'a dyn ProgramGenerator,
		name: &'a str,
		shader_code: &'a str,
		format: &'a str,
		domain: &'a str,
		material: &'a JsonObject,
		shader_json: &'a Value,
		stage: &'a str,
	) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), ()>>;
}

/// The `PlatformShaderCompiler` struct routes production BEMA shaders through the active platform compiler.
struct PlatformShaderCompiler;

impl ShaderCompiler for PlatformShaderCompiler {
	fn compile<'a>(
		&'a self,
		generator: &'a dyn ProgramGenerator,
		name: &'a str,
		shader_code: &'a str,
		format: &'a str,
		domain: &'a str,
		material: &'a JsonObject,
		shader_json: &'a Value,
		stage: &'a str,
	) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), ()>> {
		Box::pin(compile_shader(
			generator,
			name,
			shader_code,
			format,
			domain,
			material,
			shader_json,
			stage,
		))
	}
}

pub struct BEMAAssetHandler {
	generator: Option<Arc<dyn ProgramGenerator>>,
	compiler: Arc<dyn ShaderCompiler>,
}

impl Default for BEMAAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl BEMAAssetHandler {
	pub fn new() -> BEMAAssetHandler {
		BEMAAssetHandler {
			generator: None,
			compiler: Arc::new(PlatformShaderCompiler),
		}
	}

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
	}
}

impl AssetHandler for BEMAAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "bema"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		use utils::r#async::StreamExt as _;

		if let Some(dt) = context.resource_type(url) {
			if dt != "bema" {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let (data, _, at) = context.resolve(url).await?;

		if at != "bema" {
			return Err(LoadErrors::UnsupportedType);
		}

		let asset = asset::parse_json(std::str::from_utf8(&data).map_err(|_| LoadErrors::FailedToProcess)?)
			.map_err(|_| LoadErrors::FailedToProcess)?;

		let is_material = asset.get("parent").is_none();

		if is_material {
			let asset_object = asset.as_object().ok_or(LoadErrors::FailedToProcess)?;

			let material_domain = asset["domain"].as_str().ok_or(LoadErrors::FailedToProcess)?;

			let generator = self.generator.clone().ok_or(LoadErrors::FailedToProcess)?;

			let asset_shaders = match asset["shaders"].as_object() {
				Some(v) => v,
				None => {
					return Err(LoadErrors::FailedToProcess);
				}
			};

			// Compile independent stages together while preserving declaration order in the material model.
			let shader_requests = asset_shaders.iter().map(|(shader_type, shader_json)| {
				compile_and_store_shader(
					context,
					self.compiler.clone(),
					generator.clone(),
					material_domain,
					asset_object,
					shader_json,
					shader_type,
				)
			});

			let shaders = utils::r#async::stream::iter(shader_requests)
				.buffered(4)
				.collect::<Vec<_>>()
				.await
				.into_iter()
				.collect::<Result<Vec<_>, _>>()?;

			let asset_variables = match asset["variables"].as_array() {
				Some(v) => v,
				None => {
					return Err(LoadErrors::FailedToProcess);
				}
			};

			// Texture parameters can trigger independent dependency bakes; scalar values complete immediately.
			let value_requests = asset_variables.iter().map(|variable| async move {
				let data_type = variable["data_type"].as_str().ok_or(LoadErrors::FailedToProcess)?;

				let value = variable["value"].as_str().ok_or(LoadErrors::FailedToProcess)?;

				resolve_value(context, data_type, value).await
			});

			let values = utils::r#async::stream::iter(value_requests)
				.buffered(8)
				.collect::<Vec<_>>()
				.await
				.into_iter()
				.collect::<Result<Vec<_>, _>>()?;

			let parameters = asset_variables
				.iter()
				.zip(values)
				.map(|(v, value)| {
					let name = v["name"].as_str().unwrap().to_string();

					let data_type = v["data_type"].as_str().unwrap().to_string();

					ParameterModel {
						name,
						r#type: data_type.clone(),
						value,
					}
				})
				.collect();

			let resource = MaterialModel {
				double_sided: false,
				alpha_mode: AlphaMode::Opaque,
				coverage: MaterialCoverage {
					factor: 1.0,
					texture_slot: None,
				},
				model: RenderModel {
					name: "Visibility".to_string(),
					pass: "MaterialEvaluation".to_string(),
				},
				shaders,
				parameters,
			};

			let resource = ProcessedAsset::new(url, resource);

			context.store_primary(resource, &[])
		} else {
			let parent_material_url = asset["parent"].as_str().unwrap();

			let material = context.bake_dependency(parent_material_url).await?;

			let material_repr: MaterialModel = crate::from_slice(&material.resource).unwrap();

			let authored_variables = asset["variables"].as_array().ok_or(LoadErrors::FailedToProcess)?;

			let value_requests = material_repr.parameters.iter().map(|parameter| async move {
				let value = authored_variables
					.iter()
					.find(|variable| variable["name"].as_str() == Some(parameter.name.as_str()))
					.and_then(|variable| variable["value"].as_str())
					.ok_or(LoadErrors::FailedToProcess)?;

				resolve_value(context, &parameter.r#type, value).await
			});

			let values = utils::r#async::stream::iter(value_requests)
				.buffered(8)
				.collect::<Vec<_>>()
				.await
				.into_iter()
				.collect::<Result<Vec<_>, _>>()?;

			let variables = material_repr
				.parameters
				.iter()
				.zip(values)
				.map(|(v, value)| VariantVariableModel {
					value,
					name: v.name.clone(),
					r#type: v.r#type.clone(),
				})
				.collect();

			let alpha_mode = match asset.get("transparency") {
				Some(Value::Bool(v)) => {
					if *v {
						AlphaMode::Blend
					} else {
						AlphaMode::Opaque
					}
				}
				Some(Value::String(s)) => match s.as_str() {
					"Opaque" => AlphaMode::Opaque,
					"Blend" => AlphaMode::Blend,
					_ => AlphaMode::Opaque,
				},
				_ => AlphaMode::Opaque,
			};

			let resource = ProcessedAsset::new(
				url,
				VariantModel {
					material,
					variables,
					alpha_mode,
				},
			);

			context.store_primary(resource, &[])
		}
	}
}

/// Converts a shader source into a compiled shader and binary payload.
async fn compile_shader(
	generator: &dyn ProgramGenerator,
	name: &str,
	shader_code: &str,
	format: &str,
	_domain: &str,
	material: &JsonObject,
	_shader_json: &Value,
	stage: &str,
) -> Result<(Shader, Box<[u8]>), ()> {
	let root_node = if format == "glsl" {
		// besl::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		panic!()
	} else if format == "besl" {
		if let Ok(e) = besl::parse(shader_code) {
			e
		} else {
			log::error!(
				"Failed to parse BESL material shader. The most likely cause is invalid BESL syntax. See {}.",
				online_docs_url(BESL_DOCS_PATH)
			);

			return Err(());
		}
	} else {
		log::error!(
			"Unknown material shader format '{format}'. The most likely cause is an unsupported format in the .bema declaration. See {}.",
			online_docs_url(BEMA_DOCS_PATH)
		);

		return Err(());
	};

	compile_shader_program(generator, name, root_node, _domain, material, stage).await
}

/// Compiles a BESL shader program into a stored shader model and binary payload.
pub(crate) async fn compile_shader_program(
	generator: &dyn ProgramGenerator,
	name: &str,
	root_node: besl::parser::Node<'_>,
	_domain: &str,
	material: &JsonObject,
	stage: &str,
) -> Result<(Shader, Box<[u8]>), ()> {
	let root = generator.transform(root_node, material);

	let root_node = match besl::lex(root) {
		Ok(e) => e,
		Err(e) => {
			log::error!(
				"Failed to compile shader '{name}' for stage '{stage}': {e:#?}. See {}.",
				online_docs_url(BESL_DOCS_PATH)
			);

			return Err(());
		}
	};

	if root_node.get_main().is_none() {
		log::error!(
			"Failed to compile shader '{name}' for stage '{stage}'. The generated BESL program has no main function. See {}.",
			online_docs_url(BESL_DOCS_PATH)
		);

		return Err(());
	}

	let settings = match stage {
		"Vertex" => ShaderGenerationSettings::vertex(),
		"Fragment" => ShaderGenerationSettings::fragment(),
		"Compute" => ShaderGenerationSettings::compute(Extent::line(128)),
		_ => {
			panic!("Invalid shader stage")
		}
	}
	.name(name.to_string());

	let evaluation = ProgramEvaluation::from_program(&root_node).map_err(|error| {
		log::error!(
			"Failed to reflect shader '{name}' for stage '{stage}': {error}. See {}.",
			online_docs_url(BESL_DOCS_PATH)
		);
	})?;

	let shader_program = PlatformShaderGenerator::new()
		.generate(&settings, &root_node)
		.await
		.map_err(|error| {
			log::error!(
				"Failed to compile shader '{name}' for stage '{stage}': {error}. See {}.",
				online_docs_url(BESL_DOCS_PATH)
			);
		})?;

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		_ => {
			panic!("Invalid shader stage")
		}
	};

	let interface = ShaderInterface {
		workgroup_size: shader_program.extent().map(|e| (e.width(), e.height(), e.depth())),
		bindings: evaluation
			.into_bindings()
			.into_iter()
			.map(|binding| {
				Binding::named(
					binding.name,
					binding.slot,
					binding.kind,
					binding.count,
					binding.buffer_stride,
					binding.read,
					binding.write,
				)
			})
			.collect(),
	};

	let language = PlatformShaderLanguage::current_platform();

	let entry_point = shader_program.entry_point();

	let (artifact, payload) =
		finalize_platform_shader_artifact(language, stage, name, entry_point, shader_program.into_binary()).map_err(
			|error| {
				log::error!(
					"Failed to finalize shader artifact '{name}' for stage '{stage:?}': {error}. See {}.",
					online_docs_url(BESL_DOCS_PATH)
				);
			},
		)?;

	let shader = Shader {
		id: name.to_string(),
		stage,
		interface,
		artifact,
		source_hash: 0,
	};

	Ok((shader, payload))
}

/// Compiles a shader definition and stores the resulting resource and binary payload.
async fn compile_and_store_shader(
	context: BakeContext<'_>,
	compiler: Arc<dyn ShaderCompiler>,
	generator: Arc<dyn ProgramGenerator>,
	domain: &str,
	material: &JsonObject,
	shader_json: &Value,
	stage: &str,
) -> Result<ReferenceModel<Shader>, LoadErrors> {
	let path = shader_json.as_str().ok_or(LoadErrors::FailedToProcess)?;

	let path = ResourceId::new(path);

	let (arlp, _, format) = context.resolve(path).await?;

	let shader_code = std::str::from_utf8(&arlp)
		.map_err(|_| LoadErrors::FailedToProcess)?
		.to_string();

	let material = material.clone();

	let domain = domain.to_string();

	let stage = stage.to_string();

	let format = format.to_string();

	let name = path.get_base().as_ref().to_string();

	let shader_json = shader_json.clone();

	let (shader, result_shader_bytes) = compiler
		.compile(
			generator.as_ref(),
			&name,
			&shader_code,
			&format,
			&domain,
			&material,
			&shader_json,
			&stage,
		)
		.await
		.map_err(|_| LoadErrors::FailedToProcess)?;

	context
		.store_generated(ProcessedAsset::new(path, shader), &result_shader_bytes)
		.map(Into::into)
}

/// Resolves a material parameter value based on its type.
async fn resolve_value(context: BakeContext<'_>, data_type: &str, value: &str) -> Result<ValueModel, LoadErrors> {
	let to_color = |name: &str| match name {
		"Red" => [1f32, 0f32, 0f32, 1f32],
		"Green" => [0f32, 1f32, 0f32, 1f32],
		"Blue" => [0f32, 0f32, 1f32, 1f32],
		"Purple" => [1f32, 0f32, 1f32, 1f32],
		"White" => [1f32, 1f32, 1f32, 1f32],
		"Black" => [0f32, 0f32, 0f32, 1f32],
		_ => [1f32, 0f32, 1f32, 1f32],
	};

	match data_type {
		"vec4f" => {
			let value = to_color(value);

			Ok(ValueModel::Vector4([value[0], value[1], value[2], value[3]]))
		}
		"vec3f" => {
			let value = to_color(value);

			Ok(ValueModel::Vector3([value[0], value[1], value[2]]))
		}
		"float" => Ok(ValueModel::Scalar(0f32)),
		"Texture2D" => {
			let image = context.bake_dependency(value).await?;

			Ok(ValueModel::Image(image))
		}
		_ => Err(LoadErrors::FailedToProcess),
	}
}

const BEMA_DOCS_PATH: &str = "develop/design/resource-management/bema";

const BESL_DOCS_PATH: &str = "reference/besl";

#[cfg(test)]

pub mod tests {

	use std::sync::Arc;

	use serde_json::Value;

	use super::ProgramGenerator;
	use crate::asset::JsonObject;
	use crate::{
		asset::{
			handler::implementations::bema::BEMAAssetHandler, handler::AssetHandler, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend, ResourceId,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::VariantModel,
		ReferenceModel,
	};

	struct TestShaderCompiler;

	impl super::ShaderCompiler for TestShaderCompiler {
		fn compile<'a>(
			&'a self,
			_generator: &'a dyn ProgramGenerator,
			name: &'a str,
			_shader_code: &'a str,
			format: &'a str,
			_domain: &'a str,
			_material: &'a JsonObject,
			_shader_json: &'a Value,
			stage: &'a str,
		) -> crate::r#async::BoxedFuture<'a, Result<(crate::resources::material::Shader, Box<[u8]>), ()>> {
			Box::pin(async move {

				assert_eq!(format, "besl");

				let stage = match stage {
					"Vertex" => crate::types::ShaderTypes::Vertex,
					"Fragment" => crate::types::ShaderTypes::Fragment,
					"Compute" => crate::types::ShaderTypes::Compute,
					_ => return Err(()),
				};

				Ok((
					crate::resources::material::Shader {
						id: name.to_string(),
						stage,
						interface: crate::resources::material::ShaderInterface {
							workgroup_size: Some((128, 0, 0)),
							bindings: vec![crate::resources::material::Binding::new(
								0,
								crate::resources::material::BindingKind::StorageBuffer,
								1,
								Some(4),
								true,
								false,
							)],
						},
						artifact: crate::resources::material::ShaderArtifact::Msl {
							entry_point: crate::shader::besl::backends::msl::MSL_ENTRY_POINT.to_string(),
						},
						source_hash: 42,
					},
					b"compiled-test-shader".to_vec().into_boxed_slice(),
				))
			})
		}
	}

	/// The `RootTestShaderGenerator` struct supplies the complete test renderer contract used by BEMA integration tests.
	pub struct RootTestShaderGenerator {}

	/// The `MinimalTestShaderGenerator` struct isolates importer tests from renderer-specific material shader contracts.
	pub struct MinimalTestShaderGenerator;

	impl ProgramGenerator for MinimalTestShaderGenerator {
		fn transform<'a>(&self, _: besl::parser::Node<'a>, _: &'a JsonObject) -> besl::parser::Node<'a> {
			besl::parser::Node::root_with_children(vec![besl::parser::Node::main_function(Vec::new())])
		}
	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
			let material_struct = besl::parser::Node::buffer("Material", vec![besl::parser::Node::member("color", "vec4f")]);

			let sample_function =
				besl::parser::Node::function("sample_", vec![besl::parser::Node::member("t", "u32")], "void", vec![]);

			let mid_test_shader_generator = MidTestShaderGenerator::new();

			root.add(vec![material_struct, sample_function]);

			mid_test_shader_generator.transform(root, material)
		}
	}

	pub struct MidTestShaderGenerator {}

	impl MidTestShaderGenerator {
		pub fn new() -> MidTestShaderGenerator {
			MidTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for MidTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
			let binding = besl::parser::Node::binding(
				"materials",
				besl::parser::Node::buffer("Materials", vec![besl::parser::Node::member("materials", "Material[16]")]),
				0,
				true,
				false,
			);

			let leaf_test_shader_generator = LeafTestShaderGenerator::new();

			root.add(vec![binding]);

			leaf_test_shader_generator.transform(root, material)
		}
	}

	struct LeafTestShaderGenerator {}

	impl LeafTestShaderGenerator {
		pub fn new() -> LeafTestShaderGenerator {
			LeafTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for LeafTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, _: &JsonObject) -> besl::parser::Node<'a> {
			let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("material_index", "u32")]);

			let main = besl::parser::Node::function(
				"main",
				vec![],
				"void",
				vec![besl::parser::Node::glsl(
					"push_constant;\nmaterials;\nsample_(0);\n",
					&["push_constant", "materials", "sample_"],
					&[],
				)],
			);

			root.add(vec![push_constant, main]);

			root
		}
	}

	#[r#async::test]
	async fn load_material() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			// Authored material files accept the JSON5 conveniences used by hand-written assets.
			domain: 'World',
				type: 'Surface',
				shaders: {
					Compute: 'load_material_fragment.besl',
				},
			variables: [
				{
					name: 'color',
					data_type: 'vec4f',
					type: 'Static',
					value: 'Purple',
				},
			],
		}"#;

		asset_storage_backend.add_file("load_material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("load_material_fragment.besl", shader_file.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		let mut asset_handler = BEMAAssetHandler::new();

		asset_handler.compiler = Arc::new(TestShaderCompiler);

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		asset_manager.add_asset_handler(asset_handler);

		asset_manager
			.bake("load_material.bema")
			.await
			.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = resource_storage_backend
			.get_resource(ResourceId::new("load_material_fragment.besl"))
			.expect("Expected shader");

		assert_eq!(shader.id, "load_material_fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("load_material_fragment.besl"))
			.expect("Expected shader data");

		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert_eq!(shader_spirv, "compiled-test-shader");

		let shader_model: crate::resources::material::Shader = crate::from_slice(&shader.resource).unwrap();

		assert_eq!(shader_model.id, "load_material_fragment.besl");
		assert!(matches!(shader_model.stage, crate::types::ShaderTypes::Compute));
		assert_eq!(shader_model.interface.workgroup_size, Some((128, 0, 0)));
		assert_eq!(shader_model.interface.bindings.len(), 1);
		assert_eq!(shader_model.source_hash, 42);
		assert!(matches!(
			shader_model.artifact,
			crate::resources::material::ShaderArtifact::Msl { ref entry_point }
				if entry_point == crate::shader::besl::backends::msl::MSL_ENTRY_POINT
		));

		let material = resource_storage_backend
			.get_resource(ResourceId::new("load_material.bema"))
			.expect("Expected material");

		assert_eq!(material.id, "load_material.bema");
		assert_eq!(material.class, "Material");
	}

	#[r#async::test]
	async fn load_variant() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			"domain": "World",
				"type": "Surface",
				"shaders": {
					"Compute": "load_variant_fragment.besl"
				},
			"variables": [
				{
					"name": "color",
					"data_type": "vec4f",
					"type": "Static",
					"value": "Purple"
				}
			]
		}"#;

		asset_storage_backend.add_file("load_variant_material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("load_variant_fragment.besl", shader_file.as_bytes());

		let variant_json = r#"{
			"parent": "load_variant_material.bema",
			"variables": [
				{
					"name": "color",
					"value": "White"
				}
			]
		}"#;

		asset_storage_backend.add_file("load_variant.bema", variant_json.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		let mut asset_handler = BEMAAssetHandler::new();

		asset_handler.compiler = Arc::new(TestShaderCompiler);

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		asset_manager.add_asset_handler(asset_handler);

		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("load_variant.bema")
			.await
			.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = resource_storage_backend
			.get_resource(ResourceId::new("load_variant_fragment.besl"))
			.expect("Expected shader");

		assert_eq!(shader.id, "load_variant_fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("load_variant_fragment.besl"))
			.expect("Expected shader data");

		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(!shader_spirv.is_empty());

		let material = resource_storage_backend
			.get_resource(ResourceId::new("load_variant_material.bema"))
			.expect("Expected material");

		assert_eq!(material.id, "load_variant_material.bema");
		assert_eq!(material.class, "Material");

		let variant = resource_storage_backend
			.get_resource(ResourceId::new("load_variant.bema"))
			.expect("Expected variant");

		assert_eq!(variant.id, "load_variant.bema");
		assert_eq!(variant.class, "Variant");
	}
}

use std::sync::Arc;

use serde_json::Value;
use utils::Extent;

use super::{
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	ResourceId,
};
use crate::shader::{
	artifact::finalize_platform_shader_artifact,
	besl::{
		backends::platform::{PlatformShaderGenerator, PlatformShaderLanguage},
		evaluation::ProgramEvaluation,
	},
};
use crate::{
	asset::{self, JsonObject},
	online_docs_url,
	r#async::spawn_cpu_task,
	resource,
	resources::material::{
		Binding, MaterialCoverage, MaterialModel, ParameterModel, RenderModel, Shader, ShaderInterface, ValueModel,
		VariantModel, VariantVariableModel,
	},
	shader::generator::ShaderGenerationSettings,
	types::{AlphaMode, ShaderTypes},
	ProcessedAsset, ReferenceModel,
};
