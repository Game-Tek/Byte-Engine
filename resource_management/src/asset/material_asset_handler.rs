use std::{cell::RefCell, ops::Deref};

use crate::{resource::material_resource_handler::ProgramGenerator, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, types::{AlphaMode, Material, Model, Property, Shader, ShaderTypes, Value, Variant, VariantVariable}, GenericResourceSerialization, ProcessedResources, StorageBackend, TypedResource};

use super::{asset_handler::AssetHandler, AssetResolver,};

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
	fn load<'a>(&'a self, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, id: &'a str, json: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
		Box::pin(async move {
			let url = json["url"].as_str().ok_or("No url provided").ok()?;

			if let Some(dt) = asset_resolver.get_type(url) {
				if dt != "json" { return None; }
			}

			let (data, at) = asset_resolver.resolve(url).await?;

			if at != "json" { return None; }

			let asset_json = json::parse(std::str::from_utf8(&data).ok()?).ok()?;

			let is_material = asset_json["parent"].is_null();

			if is_material {
				let material_domain = asset_json["domain"].as_str().ok_or("Domain not found".to_string()).ok()?;
				
				let generator = self.generator.as_ref().or_else(|| { log::warn!("No shader generator set for material asset handler"); None })?;

				let shaders = asset_json["shaders"].entries().filter_map(|(s_type, shader_json)| {
					smol::block_on(transform_shader(generator.deref(), asset_resolver, storage_backend, &material_domain, &asset_json, &shader_json, s_type))
				}).collect::<Vec<_>>();

				for variable in asset_json["variables"].members() {
					if variable["data_type"].as_str().unwrap() == "Texture2D" {
						let texture_url = variable["value"].as_str().unwrap();
					}
				}

				let resource = GenericResourceSerialization::new(id, Material {
					albedo: Property::Factor(Value::Vector3([1f32, 0f32, 0f32])),
					normal: Property::Factor(Value::Vector3([0f32, 0f32, 1f32])),
					roughness: Property::Factor(Value::Scalar(0.5f32)),
					metallic: Property::Factor(Value::Scalar(0.0f32)),
					emissive: Property::Factor(Value::Vector3([0f32, 0f32, 0f32])),
					occlusion: Property::Factor(Value::Scalar(0f32)),
					double_sided: false,
					alpha_mode: AlphaMode::Opaque,
					model: Model {
						name: "Visibility".to_string(),
						pass: "MaterialEvaluation".to_string(),
					},
					shaders: shaders.iter().map(|(s, b)| TypedResource::new_with_buffer("name", 0, s.clone(), b.clone())).collect(),
				});

				storage_backend.store(resource, &data).await.ok()?;
			} else {
				let variant_json = asset_json;

				let parent_material_url = variant_json["parent"].as_str().unwrap();

				let m_json = json::parse(&String::from_utf8_lossy(&asset_resolver.resolve(parent_material_url).await?.0)).ok()?;

				self.load(asset_resolver, storage_backend, parent_material_url, &m_json).await?.ok()?;

				let resource = GenericResourceSerialization::new(id, Variant{
					parent: parent_material_url.to_string(),
					variables: variant_json["variables"].members().map(|v| {
						VariantVariable {
							name: v["name"].to_string(),
							value: v["value"].to_string(),
						}
					}).collect::<Vec<_>>()
				});

				storage_backend.store(resource, &[]).await.ok()?;
			}

			Some(Ok(()))
		})
	}
}

