use std::io::Read;

use log::{warn, debug, error};
use serde::{Serialize, Deserialize};

use crate::{rendering::render_system, jspd::{self}};

use super::{SerializedResourceDocument, GenericResourceSerialization, Resource, ProcessedResources, resource_handler::ResourceHandler, resource_manager::ResourceManager};

pub struct MaterialResourcerHandler {

}

#[derive(Debug, Serialize, Deserialize)]
pub struct Model {
	/// The name of the model.
	pub name: String,
	/// The render pass of the model.
	pub pass: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Material {
	/// The render model this material is for.
	pub model: Model,
}

impl Resource for Material {
	fn get_class(&self) -> &'static str { "Material" }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VariantVariable {
	pub name: String,
	pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Variant {
	/// Parent material asset url.
	pub parent: String,
	pub variables: Vec<VariantVariable>,
}

impl Resource for Variant {
	fn get_class(&self) -> &'static str { "Variant" }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Shader {
	pub stage: render_system::ShaderTypes,
}

impl Resource for Shader {
	fn get_class(&self) -> &'static str { "Shader" }
}

impl MaterialResourcerHandler {
	pub fn new() -> Self {
		Self {

		}
	}

	fn default_vertex_shader() -> &'static str {
		"void main() { gl_Position = pc.camera.view_projection * pc.meshes[gl_InstanceIndex].model * vec4(in_position, 1.0); out_instance_index = gl_InstanceIndex; }"
	}

	fn default_fragment_shader() -> &'static str {
		"void main() { out_color = get_debug_color(in_instance_index); }"
	}
}

impl ResourceHandler for MaterialResourcerHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"json" => true,
			"glsl" => true,
			"besl" => true,
			_ => false
		}
	}

	fn read(&self, _resource: &Box<dyn std::any::Any>, file: &mut std::fs::File, buffers: &mut [super::Buffer]) {
		file.read_exact(buffers[0].buffer).unwrap();
	}

	fn process(&self, resource_manager: &ResourceManager, asset_url: &str,) -> Result<Vec<ProcessedResources>, String> {
		let (bytes, _) = resource_manager.read_asset_from_source(asset_url).unwrap();

		let asset_json = json::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();

		let is_material = asset_json["parent"].is_null();

		if is_material {
			let material_domain = match &asset_json["domain"] {
				json::JsonValue::Null => { "Common".to_string() }
				json::JsonValue::Short(s) => { s.to_string() }
				json::JsonValue::String(s) => { s.to_string() }
				_ => { panic!("Invalid domain") }
			};

			let material_type = match &asset_json["type"] {
				json::JsonValue::Null => { "Raw".to_string() }
				json::JsonValue::Short(s) => { s.to_string() }
				json::JsonValue::String(s) => { s.to_string() }
				_ => { panic!("Invalid type") }
			};
			
			let mut required_resources = asset_json["shaders"].entries().filter_map(|(s_type, shader_json)| {
				Self::produce_shader(resource_manager, &material_domain, &asset_json, &shader_json, s_type)
			}).collect::<Vec<_>>();

			for variable in asset_json["variables"].members() {
				if variable["data_type"].as_str().unwrap() == "Texture2D" {
					let texture_url = variable["value"].as_str().unwrap();

					required_resources.push(ProcessedResources::Ref(texture_url.to_string()));
				}
			}

			Ok(vec![ProcessedResources::Generated((GenericResourceSerialization::new(asset_url.to_string(), Material {
				model: Model {
					name: Self::RENDER_MODEL.to_string(),
					pass: "MaterialEvaluation".to_string(),
				},
			}).required_resources(&required_resources), Vec::new()))])
		} else {
			let variant_json = asset_json;

			let parent_material_url = variant_json["parent"].as_str().unwrap();

			let material_resource_document = GenericResourceSerialization::new(asset_url.to_string(), Variant{
				parent: parent_material_url.to_string(),
				variables: variant_json["variables"].members().map(|v| {
					VariantVariable {
						name: v["name"].to_string(),
						value: v["value"].to_string(),
					}
				}).collect::<Vec<_>>()
			}).required_resources(&[ProcessedResources::Ref(parent_material_url.to_string())]);

			Ok(vec![ProcessedResources::Generated((material_resource_document.into(), Vec::new()))])
		}
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>)> {
		vec![("Material",
			Box::new(|_document| {
				Box::new(Material::deserialize(polodb_core::bson::Deserializer::new(_document.into())).unwrap())
			})),
			("Shader",
			Box::new(|_document| {
				Box::new(Shader {
					stage: render_system::ShaderTypes::Compute,
				})
			})),
			("Variant",
			Box::new(|document| {
				Box::new(Variant::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap())
			})),
		]
	}
}

