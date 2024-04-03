use std::ops::Deref;

use log::debug;

use crate::{shader_generation::{ShaderGenerationSettings, ShaderGenerator}, types::{AlphaMode, Material, MaterialModel, Model, Parameter, Property, Shader, ShaderTypes, Value, Variant, VariantVariable}, GenericResourceResponse, GenericResourceSerialization, Solver, StorageBackend, TypedResource, TypedResourceModel};

use super::{asset_handler::AssetHandler, asset_manager::AssetManager, AssetResolver};

pub trait ProgramGenerator {
	/// Transforms a program.
	fn transform(&self, program_state: &mut jspd::parser::ProgramState, material: &json::JsonValue) -> Vec<jspd::parser::NodeReference>;
}

pub struct MaterialAssetHandler {
	generator: Option<Box<dyn ProgramGenerator>>,
}

impl MaterialAssetHandler {
	pub fn new() -> MaterialAssetHandler {
		MaterialAssetHandler {
			generator: None,
		}
	}

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Box::new(generator));
    }
}

impl AssetHandler for MaterialAssetHandler {
	fn load<'a>(&'a self, asset_manager: &'a AssetManager, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, url: &'a str, json: Option<&'a json::JsonValue>) -> utils::BoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
		Box::pin(async move {
			if let Some(dt) = asset_resolver.get_type(url) {
				if dt != "json" { return Err("Not my type".to_string()); }
			}

			let (data, at) = asset_resolver.resolve(url).await.ok_or("Failed to resolve asset".to_string())?;

			let asset_json = json::parse(std::str::from_utf8(&data).or_else(|_| { Err("Failed to parse JSON") })?).or_else(|_| { Err("Failed to parse JSON") })?;

			let is_material = asset_json["parent"].is_null();

			let to_value = |t: &str, v: &str| {
				match t {
					"vec4f" => Value::Vector4([0f32, 0f32, 0f32, 0f32]),
					"vec3f" => Value::Vector3([0f32, 0f32, 0f32]),
					"float" => Value::Scalar(0f32),
					"Texture2D" => Value::Image(smol::block_on(asset_manager.load_typed_resource(v)).unwrap()),
					_ => panic!("Unknown data type")
				}
			};

			let resource = if is_material {
				let material_domain = asset_json["domain"].as_str().ok_or("Domain not found".to_string()).or_else(|e| { debug!("{}", e); Err("Domain not found".to_string()) })?;
				
				let generator = self.generator.as_ref().or_else(|| { log::warn!("No shader generator set for material asset handler"); None }).ok_or("No shader generator set".to_string()).or_else(|e| { debug!("{}", e); Err("No shader generator set".to_string()) })?;

				let shaders = asset_json["shaders"].entries().filter_map(|(s_type, shader_json)| { // TODO: desilence
					smol::block_on(transform_shader(generator.deref(), asset_resolver, storage_backend, &material_domain, &asset_json, &shader_json, s_type))
				}).collect::<Vec<_>>();

				let parameters = asset_json["variables"].members().map(|v: &json::JsonValue| {
					let name = v["name"].to_string();
					let data_type = v["data_type"].to_string();
					let value = v["value"].to_string();

					Parameter {
						name,
						r#type: data_type.clone(),
						value: to_value(&data_type, &value),
					}
				}).collect::<Vec<_>>();

				let resource = GenericResourceSerialization::new(url, Material {
					double_sided: false,
					alpha_mode: AlphaMode::Opaque,
					model: Model {
						name: "Visibility".to_string(),
						pass: "MaterialEvaluation".to_string(),
					},
					shaders: shaders.iter().map(|(s, b)| TypedResource::new_with_buffer(&s.id, 0, s.clone(), b.clone())).collect(),
					parameters,
				});

				storage_backend.store(resource.clone(), &data).await;

				resource
			} else {
				let variant_json = asset_json;

				let parent_material_url = variant_json["parent"].as_str().unwrap();

				let m_json = json::parse(&String::from_utf8_lossy(&asset_resolver.resolve(parent_material_url).await.ok_or("Failed to resolve parent material".to_string())?.0)).or_else(|_| { Err("Failed to parse JSON") })?;

				let material = match self.load(asset_manager, asset_resolver, storage_backend, parent_material_url, None).await {
					Ok(Some(m)) => { m }
					Ok(None) | Err(_) => {
						log::error!("Failed to load parent material");						
						return Err("Failed to load parent material".to_string());
					}
				};

				let material: TypedResourceModel<MaterialModel> = material.try_into().or_else(|_| { Err("Failed to convert material") })?;

				let resource = GenericResourceSerialization::new(url, Variant {
					material: material.solve(storage_backend).map_err(|_| { "Failed to solve material".to_string() })?,
					variables: variant_json["variables"].members().map(|v| {
						VariantVariable {
							name: v["name"].to_string(),
							value: v["value"].to_string(),
						}
					}).collect::<Vec<_>>()
				});

				match storage_backend.store(resource.clone(), &[]).await {
					Ok(_) => {}
					Err(_) => {
						log::error!("Failed to store resource {}", url);
					}
				}

				resource
			};

			Ok(Some(resource))
		})
	}
}

