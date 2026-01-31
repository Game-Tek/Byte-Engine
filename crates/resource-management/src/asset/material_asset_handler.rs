use std::sync::Arc;

use log::debug;
use utils::{json::{self, JsonContainerTrait, JsonValueTrait}, Extent};

use crate::{ProcessedAsset, ReferenceModel, asset, r#async::{BoxedFuture, spawn_cpu_task}, resource, resources::material::{Binding, MaterialModel, ParameterModel, RenderModel, Shader, ShaderInterface, ValueModel, VariantModel, VariantVariableModel}, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator, types::{AlphaMode, ShaderTypes}};

use super::{asset_handler::{Asset, AssetHandler, LoadErrors}, asset_manager::AssetManager, ResourceId};

pub trait ProgramGenerator: Send + Sync {
	/// Transforms a program.
	fn transform<'a>(&self, node: besl::parser::Node<'a>, material: &'a json::Object) -> besl::parser::Node<'a>;
}

struct MaterialAsset {
    id: String,
    asset: json::Object,
    generator: Arc<dyn ProgramGenerator>,
}

impl Asset for MaterialAsset {
    fn requested_assets(&self) -> Vec<String> {
        let asset = &self.asset;
        let is_material = asset.get(&"parent").is_none();
        if is_material {
            asset["variables"].as_array().unwrap().iter().filter_map(|v|
                if v["data_type"] == "Texture2D" {
                    Some(v["value"].as_str().unwrap().to_string())
                } else {
                    None
                }
            ).collect()
        } else {
            Vec::new()
        }
    }

    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), String>> {
		Box::pin(async move {
			let asset = &self.asset;

			let is_material = asset.get(&"parent").is_none();

			if is_material {
				let material_domain = asset["domain"].as_str().ok_or("Domain not found. The material JSON is missing a domain field.".to_string()).or_else(|e| { debug!("{}", e); Err("Domain not found. The material JSON is missing a domain field.".to_string()) })?;

				let generator = self.generator.clone();

				let asset_shaders = match asset["shaders"].as_object() {
					Some(v) => v,
					None => { return Err("Shaders not found. The material JSON is missing shader definitions.".to_string()); }
				};

				let mut shaders = Vec::with_capacity(asset_shaders.len());
				for (s_type, shader_json) in asset_shaders.iter() {
					let shader = transform_shader(generator.clone(), storage_backend, asset_storage_backend, &material_domain, &asset, shader_json, s_type).await?;
					shaders.push(shader);
				}

				let asset_variables = match asset["variables"].as_array() {
					Some(v) => v,
					None => { return Err("Variables not found. The material JSON is missing variable definitions.".to_string()); }
				};

				let mut values = Vec::with_capacity(asset_variables.len());
				for v in asset_variables.iter() {
					let data_type = v["data_type"].as_str().unwrap().to_string();
					let value = v["value"].as_str().unwrap().to_string();

					let value = resolve_value(asset_manager, storage_backend, &data_type, &value).await?;
					values.push(value);
				}

				let parameters = asset_variables.iter().zip(values.into_iter()).map(|(v, value)| {
					let name = v["name"].as_str().unwrap().to_string();
					let data_type = v["data_type"].as_str().unwrap().to_string();

					ParameterModel {
						name,
						r#type: data_type.clone(),
						value,
					}
				}).collect();

				let resource = MaterialModel {
					double_sided: false,
					alpha_mode: AlphaMode::Opaque,
					model: RenderModel {
						name: "Visibility".to_string(),
						pass: "MaterialEvaluation".to_string(),
					},
					shaders: shaders.into_iter().map(|(s, _)| s).collect(),
					parameters,
				};

				let resource = ProcessedAsset::new(url, resource);

				storage_backend.store(&resource, &[]).or_else(|_| { Err("Failed to store material resource. The storage backend likely rejected the write.".to_string()) })?;
			} else {
				let parent_material_url = asset["parent"].as_str().unwrap();

				let material = asset_manager.load(parent_material_url, storage_backend).await.or_else(|_| { Err("Failed to load parent material. The referenced material asset could not be loaded.".to_string()) })?;

				let material_repr: MaterialModel = pot::from_slice(&material.resource).unwrap();

				let mut values = Vec::with_capacity(material_repr.parameters.len());
				for v in material_repr.parameters.iter() {
					let value = match asset["variables"].as_array() {
						Some(variables) => {
							match variables.iter().find(|v2| { v2["name"].as_str().unwrap() == v.name }) {
								Some(v) => v["value"].as_str().unwrap().to_string(),
								None => { return Err("Variable not found. The variant JSON is missing an override.".to_string()); }
							}
						}
						None => { return Err("Variable not found. The variant JSON is missing overrides.".to_string()); }
					};

					let resolved = resolve_value(asset_manager, storage_backend, &v.r#type, &value).await?;
					values.push(resolved);
				}

				let variables = material_repr.parameters.iter().zip(values.into_iter()).map(|(v, value)| {
					VariantVariableModel {
						value,
						name: v.name.clone(),
						r#type: v.r#type.clone(),
					}
				}).collect();

				let alpha_mode = match asset.get(&"transparency").map(|e| e.as_ref()) {
					Some(json::ValueRef::Bool(v)) => {
						if v { AlphaMode::Blend } else { AlphaMode::Opaque }
					}
					Some(json::ValueRef::String(s)) => {
						match s {
							"Opaque" => AlphaMode::Opaque,
							"Blend" => AlphaMode::Blend,
							_ => AlphaMode::Opaque
						}
					}
					_ => { AlphaMode::Opaque }
				};

				let resource = ProcessedAsset::new(url, VariantModel {
					material,
					variables,
					alpha_mode,
				});

				if let Err(_) = storage_backend.store(&resource, &[]) {
					log::error!("Failed to store resource {:#?}", url);
				}
			};

			Ok(())
		})
    }
}

pub struct MaterialAssetHandler {
	generator: Option<Arc<dyn ProgramGenerator>>,
}

impl MaterialAssetHandler {
	pub fn new() -> MaterialAssetHandler {
		MaterialAssetHandler {
			generator: None,
		}
	}

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
    }
}

