use std::cell::RefCell;

use crate::shader_generator::{MatrixLayouts, ShaderGenerationSettings, ShaderGenerator, Stages};
use crate::shader_graph::{build_graph, topological_sort};

/// The `MSLShaderGenerator` struct generates Metal Shading Language shaders from BESL ASTs.
///
/// # Parameters
///
/// - *minified*: Controls whether the shader string output is minified. Is `true` by default in release builds.
pub struct MSLShaderGenerator {
	minified: bool,
}

impl ShaderGenerator for MSLShaderGenerator {}

impl MSLShaderGenerator {
	/// Creates a new MSLShaderGenerator.
	pub fn new() -> Self {
		MSLShaderGenerator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}

impl MSLShaderGenerator {
	/// Generates an MSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The settings for the shader compilation.
	/// * `main_function_node` - The main function node of the shader.
	///
	/// # Returns
	///
	/// The MSL shader as a string.
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
			panic!(
				"MSL shader generation requires a function node as the main function. The provided node was not a function."
			);
		}

		let graph = build_graph(main_function_node.clone());

		let order = topological_sort(&graph)
			.into_iter()
			.filter(|node| !node.borrow().node().is_leaf())
			.collect::<Vec<_>>();

		self.generate_msl_header_block(&mut string, shader_compilation_settings);

		match shader_compilation_settings.stage {
			Stages::Compute { .. } => self.generate_compute_shader(&mut string, &order, main_function_node),
			_ => {
				for node in order {
					self.emit_node_string(&mut string, &node);
				}
			}
		}

