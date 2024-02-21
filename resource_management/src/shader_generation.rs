use std::{cell::RefCell, collections::{HashMap, HashSet}, ops::Deref};

struct ShaderGenerator {
	minified: bool,
}

impl ShaderGenerator {
	pub fn new() -> Self {
		ShaderGenerator {
			minified: false,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	pub fn compilation(&self) -> ShaderCompilation {
		ShaderCompilation {
			minified: self.minified,
			present_symbols: HashSet::new(),
		}
	}
}

pub struct GLSLSettings {
	version: String,
}

pub struct ShaderGenerationSettings {
	glsl: GLSLSettings,
	stage: String,
}

struct ShaderCompilation {
	minified: bool,
	present_symbols: HashSet<jspd::NodeReference>,
}

impl ShaderCompilation {
	pub fn generate_shader(&mut self, main_function_node: &jspd::NodeReference) -> String {
		// let mut string = shader_generator::generate_glsl_header_block(&shader_generator::ShaderGenerationSettings::new("Compute"));

		let mut string = String::with_capacity(2048);
	
		self.generate_shader_internal(&mut string, main_function_node);
	
		string
	}

	pub fn generate_glsl_shader(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &jspd::NodeReference) -> String {
		// let mut string = shader_generator::generate_glsl_header_block(&shader_generator::ShaderGenerationSettings::new("Compute"));
		let mut string = String::with_capacity(2048);
	
		self.generate_glsl_header_block(&mut string, shader_compilation_settings);

		self.generate_shader_internal(&mut string, main_function_node);
	
		string
	}

	fn generate_shader_internal(&mut self, string: &mut String, main_function_node: &jspd::NodeReference) {
		if self.present_symbols.contains(main_function_node) { return; }

		let node = RefCell::borrow(&main_function_node);
	
		match node.node() {
			jspd::Nodes::Null => {}
			jspd::Nodes::Scope { children, .. } => {
				for child in children {
					self.generate_shader_internal(string, &child,);
				}
			}
			jspd::Nodes::Function { name, statements, return_type, .. } => {
				let mut l_string = String::with_capacity(128);

				self.generate_shader_internal(&mut l_string, &return_type);

				l_string.push_str("void ");

				l_string.push_str(name);

				if self.minified { l_string.push_str("(){"); } else { l_string.push_str("() {\n"); }
	
				for statement in statements {
					if !self.minified { l_string.push('\t'); }
					self.generate_shader_internal(&mut l_string, &statement,);
					if !self.minified { l_string.push_str(";\n"); } else { l_string.push(';'); }
				}
				
				l_string.push_str("}\n");

				string.insert_str(0, &l_string);

				self.present_symbols.insert(main_function_node.clone());
			}
			jspd::Nodes::Struct { name, fields, .. } => {
				if name == "void" || name == "vec2f" || name == "vec3f" || name == "vec4f" || name == "mat2f" || name == "mat3f" || name == "mat4f" || name == "f32" || name == "u32" || name == "i32" { return; }

				let mut l_string = String::with_capacity(128);

				l_string.push_str("struct ");
				l_string.push_str(name.as_str());

				if self.minified { l_string.push('{'); } else { l_string.push_str(" {\n"); }

				for field in fields {
					if !self.minified { l_string.push('\t'); }
					self.generate_shader_internal(&mut l_string, &field,);
					if self.minified { l_string.push(';') } else { l_string.push_str(";\n"); }
				}

				l_string.push_str("}\n");

				string.insert_str(0, &l_string);

				self.present_symbols.insert(main_function_node.clone());
			}
			jspd::Nodes::Member { name, r#type } => {
				self.generate_shader_internal(string, &r#type); // Demand the type to be present in the shader
				if let Some(type_name) = r#type.borrow().get_name() {
					string.push_str(type_name.as_str());
					string.push(' ');
				}
				string.push_str(name.as_str());
			}
			jspd::Nodes::GLSL { code } => {
				string.push_str(code);
			}
			jspd::Nodes::Expression(expression) => {
				match expression {
					jspd::Expressions::Operator { operator, left, right } => {
						if operator == &jspd::Operators::Assignment {
							self.generate_shader_internal(string, &left,);
							if self.minified { string.push('=') } else { string.push_str(" = "); }
							self.generate_shader_internal(string, &right,);
						}
					}
					jspd::Expressions::FunctionCall { parameters, function, .. } => {
						self.generate_shader_internal(string, &function);

						let function = RefCell::borrow(&function);
						let name = function.get_name().unwrap();

						string.push_str(&format!("{}(", name));
						for (i, parameter) in parameters.iter().enumerate() {
							if i > 0 {
								if self.minified { string.push(',') } else { string.push_str(", "); }
							}

							self.generate_shader_internal(string, &parameter,);
						}
						string.push_str(&format!(")"));
					}
					jspd::Expressions::Member { name, source, .. } => {
						if let Some(source) = source {
							self.generate_shader_internal(string, &source);
						}
						
						string.push_str(name);
					}
					jspd::Expressions::VariableDeclaration { name, r#type } => {
						string.push_str(&format!("{} {}", r#type, name));
					}
					jspd::Expressions::Literal { value } => {
						string.push_str(&format!("{}", value));
					}
					jspd::Expressions::Return => {
						string.push_str("return");
					}
					jspd::Expressions::Accessor { left, right } => {
						self.generate_shader_internal(string, &left,);
						string.push('.');
						self.generate_shader_internal(string, &right,);
					}
				}
			}
			jspd::Nodes::Binding { name, set, binding, read, write, .. } => {
				let mut l_string = String::with_capacity(128);
				l_string.push_str(&format!("layout(set={}, binding={}) uniform ", set, binding));
				if *read && !*write { l_string.push_str("readonly "); }
				if *write && !*read { l_string.push_str("writeonly "); }
				l_string.push_str(&name);
				if !self.minified { l_string.push_str(";\n"); } else { l_string.push(';'); }
				string.insert_str(0, &l_string);
			}
		}
	}

	fn generate_glsl_header_block(&self, glsl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		let glsl_version = &compilation_settings.glsl.version;
	
		glsl_block.push_str(&format!("#version {glsl_version} core\n"));
	
		// shader type
	
		let shader_stage = compilation_settings.stage.as_str();
	
		match shader_stage {
			"Vertex" => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			"Fragment" => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			"Compute" => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			"Mesh" => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
			_ => glsl_block.push_str("#define BE_UNKNOWN_SHADER_TYPE\n")
		}
	
		// extensions
	
		glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
		glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
		glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
		glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");
	
		match shader_stage {
			"Compute" => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_arithmetic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_shuffle:enable\n");
			}
			"Mesh" => {
				glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
			}
			_ => {}
		}
		// memory layout declarations
	
		glsl_block.push_str("layout(row_major) uniform; layout(row_major) buffer;\n");
	
		glsl_block.push_str("const float PI = 3.14159265359;");
	}
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, ops::Deref};

    use crate::shader_generation::ShaderGenerator;

	#[test]
	fn empty_script() {
		let script = r#"
		"#;

		let script_node = jspd::compile_to_jspd(&script, None).unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&script_node);

		println!("{}", shader);
	}

	#[test]
	fn binding() {
		let script = r#"
		main: fn () -> void {
			buffer;
		}
		"#;

		let root_node = jspd::Node::scope("root".to_string(), vec![jspd::Node::binding("buffer".to_string(), 0, 0, true, false)]);

		let script_node = jspd::compile_to_jspd(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		println!("{}", shader);
	}

	#[test]
	fn fragment_shader() {
		let script = r#"
		main: fn () -> void {
			albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let script_node = jspd::compile_to_jspd(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main() {\n\tvec3f albedo = vec3f(1.0, 0.0, 0.0);\n}\n");

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main(){vec3f albedo=vec3f(1.0,0.0,0.0);}\n");
	}

	#[test]
	fn cull_unused_functions() {
		let script = r#"
		used_by_used: fn () -> void {}
		used: fn() -> void {
			used_by_used();
		}
		not_used: fn() -> void {}

		main: fn () -> void {
			used();
		}
		"#;

		let main_function_node = jspd::compile_to_jspd(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void used_by_used() {\n}\nvoid used() {\n\tused_by_used();\n}\nvoid main() {\n\tused();\n}\n");
	}

	#[test]
	fn structure() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		use_vertex: fn () -> Vertex {}

		main: fn () -> void {
			use_vertex();
		}
		"#;

		let main_function_node = jspd::compile_to_jspd(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "struct Vertex {\n\tvec3f position;\n\tvec3f normal;\n}\nvoid use_vertex() {\n}\nvoid main() {\n\tuse_vertex();\n}\n");
	}
}