impl AssetHandler for MaterialAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "bema"
	}

	fn load<'a>(&'a self, _: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>,) -> BoxedFuture<'a, Result<Box<dyn Asset>, LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if dt != "bema" { return Err(LoadErrors::UnsupportedType); }
			}

			let (data, _, at) = asset_storage_backend.resolve(url).await.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			if at != "bema" {
				return Err(LoadErrors::UnsupportedType);
			}

			let asset_json = json::from_str(std::str::from_utf8(&data).or_else(|_| { Err(LoadErrors::FailedToProcess) })?).or_else(|_| { Err(LoadErrors::FailedToProcess) })?;

			Ok(Box::new(MaterialAsset {
				id: url.to_string(),
				asset: asset_json,
				generator: self.generator.clone().ok_or(LoadErrors::FailedToProcess)?,
			}) as Box<dyn Asset>)
		})
	}
}

/// Converts a shader source into a compiled shader and binary payload.
fn compile_shader(generator: &dyn ProgramGenerator, name: &str, shader_code: &str, format: &str, domain: &str, material: &json::Object, shader_json: &json::Value, stage: &str) -> Result<(Shader, Box<[u8]>), ()> {
	let root_node = if format == "glsl" {
		// besl::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		panic!()
	} else if format == "besl" {
		if let Ok(e) = besl::parse(&shader_code) {
			e
		} else {
			log::error!("Error compiling shader");
			return Err(());
		}
	} else {
		log::error!("Unknown shader format");
		return Err(());
	};

	let root = generator.transform(root_node, material);

	let root_node = match besl::lex(root) {
		Ok(e) => e,
		Err(e) => {
			log::error!("Error compiling shader: {:#?}", e);
			return Err(());
		}
	};

	let main_node = root_node.get_main().ok_or(())?;

	let settings = match stage {
		"Vertex" => ShaderGenerationSettings::vertex(),
		"Fragment" => ShaderGenerationSettings::fragment(),
		"Compute" => ShaderGenerationSettings::compute(Extent::line(128)),
		_ => { panic!("Invalid shader stage") }
	};

	let shader_program = SPIRVShaderGenerator::new().generate(&settings, &main_node).map_err(|e| {
		log::error!("Error compiling shader: {:#?}", e);
	})?;

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		_ => { panic!("Invalid shader stage") }
	};

	let interface = ShaderInterface {
		workgroup_size: shader_program.extent().map(|e| (e.width(), e.height(), e.depth())),
		bindings: shader_program.bindings().iter().map(|b| Binding::new(b.set, b.binding, b.read, b.write)).collect(),
	};

	let shader = Shader {
		id: name.to_string(),
		stage,
		interface,
	};

	Ok((shader, shader_program.into_binary()))
}

