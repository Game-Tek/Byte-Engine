//! The shader_generator module provides utilities to generate shaders.

use std::fmt::format;

use json;

pub struct ShaderGenerator {

}

impl ShaderGenerator {
	pub fn new() -> ShaderGenerator {
		ShaderGenerator {}
	}

	fn generate_glsl_extension_block(shader_type: &str) -> String {
		let mut shader_string = String::with_capacity(1024);

		shader_string.push_str("#extension GL_EXT_shader_16bit_storage : enable\n");
		shader_string.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable\n");
		shader_string.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable\n");
		shader_string.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable\n");
		shader_string.push_str("#extension GL_EXT_nonuniform_qualifier : enable\n");
		shader_string.push_str("#extension GL_EXT_scalar_block_layout : enable\n");
		shader_string.push_str("#extension GL_EXT_buffer_reference : enable\n");
		shader_string.push_str("#extension GL_EXT_buffer_reference2 : enable\n");
		shader_string.push_str("#extension GL_EXT_shader_image_load_formatted : enable\n");

		match shader_type {
			"compute" => {
				shader_string.push_str("#extension GL_KHR_shader_subgroup_basic : enable\n");
				shader_string.push_str("#extension GL_KHR_shader_subgroup_arithmetic  : enable\n");
				shader_string.push_str("#extension GL_KHR_shader_subgroup_ballot : enable\n");
				shader_string.push_str("#extension GL_KHR_shader_subgroup_shuffle : enable\n");
			}
			_ => {}
		}

		shader_string
	}

