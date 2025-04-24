use std::sync::Arc;

use log::debug;
use utils::{json::{self, JsonContainerTrait, JsonValueTrait}, Extent};

use crate::{asset, resources::material::{Binding, MaterialModel, ParameterModel, RenderModel, Shader, ShaderInterface, ValueModel, VariantModel, VariantVariableModel}, resource, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator, types::{AlphaMode, ShaderTypes}, ProcessedAsset, ReferenceModel};

use super::{asset_handler::{Asset, AssetHandler, LoadErrors}, asset_manager::AssetManager, ResourceId};

pub trait ProgramGenerator: Send + Sync {
	/// Transforms a program.
	fn transform(&self, node: besl::parser::Node, material: &json::Object) -> besl::parser::Node;
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

    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>) -> Result<(), String> {
        let asset = &self.asset;

        let is_material = asset.get(&"parent").is_none();

		let to_value = |t: String, v: String| {
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
				"Texture2D" => ValueModel::Image(asset_manager.load(v, storage_backend).unwrap()),
				_ => panic!("Unknown data type")
			}
		};

		if is_material {
			let material_domain = asset["domain"].as_str().ok_or("Domain not found".to_string()).or_else(|e| { debug!("{}", e); Err("Domain not found".to_string()) })?;

			let generator = self.generator.as_ref();

			let generator = generator;

			let asset_shaders = match asset["shaders"].as_object() {
				Some(v) => v,
				None => { return Err("Shaders not found".to_string()); }
			};

			let shaders = asset_shaders.iter().map(|(s_type, shader_json): (&str, &json::Value)| {
				transform_shader(generator, storage_backend, asset_storage_backend, &material_domain, &asset, &shader_json, s_type)
			}).collect::<Result<Vec<_>, _>>().or(Err("Failed to build shader(s)".to_string()))?;

			let asset_variables = match asset["variables"].as_array() {
				Some(v) => v,
				None => { return Err("Variables not found".to_string()); }
			};

			let values = asset_variables.iter().map(|v: &json::Value| {
				let data_type = v["data_type"].as_str().unwrap().to_string();
				let value = v["value"].as_str().unwrap().to_string();

				to_value(data_type, value)
			});

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

			storage_backend.store(&resource, &[]).or_else(|_| { Err("Failed to store resource".to_string()) })?;

			resource
		} else {
			let parent_material_url = asset["parent"].as_str().unwrap();

			let material = asset_manager.load(parent_material_url, storage_backend).or_else(|_| { Err("Failed to load parent material") })?;

			let material_repr: MaterialModel = pot::from_slice(&material.resource).unwrap();

			let values = material_repr.parameters.iter().map(|v: &ParameterModel| {
				let value = match asset["variables"].as_array() {
					Some(variables) => {
						match variables.iter().find(|v2| { v2["name"].as_str().unwrap() == v.name }) {
							Some(v) => v["value"].as_str().unwrap().to_string(),
							None => { return Err("Variable not found".to_string()); }
						}
					}
					None => { return Err("Variable not found".to_string()); }
				};

				Ok(to_value(v.r#type.clone(), value))
			}).collect::<Result<Vec<_>, _>>()?;

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

			match storage_backend.store(&resource, &[]) {
				Ok(_) => {}
				Err(_) => {
					log::error!("Failed to store resource {:#?}", url);
				}
			}

			resource
		};

		Ok(())
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

	fn load<'a>(&'a self, _: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>,) -> Result<Box<dyn Asset>, LoadErrors> {
		if let Some(dt) = storage_backend.get_type(url) {
			if dt != "bema" { return Err(LoadErrors::UnsupportedType); }
		}

		let (data, _, at) = asset_storage_backend.resolve(url).or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

		if at != "bema" {
			return Err(LoadErrors::UnsupportedType);
		}

		let asset_json = json::from_str(std::str::from_utf8(&data).or_else(|_| { Err(LoadErrors::FailedToProcess) })?).or_else(|_| { Err(LoadErrors::FailedToProcess) })?;

		Ok(Box::new(MaterialAsset {
		    id: url.to_string(),
			asset: asset_json,
			generator: self.generator.clone().ok_or(LoadErrors::FailedToProcess)?,
		}) as Box<dyn Asset>)
	}
}

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

	let main_node = root_node.borrow().get_main().ok_or(())?;

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

fn transform_shader(generator: &dyn ProgramGenerator, storage_backend: &dyn resource::StorageBackend, asset_storage_backend: &dyn asset::StorageBackend, domain: &str, material: &json::Object, shader_json: &json::Value, stage: &str) -> Result<(ReferenceModel<Shader>, Box<[u8]>), ()> {
	let path = shader_json.as_str().ok_or(())?;
	let path = ResourceId::new(path);
	let (arlp, _, format) = asset_storage_backend.resolve(path).or(Err(()))?;

	let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

	let (shader, result_shader_bytes) = compile_shader(generator, path.get_base().as_ref(), &shader_code, &format, domain, material, shader_json, stage).or(Err(()))?;

	let r = storage_backend.store(&ProcessedAsset::new(path, shader), &result_shader_bytes).or(Err(()))?;

	Ok((r.into(), result_shader_bytes))
}


#[cfg(test)]
pub mod tests {
	use utils::json;

	use crate::{asset::{storage_backend::tests::TestStorageBackend as AssetTestStorageBackend, asset_handler::AssetHandler, asset_manager::AssetManager, material_asset_handler::MaterialAssetHandler, ResourceId}, glsl_shader_generator::GLSLShaderGenerator, resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend, resources::material::VariantModel, shader_generator::ShaderGenerationSettings, ReferenceModel};

	use super::ProgramGenerator;

	pub struct RootTestShaderGenerator {

	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform(&self, mut root: besl::parser::Node, material: &json::Object) -> besl::parser::Node {
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
		fn transform(&self, mut root: besl::parser::Node, material: &json::Object) -> besl::parser::Node {
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
		fn transform(&self, mut root: besl::parser::Node, _: &json::Object) -> besl::parser::Node {
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

		let glsl = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::fragment(), &main_node).expect("Failed to generate GLSL");

		assert!(glsl.contains("layout(push_constant"));
		assert!(glsl.contains("uint32_t material_index"));
		assert!(glsl.contains("materials[16]"));
		assert!(glsl.contains("void sample_("));
	}

	#[test]
	fn load_material() {
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

		let asset = asset_handler.load(&asset_manager, &resource_storage_backend, asset_manager.get_storage_backend(), ResourceId::new("material.bema"),).expect("Failed to load material");

		let _ = asset.load(&asset_manager, &resource_storage_backend, asset_manager.get_storage_backend(), ResourceId::new("material.bema"));

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

	#[test]
	fn load_variant() {
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

		let _: ReferenceModel<VariantModel> = asset_manager.load("variant.bema", &resource_storage_backend).expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();
		assert_eq!(generated_resources.len(), 3);

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

		let variant = resource_storage_backend.get_resource(ResourceId::new("variant.bema")).expect("Expected variant");

		assert_eq!(variant.id, "variant.bema");
		assert_eq!(variant.class, "Variant");
	}
}
