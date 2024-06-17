use std::{ops::Deref, sync::Arc};

use log::debug;
use utils::Extent;

use crate::{material::{MaterialModel, ParameterModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel}, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, types::{AlphaMode, ShaderTypes}, GenericResourceResponse, ProcessedAsset, ReferenceModel, StorageBackend};

use super::{asset_handler::AssetHandler, asset_manager::AssetManager};

pub trait ProgramGenerator: Send + Sync {
	/// Transforms a program.
	fn transform(&self, node: besl::parser::Node, material: &json::JsonValue) -> besl::parser::Node;
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

	fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: &'a str,) -> utils::SendSyncBoxedFuture<'a, Result<(), String>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if dt != "bema" { return Err("Not my type".to_string()); }
			}

			let (data, _, at) = storage_backend.resolve(url).await.or(Err("Failed to resolve asset".to_string()))?;

			if at != "bema" {
				return Err("Not my type".to_string());
			}

			let asset_json = json::parse(std::str::from_utf8(&data).or_else(|_| { Err("Failed to parse JSON") })?).or_else(|_| { Err("Failed to parse JSON") })?;

			let is_material = asset_json["parent"].is_null();

			let to_value = |t: &str, v: &str| {
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

				match t {
					"vec4f" => {
						let value = to_color(v);
						ValueModel::Vector4([value[0], value[1], value[2], value[3]])
					}
					"vec3f" => {
						let value = to_color(v);
						ValueModel::Vector3([value[0], value[1], value[2]])
					}
					"float" => ValueModel::Scalar(0f32),
					"Texture2D" => ValueModel::Image(smol::block_on(asset_manager.load(v)).unwrap()),
					_ => panic!("Unknown data type")
				}
			};

			let resource = if is_material {
				let material_domain = asset_json["domain"].as_str().ok_or("Domain not found".to_string()).or_else(|e| { debug!("{}", e); Err("Domain not found".to_string()) })?;
				
				let generator = self.generator.clone().ok_or("Generator not set".to_string())?;

				let shaders = asset_json["shaders"].entries().map(|(s_type, shader_json)| {
					smol::block_on(transform_shader(generator.deref(), storage_backend, &material_domain, &asset_json, &shader_json, s_type))
				}).collect::<Option<Vec<_>>>().ok_or("Failed to build shader(s)".to_string())?;

				let parameters = asset_json["variables"].members().map(|v: &json::JsonValue| {
					let name = v["name"].to_string();
					let data_type = v["data_type"].to_string();
					let value = v["value"].to_string();

					ParameterModel {
						name,
						r#type: data_type.clone(),
						value: to_value(&data_type, &value),
					}
				}).collect::<Vec<_>>();

				let resource = ProcessedAsset::new(url, MaterialModel {
					double_sided: false,
					alpha_mode: AlphaMode::Opaque,
					model: RenderModel {
						name: "Visibility".to_string(),
						pass: "MaterialEvaluation".to_string(),
					},
					shaders: shaders.into_iter().map(|(s, _)| s).collect(),
					parameters,
				});

				storage_backend.store(&resource, &data).await;

				resource
			} else {
				let variant_json = asset_json;

				let parent_material_url = variant_json["parent"].as_str().unwrap();

				let material = asset_manager.load(parent_material_url).await.or_else(|_| { Err("Failed to load parent material") })?;

				let variables = material.resource.as_document().unwrap().get_array("parameters").unwrap().iter().map(|v| {
					let v = v.as_document().unwrap();
					let name = v.get_str("name").unwrap().to_string();
					let r#type = v.get_str("type").unwrap().to_string();
					let value = variant_json["variables"].members().find(|v2| { v2["name"].to_string() == name }).unwrap()["value"].to_string();

					VariantVariableModel {
						value: to_value(&r#type, &value),
						name,
						r#type,
					}
				}).collect::<Vec<_>>();

				let resource = ProcessedAsset::new(url, VariantModel {
					material,
					variables,
				});

				match storage_backend.store(&resource, &[]).await {
					Ok(_) => {}
					Err(_) => {
						log::error!("Failed to store resource {}", url);
					}
				}

				resource
			};

			Ok(())
		})
	}
}

