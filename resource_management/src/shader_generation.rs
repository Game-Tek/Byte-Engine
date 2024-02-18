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

	fn generate_shader_internal(&mut self, string: &mut String, main_function_node: &jspd::NodeReference) {
		if self.present_symbols.contains(main_function_node) { return; }

		let node = RefCell::borrow(&main_function_node);
	
		match node.node() {
			jspd::Nodes::Null => {}
			jspd::Nodes::Scope { name: _, children } => {
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
					jspd::Expressions::FunctionCall { name, parameters, function, .. } => {
						self.generate_shader_internal(string, &function);

						string.push_str(&format!("{}(", name));
						for (i, parameter) in parameters.iter().enumerate() {
							if i > 0 {
								if self.minified { string.push(',') } else { string.push_str(", "); }
							}

							self.generate_shader_internal(string, &parameter,);
						}
						string.push_str(&format!(")"));
					}
					jspd::Expressions::Member { name } => {
						string.push_str(name);
					}
					jspd::Expressions::VariableDeclaration { name, r#type } => {
						string.push_str(&format!("{} {}", r#type, name));
					}
					jspd::Expressions::Literal { value } => {
						string.push_str(&format!("{}", value));
					}
					_ => panic!("Invalid expression")
				}
			}
		}
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

		let main_function_node = jspd::compile_to_jspd(&script).unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main_function_node);

		println!("{}", shader);
	}

	#[test]
	fn fragment_shader() {
		let script = r#"
		main: fn () -> void {
			albedo: vec3 = vec3(1.0, 0.0, 0.0);
		}
		"#;

		let main_function_node = jspd::compile_to_jspd(&script).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main() {\n\tvec3 albedo = vec3(1.0, 0.0, 0.0);\n}\n");

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);}\n");
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

		let main_function_node = jspd::compile_to_jspd(&script).unwrap();

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

		let main_function_node = jspd::compile_to_jspd(&script).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "struct Vertex {\n\tvec3f position;\n\tvec3f normal;\n}\nvoid use_vertex() {\n}\nvoid main() {\n\tuse_vertex();\n}\n");
	}
}