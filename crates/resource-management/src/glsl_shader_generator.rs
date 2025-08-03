use std::{cell::RefCell, collections::{HashMap, HashSet}};

use crate::shader_generator::{MatrixLayouts, ShaderGenerationSettings, ShaderGenerator, Stages};

/// Shader generator.
///
/// # Parameters
///
/// - *minified*: Controls wheter the shader string output is minified. Is `true` by default in release builds.
pub struct GLSLShaderGenerator {
	minified: bool,
}

impl ShaderGenerator for GLSLShaderGenerator {}

impl GLSLShaderGenerator {
	/// Creates a new ShaderGenerator.
	pub fn new() -> Self {
		GLSLShaderGenerator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}

#[derive(Clone, Debug)]
struct Graph {
	set: HashMap<besl::NodeReference, Vec<besl::NodeReference>>,
}

impl Graph {
	pub fn new() -> Self {
		Graph {
			set: HashMap::with_capacity(1024),
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set.entry(from).or_insert(Vec::new()).push(to);
	}
}

fn topological_sort(graph: &Graph) -> Vec<besl::NodeReference> {
	let mut visited = HashSet::new();
	let mut stack = Vec::new();

	for (node, _) in graph.set.iter() {
		if !visited.contains(node) {
			topological_sort_util(node.clone(), graph, &mut visited, &mut stack);
		}
	}

	fn topological_sort_util(node: besl::NodeReference, graph: &Graph, visited: &mut HashSet<besl::NodeReference>, stack: &mut Vec<besl::NodeReference>) {
		visited.insert(node.clone());

		if let Some(neighbours) = graph.set.get(&node) {
			for neighbour in neighbours {
				if !visited.contains(neighbour) {
					topological_sort_util(neighbour.clone(), graph, visited, stack);
				}
			}
		}

		stack.push(node);
	}

	stack
}

impl GLSLShaderGenerator {
	/// Generates a GLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The settings for the shader compilation.
	/// * `main_function_node` - The main function node of the shader.
	///
	/// # Returns
	///
	/// The GLSL shader as a string.
	///
	/// # Panics
	///
	/// Panics if the main function node is not a function node.
	pub fn generate(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &besl::NodeReference) -> Result<String, ()> {
		let mut string = String::with_capacity(2048);

		if !matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }) {
			panic!("GLSL shader generation requires a function node as the main function.");
		}

		let graph = self.build_graph(main_function_node.clone());

		let order = topological_sort(&graph);
		// Generate only direct definition to non leaf nodes
		let order = order.into_iter().filter(|n| matches!(n.borrow().node(), besl::Nodes::Function { .. }) || matches!(n.borrow().node(), besl::Nodes::Struct { .. }) || matches!(n.borrow().node(), besl::Nodes::Binding { .. }) || matches!(n.borrow().node(), besl::Nodes::PushConstant { .. }) || matches!(n.borrow().node(), besl::Nodes::Specialization { .. }));

		self.generate_glsl_header_block(&mut string, shader_compilation_settings);

		for node in order {
			self.emit_node_string(&mut string, &node);
		}

		Ok(string)
	}

