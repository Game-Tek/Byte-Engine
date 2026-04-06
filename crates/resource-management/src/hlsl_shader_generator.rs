use std::cell::RefCell;

use crate::shader_generator::{
	emit_comma_separated_nodes, emit_statement_block, is_builtin_struct_type, operator_token, ordered_shader_nodes,
	MatrixLayouts, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
};

/// HLSL Shader generator.
///
/// # Parameters
///
/// - *minified*: Controls whether the shader string output is minified. Is `true` by default in release builds.
pub struct HLSLShaderGenerator {
	minified: bool,
}

impl ShaderGenerator for HLSLShaderGenerator {}

impl HLSLShaderGenerator {
	/// Creates a new HLSLShaderGenerator.
	pub fn new() -> Self {
		HLSLShaderGenerator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}

impl HLSLShaderGenerator {
	/// Generates an HLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The settings for the shader compilation.
	/// * `main_function_node` - The main function node of the shader.
	///
	/// # Returns
	///
	/// The HLSL shader as a string.
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
		let order = ordered_shader_nodes(main_function_node, "HLSL");

		self.generate_hlsl_header_block(&mut string, shader_compilation_settings);

		for node in order {
			self.emit_node_string(&mut string, &node);
		}

		Ok(string)
	}

	/// Translates BESL intrinsic type names to HLSL type names.
	/// Example: `vec2f` -> `float2`
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "uint16_t2",
			"vec3u" => "uint3",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "Texture2D",
			"ArrayTexture2D" => "Texture2DArray",
			_ => source,
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(&this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();
		let space_char = formatting.space_str();

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

				emit_comma_separated_nodes(string, formatting, params, |string, param| {
					self.emit_node_string(string, param)
				});

				formatting.push_block_start(string);
				emit_statement_block(string, formatting, statements, 1, |string, statement| {
					self.emit_node_string(string, statement)
				});

				if self.minified {
					string.push('}')
				} else {
					string.push_str("}\n");
				}
			}
			besl::Nodes::Struct { name, fields, .. } => {
				if is_builtin_struct_type(name, false) {
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
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, &field);
					formatting.push_statement_end(string);
				}

