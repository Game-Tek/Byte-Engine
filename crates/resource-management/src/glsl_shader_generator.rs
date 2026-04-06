use std::cell::RefCell;

use crate::shader_generator::{MatrixLayouts, ShaderGenerationSettings, ShaderGenerator, Stages};
use crate::shader_graph::{build_graph, topological_sort};

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
	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		let mut string = String::with_capacity(2048);

		if !matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }) {
			panic!("GLSL shader generation requires a function node as the main function.");
		}

		let graph = build_graph(main_function_node.clone());

		let order = topological_sort(&graph);
		let order = order.into_iter().filter(|n| !n.borrow().node().is_leaf());

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

	fn emit_call_arguments(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		for (i, argument) in arguments.iter().enumerate() {
			if i > 0 {
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
			}
			self.emit_node_string(string, argument);
		}
	}

	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic {
			name,
			elements: definition,
			..
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		if has_body {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		}

		match name.as_str() {
			"max" | "clamp" | "log2" | "pow" => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"thread_id" => {
				string.push_str("uvec2(gl_GlobalInvocationID.xy)");
			}
			"thread_idx" => {
				string.push_str("uint(gl_LocalInvocationID.x)");
			}
			"threadgroup_position" => {
				string.push_str("uint(gl_WorkGroupID.x)");
			}
			"set_mesh_output_counts" => {
				string.push_str("SetMeshOutputsEXT(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"set_mesh_vertex_position" => {
				string.push_str("gl_MeshVerticesEXT[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].gl_Position = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"set_mesh_triangle" => {
				string.push_str("gl_PrimitiveTriangleIndicesEXT[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("] = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"image_load" => {
				string.push_str("imageLoad(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				string.push_str("ivec2(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str("))");
			}
			"write" => {
				string.push_str("imageStore(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				string.push_str("ivec2(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(")");
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[2]);
				string.push(')');
			}
			"guard_image_bounds" => {
				string.push_str("if(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".x>=uint(imageSize(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(").x)||");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".y>=uint(imageSize(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(").y)){return;}");
			}
			_ => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(&this_node);

		let break_char = if self.minified { "" } else { "\n" };
		let space_char = if self.minified { "" } else { " " };

		match node.node() {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { .. } => {}
			besl::Nodes::Function {
				name,
				statements,
				return_type,
				params,
				..
			} => {
				string.push_str(Self::translate_type(&return_type.borrow().get_name().unwrap()));

				string.push(' ');

				string.push_str(name);

				string.push('(');

				for (i, param) in params.iter().enumerate() {
					if i > 0 {
						if !self.minified {
							string.push_str(", ");
						} else {
							string.push(',');
						}
					}

					self.emit_node_string(string, param);
				}

				if self.minified {
					string.push_str("){");
				} else {
					string.push_str(") {\n");
				}

				for statement in statements {
					if !self.minified {
						string.push('\t');
					}
					self.emit_node_string(string, &statement);
					if !self.minified {
						string.push_str(";\n");
					} else {
						string.push(';');
					}
				}

				if self.minified {
					string.push('}')
				} else {
					string.push_str("}\n");
				}
			}
			besl::Nodes::Struct { name, fields, .. } => {
				if name == "void"
					|| name == "vec2u16"
					|| name == "vec2u"
					|| name == "vec2i"
					|| name == "vec2f"
					|| name == "vec3f"
					|| name == "vec4f"
					|| name == "mat2f"
					|| name == "mat3f"
					|| name == "mat4f"
					|| name == "f32"
					|| name == "u8" || name == "u16"
					|| name == "u32"
					|| name == "i32"
					|| name == "Texture2D"
					|| name == "ArrayTexture2D"
				{
					return;
				}

				string.push_str("struct ");
				string.push_str(name.as_str());

				if self.minified {
					string.push('{');
				} else {
					string.push_str(" {\n");
				}

				for field in fields {
					if !self.minified {
						string.push('\t');
					}
					self.emit_node_string(string, &field);
					if self.minified {
						string.push(';')
					} else {
						string.push_str(";\n");
					}
				}

				string.push_str("};");

				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::PushConstant { members } => {
				if self.minified {
					string.push_str("layout(push_constant)uniform PushConstant{");
				} else {
					string.push_str("layout(push_constant) uniform PushConstant {");
				}

				if !self.minified {
					string.push('\n');
				}

				for member in members {
					if !self.minified {
						string.push('\t');
					}
					self.emit_node_string(string, &member);
					if self.minified {
						string.push(';')
					} else {
						string.push_str(";\n");
					}
				}

				if self.minified {
					string.push_str("}push_constant;");
				} else {
					string.push_str("} push_constant;");
				}

				if !self.minified {
					string.push('\n');
				}
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
								besl::Nodes::Member {
									name: member_name,
									r#type,
									..
								} => {
									let member_name = format!("{}_{}", name, { member_name });
									string.push_str(&format!(
										"layout(constant_id={})const {} {}={};{}",
										i,
										Self::translate_type(&r#type.borrow().get_name().unwrap()),
										&member_name,
										"1.0f",
										if !self.minified { "\n" } else { "" }
									));
									members.push(member_name);
								}
								_ => {}
							}
						}
					}
					_ => {}
				}

				string.push_str(&format!(
					"const {} {}={};{}",
					&type_name,
					name,
					format!("{}({})", &type_name, members.join(",")),
					if !self.minified { "\n" } else { "" }
				));
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
			besl::Nodes::Raw { glsl, .. } => {
				if let Some(code) = glsl {
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => {
				string.push_str(&format!(
					"{} {}",
					Self::translate_type(&r#type.borrow().get_name().unwrap()),
					name
				));
			}
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(&format.get_name().unwrap());
				let is_flat = type_name == "int8_t"
					|| type_name == "uint8_t"
					|| type_name == "int16_t"
					|| type_name == "uint16_t"
					|| type_name == "int"
					|| type_name == "int32_t"
					|| type_name == "uint"
					|| type_name == "uint32_t"
					|| type_name == "int64_t"
					|| type_name == "uint64_t";
				string.push_str(&format!(
					"layout(location={}){space_char}{}in {} {};{break_char}",
					location,
					if is_flat { format!("flat{space_char}") } else { format!("") },
					type_name,
					name
				));
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} => {
				if let Some(count) = count {
					string.push_str(&format!(
						"layout(location={}){space_char}perprimitiveEXT out {} {}[{}];{break_char}",
						location,
						Self::translate_type(&format.borrow().get_name().unwrap()),
						name,
						count
					));
				} else {
					string.push_str(&format!(
						"layout(location={}){space_char}out {} {};{break_char}",
						location,
						Self::translate_type(&format.borrow().get_name().unwrap()),
						name
					));
				}
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { operator, left, right } => {
					self.emit_node_string(string, &left);
					let operator = match operator {
						besl::Operators::Plus => "+",
						besl::Operators::Minus => "-",
						besl::Operators::Multiply => "*",
						besl::Operators::Divide => "/",
						besl::Operators::Modulo => "%",
						besl::Operators::ShiftLeft => "<<",
						besl::Operators::ShiftRight => ">>",
						besl::Operators::BitwiseAnd => "&",
						besl::Operators::BitwiseOr => "|",
						besl::Operators::Assignment => "=",
						besl::Operators::Equality => "==",
						besl::Operators::LessThan => "<",
					};
					if self.minified {
						string.push_str(operator);
					} else {
						string.push(' ');
						string.push_str(operator);
						string.push(' ');
					}
					self.emit_node_string(string, &right);
				}
				besl::Expressions::FunctionCall {
					parameters, function, ..
				} => {
					let function = RefCell::borrow(&function);
					let name = function.get_name().unwrap();

					let name = Self::translate_type(&name);

					string.push_str(&format!("{}(", name));
					for (i, parameter) in parameters.iter().enumerate() {
						if i > 0 {
							if self.minified {
								string.push(',')
							} else {
								string.push_str(", ");
							}
						}
						self.emit_node_string(string, &parameter);
					}
					string.push_str(&format!(")"));
				}
				besl::Expressions::IntrinsicCall {
					intrinsic,
					arguments,
					elements,
				} => {
					self.emit_intrinsic_call(string, intrinsic, arguments, elements);
				}
				besl::Expressions::Expression { elements } => {
					for element in elements {
						self.emit_node_string(string, &element);
					}
				}
				besl::Expressions::Macro { .. } => {}
				besl::Expressions::Member { name, source, .. } => match source.borrow().node() {
					besl::Nodes::Literal { value, .. } => {
						self.emit_node_string(string, &value);
					}
					_ => {
						string.push_str(name);
					}
				},
				besl::Expressions::VariableDeclaration { name, r#type } => {
					string.push_str(&format!(
						"{} {}",
						Self::translate_type(&r#type.borrow().get_name().unwrap()),
						name
					));
				}
				besl::Expressions::Literal { value } => {
					string.push_str(&value);
				}
				besl::Expressions::Return { value } => {
					string.push_str("return");
					if let Some(value) = value {
						if !self.minified {
							string.push(' ');
						}
						self.emit_node_string(string, value);
					}
				}
				besl::Expressions::Accessor { left, right } => {
					self.emit_node_string(string, &left);
					if left.borrow().node().is_indexable() {
						string.push('[');
						self.emit_node_string(string, &right);
						string.push(']');
					} else {
						string.push('.');
						self.emit_node_string(string, &right);
					}
				}
			},
			besl::Nodes::Conditional { condition, statements } => {
				string.push_str("if(");
				self.emit_node_string(string, condition);
				if self.minified {
					string.push_str("){");
				} else {
					string.push_str(") {\n");
				}

				for statement in statements {
					if !self.minified {
						string.push('\t');
					}
					self.emit_node_string(string, statement);
					if self.minified {
						string.push(';');
					} else {
						string.push_str(";\n");
					}
				}

				string.push('}');
				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::Binding {
				name,
				set,
				binding,
				read,
				write,
				r#type,
				count,
				..
			} => {
				let binding_type = match r#type {
					besl::BindingTypes::Buffer { .. } => "buffer",
					besl::BindingTypes::Image { format, .. } => match format.as_str() {
						"r8ui" | "r16ui" | "r32ui" => "uniform uimage2D",
						_ => "uniform image2D",
					},
					besl::BindingTypes::CombinedImageSampler { format } => match format.as_str() {
						"ArrayTexture2D" => "uniform sampler2DArray",
						_ => "uniform sampler2D",
					},
				};

				string.push_str(&format!("layout(set={},binding={}", set, binding));

				match r#type {
					besl::BindingTypes::Buffer { .. } => {
						string.push_str(",scalar");
					}
					besl::BindingTypes::Image { format } => {
						string.push(',');
						string.push_str(&format);
					}
					besl::BindingTypes::CombinedImageSampler { .. } => {}
				}

				match r#type {
					besl::BindingTypes::Buffer { .. } | besl::BindingTypes::Image { .. } => {
						string.push_str(&format!(
							") {}{} ",
							if *read && !*write {
								"readonly "
							} else if *write && !*read {
								"writeonly "
							} else {
								""
							},
							binding_type
						));
					}
					besl::BindingTypes::CombinedImageSampler { .. } => {
						string.push_str(&format!(") {} ", binding_type));
					}
				}

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						string.push_str(&format!("_{}{{", name));

						for member in members.iter() {
							self.emit_node_string(string, &member);
							if !self.minified {
								string.push_str(";\n");
							} else {
								string.push(';');
							}
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

				if !self.minified {
					string.push_str(";\n");
				} else {
					string.push(';');
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, &element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, &value);
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str(&format!(
					"const {} {} = ",
					Self::translate_type(&r#type.borrow().get_name().unwrap()),
					name,
				));
				self.emit_node_string(string, &value);
				string.push_str(&format!(";{break_char}"));
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
			Stages::Mesh { .. } => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
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
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => {
				glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
				glsl_block.push_str(&format!(
					"layout(triangles,max_vertices={},max_primitives={}) out;\n",
					maximum_vertices, maximum_primitives
				));
			}
			_ => {}
		}

		// local_size
		match compilation_settings.stage {
			Stages::Compute { local_size } | Stages::Mesh { local_size, .. } => {
				glsl_block.push_str(&format!(
					"layout(local_size_x={},local_size_y={},local_size_z={}) in;\n",
					local_size.width(),
					local_size.height(),
					local_size.depth()
				));
			}
			_ => {}
		}

		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;\n"),
			MatrixLayouts::ColumnMajor => glsl_block.push_str("layout(column_major) uniform;layout(column_major) buffer;\n"),
		}

		glsl_block.push_str("const float PI = 3.14159265359;");

		if !self.minified {
			glsl_block.push('\n');
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::shader_generator::{self, ShaderGenerationSettings};
	use std::cell::RefCell;

	macro_rules! assert_string_contains {
		($haystack:expr, $needle:expr) => {
			assert!(
				$haystack.contains($needle),
				"Expected string to contain '{}', but it did not. String: '{}'",
				$needle,
				$haystack
			);
		};
	}

	#[test]
	fn bindings() {
		let main = shader_generator::tests::bindings();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// We have to split the assertions because the order of the bindings is not guaranteed.
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar) buffer _buff{float member;}buff;");
		assert_string_contains!(shader, "layout(set=0,binding=1,r8) writeonly uniform image2D image;");
		assert_string_contains!(shader, "layout(set=1,binding=0) uniform sampler2D texture;");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");

		// Assert that main is the last element in the shader string, which means that the bindings are before it.
		shader.ends_with("void main(){buff;image;texture;}");
	}

	#[test]
	fn specializtions() {
		let main = shader_generator::tests::specializations();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"layout(constant_id=0)const float color_x=1.0f;layout(constant_id=1)const float color_y=1.0f;layout(constant_id=2)const float color_z=1.0f;const vec3 color=vec3(color_x,color_y,color_z);void main(){color;}"
		);
	}

	#[test]
	fn input() {
		let main = shader_generator::tests::input();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(location=0)in vec3 color;void main(){color;}");
	}

	#[test]
	fn output() {
		let main = shader_generator::tests::output();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(location=0)out vec3 color;void main(){color;}");
	}

	#[test]
	fn fragment_shader() {
		let main = shader_generator::tests::fragment_shader();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);}");
	}

	#[test]
	fn cull_unused_functions() {
		let main = shader_generator::tests::cull_unused_functions();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn structure() {
		let main = shader_generator::tests::structure();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct Vertex{vec3 position;vec3 normal;};Vertex use_vertex(){}void main(){use_vertex();}"
		);
	}

	#[test]
	fn push_constant() {
		let main = shader_generator::tests::push_constant();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"layout(push_constant)uniform PushConstant{uint32_t material_id;}push_constant;void main(){push_constant;}"
		);
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
			main.add_child(
				besl::Node::glsl(
					"gl_Position = vec4(0)".to_string(),
					vec![vertex_struct, used_function],
					vec![],
				)
				.into(),
			);
		}

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = shader_generator::tests::intrinsic();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){0 + 1.0 * 2;}");
	}

	#[test]
	fn test_multi_language_raw_code() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();

		{
			let mut main = main.borrow_mut();
			// Create a RawCode node with both GLSL and HLSL variants
			main.add_child(
				besl::Node::raw(
					Some("gl_Position = vec4(0)".to_string()),
					Some("output.position = float4(0, 0, 0, 1)".to_string()),
					vec![vertex_struct],
					vec![],
				)
				.into(),
			);
		}

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// GLSL generator should use the GLSL code
		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
		// Should NOT contain HLSL code
		assert!(!shader.contains("float4"), "GLSL shader should not contain HLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = shader_generator::tests::const_variable();

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "const float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn mesh_intrinsics_emit_glsl_mesh_commands() {
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "SetMeshOutputsEXT(4,2);");
		assert_string_contains!(shader, "gl_MeshVerticesEXT[0].gl_Position = vec4(1.0,2.0,3.0,1.0);");
		assert_string_contains!(shader, "gl_PrimitiveTriangleIndicesEXT[0] = uvec3(0,1,2);");
	}

	#[test]
	fn conditional_blocks_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected conditional shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(n<1){n=2;}");
	}

	#[test]
	fn bitwise_operators_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = GLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint32_t packed=1<<8|2&255;");
	}
}