async fn transform_shader(generator: &dyn ProgramGenerator, asset_resolver: &dyn AssetResolver, storage_backend: &dyn StorageBackend, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Option<(Shader, Box<[u8]>)> {
	let path = shader_json.as_str()?;
	let (arlp, format) = asset_resolver.resolve(&path).await?;

	let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

	let mut program_state = if format == "glsl" {
		// jspd::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		jspd::parser::ProgramState::new()
	} else if format == "besl" {
		if let Ok(e) = jspd::parse(&shader_code,/*Some(parent_scope.clone())*/) {
			e
		} else {
			log::error!("Error compiling shader");
			return None;
		}
	} else {
		log::error!("Unknown shader format");
		return None;
	};

	let generator_elements = generator.transform(&mut program_state, material);

	let root_node = match jspd::lex(jspd::parser::NodeReference::root_with_children(generator_elements), &program_state) {
		Ok(e) => e,
		Err(e) => {
			log::error!("Error compiling shader: {:#?}", e);
			return None;
		}
	};

	let main_node = root_node.borrow().get_main()?;

	let glsl = ShaderGenerator::new().minified(!cfg!(debug_assertions)).compilation().generate_glsl_shader(&ShaderGenerationSettings::new(stage), &main_node);

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

	storage_backend.store(GenericResourceSerialization::new(path, shader.clone()), result_shader_bytes).await.ok()?;

	Some((shader, result_shader_bytes.into()))
}

fn default_vertex_shader() -> &'static str {
	"void main() { gl_Position = pc.camera.view_projection * pc.meshes[gl_InstanceIndex].model * vec4(in_position, 1.0); out_instance_index = gl_InstanceIndex; }"
}

fn default_fragment_shader() -> &'static str {
	"void main() { out_color = get_debug_color(in_instance_index); }"
}

#[cfg(test)]
pub mod tests {
	use super::{MaterialAssetHandler, ProgramGenerator};
	use crate::asset::{asset_handler::AssetHandler, asset_manager::AssetManager, tests::{TestAssetResolver, TestStorageBackend}};

	pub struct RootTestShaderGenerator {

	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform(&self, program_state: &mut jspd::parser::ProgramState, material: &json::JsonValue) -> Vec<jspd::parser::NodeReference> {
			let material_struct = jspd::parser::NodeReference::buffer("Material", vec![jspd::parser::NodeReference::member("color", "vec4f")]);
			program_state.insert("Material".to_string(), material_struct.clone());

			let sample_function = jspd::parser::NodeReference::function("sample_", vec![jspd::parser::NodeReference::member("t", "u32")], "void", vec![]);

			program_state.insert("sample_".to_string(), sample_function.clone());

			let mid_test_shader_generator = MidTestShaderGenerator::new();

			let child = mid_test_shader_generator.transform(program_state, material);

			// jspd::parser::NodeReference::scope("RootTestShaderGenerator", vec![material, child])
			vec![material_struct, sample_function].into_iter().chain(child.into_iter()).collect()
		}
	}

	pub struct MidTestShaderGenerator {}