	/// Generates a shader from a source string and an environment\
	/// Environment:
	/// 	glsl:
	/// 		version: expects a string, if this value is not present, the default version is 450
	/// 	shader:
	/// 		type: expects a string, can be "vertex", "fragment" or "compute"
	/// 		vertex_layout: expects an array of objects, each object must have a "type" and a "name" field
	/// 		input: expects an array of objects, each object must have a "type" and a "name" field
	/// 		output: expects an array of objects, each object must have a "type" and a "name" field
	/// 		local_size: expects an array of 3 numbers
	/// 		descriptor_sets: expects an array of objects, each object must have a "set", "binding" and "value" field
	/// 			set: expects a number
	/// 			binding: expects a number
	/// 			value: expects an object with a "type" and a "name" field
	/// 		push_constants: expects an array of objects, each object must have a "type" and a "name" field
	/// 		structs: expects an array of objects, each object must have a "name" and a "members" field
	/// 			name: expects a string
	/// 			members: expects an array of objects, each object must have a "type" and a "name" field
	/// 
	/// Result:
	/// 	Returns a string with the generated shader
	/// 	Generated shaders will always look like
	/// 		#version
	/// 		#shader type
	/// 		#extensions
	/// 		memory layout declarations
	/// 		struct declarations
	/// 		descritor set declarations
	/// 		push constant declarations
	/// 		in blocks
	/// 		out blocks
	/// 		function declarations
	/// 		"main decorators"
	/// 		main function
	/// 		
	pub fn generate_shader(&self, base_shader: &str, environment: json::JsonValue) -> String {
		let mut shader_string = String::new();

		// version

		let glsl_version = &environment["glsl"]["version"];

		if let json::JsonValue::String(glsl_version) = glsl_version {
			shader_string.push_str("#version ");
			shader_string.push_str(&glsl_version); // TODO: Get only first number
			shader_string.push_str(" core\n");
		} else {
			shader_string.push_str("#version 450 core\n");
		}

		// shader type

		let shader_type = &environment["shader"]["type"];

		let shader_type = shader_type.as_str().unwrap_or("");

		match shader_type {
			"vertex" => shader_string.push_str("#pragma shader_stage(vertex)\n"),
			"fragment" => shader_string.push_str("#pragma shader_stage(fragment)\n"),
			"compute" => shader_string.push_str("#pragma shader_stage(compute)\n"),
			_ => shader_string.push_str("#define BE_UNKNOWN_SHADER_TYPE\n")
		}

		// extensions

		shader_string += Self::generate_glsl_extension_block(shader_type).as_str();

		// memory layout declarations

		shader_string.push_str("layout(row_major) uniform; layout(row_major) buffer;\n");

		// struct declarations

		let structs = &environment["shader"]["structs"];

		if let json::JsonValue::Array(structs) = structs {
			for e in structs {
				shader_string.push_str("struct ");
				shader_string.push_str(&e["name"].as_str().unwrap());
				shader_string.push_str(" {\n");

				let members = &e["members"];

				if let json::JsonValue::Array(members) = members {
					for e in members {
						shader_string.push_str(&translate_type(&e["type"].as_str().unwrap()));
						shader_string.push_str(" ");
						shader_string.push_str(&e["name"].as_str().unwrap());
						shader_string.push_str(";\n");
					}
				}

				shader_string.push_str("};\n");
			}

			for e in structs {
				shader_string.push_str("layout(buffer_reference,scalar,buffer_reference_align=2) buffer ");
				shader_string.push_str(&e["name"].as_str().unwrap());
				shader_string.push_str("Pointer {\n");

				let members = &e["members"];

				if let json::JsonValue::Array(members) = members {
					for e in members {
						shader_string.push_str(&translate_type(&e["type"].as_str().unwrap()));
						shader_string.push_str(" ");
						shader_string.push_str(&e["name"].as_str().unwrap());
						shader_string.push_str(";\n");
					}
				}

				shader_string.push_str("};\n");
			}
		}

		// descriptor set declarations

		let descriptor_sets = &environment["shader"]["descriptor_sets"];

		if let json::JsonValue::Array(descriptor_sets) = descriptor_sets {
			for e in descriptor_sets {
				shader_string.push_str("layout(set=");
				shader_string.push_str(&e["set"].to_string());
				shader_string.push_str(", binding=");
				shader_string.push_str(&e["binding"].to_string());
				shader_string.push_str(") uniform ");
				shader_string.push_str(&translate_type(&e["value"]["type"].as_str().unwrap()));
				shader_string.push_str(" ");
				shader_string.push_str(&e["value"]["name"].as_str().unwrap());
				shader_string.push_str(";\n");
			}
		}

		// push constant declarations

		let push_constants = &environment["shader"]["push_constants"];

		if let json::JsonValue::Array(push_constants) = push_constants {
			shader_string.push_str("layout(push_constant) uniform push_constants {\n");

			for e in push_constants {
				shader_string.push_str(&translate_type(&e["type"].as_str().unwrap()));
				shader_string.push_str(" ");
				shader_string.push_str(&e["name"].as_str().unwrap());
				shader_string.push_str(";\n");
			}

			shader_string.push_str("} pc;\n");
		}

		// in blocks

		match shader_type {
			"vertex" => {
				let vertex_layout = &environment["shader"]["vertex_layout"];

				if let json::JsonValue::Array(vertex_layout) = vertex_layout {
					let mut pos = 0;
		
					for e in vertex_layout {
						shader_string.push_str("layout(location=");
						shader_string.push_str(&pos.to_string());
						shader_string.push_str(") in ");
						shader_string.push_str(&translate_type(&e["type"].as_str().unwrap()));
						shader_string.push_str(" ");
						shader_string.push_str(&e["name"].as_str().unwrap());
						shader_string.push_str(";\n");
		
						pos += 1;
					}
				}
			}
			"fragment" => {
				let input = &environment["shader"]["input"];

				if let json::JsonValue::Array(input) = input {
					let mut pos = 0;
		
					for e in input {
						shader_string.push_str("layout(location=");
						shader_string.push_str(&pos.to_string());
						shader_string.push_str(") in ");
						shader_string.push_str(&translate_type(&e["type"].as_str().unwrap()));
						shader_string.push_str(" ");
						shader_string.push_str(&e["name"].as_str().unwrap());
						shader_string.push_str(";\n");
		
						pos += 1;
					}
				}
	
				let output = &environment["shader"]["output"];
	
				if let json::JsonValue::Array(output) = output {
					let mut pos = 0;
		
					for e in output {
						shader_string.push_str("layout(location=");
						shader_string.push_str(&pos.to_string());
						shader_string.push_str(") out ");
						shader_string.push_str(&e["type"].as_str().unwrap());
						shader_string.push_str(" ");
						shader_string.push_str(&e["name"].as_str().unwrap());
						shader_string.push_str(";\n");
		
						pos += 1;
					}
				}
			}
			"compute" => {
				let local_size = &environment["shader"]["local_size"];

				if let json::JsonValue::Array(local_size) = local_size {
					shader_string.push_str("layout(local_size_x=");
					shader_string.push_str(&local_size[0].to_string());
					shader_string.push_str(", local_size_y=");
					shader_string.push_str(&local_size[1].to_string());
					shader_string.push_str(", local_size_z=");
					shader_string.push_str(&local_size[2].to_string());
					shader_string.push_str(") in;\n");
				}
			}
			_ => {}
		}

		// main function	

		shader_string.push_str(base_shader);

		shader_string
	}