impl MaterialResourcerHandler {
	const RENDER_MODEL: &'static str = "Visibility";

	fn treat_shader(path: &str, domain: &str, stage: &str, material: &json::JsonValue, shader_node: jspd::lexer::Node,) -> Option<Result<ProcessedResources, String>> {
		let visibility = crate::rendering::visibility_shader_generator::VisibilityShaderGenerator::new();

		let glsl = visibility.transform(material, &shader_node, stage)?;

		debug!("Generated shader: {}", &glsl);

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

		let compilation_artifact = match binary { Ok(binary) => { binary } Err(err) => {
			error!("Failed to compile shader: {}", err);
			error!("{}", &glsl);
			return Some(Err(err.to_string()));
		} };

		if compilation_artifact.get_num_warnings() > 0 {
			warn!("Shader warnings: {}", compilation_artifact.get_warning_messages());
		}

		let result_shader_bytes = compilation_artifact.as_binary_u8();

		let stage = match stage {
			"Vertex" => render_system::ShaderTypes::Vertex,
			"Fragment" => render_system::ShaderTypes::Fragment,
			"Compute" => render_system::ShaderTypes::Compute,
			_ => { panic!("Invalid shader stage") }
		};

		let resource = GenericResourceSerialization::new(path.to_string(), Shader {
			stage: stage,
		});

		Some(Ok(ProcessedResources::Generated((resource, Vec::from(result_shader_bytes)))))
	}

	fn produce_shader(resource_manager: &ResourceManager, domain: &str, material: &json::JsonValue, shader_json: &json::JsonValue, stage: &str) -> Option<ProcessedResources> {
		let shader_option = match shader_json {
			json::JsonValue::Null => { None }
			json::JsonValue::Short(path) => {
				let (arlp, format) = resource_manager.read_asset_from_source(&path).ok()?;

				let shader_code = std::str::from_utf8(&arlp).unwrap().to_string();

				if format == "glsl" {
					Some((jspd::lexer::Node {
						node: jspd::lexer::Nodes::GLSL { code: shader_code },
					}, path.to_string()))
				} else if format == "besl" {
					Some((jspd::compile_to_jspd(&shader_code).unwrap(), path.to_string()))
				} else {
					None
				}
			}
			json::JsonValue::String(path) => {
				let arlp = "assets/".to_string() + path.as_str();

				if path.ends_with(".glsl") {
					let shader_code = std::fs::read_to_string(&arlp).unwrap();
					Some((jspd::lexer::Node {
						node: jspd::lexer::Nodes::GLSL { code: shader_code },
					}, path.to_string()))
				} else if path.ends_with(".besl") {
					let shader_code = std::fs::read_to_string(&arlp).unwrap();
					Some((jspd::compile_to_jspd(&shader_code).unwrap(), path.to_string()))
				} else {
					None
				}
			}
			_ => {
				error!("Invalid {stage} shader");
				None
			}
		};

		if let Some((shader, path)) = shader_option {
			Some(Self::treat_shader(&path, domain, stage, material, shader,)?.unwrap())
		} else {
			let default_shader = match stage {
				"Vertex" => MaterialResourcerHandler::default_vertex_shader(),
				"Fragment" => MaterialResourcerHandler::default_fragment_shader(),
				_ => { panic!("Invalid shader stage") }
			};

			let shader_node = jspd::lexer::Node {
				node: jspd::lexer::Nodes::GLSL { code: default_shader.to_string() },
			};

			Some(Self::treat_shader("", domain, stage, material, shader_node,)?.unwrap())
		}
	}
}

#[cfg(test)]
mod tests {
    use crate::resource_manager::resource_manager::ResourceManager;

	#[test]
	fn load_material() {
		let mut resource_manager = ResourceManager::new();

		let (response, _) = resource_manager.get("solid").expect("Failed to load material");

		assert_eq!(response.resources.len(), 2); // 1 material, 1 shader

		let resource_container = &response.resources[0];

		assert_eq!(resource_container.class, "Shader");

		let resource_container = &response.resources[1];

		assert_eq!(resource_container.class, "Material");
	}
}