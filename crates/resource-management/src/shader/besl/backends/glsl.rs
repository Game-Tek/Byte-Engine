use std::cell::RefCell;

use crate::shader::generator::{
	ordered_shader_nodes, MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
};

/// The `Generator` struct exists to produce GLSL source for Vulkan-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	minified: bool,
	current_stage_interpolates_inputs: bool,
	current_stage_interpolates_outputs: bool,
	current_stage_supports_workgroup_storage: bool,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates a GLSL generator with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage_interpolates_inputs: false,
			current_stage_interpolates_outputs: false,
			current_stage_supports_workgroup_storage: false,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	/// Reports whether one reachable AST branch uses the requested intrinsic.
	fn uses_intrinsic(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
		match node.borrow().node() {
			besl::Nodes::Function { statements, .. } => statements
				.iter()
				.any(|statement| Self::uses_intrinsic(statement, intrinsic_name)),
			besl::Nodes::Conditional { condition, statements } => {
				Self::uses_intrinsic(condition, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				Self::uses_intrinsic(initializer, intrinsic_name)
					|| Self::uses_intrinsic(condition, intrinsic_name)
					|| Self::uses_intrinsic(update, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				} => {
					intrinsic.borrow().get_name().as_deref() == Some(intrinsic_name)
						|| arguments
							.iter()
							.any(|argument| Self::uses_intrinsic(argument, intrinsic_name))
				}
				besl::Expressions::Operator { left, right, .. } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::FunctionCall { parameters, .. } => parameters
					.iter()
					.any(|parameter| Self::uses_intrinsic(parameter, intrinsic_name)),
				besl::Expressions::Expression { elements } => {
					elements.iter().any(|element| Self::uses_intrinsic(element, intrinsic_name))
				}
				besl::Expressions::Macro { body, .. } => Self::uses_intrinsic(body, intrinsic_name),
				besl::Expressions::Member { source, .. } => Self::uses_intrinsic(source, intrinsic_name),
				besl::Expressions::Return { value } => value
					.as_ref()
					.is_some_and(|value| Self::uses_intrinsic(value, intrinsic_name)),
				besl::Expressions::Accessor { left, right } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. }
				| besl::Expressions::Continue => false,
			},
			_ => false,
		}
	}

	/// Reports whether reachable code uses one of BESL's compute-only subgroup operations.
	fn uses_subgroup_intrinsics(order: &[besl::NodeReference]) -> bool {
		const SUBGROUP_INTRINSICS: [&str; 6] = [
			"subgroup_ballot",
			"subgroup_ballot_any",
			"subgroup_ballot_find_lsb",
			"subgroup_ballot_count",
			"subgroup_ballot_and_not",
			"subgroup_broadcast_u32",
		];
		order.iter().any(|node| {
			SUBGROUP_INTRINSICS
				.iter()
				.any(|intrinsic| Self::uses_intrinsic(node, intrinsic))
		})
	}
}

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
		if uses_subgroup_intrinsics && !matches!(shader_compilation_settings.stage, Stages::Compute { .. }) {
			return Err(());
		}

		self.generate_glsl_header_block(&mut string, shader_compilation_settings, uses_subgroup_intrinsics);

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
			"vec2f" => "vec2",
			"vec2u" => "uvec2",
			"vec2i" => "ivec2",
			"vec2u16" => "u16vec2",
			"vec4u16" => "u16vec4",
			"vec3u" => "uvec3",
			"vec4u" => "uvec4",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"mat4x3f" => "mat4x3",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "in sampler2D",
			"Texture3D" => "in sampler3D",
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

	fn emit_visibility_texture_sample(
		&mut self,
		string: &mut String,
		texture_index: &besl::NodeReference,
		uv: &besl::NodeReference,
		xy_only: bool,
	) {
		string.push_str("texture(textures[nonuniformEXT(");
		self.emit_node_string(string, texture_index);
		string.push_str(")],");
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv);
		string.push(')');
		if xy_only {
			string.push_str(".xy");
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

		match name.as_str() {
			"sample_material" => {
				self.emit_visibility_texture_sample(string, &arguments[0], &arguments[1], false);
				return;
			}
			"sample_normal" => {
				string.push_str("unit_vector_from_xy(");
				self.emit_visibility_texture_sample(string, &arguments[0], &arguments[1], true);
				string.push(')');
				return;
			}
			"texture_lod" => {
				string.push_str("textureLod(");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				if let Some(lod) = arguments.get(2) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
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
			"subgroup_broadcast_u32" => {
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

	fn generate_glsl_header_block(
		&self,
		glsl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		uses_subgroup_intrinsics: bool,
	) {
		let glsl_version = &compilation_settings.glsl.version;

		glsl_block.push_str(&format!("#version {glsl_version} core\n"));

		// shader type

		match compilation_settings.stage {
			Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			Stages::Task { .. } => panic!(
				"GLSL task shader lowering is unsupported. The most likely cause is that a task BESL shader was sent to the deferred GLSL backend."
			),
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
			Stages::Compute { .. } if uses_subgroup_intrinsics => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:require\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:require\n");
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
					local_size.width().max(1),
					local_size.height().max(1),
					local_size.depth().max(1)
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
#[cfg(test)]
mod tests {
	use std::cell::RefCell;

	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

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
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// We have to split the assertions because the order of the bindings is not guaranteed.
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar) buffer _buff{float member;}buff;");
		assert_string_contains!(shader, "layout(set=0,binding=1,r8) writeonly uniform image2D image;");
		assert_string_contains!(shader, "layout(set=0,binding=2) uniform sampler2D texture;");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");

		// Assert that main is the last element in the shader string, which means that the bindings are before it.
		shader.ends_with("void main(){buff;image;texture;}");
	}

	#[test]
	fn compute_subgroup_intrinsics_require_and_lower_to_glsl_subgroup_operations() {
		let root = besl::compile_to_besl(
			r#"
			main: fn () -> void {
				let mask: vec4u = subgroup_ballot(thread_idx() < 4);
				let leader: u32 = subgroup_ballot_find_lsb(mask);
				let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
				let remaining: vec4u = subgroup_ballot_and_not(mask, subgroup_ballot(value == 0));
				if (subgroup_ballot_any(remaining)) {
					let count: u32 = subgroup_ballot_count(remaining);
					count;
				}
			}
			"#,
			None,
		)
		.expect("Expected subgroup fixture source to link");
		let main = root.get_main().expect("Expected subgroup fixture main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(32)), &main)
			.expect("Expected subgroup fixture to lower to GLSL");

		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_basic:require");
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_ballot:require");
		assert_string_contains!(shader, "subgroupBallot(uint(gl_LocalInvocationIndex)<4)");
		assert_string_contains!(shader, "subgroupBroadcast(uint(gl_LocalInvocationIndex),leader)");
		assert_string_contains!(shader, "subgroupBallotFindLSB(mask)");
		assert_string_contains!(shader, "subgroupBallotBitCount(remaining)");
	}

	#[test]
	fn source_storage_image_descriptor_emits_explicit_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<StorageImage<rgba16f>, 4, write>; main: fn () -> void { image; }",
			None,
		)
		.expect("Expected formatted storage image descriptor to compile");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected formatted storage image shader main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected formatted storage image GLSL generation");

		assert_string_contains!(shader, "layout(set=0,binding=4,rgba16f) writeonly uniform image2D image;");
	}

	#[test]
	fn source_unformatted_storage_image_descriptor_omits_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<StorageImage, 5, write>; main: fn () -> void { image; }",
			None,
		)
		.expect("Expected unformatted storage image descriptor to compile");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected unformatted storage image shader main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected unformatted storage image GLSL generation");

		assert_string_contains!(shader, "layout(set=0,binding=5) writeonly uniform image2D image;");
		assert!(
			!shader.contains("binding=5,"),
			"Unformatted storage image emitted a dangling GLSL format comma: {shader}"
		);
	}

	#[test]
	fn vec4u16_uses_the_native_glsl_packed_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 GLSL generation");

		assert_string_contains!(shader, "u16vec4 value;");
		assert!(!shader.contains("struct vec4u16"));
	}

	#[test]
	fn same_named_buffer_members_lower_to_glsl() {
		let main = generator::tests::same_named_buffer_member_access();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "pixel_mapping.pixel_mapping[0]=meshes.meshes[1];");
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
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
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(location=0)in vec3 color;void main(){color;}");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "layout(location=0)out vec3 color;void main(){color;}");
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_flat_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected packed integer vertex GLSL generation");
		let fragment_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Expected packed integer fragment GLSL generation");

		assert_string_contains!(vertex_shader, "layout(location=0)in u16vec2 packed_input;");
		assert_string_contains!(vertex_shader, "layout(location=1)flat out u16vec4 packed_output;");
		assert_string_contains!(fragment_shader, "layout(location=0)flat in u16vec2 packed_input;");
		assert_string_contains!(fragment_shader, "layout(location=1)out u16vec4 packed_output;");
	}

	#[test]
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);albedo;}");
	}

	#[test]
	fn fwidth_intrinsic_lowers_to_glsl() {
		let program = besl::compile_to_besl("main: fn() -> void { let edge_width: f32 = fwidth(1.0); edge_width; }", None)
			.expect("Failed to compile fwidth BESL shader");
		let main = program.get_main().expect("Expected fwidth BESL shader main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate fwidth GLSL shader");

		assert_string_contains!(shader, "fwidth(1.0)");
	}

	#[test]
	fn cull_unused_functions() {
		let main = generator::tests::cull_unused_functions();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn culls_dead_locals_before_glsl_emission() {
		let root = besl::compile_to_besl(
			r#"
			expensive: fn() -> f32 {
				return 42.0;
			}
			main: fn() -> void {
				let x: f32 = expensive();
				return;
			}
		"#,
			None,
		)
		.expect("Expected dead-local BESL fixture to link");
		let main = root.get_main().expect("Expected dead-local fixture main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected dead-local GLSL generation");

		assert_string_contains!(shader, "void main(){return;}");
		assert!(
			!shader.contains("expensive"),
			"Dead helper function reached GLSL emission: {shader}"
		);
		assert!(!shader.contains("float x"), "Dead local reached GLSL emission: {shader}");
	}

	#[test]
	fn structure() {
		let main = generator::tests::structure();

		let shader = Generator::new()
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
		let main = generator::tests::push_constant();

		let shader = Generator::new()
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

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = generator::tests::intrinsic();

		let shader = Generator::new()
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
					Some("out.position = float4(0, 0, 0, 1)".to_string()),
					vec![vertex_struct],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
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
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "const float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn const_array_variable_lowers_to_glsl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
			value;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "const float[3] WEIGHTS = float[3](0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
	}

	#[test]
	fn atomic_compare_exchange_lowers_to_glsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected compare-exchange shader source to lex");
		let main = root.get_main().expect("Expected compare-exchange main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Expected compare-exchange source to lower to GLSL");

		assert_string_contains!(
			shader,
			"atomicCompSwap(shared_keys[uint(gl_LocalInvocationIndex)],4294967295,7)"
		);
	}

	#[test]
	fn mesh_intrinsics_emit_glsl_mesh_commands() {
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
			set_mesh_primitive_render_target_array_index(0, 3);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "SetMeshOutputsEXT(4,2);");
		assert_string_contains!(shader, "gl_MeshVerticesEXT[0].gl_Position = vec4(1.0,2.0,3.0,1.0);");
		assert_string_contains!(shader, "gl_PrimitiveTriangleIndicesEXT[0] = uvec3(0,1,2);");
		assert_string_contains!(shader, "gl_MeshPrimitivesEXT[0].gl_Layer = int(3);");
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

		let shader = Generator::new()
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
			packed;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint32_t packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "for(uint32_t i=0;i<=4;i=(i+1)){if(i>=2){continue;};};");
	}

	#[test]
	fn scalar_math_intrinsics_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let a: f32 = abs(0.0 - 2.5);
			let b: f32 = sqrt(9.0);
			let c: f32 = exp(1.0);
			let d: f32 = fract(1.25);
			let e: f32 = radians(180.0);
			let f: f32 = inversesqrt(4.0);
			let g: f32 = smoothstep(0.0, 1.0, 0.5);
			let h: f32 = mix(2.0, 4.0, 0.25);
			let i: vec2f = round(vec2f(1.2, 1.8));
			a;
			b;
			c;
			d;
			e;
			f;
			g;
			h;
			i;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "abs(0.0-2.5)");
		assert_string_contains!(shader, "sqrt(9.0)");
		assert_string_contains!(shader, "exp(1.0)");
		assert_string_contains!(shader, "fract(1.25)");
		assert_string_contains!(shader, "radians(180.0)");
		assert_string_contains!(shader, "inversesqrt(4.0)");
		assert_string_contains!(shader, "smoothstep(0.0,1.0,0.5)");
		assert_string_contains!(shader, "mix(2.0,4.0,0.25)");
		assert_string_contains!(shader, "round(vec2(1.2,1.8))");
	}

	#[test]
	fn scalar_max_and_clamp_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
			maximum;
			clamped;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "max(1.0,2.0)");
		assert_string_contains!(shader, "clamp(1.5,0.0,1.0)");
	}

	#[test]
	fn fetch_intrinsic_lowers_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
			texel;
		}
		"#;

		let mut root = besl::Node::root();
		root.add_child(
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				true,
				false,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "vec4 texel=texelFetch(texture,ivec2(coord),0);");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_glsl() {
		let main = generator::tests::return_value();

		let minified_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(minified_shader, "float main(){return 1.0;}");

		let pretty_shader = Generator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(pretty_shader, "float main() {\n\treturn 1.0;\n}\n");
	}
}

pub use Generator as GLSLShaderGenerator;