	pub fn generate(&self, program_spec: &json::JsonValue, compilation_settings: &json::JsonValue) -> String {
		let mut nodes = Vec::with_capacity(32);

		nodes.push(("GLSL".to_string(), {
			let mut glsl_block = String::with_capacity(512);
			let glsl_version = &program_spec["glsl"]["version"];

			if let json::JsonValue::String(glsl_version) = glsl_version {
				glsl_block.push_str(&format!("#version {glsl_version} core\n"));
			} else {
				glsl_block.push_str("#version 450 core\n");
			}

			// shader type

			let shader_type = &compilation_settings["type"];

			let shader_type = shader_type.as_str().unwrap_or("");

			match shader_type {
				"vertex" => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
				"fragment" => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
				"compute" => glsl_block.push_str("#pragma shader_stage(compute)\n"),
				_ => glsl_block.push_str("#define BE_UNKNOWN_SHADER_TYPE\n")
			}

			// extensions

			glsl_block += Self::generate_glsl_extension_block(shader_type).as_str();
			// memory layout declarations

			glsl_block.push_str("layout(row_major) uniform; layout(row_major) buffer;\n");

			glsl_block
		}));

		let root = &program_spec["root"];

		let root = root.entries().next().unwrap();

		#[derive(Clone)]
		struct ProgramState {
			in_position: u32, out_position: u32, push_constants: u32,
			nodes: Vec<(String, String)>,
		}

		fn process_node(string: Option<&mut String>, (name, node): (&str, &json::JsonValue), compilation_settings: &json::JsonValue, program_state: &mut ProgramState) {
			if !matches!(node, json::JsonValue::Object(_)) { return; }

			if let Some(only_under) = (&node["__only_under"]).as_str() {
				if only_under != compilation_settings["type"].as_str().unwrap() { return; }
			}

			let node_type = if let Some(ty) = node["type"].as_str() { ty } else { return; };

			match node_type {
				"scope" => {
					for entry in node.entries() {
						process_node(None, entry, compilation_settings, program_state);
					}
				}
				"struct" => {
					let mut shader_string = String::with_capacity(64);
					shader_string.push_str(format!("struct {name} {{").as_str());

					for entry in node.entries() {
						process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
					}

					shader_string.push_str(" };\n");

					shader_string.push_str(&format!("layout(buffer_reference,scalar,buffer_reference_align=2) buffer {name}Pointer {{"));

					for entry in node.entries() {
						process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
					}

					shader_string.push_str(" };\n");

					program_state.nodes.push((name.to_string(), shader_string));
				}
				"in" => {
					let mut shader_string = String::with_capacity(32);

					let location = program_state.in_position;
					shader_string.push_str(format!("layout(location={location})").as_str());

					if let Some(interpolation) = node["interpolation"].as_str() {
						shader_string.push_str(&format!(" {interpolation}"));
					}

					shader_string.push_str(" in");

					program_state.in_position += 1;

					for entry in node.entries() {
						process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
					}

					shader_string.push_str("\n");

					program_state.nodes.push((name.to_string(), shader_string));
				}
				"out" => {
					let mut shader_string = String::with_capacity(32);

					let location = program_state.out_position;
					shader_string.push_str(format!("layout(location={location}) out").as_str());

					if let Some(interpolation) = node["interpolation"].as_str() {
						shader_string.push_str(&format!(" {interpolation}"));
					}

					program_state.out_position += 1;

					for entry in node.entries() {
						process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
					}

					shader_string.push_str("\n");

					program_state.nodes.push((name.to_string(), shader_string));
				}
				"function" => {
					let mut shader_string = String::with_capacity(128);

					if node.has_key("code") {
						let code = node["code"].as_str().unwrap();
						shader_string.push_str(code);
					} else {
						let return_type = node["data_type"].as_str().unwrap();
	
						shader_string.push_str(format!("{return_type} {name}() {{").as_str());
	
						let statements = &node["statements"];
	
						for statement in statements.members() {
							shader_string.push_str("\n\t");
							for token in statement.members() {
								shader_string.push_str(token.as_str().unwrap());
							}
							shader_string.push_str(";");
						}

						for entry in node.entries() {
							process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
						}
	
						shader_string.push_str("\n}");
					}


					program_state.nodes.push((name.to_string(), shader_string));
				}
				"member" => {
					let source_data_type = node["data_type"].as_str().unwrap();
					let mut data_type = translate_type(source_data_type);

					if source_data_type.ends_with('*') {
						data_type.push_str("Pointer");
					}

					if let Some(shader_string) = string {
						shader_string.push_str(format!(" {data_type} {name};").as_str());
					}
				}
				"push_constant" => {
					let mut shader_string = if let Some(pc) = program_state.nodes.iter().find(|n| n.0 == "push_constant") {
						pc.1.clone()
					} else {
						program_state.nodes.push(("push_constant".to_string(), String::new()));
						format!("layout(push_constant) uniform push_constants {{")
					};

					program_state.push_constants += 1;

					for entry in node.entries() {
						process_node(Some(&mut shader_string), entry, compilation_settings, program_state);
					}

					program_state.nodes.iter_mut().find(|n| n.0 == "push_constant").unwrap().1 = shader_string;
				}
				_ => {
					println!("Unknown node type: {}", node_type);

					for entry in node.entries() {
						process_node(None, entry, compilation_settings, program_state);
					}
				}
			}
		}

		let mut program_state = ProgramState {
			in_position: 0, out_position: 0, push_constants: 0,
			nodes,
		};

		process_node(None, root, &compilation_settings, &mut program_state);

		if let Some(pc) = program_state.nodes.iter_mut().find(|n| n.0 == "push_constant") {
			pc.1.push_str(" } pc;\n");
		}

		fn order(node: &(String, String)) -> u32 {
			match node.0.as_str() {
				"push_constant" => 1,
				"main" => 2,
				_ => 0,
			}
		}

		program_state.nodes.sort_by(|a, b| order(a).cmp(&order(b)));

		{
			let mut shader_string = String::with_capacity(1024);
			program_state.nodes.iter().for_each(|n| shader_string.push_str(n.1.as_str()));
			shader_string
		}
	}
}