/// Loads and compiles a shader definition into a stored resource.
async fn transform_shader(generator: Arc<dyn ProgramGenerator>, storage_backend: &dyn resource::StorageBackend, asset_storage_backend: &dyn asset::StorageBackend, domain: &str, material: &json::Object, shader_json: &json::Value, stage: &str) -> Result<(ReferenceModel<Shader>, Box<[u8]>), String> {
	let path = shader_json.as_str().ok_or("Invalid shader path. The shader entry is missing a file path.".to_string())?;
	let path = ResourceId::new(path);
	let (arlp, _, format) = asset_storage_backend.resolve(path).await.or(Err("Failed to load shader source. The shader file is missing or unreadable.".to_string()))?;

	let shader_code = std::str::from_utf8(&arlp).map_err(|_| "Failed to decode shader source. The shader file is not valid UTF-8.".to_string())?.to_string();

	let material = material.clone();
	let domain = domain.to_string();
	let stage = stage.to_string();
	let format = format.to_string();
	let name = path.get_base().as_ref().to_string();
	let shader_json = shader_json.clone();

	let (shader, result_shader_bytes) = spawn_cpu_task(move || {
		compile_shader(generator.as_ref(), &name, &shader_code, &format, &domain, &material, &shader_json, &stage)
	}).await.map_err(|_| "Failed to compile shader. The compilation task was cancelled.".to_string())?
		.map_err(|_| "Failed to compile shader. The shader source likely contains errors.".to_string())?;

	let r = storage_backend.store(&ProcessedAsset::new(path, shader), &result_shader_bytes).or(Err("Failed to store shader resource. The storage backend likely rejected the write.".to_string()))?;

	Ok((r.into(), result_shader_bytes))
}

/// Resolves a material parameter value based on its type.
async fn resolve_value(asset_manager: &AssetManager, storage_backend: &dyn resource::StorageBackend, data_type: &str, value: &str) -> Result<ValueModel, String> {
	let to_color = |name: &str| {
		match name {
			"Red" => [1f32, 0f32, 0f32, 1f32],
			"Green" => [0f32, 1f32, 0f32, 1f32],
			"Blue" => [0f32, 0f32, 1f32, 1f32],
			"Purple" => [1f32, 0f32, 1f32, 1f32],
			"White" => [1f32, 1f32, 1f32, 1f32],
			"Black" => [0f32, 0f32, 0f32, 1f32],
			_ => [1f32, 0f32, 1f32, 1f32]
		}
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
			let image = asset_manager
				.load(value, storage_backend)
				.await
				.map_err(|_| "Failed to load texture value. The referenced texture asset could not be loaded.".to_string())?;
			Ok(ValueModel::Image(image))
		}
		_ => Err("Unknown data type. The material variable type is unsupported.".to_string())
	}
}


#[cfg(test)]
pub mod tests {
	use utils::json;

