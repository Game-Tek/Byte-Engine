use std::{collections::hash_map::DefaultHasher, hash::Hasher};

use polodb_core::bson::Document;

use crate::{rendering::shader_generator::ShaderGenerator, beshader_compiler, shader_generator};

use super::ResourceHandler;

pub struct MaterialResourcerHandler {

}

pub struct Material {

}

impl MaterialResourcerHandler {
	pub fn new() -> Self {
		Self {

		}
	}
}

impl ResourceHandler for MaterialResourcerHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"json" => true,
			_ => false
		}
	}

	fn process(&self, bytes: &[u8]) -> Result<Vec<(Document, Vec<u8>)>, String> {
		let material_json = json::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();

		let t = material_json["type"].as_str().unwrap();
		let vertex  = material_json["vertex"].as_str().unwrap();
		let fragment = material_json["fragment"].as_str().unwrap();

		fn treat_shader(path: &str, t: &str) -> (Document, Vec<u8>) {
			let path = "resources/".to_string() + path;
			let shader_code = std::fs::read_to_string(path).unwrap();
			let shader = beshader_compiler::parse(beshader_compiler::tokenize(&shader_code));

			let mut shader_spec = json::object! { glsl: { version: "450" } };

			let common = crate::rendering::common_shader_generator::CommonShaderGenerator::new();

			let c = common.process();

			shader_spec["root"][c.0] = c.1;

			let visibility = crate::rendering::visibility_shader_generator::VisibilityShaderGenerator::new();

			let v = visibility.process();

			shader_spec["root"][c.0][v.0] = v.1;

			let shader_generator = shader_generator::ShaderGenerator::new();

			let glsl = shader_generator.generate(&shader_spec, &json::object!{ path: "Common.Visibility", type: t });

			let compiler = shaderc::Compiler::new().unwrap();
			let mut options = shaderc::CompileOptions::new().unwrap();

			options.set_optimization_level(shaderc::OptimizationLevel::Performance);
			options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_2 as u32);
			options.set_generate_debug_info();
			options.set_target_spirv(shaderc::SpirvVersion::V1_5);
			options.set_invert_y(true);

			let binary = compiler.compile_into_spirv(&glsl, shaderc::ShaderKind::InferFromSource, "shader_name", "main", Some(&options));

			let compilation_artifact = match binary { Ok(binary) => { binary } Err(error) => { panic!(); } };

			if compilation_artifact.get_num_warnings() > 0 {
				println!("Shader warnings: {}", compilation_artifact.get_warning_messages());
			}

			let result_shader_bytes = compilation_artifact.as_binary_u8();

			let mut hasher = DefaultHasher::new();

			std::hash::Hash::hash(&result_shader_bytes, &mut hasher);

			let hash = hasher.finish() as i64;

			let resource = polodb_core::bson::doc!{
				"class": "Shader",
				"hash": hash
			};

			(resource, Vec::from(result_shader_bytes))
		}

		let a = treat_shader(vertex, "vertex");
		let b = treat_shader(fragment, "fragment");

		let material_resource_document = polodb_core::bson::doc!{
			"class": "Material",
			"required_resources": [
				{
					"path": vertex,
				},
				{
					"path": fragment,
				}
			],
			"resource": {}
		};

		Ok(vec![a, b])
	}

	fn get_deserializer(&self) -> Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send> {
		Box::new(|document| {
			Box::new(Material {})
		})
	}
}