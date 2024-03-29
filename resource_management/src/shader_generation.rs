use std::{cell::RefCell, collections::HashSet};

pub struct ShaderGenerator {
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

impl Default for GLSLSettings {
	fn default() -> Self {
		Self {
			version: "450".to_string(),
		}
	}
}

pub struct ShaderGenerationSettings {
	glsl: GLSLSettings,
	stage: String,
}

impl ShaderGenerationSettings {
	pub fn new(stage: &str) -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: stage.to_string() }
	}
}

pub struct ShaderCompilation {
	minified: bool,
	present_symbols: HashSet<jspd::NodeReference>,
}

impl ShaderCompilation {
	pub fn generate_shader(&mut self, main_function_node: &jspd::NodeReference) -> String {
		let mut string = String::with_capacity(2048);
	
		self.generate_shader_internal(&mut string, main_function_node);
	
		string
	}

	pub fn generate_glsl_shader(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &jspd::NodeReference) -> String {
		let mut string = String::with_capacity(2048);
		
		if !matches!(main_function_node.borrow().node(), jspd::Nodes::Function { .. }) {
			panic!("GLSL shader generation requires a function node as the main function.");
		}

		self.generate_shader_internal(&mut string, main_function_node);
		
		{
			let mut glsl_block = String::with_capacity(248);
			self.generate_glsl_header_block(&mut glsl_block, shader_compilation_settings);
			glsl_block.push_str("layout(local_size_x=32) in;\n");
			string.insert_str(0, &glsl_block);
		}
	
		string
	}

	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f" => "vec2",
			"vec2u16" => "u16vec2",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			_ => source,
		}
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
			jspd::Nodes::Function { name, statements, return_type, params, .. } => {
				let mut l_string = String::with_capacity(128);

				self.generate_shader_internal(&mut l_string, &return_type);

				l_string.push_str(Self::translate_type(&return_type.borrow().get_name().unwrap()));

				l_string.push(' ');

				l_string.push_str(name);

				l_string.push('(');

				for (i, param) in params.iter().enumerate() {
					if i > 0 {
						if !self.minified { l_string.push_str(", "); } else { l_string.push(','); }
					}

					self.generate_shader_internal(&mut l_string, &param);
				}

				if self.minified { l_string.push_str("){"); } else { l_string.push_str(") {\n"); }
	
				for statement in statements {
					if !self.minified { l_string.push('\t'); }
					self.generate_shader_internal(&mut l_string, &statement,);
					if !self.minified { l_string.push_str(";\n"); } else { l_string.push(';'); }
				}
				
				if self.minified { l_string.push('}') } else { l_string.push_str("}\n"); }

				string.insert_str(0, &l_string);

				self.present_symbols.insert(main_function_node.clone());
			}
			jspd::Nodes::Struct { name, fields, .. } => {
				if name == "void" || name == "vec2u16" || name == "vec2f" || name == "vec3f" || name == "vec4f" || name == "mat2f" || name == "mat3f" || name == "mat4f" || name == "f32" || name == "u8" || name == "u16" || name == "u32" || name == "i32" { return; }

				let mut l_string = String::with_capacity(128);

				l_string.push_str("struct ");
				l_string.push_str(name.as_str());

				if self.minified { l_string.push('{'); } else { l_string.push_str(" {\n"); }

				for field in fields {
					if !self.minified { l_string.push('\t'); }
					self.generate_shader_internal(&mut l_string, &field,);
					if self.minified { l_string.push(';') } else { l_string.push_str(";\n"); }
				}

				l_string.push_str("};");

				if !self.minified { l_string.push('\n'); }

				string.insert_str(0, &l_string);

				self.present_symbols.insert(main_function_node.clone());
			}
			jspd::Nodes::PushConstant { members } => {
				let mut l_string = String::with_capacity(128);

				l_string.push_str("layout(push_constant) uniform PushConstant {");

				if !self.minified { l_string.push('\n'); }

				for member in members {
					if !self.minified { l_string.push('\t'); }
					self.generate_shader_internal(&mut l_string, &member,);
					if self.minified { l_string.push(';') } else { l_string.push_str(";\n"); }
				}

				l_string.push_str("} push_constant;");

				if !self.minified { l_string.push('\n'); }

				string.insert_str(0, &l_string);
			}
			jspd::Nodes::Specialization { name, r#type } => {
				let mut l_string = String::with_capacity(128);

				let mut members = Vec::new();

				let t = &r#type.borrow().get_name().unwrap();
				let type_name = Self::translate_type(t);

				match r#type.borrow().node() {
					jspd::Nodes::Struct { fields, .. } => {
						for (i, field) in fields.iter().enumerate() {
							match field.borrow().node() {
								jspd::Nodes::Member { name: member_name, r#type, .. } => {
									let member_name = format!("{}_{}", name, {member_name});
									l_string.push_str(&format!("layout(constant_id={}) const {} {} = {};\n", i, Self::translate_type(&r#type.borrow().get_name().unwrap()), &member_name, "1.0"));
									members.push(member_name);
								}
								_ => {}
							}
						}
					}
					_ => {}
				}

				l_string.push_str(&format!("const {} {} = {};\n", &type_name, name, format!("{}({})", &type_name, members.join(","))));

				string.insert_str(0, &l_string);
			}
			jspd::Nodes::Member { name, r#type, count } => {
				self.generate_shader_internal(string, &r#type); // Demand the type to be present in the shader
				if let Some(type_name) = r#type.borrow().get_name() {
					let type_name = Self::translate_type(type_name.as_str());

					string.push_str(type_name);
					string.push(' ');
				}
				string.push_str(name.as_str());
				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}
			}
			jspd::Nodes::GLSL { code, input, .. } => {
				for reference in input {
					self.generate_shader_internal(string, reference);
				}

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

						let name = Self::translate_type(&name);

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
							match source.borrow().node() {
								jspd::Nodes::Expression { .. } => {}
								_ => {
									self.generate_shader_internal(string, &source);
								}
							}
						}
						
						string.push_str(name);
					}
					jspd::Expressions::VariableDeclaration { name, r#type } => {
						self.generate_shader_internal(string, r#type);

						string.push_str(&format!("{} {}", Self::translate_type(&r#type.borrow().get_name().unwrap()), name));
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
			jspd::Nodes::Binding { name, set, binding, read, write, r#type, count, .. } => {
				let mut l_string = String::with_capacity(128);

				let binding_type = match r#type {
					jspd::BindingTypes::Buffer{ .. } => "buffer",
					jspd::BindingTypes::Image{ format, .. } => {
						match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uniform uimage2D",
							_ => "uniform image2D"
						}
					},
					jspd::BindingTypes::CombinedImageSampler => "uniform sampler2D",
				};

				l_string.push_str(&format!("layout(set={},binding={}", set, binding));

				match r#type {
					jspd::BindingTypes::Buffer{ .. } => {
						l_string.push_str(",scalar");
					}
					jspd::BindingTypes::Image { format } => {
						l_string.push(',');
						l_string.push_str(&format);
					}
					jspd::BindingTypes::CombinedImageSampler => {}
				}

				match r#type {
					jspd::BindingTypes::Buffer{ .. } | jspd::BindingTypes::Image { .. } => {
						l_string.push_str(&format!(") {}{} ", if *read && !*write { "readonly " } else if *write && !*read { "writeonly " } else { "" }, binding_type));
					}
					jspd::BindingTypes::CombinedImageSampler => {
						l_string.push_str(&format!(") {} ", binding_type));
					}
				}

				match r#type {
					jspd::BindingTypes::Buffer{ r#type } => {						
						match RefCell::borrow(&r#type).node() {
							jspd::Nodes::Struct { name, fields, .. } => {
								l_string.push_str(&name);
								l_string.push('{');

								if !self.minified { l_string.push('\n'); }

								for field in fields {
									if !self.minified { l_string.push('\t'); }
									self.generate_shader_internal(&mut l_string, &field,);
									if self.minified { l_string.push(';') } else { l_string.push_str(";\n"); }
								}

								l_string.push('}');
							}
							_ => { panic!("Need struct node type for buffer binding type."); }
						}
					}
					_ => {}
				}

				l_string.push_str(&name);

				if let Some(count) = count {
					l_string.push('[');
					l_string.push_str(count.to_string().as_str());
					l_string.push(']');
				}

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
		glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;");

		glsl_block.push_str("const float PI = 3.14159265359;\n");

		if !self.minified { glsl_block.push('\n'); }
	}
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

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
	fn bindings() {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let buffer_type = jspd::Node::r#struct("BufferType", vec![]).into();

		let mut root_node = jspd::Node::scope("root".to_string());
		
		root_node.add_children(vec![
			jspd::Node::binding("buff", jspd::BindingTypes::buffer(buffer_type), 0, 0, true, true).into(),
			jspd::Node::binding("image", jspd::BindingTypes::Image{ format: "r8".to_string() }, 0, 1, false, true).into(),
			jspd::Node::binding("texture", jspd::BindingTypes::CombinedImageSampler, 1, 0, true, false).into(),
		]);

		let script_node = jspd::compile_to_jspd(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "layout(set=1,binding=0) uniform sampler2D texture;layout(set=0,binding=1,r8) writeonly uniform image2D image;layout(set=0,binding=0,scalar) buffer BufferType{}buff;void main(){buff;image;texture;}");
	}

	#[test]
	fn fragment_shader() {
		let script = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let script_node = jspd::compile_to_jspd(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main() {\n\tvec3 albedo = vec3(1.0, 0.0, 0.0);\n}\n");

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);}");
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

		assert_eq!(shader, "struct Vertex {\n\tvec3 position;\n\tvec3 normal;\n};\nVertex use_vertex() {\n}\nvoid main() {\n\tuse_vertex();\n}\n");
	}

	#[test]
	fn push_constant() {
		let script = r#"
		main: fn () -> void {
			push_constant;
		}
		"#;

		let mut root_node = jspd::Node::root();

		let u32_t = root_node.get_child("u32").unwrap();
		root_node.add_child(jspd::Node::push_constant(vec![jspd::Node::member("material_id", u32_t.clone()).into()]).into());

		let program_node = jspd::compile_to_jspd(&script, Some(root_node)).unwrap();

		let main_node = RefCell::borrow(&program_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main_node);

		assert_eq!(shader, "layout(push_constant) uniform PushConstant {\n\tuint32_t material_id;\n} push_constant;\nvoid main() {\n\tpush_constant;\n}\n");
	}
}