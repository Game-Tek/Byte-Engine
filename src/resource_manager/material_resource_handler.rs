use std::{collections::hash_map::DefaultHasher, hash::Hasher, rc::Rc};

use log::{warn, debug, error};
use polodb_core::bson::Document;
use serde::{Serialize, Deserialize};

use crate::{rendering::{shader_generator::ShaderGenerator, render_system}, shader_generator, jspd::{self, lexer}};

use super::ResourceHandler;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct Shader {
	pub stage: render_system::ShaderTypes,
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
			_ => false
		}
	}

	fn process(&self, asset_url: &str, bytes: &[u8]) -> Result<Vec<(Document, Vec<u8>)>, String> {
		let material_json = json::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();

		let material_domain = match &material_json["domain"] {
			json::JsonValue::Null => { "Common".to_string() }
			json::JsonValue::Short(s) => { s.to_string() }
			json::JsonValue::String(s) => { s.to_string() }
			_ => { panic!("Invalid domain") }
		};

		let material_type = match &material_json["type"] {
			json::JsonValue::Null => { "Raw".to_string() }
			json::JsonValue::Short(s) => { s.to_string() }
			json::JsonValue::String(s) => { s.to_string() }
			_ => { panic!("Invalid type") }
		};
		
		let mut shaders = material_json["shaders"].entries().filter_map(|(s_type, shader_json)| {
			Self::produce_shader(&material_domain, &shader_json, s_type)
		}).collect::<Vec<_>>();
		
		let required_resources = shaders.iter().map(|s| polodb_core::bson::doc! { "path": s.1.clone() }).collect::<Vec<_>>();

		let material_resource_document = polodb_core::bson::doc!{
			"class": "Material",
			"required_resources": required_resources,
			"resource": {}
		};

		shaders.push(((material_resource_document.clone(), Vec::new()), "".to_string()));

		Ok(shaders.iter().map(|s| s.0.clone()).collect::<Vec<_>>())
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>)> {
		vec![("Material",
		Box::new(|_document| {
			Box::new(Material {
				model: Model {
					name: Self::RENDER_MODEL.to_string(),
					pass: "MaterialEvaluation".to_string(),
				},
			})
		})),
		("Shader",
		Box::new(|_document| {
			Box::new(Shader {
				stage: render_system::ShaderTypes::Compute,
			})
		}))]
	}
}

impl MaterialResourcerHandler {
	const RENDER_MODEL: &str = "Visibility";

	fn treat_shader(path: &str, domain: &str, stage: &str, shader_node: jspd::lexer::Node,) -> Option<Result<(Document, Vec<u8>), String>> {
		let visibility = crate::rendering::visibility_shader_generator::VisibilityShaderGenerator::new();

		let glsl = visibility.transform(&shader_node, stage)?;

		debug!("Generated shader: {}", &glsl);

		let compiler = shaderc::Compiler::new().unwrap();
		let mut options = shaderc::CompileOptions::new().unwrap();

		options.set_optimization_level(shaderc::OptimizationLevel::Performance);
		options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_2 as u32);

		if cfg!(debug_assertions) {
			options.set_generate_debug_info();
		}

		options.set_target_spirv(shaderc::SpirvVersion::V1_5);
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

		let mut hasher = DefaultHasher::new();

		std::hash::Hash::hash(&result_shader_bytes, &mut hasher);

		let hash = hasher.finish() as i64;

		let resource = polodb_core::bson::doc!{
			"path": path,
			"class": "Shader",
			"hash": hash,
			"resource": {
				"stage": stage,
			}
		};

		Some(Ok((resource, Vec::from(result_shader_bytes))))
	}

	fn produce_shader(domain: &str, shader_json: &json::JsonValue, stage: &str) -> Option<((Document, Vec<u8>), String)> {
		let shader_option = match shader_json {
			json::JsonValue::Null => { None }
			json::JsonValue::Short(path) => {
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
			Some((Self::treat_shader(&path, domain, stage, shader,)?.unwrap(), path))
		} else {
			let default_shader = match stage {
				"Vertex" => MaterialResourcerHandler::default_vertex_shader(),
				"Fragment" => MaterialResourcerHandler::default_fragment_shader(),
				_ => { panic!("Invalid shader stage") }
			};

			let shader_node = jspd::lexer::Node {
				node: jspd::lexer::Nodes::GLSL { code: default_shader.to_string() },
			};

			Some((Self::treat_shader("", domain, stage, shader_node,)?.unwrap(), "".to_string()))
		}
	}
}