		Ok(string)
	}

	fn generate_compute_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
	) {
		let mut bindings = Vec::new();
		let mut push_constant = None;

		for node in order {
			match node.borrow().node() {
				besl::Nodes::Binding { r#type, .. } => {
					if let besl::BindingTypes::Buffer { members } = r#type {
						self.emit_buffer_binding_struct(string, node, members.as_slice());
					}
					bindings.push(node.clone());
				}
				besl::Nodes::PushConstant { .. } => {
					if push_constant.is_none() {
						push_constant = Some(node.clone());
					}
				}
				besl::Nodes::Function { name, .. } if name == "main" => {}
				_ => self.emit_node_string(string, node),
			}
		}

		self.emit_compute_entry_point(string, main_function_node, bindings.as_slice(), push_constant.as_ref());
	}

	fn emit_buffer_binding_struct(
		&mut self,
		string: &mut String,
		binding_node: &besl::NodeReference,
		members: &[besl::NodeReference],
	) {
		let binding = binding_node.borrow();
		let besl::Nodes::Binding { name, .. } = binding.node() else {
			return;
		};

		string.push_str("struct _");
		string.push_str(name);
		if self.minified {
			string.push('{');
		} else {
			string.push_str(" {\n");
		}

		for member in members {
			if !self.minified {
				string.push('\t');
			}
			self.emit_node_string(string, member);
			if self.minified {
				string.push(';');
			} else {
				string.push_str(";\n");
			}
		}

		string.push_str("};");
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_compute_entry_point(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		bindings: &[besl::NodeReference],
		push_constant: Option<&besl::NodeReference>,
	) {
		let break_char = if self.minified { "" } else { "\n" };
		let node = RefCell::borrow(main_function_node);

		let besl::Nodes::Function {
			name,
			statements,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str("kernel void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');
		string.push_str("uint2 gid [[thread_position_in_grid]]");

		for param in params {
			if self.minified {
				string.push(',');
			} else {
				string.push_str(", ");
			}
			self.emit_node_string(string, param);
		}

		if let Some(push_constant) = push_constant {
			if self.minified {
				string.push(',');
			} else {
				string.push_str(", ");
			}
			self.emit_compute_push_constant_parameter(string, push_constant);
		}

		for binding in bindings {
			self.emit_compute_binding_parameter(string, binding);
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
			self.emit_node_string(string, statement);
			string.push(';');
			string.push_str(break_char);
		}

		string.push('}');
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_compute_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str("constant PushConstant& push_constant [[buffer(0)]]");
	}

	fn emit_compute_binding_parameter(&self, string: &mut String, binding_node: &besl::NodeReference) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			set,
			binding,
			read,
			write,
			r#type,
			..
		} = node.node()
		else {
			return;
		};

		let index = set * 100 + binding;
		let separator = if self.minified { "," } else { ", " };

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = if *write { "device" } else { "constant" };
				string.push_str(separator);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {} [[buffer({})]]", name, name, index));
			}
			besl::BindingTypes::Image { format } => {
				let element_type = match format.as_str() {
					"r8ui" | "r16ui" | "r32ui" => "uint",
					_ => "float",
				};
				let access = if *read && *write {
					"access::read_write"
				} else if *write {
					"access::write"
				} else {
					"access::read"
				};

				string.push_str(separator);
				string.push_str(&format!(
					"texture2d<{}, {}> {} [[texture({})]]",
					element_type, access, name, index
				));
			}
			besl::BindingTypes::CombinedImageSampler { format } => {
				let texture_type = match format.as_str() {
					"ArrayTexture2D" => "texture2d_array<float>",
					_ => "texture2d<float>",
				};

				string.push_str(separator);
				string.push_str(&format!("{} {} [[texture({})]]", texture_type, name, index));
				string.push_str(separator);
				string.push_str(&format!("sampler {}_sampler [[sampler({})]]", name, index));
			}
		}
	}

	/// Translates BESL intrinsic type names to MSL type names.
	/// Example: `vec2f` -> `float2`
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "ushort2",
			"vec3u" => "uint3",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"f32" => "float",
			"u8" => "uchar",
			"u16" => "ushort",
			"u32" => "uint",
			"i32" => "int",
			"Texture2D" => "texture2d<float>",
			"ArrayTexture2D" => "texture2d_array<float>",
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
				string.push_str("gid");
			}
			"image_load" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"write" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".write(");
				self.emit_node_string(string, &arguments[2]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"guard_image_bounds" => {
				string.push_str("if(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".x>=");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_width()||");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".y>=");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_height()){return;}");
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
				string.push_str("struct PushConstant");
				if self.minified {
					string.push('{');
				} else {
					string.push_str(" {\n");
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

				string.push_str("};");
				if !self.minified {
					string.push('\n');
				}

				// TODO: Confirm push constant mapping for Metal argument buffers.
				if self.minified {
					string.push_str("constant PushConstant& push_constant [[buffer(0)]];");
				} else {
					string.push_str("constant PushConstant& push_constant [[buffer(0)]];\n");
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
										"constant {} {} [[function_constant({})]] = {};{}",
										Self::translate_type(&r#type.borrow().get_name().unwrap()),
										&member_name,
										i,
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
					"constant {} {}={};{}",
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
			besl::Nodes::Raw { glsl, hlsl, .. } => {
				// TODO: BESL Raw nodes do not expose MSL. Using HLSL as the closest fallback.
				if let Some(code) = hlsl.as_ref().or(glsl.as_ref()) {
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
				// TODO: Map interpolation qualifiers to Metal (flat/linear).
				string.push_str(&format!("{} {} [[attribute({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Output { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(&format.get_name().unwrap());
				string.push_str(&format!("{} {} [[color({})]];{break_char}", type_name, name, location));
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
						besl::Operators::Assignment => "=",
						besl::Operators::Equality => "==",
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
					string.push('.');
					self.emit_node_string(string, &right);
				}
			},
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
				let index = set * 100 + binding;

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						string.push_str("struct _");
						string.push_str(&name);
						if self.minified {
							string.push('{');
						} else {
							string.push_str(" {\n");
						}

						for member in members.iter() {
							if !self.minified {
								string.push('\t');
							}
							self.emit_node_string(string, &member);
							if !self.minified {
								string.push_str(";\n");
							} else {
								string.push(';');
							}
						}

						if self.minified {
							string.push_str("};");
						} else {
							string.push_str("};\n");
						}

						let address_space = if *write { "device" } else { "constant" };

						string.push_str(address_space);
						string.push(' ');
						string.push_str(&format!("_{}* {}", name, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[buffer({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::Image { format } => {
						let element_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uint",
							_ => "float",
						};

						let access = if *read && *write {
							"access::read_write"
						} else if *write {
							"access::write"
						} else {
							"access::read"
						};

						string.push_str(&format!("texture2d<{}, {}> {}", element_type, access, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						let texture_type = match format.as_str() {
							"ArrayTexture2D" => "texture2d_array<float>",
							_ => "texture2d<float>",
						};

						string.push_str(texture_type);
						string.push(' ');
						string.push_str(&name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}

						string.push_str("sampler ");
						string.push_str(&format!("{}_sampler", name));
						string.push_str(&format!(" [[sampler({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
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
		}
	}

	fn generate_msl_header_block(&self, msl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		msl_block.push_str("#include <metal_stdlib>\n");
		msl_block.push_str("using namespace metal;\n");

		match compilation_settings.stage {
			Stages::Vertex => msl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => msl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => msl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task => msl_block.push_str("// #pragma shader_stage(task)\n"),
			Stages::Mesh { .. } => msl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		match compilation_settings.stage {
			Stages::Compute { .. } => {
				msl_block.push_str("// Note: Metal threadgroup sizes are set on the pipeline state.\n");
			}
			Stages::Mesh { .. } => {
				msl_block.push_str("// Note: Metal mesh shader configuration requires manual setup.\n");
			}
			_ => {}
		}

		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => msl_block.push_str("// Matrix layout: row major\n"),
			MatrixLayouts::ColumnMajor => msl_block.push_str("// Matrix layout: column major\n"),
		}

		msl_block.push_str("constant float PI = 3.14159265359;");

		if !self.minified {
			msl_block.push('\n');
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]];");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]];");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(100)]];");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(100)]];");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
	}

	#[test]
	fn specializtions() {
		let main = shader_generator::tests::specializations();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float color_x [[function_constant(0)]] = 1.0f;");
		assert_string_contains!(shader, "constant float color_y [[function_constant(1)]] = 1.0f;");
		assert_string_contains!(shader, "constant float color_z [[function_constant(2)]] = 1.0f;");
		assert_string_contains!(shader, "constant float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn input() {
		let main = shader_generator::tests::input();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color [[attribute(0)]];");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn output() {
		let main = shader_generator::tests::output();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color [[color(0)]];");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn fragment_shader() {
		let main = shader_generator::tests::fragment_shader();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){float3 albedo=float3(1.0,0.0,0.0);}");
	}

	#[test]
	fn cull_unused_functions() {
		let main = shader_generator::tests::cull_unused_functions();

		let shader = MSLShaderGenerator::new()
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct Vertex{float3 position;float3 normal;};Vertex use_vertex(){}void main(){use_vertex();}"
		);
	}

	#[test]
	fn push_constant() {
		let main = shader_generator::tests::push_constant();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct PushConstant{uint material_id;};");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(0)]];");
		assert_string_contains!(shader, "void main(){push_constant;}");
	}

	#[test]
	fn test_msl() {
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
				besl::Node::hlsl(
					"output.position = float4(0, 0, 0, 1)".to_string(),
					vec![vertex_struct, used_function],
					vec![],
				)
				.into(),
			);
		}

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){output.position = float4(0, 0, 0, 1);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = shader_generator::tests::intrinsic();

		let shader = MSLShaderGenerator::new()
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// MSL generator should use the HLSL code as the closest fallback
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void main(){output.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "MSL shader should not contain GLSL code");
	}
}