/// Translates a type from the json format to the glsl format.
fn translate_type(value: &str) -> String {
	let mut r = String::from(value);

	if r.ends_with('*') {
		r.pop();
	}

	match r.as_str() {
		"i32" => "int",
		"u32" => "uint",
		"f32" => "float",
		"vec3f" => "vec3",
		"vec4f" => "vec4",
		"mat3f" => "mat3",
		"mat4f" => "mat4",
		"mat3x4f" => "mat4x3",
		"mat4x3f" => "mat3x4",
		"mat3x4" => "mat4x3",
		"mat4x3" => "mat3x4",
		_ => r.as_str()
	}.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
fn test_translate_type() {
	assert_eq!(translate_type("i32"), "int");
	assert_eq!(translate_type("u32"), "uint");
	assert_eq!(translate_type("f32"), "float");
	assert_eq!(translate_type("vec3f"), "vec3");
	assert_eq!(translate_type("vec4f"), "vec4");
	assert_eq!(translate_type("vec3"), "vec3");
	assert_eq!(translate_type("vec4"), "vec4");
	assert_eq!(translate_type("mat3f"), "mat3");
	assert_eq!(translate_type("mat4f"), "mat4");
	assert_eq!(translate_type("mat3"), "mat3");
	assert_eq!(translate_type("mat4"), "mat4");
	assert_eq!(translate_type("mat3x4f"), "mat4x3");
	assert_eq!(translate_type("mat4x3f"), "mat3x4");
	assert_eq!(translate_type("mat3x4"), "mat4x3");
	assert_eq!(translate_type("mat4x3"), "mat3x4");
}

#[test]
fn test_generate_no_shader() {
	let shader_generator = ShaderGenerator::new();
}

#[test]
fn test_generate_vertex_shader() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	gl_Position = vec4(in_position, 1.0);
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { type: "vertex", vertex_layout:[{ type: "vec3", name: "in_position" }] } });

	let final_shader =
"#version 450 core
#pragma shader_stage(vertex)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(location=0) in vec3 in_position;
void main() {
	gl_Position = vec4(in_position, 1.0);
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_generate_fragment_shader() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	out_color = in_color;
}
";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { type: "fragment", input:[{ type:"vec4", name:"in_color" }], output:[{ type:"vec4", name:"out_color" }] } });

	let final_shader =