	use crate::{ReferenceModel, asset::{ResourceId, asset_handler::AssetHandler, asset_manager::AssetManager, material_asset_handler::MaterialAssetHandler, storage_backend::tests::TestStorageBackend as AssetTestStorageBackend}, r#async, glsl_shader_generator::GLSLShaderGenerator, resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend, resources::material::VariantModel, shader_generator::ShaderGenerationSettings};

	use super::ProgramGenerator;

	pub struct RootTestShaderGenerator {

	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a json::Object) -> besl::parser::Node<'a> {
			let material_struct = besl::parser::Node::buffer("Material", vec![besl::parser::Node::member("color", "vec4f")]);

			let sample_function = besl::parser::Node::function("sample_", vec![besl::parser::Node::member("t", "u32")], "void", vec![]);

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
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a json::Object) -> besl::parser::Node<'a> {
			let binding = besl::parser::Node::binding("materials", besl::parser::Node::buffer("Materials", vec![besl::parser::Node::member("materials", "Material[16]")]), 0, 0, true, false);

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
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, _: &json::Object) -> besl::parser::Node<'a> {
			let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("material_index", "u32")]);

			let main = besl::parser::Node::function("main", vec![], "void", vec![besl::parser::Node::glsl("push_constant;\nmaterials;\nsample_(0);\n", &["push_constant", "materials", "sample_"], &[])]);

			root.add(vec![push_constant, main]);

			root
		}
	}

	#[test]
	fn generate_program() {
		let test_shader_generator = RootTestShaderGenerator::new();

		let object = json::object! {};

		let root = test_shader_generator.transform(besl::parser::Node::root(), &object);

		let root = besl::lex(root).unwrap();

		let main_node = root.get_main().unwrap();

		let glsl = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::fragment(), &main_node).expect("Failed to generate GLSL");

		assert!(glsl.contains("layout(push_constant"));
		assert!(glsl.contains("uint32_t material_index"));
		assert!(glsl.contains("materials[16]"));
		assert!(glsl.contains("void sample_("));
	}

	#[r#async::test]
	async fn load_material() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "fragment.besl"
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

		asset_storage_backend.add_file("material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("fragment.besl", shader_file.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let asset_manager = AssetManager::new(asset_storage_backend);
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		let asset = asset_handler.load(&asset_manager, &resource_storage_backend, asset_manager.get_storage_backend(), ResourceId::new("material.bema"),).await.expect("Failed to load material");

		let _ = asset.load(&asset_manager, &resource_storage_backend, asset_manager.get_storage_backend(), ResourceId::new("material.bema")).await;

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = resource_storage_backend.get_resource(ResourceId::new("fragment.besl")).expect("Expected shader");

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend.get_resource_data_by_name(ResourceId::new("fragment.besl")).expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar)"));
		assert!(shader_spirv.contains("void main()"));

		let material = resource_storage_backend.get_resource(ResourceId::new("material.bema")).expect("Expected material");

		assert_eq!(material.id, "material.bema");
		assert_eq!(material.class, "Material");
	}

	#[r#async::test]
	async fn load_variant() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "fragment.besl"
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

		asset_storage_backend.add_file("material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("fragment.besl", shader_file.as_bytes());

		let variant_json = r#"{
			"parent": "material.bema",
			"variables": [
				{
					"name": "color",
					"value": "White"
				}
			]
		}"#;

		asset_storage_backend.add_file("variant.bema", variant_json.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend);
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		asset_manager.add_asset_handler(asset_handler);

		let _: ReferenceModel<VariantModel> = asset_manager.load("variant.bema", &resource_storage_backend).await.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();
		assert_eq!(generated_resources.len(), 3);

		let shader = resource_storage_backend.get_resource(ResourceId::new("fragment.besl")).expect("Expected shader");

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend.get_resource_data_by_name(ResourceId::new("fragment.besl")).expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		dbg!(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar)"));
		assert!(shader_spirv.contains("void main()"));

		let material = resource_storage_backend.get_resource(ResourceId::new("material.bema")).expect("Expected material");

		assert_eq!(material.id, "material.bema");
		assert_eq!(material.class, "Material");

		let variant = resource_storage_backend.get_resource(ResourceId::new("variant.bema")).expect("Expected variant");

		assert_eq!(variant.id, "variant.bema");
		assert_eq!(variant.class, "Variant");
	}
}