	impl MidTestShaderGenerator {
		pub fn new() -> MidTestShaderGenerator {
			MidTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for MidTestShaderGenerator {
		fn transform(&self, program_state: &mut jspd::parser::ProgramState, material: &json::JsonValue) -> Vec<jspd::parser::NodeReference> {
			let binding = jspd::parser::NodeReference::binding("materials", jspd::parser::NodeReference::buffer("Materials", vec![jspd::parser::NodeReference::member("materials", "Material[16]")]), 0, 0, true, false);
			program_state.insert("materials".to_string(), binding.clone());

			let leaf_test_shader_generator = LeafTestShaderGenerator::new();

			let child = leaf_test_shader_generator.transform(program_state, material);

			// jspd::parser::NodeReference::scope("MidTestShaderGenerator", vec![binding, child])
			vec![binding].into_iter().chain(child.into_iter()).collect()
		}
	}

	struct LeafTestShaderGenerator {}

	impl LeafTestShaderGenerator {
		pub fn new() -> LeafTestShaderGenerator {
			LeafTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for LeafTestShaderGenerator {
		fn transform(&self, program_state: &mut jspd::parser::ProgramState, material: &json::JsonValue) -> Vec<jspd::parser::NodeReference> {
			let push_constant = jspd::parser::NodeReference::push_constant(vec![jspd::parser::NodeReference::member("material_index", "u32")]);
			program_state.insert("push_constant".to_string(), push_constant.clone());

			let main = jspd::parser::NodeReference::function("main", vec![], "void", vec![jspd::parser::NodeReference::glsl("push_constant;\nmaterials;\nsample_(0);\n", vec!["push_constant".to_string(), "materials".to_string(), "sample_".to_string()], Vec::new())]);
			program_state.insert("main".to_string(), main.clone());

			// jspd::parser::NodeReference::scope("LeafTestShaderGenerator", vec![push_constant, main])
			vec![push_constant, main]
		}
	}

	#[test]
	fn generate_program() {
		let test_shader_generator = RootTestShaderGenerator::new();

		let mut program_state = jspd::parser::ProgramState::new();

		let root = test_shader_generator.transform(&mut program_state, &json::object! {});

		let root = jspd::lex(jspd::parser::NodeReference::root_with_children(root), &program_state).unwrap();

		let main_node = root.borrow().get_main().unwrap();

		let glsl = crate::shader_generation::ShaderGenerator::new().minified(true).compilation().generate_glsl_shader(&crate::shader_generation::ShaderGenerationSettings::new("Fragment"), &main_node);

		dbg!(&glsl);

		assert!(glsl.contains("readonly buffer Materials"));
		assert!(glsl.contains("layout(push_constant"));
		assert!(glsl.contains("uint32_t material_index"));
		assert!(glsl.contains("materials[16]"));
		assert!(glsl.contains("void sample_("));
	}

	#[test]
	fn load_material() {
		let asset_manager = AssetManager::new();
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
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

		asset_resolver.add_file("material.json", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, "material.json", None)).unwrap().expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar) readonly buffer Materials{\n\tMaterial materials[16];\n}materials;"));
		assert!(shader_spirv.contains("void main() {\n\tpush_constant;\nmaterials;\n"));

		let material = &generated_resources[1];

		assert_eq!(material.id, "material.json");
		assert_eq!(material.class, "Material");
	}

	#[test]
	fn load_variant() {
		let asset_manager = AssetManager::new();
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
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

		asset_resolver.add_file("material.json", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		let variant_json = r#"{
			"parent": "material.json",
			"variables": [
				{
					"name": "color",
					"value": "White"
				}
			]
		}"#;

		asset_resolver.add_file("variant.json", variant_json.as_bytes());

		smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, "variant.json", None)).unwrap().expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar) readonly buffer Materials{\n\tMaterial materials[16];\n}materials;"));
		assert!(shader_spirv.contains("void main() {\n\tpush_constant;\nmaterials;\n"));

		let material = &generated_resources[1];
		
		assert_eq!(material.id, "material.json");
		assert_eq!(material.class, "Material");

		let variant = &generated_resources[2];

		assert_eq!(variant.id, "variant.json");
		assert_eq!(variant.class, "Variant");
	}
}