"#version 450 core
#pragma shader_stage(fragment)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(location=0) in vec4 in_color;
layout(location=0) out vec4 out_color;
void main() {
	out_color = in_color;
}
";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_generate_compute_shader() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { type: "compute", local_size: [1, 1, 1] } });

	let final_shader =
"#version 450 core
#pragma shader_stage(compute)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
#extension GL_KHR_shader_subgroup_basic : enable
#extension GL_KHR_shader_subgroup_arithmetic  : enable
#extension GL_KHR_shader_subgroup_ballot : enable
#extension GL_KHR_shader_subgroup_shuffle : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(local_size_x=1, local_size_y=1, local_size_z=1) in;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_generate_shader_no_version() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { shader: { type: "compute", local_size: [1, 1, 1] } });

	let final_shader =
"#version 450 core
#pragma shader_stage(compute)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
#extension GL_KHR_shader_subgroup_basic : enable
#extension GL_KHR_shader_subgroup_arithmetic  : enable
#extension GL_KHR_shader_subgroup_ballot : enable
#extension GL_KHR_shader_subgroup_shuffle : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(local_size_x=1, local_size_y=1, local_size_z=1) in;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_generate_shader_no_shader_type() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { local_size: [1, 1, 1] } });

	let final_shader =
"#version 450 core
#define BE_UNKNOWN_SHADER_TYPE
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_push_constant() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { type: "vertex", vertex_layout:[{ type: "vec3", name: "in_position" }], push_constants: [{ type: "mat4", name: "model_matrix" }] } });

	let final_shader =
"#version 450 core
#pragma shader_stage(vertex)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(push_constant) uniform push_constants {
mat4 model_matrix;
} pc;
layout(location=0) in vec3 in_position;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_descriptor_sets() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { descriptor_sets:[{ set: 0, binding: 0, value: { type: "sampler2D", name: "texture1" } }, { set: 0, binding: 1, value: { type: "sampler2D", name: "texture2" } }, { set: 1, binding: 0, value: { type: "sampler2D", name: "texture3" } }, { set: 3, binding: 0, value: { type: "sampler2D", name: "texture4" } }] } });

	let final_shader =
"#version 450 core
#define BE_UNKNOWN_SHADER_TYPE
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(set=0, binding=0) uniform sampler2D texture1;
layout(set=0, binding=1) uniform sampler2D texture2;
layout(set=1, binding=0) uniform sampler2D texture3;
layout(set=3, binding=0) uniform sampler2D texture4;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_descriptor_sets_and_push_constants() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { descriptor_sets:[{ set: 0, binding: 0, value: { type: "sampler2D", name: "texture1" } }, { set: 0, binding: 1, value: { type: "sampler2D", name: "texture2" } }, { set: 1, binding: 0, value: { type: "sampler2D", name: "texture3" } }, { set: 3, binding: 0, value: { type: "sampler2D", name: "texture4" } }], push_constants: [{ type: "mat4", name: "model_matrix" }] } });

	let final_shader =
"#version 450 core
#define BE_UNKNOWN_SHADER_TYPE
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
layout(set=0, binding=0) uniform sampler2D texture1;
layout(set=0, binding=1) uniform sampler2D texture2;
layout(set=1, binding=0) uniform sampler2D texture3;
layout(set=3, binding=0) uniform sampler2D texture4;
layout(push_constant) uniform push_constants {
mat4 model_matrix;
} pc;
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

#[test]
fn test_struct_declarations() {
	let shader_generator = ShaderGenerator::new();

	let base_shader =
"void main() {
	return;
}";

	let shader = shader_generator.generate_shader(base_shader, json::object! { glsl: { version: "450" }, shader: { structs: [{ name: "Light", members: [{ type: "vec3", name: "position" }, { type: "vec3", name: "color" }] }] } });

	let final_shader =