async fn transform_shader(generator: &dyn ProgramGenerator, storage_backend: &dyn StorageBackend, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Option<(ReferenceModel<Shader>, Box<[u8]>)> {
	let path = shader_json.as_str()?;
	let (arlp, _, format) = storage_backend.resolve(&path).await.ok()?;

	let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

	let root_node = if format == "glsl" {
		// besl::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		panic!()
	} else if format == "besl" {
		if let Ok(e) = besl::parse(&shader_code,/*Some(parent_scope.clone())*/) {
			e
		} else {
			log::error!("Error compiling shader");
			return None;
		}
	} else {
		log::error!("Unknown shader format");
		return None;
	};

	let mut root = generator.transform(root_node, material);

	let root_node = match besl::lex(root) {
		Ok(e) => e,
		Err(e) => {
			log::error!("Error compiling shader: {:#?}", e);
			return None;
		}
	};

	let main_node = root_node.borrow().get_main()?;

	let settings = match stage {
		"Vertex" => ShaderGenerationSettings::vertex(),
		"Fragment" => ShaderGenerationSettings::fragment(),
		"Compute" => ShaderGenerationSettings::compute(Extent::square(32)),
		_ => { panic!("Invalid shader stage") }
	};

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&settings, &main_node);

	let compiler = shaderc::Compiler::new().unwrap();
	let mut options = shaderc::CompileOptions::new().unwrap();

	options.set_optimization_level(shaderc::OptimizationLevel::Performance);
	options.set_target_env(shaderc::TargetEnv::Vulkan, (1 << 22) | (3 << 12));

	if cfg!(debug_assertions) {
		options.set_generate_debug_info();
	}

	options.set_target_spirv(shaderc::SpirvVersion::V1_6);
	options.set_invert_y(true);

	let binary = compiler.compile_into_spirv(&glsl, shaderc::ShaderKind::InferFromSource, path, "main", Some(&options));

	// TODO: if shader fails to compile try to generate a failsafe shader

	let compilation_artifact = match binary {
		Ok(binary) => { binary }
		Err(err) => {
			let error_string = err.to_string();
			log::debug!("{}", &glsl);
			log::error!("Error compiling shader:\n{}", error_string);
			let error_string = ghi::shader_compilation::format_glslang_error(path, &error_string, &glsl).unwrap_or(error_string);
			log::error!("Error compiling shader:\n{}", error_string);
			if cfg!(test) {
				println!("{}", error_string);
			}
			return None;
		}
	};

	if compilation_artifact.get_num_warnings() > 0 {
		log::warn!("Shader warnings: {}", compilation_artifact.get_warning_messages());
	}

	let result_shader_bytes = compilation_artifact.as_binary_u8();

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		_ => { panic!("Invalid shader stage") }
	};

	let shader = Shader {
		id: path.to_string(),
		stage,
	};

	let r = storage_backend.store(&ProcessedAsset::new(path, shader.clone()), result_shader_bytes).await.ok()?;

	Some((r.into(), result_shader_bytes.into()))
}

#[cfg(test)]
pub mod tests {
	use super::{MaterialAssetHandler, ProgramGenerator};
	use crate::{asset::{asset_handler::AssetHandler, asset_manager::AssetManager,}, material::VariantModel, ReferenceModel};

	pub struct RootTestShaderGenerator {

	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform(&self, mut root: besl::parser::Node, material: &json::JsonValue) -> besl::parser::Node {
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
		fn transform(&self, mut root: besl::parser::Node, material: &json::JsonValue) -> besl::parser::Node {
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
		fn transform(&self, mut root: besl::parser::Node, _: &json::JsonValue) -> besl::parser::Node {
			let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("material_index", "u32")]);

			let main = besl::parser::Node::function("main", vec![], "void", vec![besl::parser::Node::glsl("push_constant;\nmaterials;\nsample_(0);\n", &["push_constant", "materials", "sample_"], Vec::new())]);

			root.add(vec![push_constant, main]);

			root
		}
	}

	#[test]
	fn generate_program() {
		let test_shader_generator = RootTestShaderGenerator::new();

		let root = test_shader_generator.transform(besl::parser::Node::root(), &json::object! {});

		let root = besl::lex(root).unwrap();

		let main_node = root.borrow().get_main().unwrap();

		let glsl = crate::shader_generation::ShaderGenerator::new().minified(true).compilation().generate_glsl_shader(&crate::shader_generation::ShaderGenerationSettings::fragment(), &main_node);

		dbg!(&glsl);

		assert!(glsl.contains("layout(push_constant"));
		assert!(glsl.contains("uint32_t material_index"));
		assert!(glsl.contains("materials[16]"));
		assert!(glsl.contains("void sample_("));
	}

	#[test]
	fn load_material() {
		let asset_manager = AssetManager::new("../assets".into(),);
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

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

		let storage_backend = asset_manager.get_test_storage_backend();

		storage_backend.add_file("material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		storage_backend.add_file("fragment.besl", shader_file.as_bytes());

		smol::block_on(asset_handler.load(&asset_manager, storage_backend, "material.bema",)).expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar)"));
		assert!(shader_spirv.contains("void main()"));

		let material = &generated_resources[1];

		assert_eq!(material.id, "material.bema");
		assert_eq!(material.class, "Material");
	}

	#[test]
	fn load_variant() {
		let mut asset_manager = AssetManager::new("../assets".into());
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

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

		{
			let storage_backend = asset_manager.get_test_storage_backend();
	
			storage_backend.add_file("material.bema", material_json.as_bytes());
	
			let shader_file = "main: fn () -> void {
				materials;
			}";
	
			storage_backend.add_file("fragment.besl", shader_file.as_bytes());
	
			let variant_json = r#"{
				"parent": "material.bema",
				"variables": [
					{
						"name": "color",
						"value": "White"
					}
				]
			}"#;

			storage_backend.add_file("variant.bema", variant_json.as_bytes());
		}

		asset_manager.add_asset_handler(asset_handler);

		let storage_backend = asset_manager.get_test_storage_backend();

		let variant: ReferenceModel<VariantModel> = smol::block_on(asset_manager.load("variant.bema")).expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar)"));
		assert!(shader_spirv.contains("void main()"));

		let material = &generated_resources[1];
		
		assert_eq!(material.id, "material.bema");
		assert_eq!(material.class, "Material");

		let variant = &generated_resources[2];

		assert_eq!(variant.id, "variant.bema");
		assert_eq!(variant.class, "Variant");
	}
}