				string.push_str("};");

				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::PushConstant { members } => {
				// HLSL: Map to root constants annotation
				if self.minified {
					string.push_str("struct PushConstant{");
				} else {
					string.push_str("// Root constants\n");
					string.push_str("struct PushConstant {\n");
				}

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, &member);
					formatting.push_statement_end(string);
				}

				if self.minified {
					string.push_str("};[[vk::push_constant]]PushConstant push_constant;");
				} else {
					string.push_str("};\n");
					string.push_str("[[vk::push_constant]] PushConstant push_constant;\n");
				}
			}
			besl::Nodes::Specialization { name, r#type } => {
				// HLSL specialization constants (static const with potential override)
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
										"[[vk::constant_id({})]]const {} {}={};{}",
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
			besl::Nodes::Raw { glsl, hlsl, .. } => {
				// Use HLSL code if available, otherwise fall back to GLSL
				if let Some(code) = hlsl {
					string.push_str(code);
				} else if let Some(code) = glsl {
					// Fall back to GLSL code (may need translation for HLSL-specific features)
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

				// HLSL uses semantics like TEXCOORD0, TEXCOORD1, etc.
				string.push_str(&format!(
					"{}{} {} : TEXCOORD{};{break_char}",
					if is_flat {
						format!("nointerpolation{space_char}")
					} else {
						format!("")
					},
					type_name,
					name,
					location
				));
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} => {
				if count.is_some() {
					return;
				}

				// HLSL uses SV_Target0, SV_Target1, etc. for render targets
				string.push_str(&format!(
					"{} {} : SV_Target{};{break_char}",
					Self::translate_type(&format.borrow().get_name().unwrap()),
					name,
					location
				));
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { operator, left, right } => {
					self.emit_node_string(string, &left);
					let operator = operator_token(operator);
					if self.minified {
						string.push_str(operator)
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
					emit_comma_separated_nodes(string, formatting, parameters, |string, parameter| {
						self.emit_node_string(string, parameter)
					});
					string.push_str(&format!(")"));
				}
				besl::Expressions::IntrinsicCall {
					elements: parameters, ..
				} => {
					for e in parameters {
						self.emit_node_string(string, &e);
					}
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
						string.push(' ');
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

				emit_statement_block(string, formatting, statements, 1, |string, statement| {
					self.emit_node_string(string, statement)
				});

				string.push('}');
				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				string.push_str("for(");
				self.emit_node_string(string, initializer);
				string.push(';');
				self.emit_node_string(string, condition);
				string.push(';');
				self.emit_node_string(string, update);
				if self.minified {
					string.push_str("){");
				} else {
					string.push_str(") {\n");
				}

				emit_statement_block(string, formatting, statements, 1, |string, statement| {
					self.emit_node_string(string, statement)
				});

				string.push('}');
				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::Binding {
				name,
				set,
				binding,
				read: _,
				write,
				r#type,
				count,
				..
			} => {
				// HLSL uses register syntax: t# for SRV/textures, u# for UAV/images, b# for CBV/constant buffers
				// Using space# for descriptor set mapping (set * 100 + binding for register number)
				let register_index = set * 100 + binding;

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						// Constant buffer or structured buffer
						// If not writable, use cbuffer (constant buffer)
						// If writable, use structured buffer
						let use_cbuffer = !*write;

						if use_cbuffer {
							string.push_str("cbuffer ");
							string.push_str(&name);
							string.push_str(&format!(" : register(b{}, space{}) {{", register_index, set));

							for member in members.iter() {
								if !self.minified {
									string.push('\n');
									string.push('\t');
								}
								self.emit_node_string(string, &member);
								if !self.minified {
									string.push_str(";\n");
								} else {
									string.push(';');
								}
							}

							if !self.minified {
								string.push_str("};\n");
							} else {
								string.push_str("};");
							}
						} else {
							// Structured buffer (RW or read-only structured buffer)
							let buffer_type = if *write { "RWStructuredBuffer" } else { "StructuredBuffer" };

							// Define the structure first
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

							// Declare the buffer
							string.push_str(&format!("{}<_{}>", buffer_type, name));
							string.push(' ');
							string.push_str(&name);

							if let Some(count) = count {
								string.push('[');
								string.push_str(count.to_string().as_str());
								string.push(']');
							}

							let register_letter = if *write { 'u' } else { 't' };
							string.push_str(&format!(" : register({}{}, space{});", register_letter, register_index, set));
							if !self.minified {
								string.push('\n');
							}
						}
					}
					besl::BindingTypes::Image { format } => {
						// UAV (unordered access view) for images
						let texture_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "RWTexture2D<uint>",
							_ => "RWTexture2D<float4>",
						};

						string.push_str(texture_type);
						string.push(' ');
						string.push_str(&name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register(u{}, space{});", register_index, set));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						// HLSL separates textures and samplers, but for combined sampler we use Texture2D
						let texture_type = match format.as_str() {
							"ArrayTexture2D" => "Texture2DArray",
							_ => "Texture2D",
						};

						string.push_str(texture_type);
						string.push_str("<float4>");
						string.push(' ');
						string.push_str(&name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register(t{}, space{});", register_index, set));
						if !self.minified {
							string.push('\n');
						}

						// Also declare a sampler with the same name + _sampler suffix
						string.push_str("SamplerState ");
						string.push_str(&name);
						string.push_str("_sampler");
						string.push_str(&format!(" : register(s{}, space{});", register_index, set));
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
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str(&format!(
					"static const {} {} = ",
					Self::translate_type(&r#type.borrow().get_name().unwrap()),
					name,
				));
				self.emit_node_string(string, &value);
				string.push_str(&format!(";{break_char}"));
			}
		}
	}

	fn generate_hlsl_header_block(&self, hlsl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		// HLSL doesn't use #version, but we can add shader model target as a comment
		hlsl_block.push_str("// Shader Model 6.0+\n");

		// Shader type as comment (user preference: Option B)
		match compilation_settings.stage {
			Stages::Vertex => hlsl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => hlsl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => hlsl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task => hlsl_block.push_str("// #pragma shader_stage(task)\n"),
			Stages::Mesh { .. } => hlsl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		// Feature requirements (Option A & C: skip most, add specific where applicable)
		// HLSL SM 6.0+ has most features built-in, so we mainly document what's expected
		hlsl_block.push_str("// Requires: 16-bit types, explicit arithmetic types\n");

		match compilation_settings.stage {
			Stages::Compute { .. } => {
				hlsl_block.push_str("// Requires: Wave intrinsics (WaveGetLaneCount, WaveGetLaneIndex, etc.)\n");
			}
			Stages::Mesh {
				maximum_vertices: _,
				maximum_primitives: _,
				..
			} => {
				hlsl_block.push_str("// Requires: Mesh shader support\n");
				hlsl_block.push_str(&format!("[outputtopology(\"triangle\")]\n"));
				hlsl_block.push_str(&format!("[numthreads(1, 1, 1)]\n"));
				hlsl_block.push_str("// Note: Mesh shader configuration needs manual setup\n");
			}
			_ => {}
		}

		// Local size for compute/mesh shaders
		match compilation_settings.stage {
			Stages::Compute { local_size } => {
				hlsl_block.push_str(&format!(
					"[numthreads({}, {}, {})]\n",
					local_size.width(),
					local_size.height(),
					local_size.depth()
				));
			}
			Stages::Mesh { local_size: _, .. } => {
				// Already added above in mesh-specific section
			}
			_ => {}
		}

		// Matrix layout
		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => hlsl_block.push_str("#pragma pack_matrix(row_major)\n"),
			MatrixLayouts::ColumnMajor => hlsl_block.push_str("#pragma pack_matrix(column_major)\n"),
		}

		// Constants
		hlsl_block.push_str("static const float PI = 3.14159265359;");

		if !self.minified {
			hlsl_block.push('\n');
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

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// The test sets read=true, write=true for buff, which makes it a RWStructuredBuffer
		// Check for structured buffer (writable buffer)
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "RWStructuredBuffer<_buff> buff : register(u0, space0);");

		// Check for RWTexture2D (image)
		assert_string_contains!(shader, "RWTexture2D<float4> image : register(u1, space0);");

		// Check for Texture2D and SamplerState (combined image sampler)
		assert_string_contains!(shader, "Texture2D<float4> texture : register(t100, space1);");
		assert_string_contains!(shader, "SamplerState texture_sampler : register(s100, space1);");

		// Check main function
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
	}

	#[test]
	fn specializtions() {
		let main = shader_generator::tests::specializations();

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "[[vk::constant_id(0)]]const float color_x=1.0f;");
		assert_string_contains!(shader, "[[vk::constant_id(1)]]const float color_y=1.0f;");
		assert_string_contains!(shader, "[[vk::constant_id(2)]]const float color_z=1.0f;");
		assert_string_contains!(shader, "const float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn input() {
		let main = shader_generator::tests::input();

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color : TEXCOORD0;");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn output() {
		let main = shader_generator::tests::output();

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color : SV_Target0;");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn fragment_shader() {
		let main = shader_generator::tests::fragment_shader();

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){float3 albedo=float3(1.0,0.0,0.0);}");
	}

	#[test]
	fn cull_unused_functions() {
		let main = shader_generator::tests::cull_unused_functions();

		let shader = HLSLShaderGenerator::new()
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

		let shader = HLSLShaderGenerator::new()
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

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct PushConstant{uint32_t material_id;};");
		assert_string_contains!(shader, "[[vk::push_constant]]PushConstant push_constant;");
		assert_string_contains!(shader, "void main(){push_constant;}");
	}

	#[test]
	fn test_hlsl() {
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

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "output.position = float4(0, 0, 0, 1)");
	}

	#[test]
	fn test_instrinsic() {
		let main = shader_generator::tests::intrinsic();

		let shader = HLSLShaderGenerator::new()
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

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// HLSL generator should use the HLSL code
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void main(){output.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "HLSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = shader_generator::tests::const_variable();

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "static const float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn conditional_blocks_lower_to_hlsl() {
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

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(n<1){n=2;}");
	}

	#[test]
	fn bitwise_operators_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint32_t packed=1<<8|2&255;");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_hlsl() {
		let main = shader_generator::tests::return_value();

		let minified_shader = HLSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(minified_shader, "float main(){return 1.0;}");

		let pretty_shader = HLSLShaderGenerator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(pretty_shader, "float main() {\n\treturn 1.0;\n}\n");
	}
}