"#version 450 core
#define BE_UNKNOWN_SHADER_TYPE
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
struct Light {
vec3 position;
vec3 color;
};
layout(buffer_reference,scalar,buffer_reference_align=2) buffer LightPointer {
vec3 position;
vec3 color;
};
void main() {
	return;
}";

	assert_eq!(shader, final_shader);
}

	#[test]
	fn test_generate() {
		let shader_generator = ShaderGenerator::new();

		let program_spec = json::object! {
			glsl: { version: "450" },
			root: {
				Common: {
					type: "scope",
					Camera: {
						type: "struct",
						__only_under: "vertex",
						view_projection: {
							type: "member",
							data_type: "mat4f"
						}
					},
					push_constant: {
						type: "push_constant",
						vp_matrix: {
							type: "member",
							data_type: "mat4f"
						}
					},
					Forward:{
						type: "scope",
						buffer: {
							type: "descriptor",
							set: 0, binding: 0,
							buffer: {
								type: "member",
								data_type: "Camera"
							}
						},
						Light: {
							type: "struct",
							__only_under: "fragment",
							position: {
								type: "member",
								data_type: "vec3f"
							},
							color: {
								type: "member",
								data_type: "vec3f"
							}
						},
						MyShader: {
							type: "scope",
							push_constant: {
								type: "push_constant",
								model_matrix: {
									type: "member",
									data_type: "mat4f"
								}
							},
							Vertex: {
								type: "scope",
								__only_under: "vertex",
								in_Position: {
									type: "in",
									in_Position: {
										type: "member",
										data_type: "vec3f",
									}
								},
								in_Normal: {
									type: "in",
									in_Normal: {
										type: "member",
										data_type: "vec3f",
									}
								},
								out_InstanceIndex: {
									type: "out",
									out_InstanceIndex: {
										type: "member",
										data_type: "u32",
									},
									interpolation: "flat"
								},
								main: {
									type: "function",
									data_type: "void",
									statements: [
										["gl_Position", "=", "vec4", "(", "0", ",", "0", ",", "0", ",", "1", ")", ";"]
									]
								}
							},
							Fragment: {
								type: "scope",
								__only_under: "fragment",
								in_InstanceIndex: {
									type: "in",
									in_InstanceIndex: {
										type: "member",
										data_type: "u32"
									},
									interpolation: "flat"
								},
								out_Color: {
									type: "out",
									out_Color: {
										type: "member",
										data_type: "vec4f"
									}
								},
								main: {
									type: "function",
									data_type: "void",
									statements: [
										["out_Color", "=", "vec4", "(", "0", ",", "0", ",", "0", ",", "1", ")", ";"]
									]
								}
							}
						}
					}
				}
			}
		};

		let generated_vertex_shader = shader_generator.generate(&program_spec, &json::object!{ path: "Common.Forward.MyShader", type: "vertex" });

		let expected_vertex_shader_string =
"#version 450 core
#pragma shader_stage(vertex)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
struct Camera { mat4 view_projection; };
layout(buffer_reference,scalar,buffer_reference_align=2) buffer CameraPointer { mat4 view_projection; }
layout(location=0) in vec3 in_Position;
layout(location=1) in vec3 in_Normal;
layout(location=0) out flat uint out_InstanceIndex;
layout(push_constant) uniform push_constants { mat4 vp_matrix; mat4 model_matrix; } pc;
void main() {
	gl_Position=vec4(0,0,0,1);
}";

		println!("{}", &generated_vertex_shader);

		//assert_eq!(generated_vertex_shader, expected_vertex_shader_string);

		shaderc::Compiler::new().unwrap().compile_into_spirv(generated_vertex_shader.as_str(), shaderc::ShaderKind::Vertex, "shader.glsl", "main", None).unwrap();

		let expected_fragment_shader_string =
"#version 450 core
#pragma shader_stage(fragment)
#extension GL_EXT_shader_16bit_storage : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
#extension GL_EXT_nonuniform_qualifier : enable
#extension GL_EXT_scalar_block_layout : enable
#extension GL_EXT_buffer_reference : enable
#extension GL_EXT_buffer_reference2 : enable
#extension GL_EXT_shader_image_load_formatted : enable
layout(row_major) uniform; layout(row_major) buffer;
struct Light { vec3 position; vec3 color; };
layout(buffer_reference,scalar,buffer_reference_align=2) buffer LightPointer { vec3 position; vec3 color; }
layout(location=0) in flat uint in_InstanceIndex;
layout(location=0) out vec4 out_Color;
layout(push_constant) uniform push_constants { mat4 vp_matrix; mat4 model_matrix; } pc;
void main() {
	out_Color=vec4(0,0,0,1);
}";

		let generated_fragment_shader = shader_generator.generate(&program_spec, &json::object!{ path: "Common.Forward.MyShader", type: "fragment" });

		println!("{}", &generated_fragment_shader);

		//assert_eq!(generated_fragment_shader, expected_fragment_shader_string);

		shaderc::Compiler::new().unwrap().compile_into_spirv(generated_fragment_shader.as_str(), shaderc::ShaderKind::Fragment, "shader.glsl", "main", None).unwrap();
	}
}