use std::{ops::Deref, sync::Arc};

use futures::future::{join, join_all, try_join_all};
use log::debug;
use polodb_core::bson::Bson;
use utils::Extent;

use crate::{material::{MaterialModel, ParameterModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel}, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, types::{AlphaMode, ShaderTypes}, GenericResourceResponse, ProcessedAsset, ReferenceModel, StorageBackend};

use super::{asset_handler::{Asset, AssetHandler, LoadErrors}, asset_manager::AssetManager, ResourceId};

pub trait ProgramGenerator: Send + Sync {
	/// Transforms a program.
	fn transform(&self, node: besl::parser::Node, material: &json::JsonValue) -> besl::parser::Node;
}

struct MaterialAsset {
    id: String,
    asset: json::JsonValue,
    generator: Arc<dyn ProgramGenerator>,
}

impl Asset for MaterialAsset {
    fn requested_assets(&self) -> Vec<String> {
        let asset = &self.asset;
        let is_material = asset["parent"].is_null();
        if is_material {
            asset["variables"].members().filter_map(|v|
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

    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: ResourceId<'a>) -> utils::SendBoxedFuture<Result<(), String>> { Box::pin(async move {
        let asset = &self.asset;

        let is_material = asset["parent"].is_null();

		let to_value = async |t: String, v: String| {
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

			let t = t.as_str();
			let v = v.as_str();

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
				"Texture2D" => ValueModel::Image(asset_manager.load(v).await.unwrap()),
				_ => panic!("Unknown data type")
			}
		};

		let resource = if is_material {
			let material_domain = asset["domain"].as_str().ok_or("Domain not found".to_string()).or_else(|e| { debug!("{}", e); Err("Domain not found".to_string()) })?;

			let generator = self.generator.as_ref();

			let generator = generator;

			let shaders = try_join_all(asset["shaders"].entries().map(|(s_type, shader_json): (&str, &json::JsonValue)| {
				transform_shader(generator, storage_backend, &material_domain, &asset, &shader_json, s_type)
			}));

			let values = join_all(asset["variables"].members().map(|v: &json::JsonValue| {
				let data_type = v["data_type"].to_string();
				let value = v["value"].to_string();

				to_value(data_type, value)
			}));

			let (shaders, values) = join(shaders, values).await;

			let parameters = asset["variables"].members().zip(values.into_iter()).map(|(v, value)| {
				let name = v["name"].to_string();
				let data_type = v["data_type"].to_string();

				ParameterModel {
					name,
					r#type: data_type.clone(),
					value,
				}
			}).collect();

			let shaders = shaders.or(Err("Failed to build shader(s)".to_string()))?;

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

			storage_backend.store(&resource, &[]).await;

			resource
		} else {
			let parent_material_url = asset["parent"].as_str().unwrap();

			let material = asset_manager.load(parent_material_url).await.or_else(|_| { Err("Failed to load parent material") })?;

			let values = join_all(material.resource.as_document().unwrap().get_array("parameters").unwrap().iter().map(|v: &Bson| {
				let v = v.as_document().unwrap();
				let name = v.get_str("name").unwrap().to_string();
				let r#type = v.get_str("type").unwrap().to_string();
				let value = asset["variables"].members().find(|v2| { v2["name"].to_string() == name }).unwrap()["value"].to_string();

				to_value(r#type, value)
			})).await;

			let variables = material.resource.as_document().unwrap().get_array("parameters").unwrap().iter().zip(values.into_iter()).map(|(v, value)| {
				let v = v.as_document().unwrap();
				let name = v.get_str("name").unwrap().to_string();
				let r#type = v.get_str("type").unwrap().to_string();

				VariantVariableModel {
					value,
					name,
					r#type,
				}
			}).collect();

			let alpha_mode = match &asset["transparency"] {
				json::JsonValue::Boolean(v) => {
					if *v { AlphaMode::Blend } else { AlphaMode::Opaque }
				}
				json::JsonValue::String(s) => {
					match s.as_str() {
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

			match storage_backend.store(&resource, &[]).await {
				Ok(_) => {}
				Err(_) => {
					log::error!("Failed to store resource {:#?}", url);
				}
			}

			resource
		};

		Ok(())
    }) }
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

	fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: ResourceId<'a>,) -> utils::SendBoxedFuture<'a, Result<Box<dyn Asset>, LoadErrors>> { Box::pin(async move {
		if let Some(dt) = storage_backend.get_type(url) {
			if dt != "bema" { return Err(LoadErrors::UnsupportedType); }
		}

		let (data, _, at) = storage_backend.resolve(url).await.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

		if at != "bema" {
			return Err(LoadErrors::UnsupportedType);
		}

		let asset_json = json::parse(std::str::from_utf8(&data).or_else(|_| { Err(LoadErrors::FailedToProcess) })?).or_else(|_| { Err(LoadErrors::FailedToProcess) })?;

		Ok(Box::new(MaterialAsset {
		    id: url.to_string(),
			asset: asset_json,
			generator: self.generator.clone().ok_or(LoadErrors::FailedToProcess)?,
		}) as Box<dyn Asset>)
	}) }
}

fn compile_shader(generator: &dyn ProgramGenerator, name: &str, shader_code: &str, format: &str, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Result<(Shader, Box<[u8]>), ()> {
	let root_node = if format == "glsl" {
		// besl::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		panic!()
	} else if format == "besl" {
		if let Ok(e) = besl::parse(&shader_code,/*Some(parent_scope.clone())*/) {
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

	let main_node = root_node.borrow().get_main().ok_or(())?;

	let settings = match stage {
		"Vertex" => ShaderGenerationSettings::vertex(),
		"Fragment" => ShaderGenerationSettings::fragment(),
		"Compute" => ShaderGenerationSettings::compute(Extent::line(128)),
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

	let binary = compiler.compile_into_spirv(&glsl, shaderc::ShaderKind::InferFromSource, name, "main", Some(&options));

	// TODO: if shader fails to compile try to generate a failsafe shader

	let compilation_artifact = match binary {
		Ok(binary) => { binary }
		Err(err) => {
			let error_string = err.to_string();
			log::debug!("{}", &glsl);
			log::error!("Error compiling shader:\n{}", error_string);
			let error_string = besl::glsl::format_glslang_error(name, &error_string, &glsl).unwrap_or(error_string);
			log::error!("Error compiling shader:\n{}", error_string);
			if cfg!(test) {
				println!("{}", error_string);
			}
			return Err(());
		}
	};

	if compilation_artifact.get_num_warnings() > 0 {
		log::warn!("Shader warnings: {}", compilation_artifact.get_warning_messages());
	}

	let result_shader_bytes: Box<[u8]> = Box::from(compilation_artifact.as_binary_u8());

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		_ => { panic!("Invalid shader stage") }
	};

	let shader = Shader {
		id: name.to_string(),
		stage,
	};

	Ok((shader, result_shader_bytes))
}

async fn transform_shader(generator: &dyn ProgramGenerator, storage_backend: &dyn StorageBackend, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Result<(ReferenceModel<Shader>, Box<[u8]>), ()> {
	let path = shader_json.as_str().ok_or(())?;
	let path = ResourceId::new(path);
	let (arlp, _, format) = storage_backend.resolve(path).await.or(Err(()))?;

	let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

	let (shader, result_shader_bytes) = compile_shader(generator, path.get_base().as_ref(), &shader_code, &format, domain, material, shader_json, stage).or(Err(()))?;

	let r = storage_backend.store(&ProcessedAsset::new(path, shader), &result_shader_bytes).await.or(Err(()))?;

	Ok((r.into(), result_shader_bytes))
}


#[cfg(test)]
pub mod tests {
	use utils::r#async::block_on;

	use super::{MaterialAssetHandler, ProgramGenerator};
	use crate::{asset::{asset_handler::AssetHandler, asset_manager::AssetManager, ResourceId,}, material::VariantModel, ReferenceModel};

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

		block_on(asset_handler.load(&asset_manager, storage_backend, ResourceId::new("material.bema"),)).expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = storage_backend.get_resource_data_by_name(ResourceId::new("fragment.besl")).expect("Expected shader data");
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

		let variant: ReferenceModel<VariantModel> = block_on(asset_manager.load("variant.bema")).expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = storage_backend.get_resource_data_by_name(ResourceId::new("fragment.besl")).expect("Expected shader data");
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