async fn transform_shader(generator: &dyn ProgramGenerator, asset_resolver: &dyn AssetResolver, storage_backend: &dyn StorageBackend, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Option<(Shader, Box<[u8]>)> {
	let path = shader_json.as_str()?;
	let (arlp, format) = asset_resolver.resolve(&path).await?;

	let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

	let root_scope = jspd::Node::root();

	let parent_scope = generator.pre_transform(root_scope);

	let shader_scope = if format == "glsl" {
		jspd::lexer::Node::glsl(shader_code, Vec::new())
	} else if format == "besl" {
		if let Ok(e) = jspd::compile_to_jspd(&shader_code, Some(parent_scope.clone())) {
			e
		} else {
			log::error!("Error compiling shader");
			return None;
		}
	} else {
		log::error!("Unknown shader format");
		return None;
	};

	let main_node = RefCell::borrow(&shader_scope).get_child("main")?;

	let main_node = generator.post_transform(main_node);

	let glsl = ShaderGenerator::new().minified(!cfg!(debug_assertions)).compilation().generate_glsl_shader(&ShaderGenerationSettings::new(stage), &main_node);

	dbg!(&glsl);

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
			let error_string = ghi::shader_compilation::format_glslang_error(path, &error_string, &glsl).unwrap_or(error_string);
			log::error!("Error compiling shader:\n{}", error_string);
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
	use std::cell::RefCell;

	use super::MaterialAssetHandler;
	use crate::{asset::{asset_handler::AssetHandler, tests::{TestAssetResolver, TestStorageBackend}}, resource::material_resource_handler::ProgramGenerator};

	pub struct TestShaderGenerator {

	}

	impl TestShaderGenerator {
		pub fn new() -> TestShaderGenerator {
			TestShaderGenerator {}
		}
	}

	impl ProgramGenerator for TestShaderGenerator {
		fn pre_transform(&self, scope: jspd::NodeReference) -> jspd::NodeReference {
			let vec4f = RefCell::borrow(&scope).get_child("vec4f").unwrap();
			RefCell::borrow_mut(&scope).add_children(vec![jspd::Node::binding("material",jspd::BindingTypes::buffer(jspd::Node::r#struct("Material", vec![jspd::Node::member("color", vec4f)])), 0, 0, true, false)]);
			scope
		}

		fn post_transform(&self, scope: jspd::NodeReference) -> jspd::NodeReference {
			scope
		}
	}

	#[test]
	fn load_material() {
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = TestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		let url = "material.json";

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Fragment": "fragment.besl"
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

		asset_resolver.add_file(url, material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			material;
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		let doc = json::object! {
			"url": url,
		};

		smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &url, &doc)).expect("Image asset handler did not handle asset").expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar) buffer readonly Material{\tvec4f color;\n}material;"));
		assert!(shader_spirv.contains("void main() {\n\tmaterial;\n}"));

		let material = &generated_resources[1];

		assert_eq!(material.id, "material.json");
		assert_eq!(material.class, "Material");
	}

	#[test]
	fn load_variant() {
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
		let mut asset_handler = MaterialAssetHandler::new();

		let shader_generator = TestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		let id = "Variant.json";

		let material_asset_json = r#"{
			"url": "material.json"
		}"#;

		asset_resolver.add_file("Material.json", material_asset_json.as_bytes());

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Fragment": "fragment.besl"
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
			material;
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		let variant_json = r#"{
			"parent": "Material.json",
			"variables": [
				{
					"name": "color",
					"value": "White"
				}
			]
		}"#;

		asset_resolver.add_file(id, variant_json.as_bytes());

		let doc = json::object! {
			"url": id,
		};

		smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &id, &doc)).expect("Image asset handler did not handle asset").expect("Failed to load material");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = &generated_resources[0];

		assert_eq!(shader.id, "fragment.besl");
		assert_eq!(shader.class, "Shader");		

		let shader_spirv = storage_backend.get_resource_data_by_name("fragment.besl").expect("Expected shader data");
		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(shader_spirv.contains("layout(set=0,binding=0,scalar) buffer readonly Material{\tvec4f color;\n}material;"));
		assert!(shader_spirv.contains("void main() {\n\tmaterial;\n}"));

		let material = &generated_resources[1];
		
		assert_eq!(material.id, "Material.json");
		assert_eq!(material.class, "Material");

		let variant = &generated_resources[2];

		assert_eq!(variant.id, "Variant.json");
		assert_eq!(variant.class, "Variant");
	}
}