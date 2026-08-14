use std::cell::RefCell;

use super::{super::is_two, analysis::Generator, header};
use crate::shader::generator::{NodeEmitter, ShaderFormatting, ShaderGenerationSettings, Stages, ordered_shader_nodes};
impl Generator {
	/// Generates a GLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
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
		// Only fragment inputs and raster-producing outputs participate in interpolation.
		self.current_stage_interpolates_inputs = matches!(shader_compilation_settings.stage, Stages::Fragment);
		self.current_stage_interpolates_outputs =
			matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Mesh { .. });
		self.current_stage_supports_workgroup_storage = matches!(shader_compilation_settings.stage, Stages::Compute { .. });
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "GLSL");
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		let uses_subgroup_intrinsics = Self::uses_subgroup_intrinsics(&order);
		let uses_f16_types = Self::uses_f16_types(&order);
		if uses_subgroup_intrinsics && !matches!(shader_compilation_settings.stage, Stages::Compute { .. }) {
			return Err(());
		}

		header::generate_glsl_header_block(
			self,
			&mut string,
			shader_compilation_settings,
			uses_subgroup_intrinsics,
			uses_f16_types,
		);

		for node in order {
			self.emit_node_string(&mut string, &node);
		}

		Ok(string)
	}

	/// Translates BESL intrinsic type names to GLSL type names, such as `vec2f` to `vec2`.
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"atomicu32" => "uint32_t",
			"vec2f16" => "f16vec2",
			"vec3f16" => "f16vec3",
			"vec4f16" => "f16vec4",
			"vec2f" => "vec2",
			"vec2u" => "uvec2",
			"vec2i" => "ivec2",
			"vec2u16" => "u16vec2",
			"vec3u16" => "u16vec3",
			"vec4u16" => "u16vec4",
			"vec3u" => "uvec3",
			"vec4u" => "uvec4",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"packed_vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"mat4x3f" => "mat4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "in sampler2D",
			"Texture3D" => "in sampler3D",
			"TextureCube" => "in samplerCube",
			"TextureCubeArray" => "in samplerCubeArray",
			"ArrayTexture2D" => "in sampler2DArray",
			_ => source,
		}
	}

	/// Reports whether a backend type needs non-interpolated raster-stage I/O.
	fn is_integer_type(type_name: &str) -> bool {
		matches!(
			type_name,
			"int8_t"
				| "uint8_t" | "int16_t"
				| "uint16_t" | "int"
				| "int32_t" | "uint"
				| "uint32_t" | "int64_t"
				| "uint64_t" | "ivec2"
				| "uvec2" | "uvec3"
				| "uvec4" | "u16vec2"
				| "u16vec4"
		)
	}

	fn emit_texture_2d_array_grad_sample(
		&mut self,
		string: &mut String,
		texture_array: &besl::NodeReference,
		texture_index: &besl::NodeReference,
		uv: &besl::NodeReference,
		uv_derivative_x: &besl::NodeReference,
		uv_derivative_y: &besl::NodeReference,
	) {
		string.push_str("textureGrad(");
		self.emit_node_string(string, texture_array);
		string.push_str("[nonuniformEXT(");
		self.emit_node_string(string, texture_index);
		string.push_str(")],");
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv);
		string.push(',');
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv_derivative_x);
		string.push(',');
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv_derivative_y);
		string.push(')');
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

		match name.as_str() {
			"sample" => {
				string.push_str("texture(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
				return;
			}
			"sample_texture_2d_array_grad" => {
				self.emit_texture_2d_array_grad_sample(
					string,
					&arguments[0],
					&arguments[1],
					&arguments[2],
					&arguments[3],
					&arguments[4],
				);
				return;
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				string.push_str("textureLod(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				if arguments.len() == 4 {
					string.push_str("vec3(");
					self.emit_node_string(string, &arguments[1]);
					if self.minified {
						string.push(',');
					} else {
						string.push_str(", ");
					}
					string.push_str("float(");
					self.emit_node_string(string, &arguments[2]);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				if let Some(lod) = arguments.get(if arguments.len() == 4 { 3 } else { 2 }) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
				if name != "texture_lod" {
					string.push_str(".x");
				}
				return;
			}
			"texture_cube_array_lod" => {
				string.push_str("textureLod(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(if self.minified { ",vec4(" } else { ", vec4(" });
				self.emit_node_string(string, &arguments[1]);
				string.push_str(if self.minified { ",float(" } else { ", float(" });
				self.emit_node_string(string, &arguments[2]);
				string.push_str(if self.minified { "))," } else { ")), " });
				self.emit_node_string(string, &arguments[3]);
				string.push(')');
				return;
			}
			_ => {}
		}

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
			"pow" if arguments.len() == 2 && is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"atan2" => {
				string.push_str("atan(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "floor"
			| "round" | "fract" | "fwidth" | "step" | "radians" | "inversesqrt" | "smoothstep" | "mix" => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"fma" => {
				string.push_str("fma(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"sincos" => {
				string.push_str("vec2(sin(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("), cos(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"round_to_i32" => {
				string.push_str("ivec2(round(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"f32" => {
				string.push_str("float(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"f16" => {
				string.push_str("float16_t(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"u16" => {
				string.push_str("uint16_t(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" | "packed_vec4f" => {
				string.push_str(Self::translate_type(name));
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"u32" => {
				string.push_str("uint(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"atomic_add" => {
				string.push_str("atomicAdd(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"atomic_compare_exchange" => {
				string.push_str("atomicCompSwap(");
				self.emit_node_string(string, &arguments[0]);
				for argument in &arguments[1..] {
					if self.minified {
						string.push(',');
					} else {
						string.push_str(", ");
					}
					self.emit_node_string(string, argument);
				}
				string.push(')');
			}
			"atomic_load" => {
				self.emit_node_string(string, &arguments[0]);
			}
			"atomic_store" => {
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push('=');
				} else {
					string.push_str(" = ");
				}
				self.emit_node_string(string, &arguments[1]);
			}
			"thread_id" => {
				string.push_str("uvec2(gl_GlobalInvocationID.xy)");
			}
			"thread_idx" => {
				string.push_str("uint(gl_LocalInvocationIndex)");
			}
			"subgroup_lane_index" => string.push_str("gl_SubgroupInvocationID"),
			"threadgroup_position" => {
				string.push_str("uint(gl_WorkGroupID.x)");
			}
			"subgroup_ballot" => {
				string.push_str("subgroupBallot(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_any" => {
				string.push_str("any(notEqual(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", uvec4(0u)))");
			}
			"subgroup_ballot_find_lsb" => {
				string.push_str("subgroupBallotFindLSB(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_count" => {
				string.push_str("subgroupBallotBitCount(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_and_not" => {
				string.push('(');
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" & ~");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => {
				string.push_str("subgroupBroadcast(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"workgroup_barrier" => {
				string.push_str("barrier()");
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
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("gl_MeshPrimitivesEXT[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].gl_Layer = int(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
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
			"image_load_u32" => {
				string.push_str("imageLoad(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				string.push_str("ivec2(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(")).x");
			}
			"fetch_u32" => {
				string.push_str("texelFetch(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				string.push_str("ivec2(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str("),0).x");
			}
			"fetch" => {
				string.push_str("texelFetch(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				if arguments.len() == 3 {
					string.push_str("ivec3(ivec2(");
				} else {
					string.push_str("ivec2(");
				}
				self.emit_node_string(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str("),int(");
					self.emit_node_string(string, layer);
					string.push_str(")),0)");
				} else {
					string.push_str("),0)");
				}
			}
			"texture_size" => {
				string.push_str("uvec2(textureSize(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(",0))");
			}
			"image_size" => {
				string.push_str("uvec2(imageSize(");
				self.emit_node_string(string, &arguments[0]);
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
				string.push(')');
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[2]);
				string.push(')');
			}
			"image_atomic_or" => {
				string.push_str("imageAtomicOr(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				string.push_str("ivec2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
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
		let node = RefCell::borrow(this_node);
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
			} => self.emit_function_node(string, this_node, name, statements, return_type, params),
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_struct_node(string, name, fields, template),
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
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, member);
					formatting.push_statement_end(string);
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

				if let besl::Nodes::Struct { fields, .. } = r#type.node() {
					for (i, field) in fields.iter().enumerate() {
						if let besl::Nodes::Member {
							name: member_name,
							r#type,
							..
						} = field.borrow().node()
						{
							let member_name = format!("{}_{}", name, { member_name });
							string.push_str(&format!(
								"layout(constant_id={})const {} {}={};{}",
								i,
								Self::translate_type(r#type.borrow().get_name().unwrap()),
								member_name,
								"1.0f",
								if !self.minified { "\n" } else { "" }
							));
							members.push(member_name);
						}
					}
				}

				string.push_str(&format!(
					"const {} {}={};{}",
					type_name,
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
			besl::Nodes::Parameter { name, r#type } => self.emit_parameter_node(string, name, r#type),
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				string.push_str(&format!(
					"layout(location={}){space_char}{}in {} {};{break_char}",
					location,
					if self.current_stage_interpolates_inputs && Self::is_integer_type(type_name) {
						"flat "
					} else {
						""
					},
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
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				if let Some(count) = count {
					string.push_str(&format!(
						"layout(location={}){space_char}perprimitiveEXT out {} {}[{}];{break_char}",
						location, type_name, name, count
					));
				} else {
					let qualifier = if self.current_stage_interpolates_outputs && Self::is_integer_type(type_name) {
						"flat "
					} else {
						""
					};
					string.push_str(&format!(
						"layout(location={}){space_char}{qualifier}out {} {};{break_char}",
						location, type_name, name
					));
				}
			}
			besl::Nodes::Workgroup { name, format, count } if self.current_stage_supports_workgroup_storage => {
				string.push_str("shared ");
				string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
				string.push(' ');
				string.push_str(name);
				if let Some(count) = count {
					string.push('[');
					string.push_str(&count.to_string());
					string.push(']');
				}
				string.push(';');
				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::TaskPayload { .. } | besl::Nodes::Workgroup { .. } => {
				panic!(
					"GLSL task storage lowering is unsupported. The most likely cause is that a task or mesh BESL shader was sent to the deferred GLSL backend."
				)
			}
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional { condition, statements } => self.emit_conditional_node(string, condition, statements),
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => self.emit_for_loop_node(string, initializer, condition, update, statements),
			besl::Nodes::Binding {
				name,
				slot,
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
						"Texture3D" => "uniform sampler3D",
						"TextureCube" => "uniform samplerCube",
						"TextureCubeArray" => "uniform samplerCubeArray",
						"ArrayTexture2D" => "uniform sampler2DArray",
						"r8ui" | "r16ui" | "r32ui" => "uniform usampler2D",
						_ => "uniform sampler2D",
					},
				};

				string.push_str(&format!("layout(set=0,binding={slot}"));

				match r#type {
					besl::BindingTypes::Buffer { .. } => {
						string.push_str(",scalar");
					}
					besl::BindingTypes::Image { format } => {
						if format != "unknown" {
							string.push(',');
							string.push_str(format);
						}
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

				if let besl::BindingTypes::Buffer { members } = r#type {
					string.push_str(&format!("_{}{{", name));

					for member in members.iter() {
						self.emit_node_string(string, member);
						self.emit_statement_end(string);
					}

					string.push('}');
				}

				string.push_str(name);

				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}

				self.emit_statement_end(string);
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, value);
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("const ");
				Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
				string.push(' ');
				string.push_str(name);
				string.push_str(" = ");
				self.emit_node_string(string, value);
				string.push_str(&format!(";{break_char}"));
			}
		}
	}
}

impl crate::shader::generator::NodeEmitter for Generator {
	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		Generator::emit_intrinsic_call(self, string, intrinsic, arguments, elements)
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		self.emit_node_string(string, left);
		if !matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) && left.borrow().node().is_indexable()
		{
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node_string(string, right);
		}
	}
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeReference) {
		self.emit_node_string(string, node)
	}
}