	/// Translates BESL intrinsic type names to GLSL type names.
	/// Example: `vec2f` -> `vec2`
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f" => "vec2",
			"vec2u" => "uvec2",
			"vec2i" => "ivec2",
			"vec2u16" => "u16vec2",
			"vec3u" => "uvec3",
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
			"Texture2D" => "in sampler2D",
			"ArrayTexture2D" => "in sampler2DArray",
			_ => source,
		}
	}

	fn build_graph(&mut self, main_function_node: besl::NodeReference) -> Graph {
		let mut graph = Graph::new();

		let node_borrow = RefCell::borrow(&main_function_node);
		let node_ref = node_borrow.node();

		match node_ref {
			besl::Nodes::Function { params, return_type, statements, name, .. } => {
				assert_eq!(name, "main");

				for p in params {
					self.build_graph_inner(main_function_node.clone(), p.clone(), &mut graph);
				}

				for statement in statements {
					self.build_graph_inner(main_function_node.clone(), statement.clone(), &mut graph);
				}

				self.build_graph_inner(main_function_node.clone(), return_type.clone(), &mut graph);
			}
			_ => panic!("Root node must be a function node."),
		}

		graph
	}

	fn build_graph_inner(&mut self, parent: besl::NodeReference, node: besl::NodeReference, graph: &mut Graph) -> () {
		graph.add(parent, node.clone());

		let node_borrow = RefCell::borrow(&node);
		let node_ref = node_borrow.node();

		match node_ref {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { children, .. } => {
				for child in children {
					self.build_graph_inner(node.clone(), child.clone(), graph);
				}
			}
			besl::Nodes::Function { statements, params, return_type, .. } => {
				for parameter in params {
					self.build_graph_inner(node.clone(), parameter.clone(), graph);
				}

				for statement in statements {
					self.build_graph_inner(node.clone(), statement.clone(), graph);
				}

				self.build_graph_inner(node.clone(), return_type.clone(), graph);
			}
			besl::Nodes::Struct { fields, .. } => {
				for field in fields {
					self.build_graph_inner(node.clone(), field.clone(), graph);
				}
			}
			besl::Nodes::PushConstant { members } => {
				for member in members {
					self.build_graph_inner(node.clone(), member.clone(), graph);
				}
			}
			besl::Nodes::Specialization { r#type, .. } => {
				self.build_graph_inner(node.clone(), r#type.clone(), graph);
			}
			besl::Nodes::Member { r#type, .. } => {
				self.build_graph_inner(node.clone(), r#type.clone(), graph);
			}
			besl::Nodes::GLSL { input, output, .. } => {
				for reference in input {
					self.build_graph_inner(node.clone(), reference.clone(), graph);
				}

				for reference in output {
					self.build_graph_inner(node.clone(), reference.clone(), graph);
				}
			}
			besl::Nodes::Parameter { r#type, .. } => {
				self.build_graph_inner(node.clone(), r#type.clone(), graph);
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { operator, left, right } => {
						if operator == &besl::Operators::Assignment {
							self.build_graph_inner(node.clone(), left.clone(), graph);
							self.build_graph_inner(node.clone(), right.clone(), graph);
						}
					}
					besl::Expressions::FunctionCall { parameters, function, .. } => {
						self.build_graph_inner(node.clone(), function.clone(), graph);

						for parameter in parameters {
							self.build_graph_inner(node.clone(), parameter.clone(), graph);
						}
					}
					besl::Expressions::IntrinsicCall { elements: parameters, .. } => {
						for e in parameters {
							self.build_graph_inner(node.clone(), e.clone(), graph);
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							self.build_graph_inner(node.clone(), element.clone(), graph);
						}
					}
					besl::Expressions::Macro { body, .. } => {
						self.build_graph_inner(node.clone(), body.clone(), graph);
					}
					besl::Expressions::Member { source, .. } => {
						match source.borrow().node() {
							besl::Nodes::Expression { .. } => {}
							besl::Nodes::Literal { .. } => {
								self.build_graph_inner(node.clone(), source.clone(), graph);
							}
							besl::Nodes::Member { .. } => {}
							_ => {
								self.build_graph_inner(node.clone(), source.clone(), graph);
							}
						}
					}
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						self.build_graph_inner(node.clone(), r#type.clone(), graph);
					}
					besl::Expressions::Literal { .. } => {
						// self.build_graph_inner(node.clone(), value.clone(), graph);
					}
					besl::Expressions::Return => {}
					besl::Expressions::Accessor { left, right } => {
						self.build_graph_inner(node.clone(), left.clone(), graph);
						self.build_graph_inner(node.clone(), right.clone(), graph);
					}
				}
			}
			besl::Nodes::Binding { r#type, .. } => {
				match r#type {
					besl::BindingTypes::Buffer{ members } => {
						for member in members {
							self.build_graph_inner(node.clone(), member.clone(), graph);
						}
					}
					besl::BindingTypes::Image { .. } => {}
					besl::BindingTypes::CombinedImageSampler { .. } => {}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.build_graph_inner(node.clone(), element.clone(), graph);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.build_graph_inner(node.clone(), value.clone(), graph);
			}
		}
	}

	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(&this_node);

		match node.node() {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { .. } => {}
			besl::Nodes::Function { name, statements, return_type, params, .. } => {
				string.push_str(Self::translate_type(&return_type.borrow().get_name().unwrap()));

				string.push(' ');

				string.push_str(name);

				string.push('(');

				for (i, param) in params.iter().enumerate() {
					if i > 0 {
						if !self.minified { string.push_str(", "); } else { string.push(','); }
					}

					self.emit_node_string(string, param);
				}

				if self.minified { string.push_str("){"); } else { string.push_str(") {\n"); }

				for statement in statements {
					if !self.minified { string.push('\t'); }
					self.emit_node_string(string, &statement);
					if !self.minified { string.push_str(";\n"); } else { string.push(';'); }
				}

				if self.minified { string.push('}') } else { string.push_str("}\n"); }
			}
			besl::Nodes::Struct { name, fields, .. } => {
				if name == "void" || name == "vec2u16" || name == "vec2u" || name == "vec2i" || name == "vec2f" || name == "vec3f" || name == "vec4f" || name == "mat2f" || name == "mat3f" || name == "mat4f" || name == "f32" || name == "u8" || name == "u16" || name == "u32" || name == "i32" || name == "Texture2D" || name == "ArrayTexture2D" { return; }

				string.push_str("struct ");
				string.push_str(name.as_str());

				if self.minified { string.push('{'); } else { string.push_str(" {\n"); }

				for field in fields {
					if !self.minified { string.push('\t'); }
					self.emit_node_string(string, &field);
					if self.minified { string.push(';') } else { string.push_str(";\n"); }
				}

				string.push_str("};");

				if !self.minified { string.push('\n'); }
			}
			besl::Nodes::PushConstant { members } => {
				if self.minified {
					string.push_str("layout(push_constant)uniform PushConstant{");
				} else {
					string.push_str("layout(push_constant) uniform PushConstant {");
				}

				if !self.minified { string.push('\n'); }

				for member in members {
					if !self.minified { string.push('\t'); }
					self.emit_node_string(string, &member);
					if self.minified { string.push(';') } else { string.push_str(";\n"); }
				}

				if self.minified {
					string.push_str("}push_constant;");
				} else {
					string.push_str("} push_constant;");
				}

				if !self.minified { string.push('\n'); }
			}
			besl::Nodes::Specialization { name, r#type } => {
				let mut members = Vec::new();

				let r#type = r#type.borrow();

				let t = r#type.get_name().unwrap();
				let type_name = Self::translate_type(t);

				match r#type.node() {
					besl::Nodes::Struct { fields, .. } => {
						for (i, field) in fields.iter().enumerate() {
							match field.borrow().node() {
								besl::Nodes::Member { name: member_name, r#type, .. } => {
									let member_name = format!("{}_{}", name, {member_name});
									string.push_str(&format!("layout(constant_id={})const {} {}={};{}", i, Self::translate_type(&r#type.borrow().get_name().unwrap()), &member_name, "1.0f", if !self.minified { "\n" } else { "" }));
									members.push(member_name);
								}
								_ => {}
							}
						}
					}
					_ => {}
				}

				string.push_str(&format!("const {} {}={};{}", &type_name, name, format!("{}({})", &type_name, members.join(",")), if !self.minified { "\n" } else { "" }));
			}
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					let type_name = Self::translate_type(type_name);

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
			besl::Nodes::GLSL { code, .. } => {
				string.push_str(code);
			}
			besl::Nodes::Parameter { name, r#type } => {
				string.push_str(&format!("{} {}", Self::translate_type(&r#type.borrow().get_name().unwrap()), name));
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { operator, left, right } => {
						self.emit_node_string(string, &left);
						if operator == &besl::Operators::Assignment {
							if self.minified { string.push('=') } else { string.push_str(" = "); }
						}
						self.emit_node_string(string, &right);
					}
					besl::Expressions::FunctionCall { parameters, function, .. } => {
						let function = RefCell::borrow(&function);
						let name = function.get_name().unwrap();

						let name = Self::translate_type(&name);

						string.push_str(&format!("{}(", name));
						for (i, parameter) in parameters.iter().enumerate() {
							if i > 0 {
								if self.minified { string.push(',') } else { string.push_str(", "); }
							}
							self.emit_node_string(string, &parameter);
						}
						string.push_str(&format!(")"));
					}
					besl::Expressions::IntrinsicCall { elements: parameters, .. } => {
						for e in parameters {
							self.emit_node_string(string, &e);
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							self.emit_node_string(string, &element);
						}
					}
					besl::Expressions::Macro { .. } => {
					}
					besl::Expressions::Member { name, source, .. } => {
						match source.borrow().node() {
							besl::Nodes::Literal { value, .. } => {
								self.emit_node_string(string, &value);
							}
							_ => {
								string.push_str(name);
							}
						}
					}
					besl::Expressions::VariableDeclaration { name, r#type } => {
						string.push_str(&format!("{} {}", Self::translate_type(&r#type.borrow().get_name().unwrap()), name));
					}
					besl::Expressions::Literal { value } => {
						string.push_str(&value);
					}
					besl::Expressions::Return => {
						string.push_str("return");
					}
					besl::Expressions::Accessor { left, right } => {
						self.emit_node_string(string, &left);
						string.push('.');
						self.emit_node_string(string, &right);
					}
				}
			}
			besl::Nodes::Binding { name, set, binding, read, write, r#type, count, .. } => {
				let binding_type = match r#type {
					besl::BindingTypes::Buffer{ .. } => "buffer",
					besl::BindingTypes::Image{ format, .. } => {
						match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uniform uimage2D",
							_ => "uniform image2D"
						}
					},
					besl::BindingTypes::CombinedImageSampler { format } => {
						match format.as_str() {
							"ArrayTexture2D" => "uniform sampler2DArray",
							_ => "uniform sampler2D"
						}
					},
				};

				string.push_str(&format!("layout(set={},binding={}", set, binding));

				match r#type {
					besl::BindingTypes::Buffer{ .. } => {
						string.push_str(",scalar");
					}
					besl::BindingTypes::Image { format } => {
						string.push(',');
						string.push_str(&format);
					}
					besl::BindingTypes::CombinedImageSampler{ .. } => {}
				}

				match r#type {
					besl::BindingTypes::Buffer{ .. } | besl::BindingTypes::Image { .. } => {
						string.push_str(&format!(") {}{} ", if *read && !*write { "readonly " } else if *write && !*read { "writeonly " } else { "" }, binding_type));
					}
					besl::BindingTypes::CombinedImageSampler{ .. } => {
						string.push_str(&format!(") {} ", binding_type));
					}
				}

				match r#type {
					besl::BindingTypes::Buffer{ members } => {
						string.push_str(&format!("_{}{{", name));

						for member in members.iter() {
							self.emit_node_string(string, &member);
							if !self.minified { string.push_str(";\n"); } else { string.push(';'); }
						}

						string.push_str("}");
					}
					_ => {}
				}

				string.push_str(&name);

				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}

				if !self.minified { string.push_str(";\n"); } else { string.push(';'); }
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, &element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, &value);
			}
		}
	}

	fn generate_glsl_header_block(&self, glsl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		let glsl_version = &compilation_settings.glsl.version;

		glsl_block.push_str(&format!("#version {glsl_version} core\n"));

		// shader type

		match compilation_settings.stage {
			Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			Stages::Task => glsl_block.push_str("#pragma shader_stage(task)\n"),
			Stages::Mesh{ .. } => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
		}

		// extensions

		glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
		glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
		glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
		glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");

		match compilation_settings.stage {
			Stages::Compute { .. } => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_arithmetic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_shuffle:enable\n");
			}
			Stages::Mesh { maximum_vertices, maximum_primitives, .. } => {
				glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
				glsl_block.push_str(&format!("layout(location=0) perprimitiveEXT out uint out_instance_index[{}];\n", maximum_primitives));
				glsl_block.push_str(&format!("layout(location=1) perprimitiveEXT out uint out_primitive_index[{}];\n", maximum_primitives));
				glsl_block.push_str(&format!("layout(triangles,max_vertices={},max_primitives={}) out;\n", maximum_vertices, maximum_primitives));
			}
			_ => {}
		}

		// local_size
		match compilation_settings.stage {
			Stages::Compute { local_size } | Stages::Mesh { local_size, .. } => {
				glsl_block.push_str(&format!("layout(local_size_x={},local_size_y={},local_size_z={}) in;\n", local_size.width(), local_size.height(), local_size.depth()));
			}
			_ => {}
		}

		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;\n"),
			MatrixLayouts::ColumnMajor => glsl_block.push_str("layout(column_major) uniform;layout(column_major) buffer;\n"),
		}

		glsl_block.push_str("const float PI = 3.14159265359;");

		if !self.minified { glsl_block.push('\n'); }
	}
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;
    use crate::shader_generator::{self, ShaderGenerationSettings, ShaderGenerator};

	macro_rules! assert_string_contains {
		($haystack:expr, $needle:expr) => {
			assert!($haystack.contains($needle), "Expected string to contain '{}', but it did not. String: '{}'", $needle, $haystack);
		};
	}

	#[test]
	fn bindings() {
		let main = shader_generator::tests::bindings();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		// We have to split the assertions because the order of the bindings is not guaranteed.
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar) buffer _buff{float member;}buff;");
		assert_string_contains!(shader, "layout(set=0,binding=1,r8) writeonly uniform image2D image;");
		assert_string_contains!(shader, "layout(set=1,binding=0) uniform sampler2D texture;");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");

		// Assert that main is the last element in the shader string, which means that the bindings are before it.
		shader.ends_with("void main(){buff;image;texture;}");
	}

	#[test]
	fn test_specializtions() {
		let main = shader_generator::tests::specializations();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(constant_id=0)const float color_x=1.0f;layout(constant_id=1)const float color_y=1.0f;layout(constant_id=2)const float color_z=1.0f;const vec3 color=vec3(color_x,color_y,color_z);void main(){color;}");
	}

	#[test]
	fn fragment_shader() {
		let main = shader_generator::tests::fragment_shader();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::fragment(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);}");
	}

	#[test]
	fn cull_unused_functions() {
		let main = shader_generator::tests::cull_unused_functions();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "void used_by_used(){}void used(){used_by_used();}void main(){used();}");
	}

	#[test]
	fn structure() {
		let main = shader_generator::tests::structure();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};Vertex use_vertex(){}void main(){use_vertex();}");
	}

	#[test]
	fn push_constant() {
		let main = shader_generator::tests::push_constant();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(push_constant)uniform PushConstant{uint32_t material_id;}push_constant;void main(){push_constant;}");
	}

	#[test]
	fn test_glsl() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		used: fn() -> void {}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();
		let used_function = RefCell::borrow(&root).get_child("used").unwrap();

		{
			let mut main = main.borrow_mut();
			main.add_child(besl::Node::glsl("gl_Position = vec4(0)".to_string(), vec![vertex_struct, used_function], vec![]).into());
		}

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = shader_generator::tests::intrinsic();

		let shader = GLSLShaderGenerator::new().minified(true).generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){0 + 1.0 * 2;}");
	}
}
