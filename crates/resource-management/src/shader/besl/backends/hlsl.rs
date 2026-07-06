use std::cell::RefCell;

use crate::shader::generator::{
	emit_comma_separated_nodes, operator_token, ordered_shader_nodes, MatrixLayouts, NodeEmitter, ShaderFormatting,
	ShaderGenerationSettings, ShaderGenerator, Stages,
};

/// The `Generator` struct exists to produce HLSL source for DirectX-backed shader pipelines.
///
/// # Parameters
///
/// - *minified*: Controls whether the shader string output is minified. Is `true` by default in release builds.
pub struct Generator {
	minified: bool,
	current_stage_is_compute: bool,
	current_compute_local_size: Option<utils::Extent>,
	current_push_constant_space: u32,
}

struct HlslBufferBindingSource {
	name: String,
	write: bool,
	flattened_member: Option<String>,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates a new Generator.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage_is_compute: false,
			current_compute_local_size: None,
			current_push_constant_space: 0,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}

impl Generator {
	fn hlsl_flattened_array_member(members: &[besl::NodeReference]) -> Option<(String, String)> {
		let [member] = members else {
			return None;
		};
		let member = member.borrow();
		let besl::Nodes::Member {
			name,
			r#type,
			count: Some(_),
		} = member.node()
		else {
			return None;
		};
		let element_type = r#type.borrow().get_name()?.to_string();
		Some((name.to_string(), element_type))
	}

	fn hlsl_buffer_binding_source(source: &besl::NodeReference) -> Option<HlslBufferBindingSource> {
		match source.borrow().node() {
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::Buffer { members },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: name.to_string(),
				write: *write,
				flattened_member: Self::hlsl_flattened_array_member(members).map(|(name, _)| name),
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => Self::hlsl_buffer_binding_source(source),
			_ => None,
		}
	}

	fn hlsl_buffer_member_target(member: &besl::NodeReference) -> Option<(String, String, bool)> {
		let member = member.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, source }) = member.node() else {
			return None;
		};
		let binding = Self::hlsl_buffer_binding_source(source)?;
		Some((binding.name, name.to_string(), binding.write))
	}

	fn hlsl_buffer_member_type(source: &besl::NodeReference, member_name: &str) -> Option<String> {
		match source.borrow().node() {
			besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} => members.iter().find_map(|member| {
				let member = member.borrow();
				let besl::Nodes::Member { name, r#type, .. } = member.node() else {
					return None;
				};
				(name == member_name)
					.then(|| r#type.borrow().get_name().map(str::to_string))
					.flatten()
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				Self::hlsl_buffer_member_type(source, member_name)
			}
			_ => None,
		}
	}

	fn hlsl_member_name(member: &besl::NodeReference) -> Option<String> {
		let member = member.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = member.node() else {
			return None;
		};
		Some(name.to_string())
	}

	fn node_type_name(node: &besl::NodeReference) -> Option<String> {
		match node.borrow().node() {
			besl::Nodes::Parameter { r#type, .. }
			| besl::Nodes::Member { r#type, .. }
			| besl::Nodes::Input { format: r#type, .. }
			| besl::Nodes::Output { format: r#type, .. }
			| besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => {
				r#type.borrow().get_name().map(str::to_string)
			}
			besl::Nodes::Function { return_type, .. }
			| besl::Nodes::Intrinsic {
				r#return: return_type, ..
			} => return_type.borrow().get_name().map(str::to_string),
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => Self::node_type_name(function),
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => Self::node_type_name(intrinsic),
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				Self::referenced_member_type_name(name, source)
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => Self::accessor_type_name(left),
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, .. })
				if *operator == besl::Operators::Multiply =>
			{
				Self::node_type_name(left)
			}
			_ => None,
		}
	}

	fn referenced_member_type_name(name: &str, source: &besl::NodeReference) -> Option<String> {
		if let besl::Nodes::Function { params, .. } = source.borrow().node() {
			return params
				.iter()
				.find(|parameter| parameter.borrow().get_name() == Some(name))
				.and_then(Self::node_type_name);
		}

		for child in source.borrow().get_children()? {
			if child.borrow().get_name() == Some(name) {
				return Self::node_type_name(&child);
			}
			if let Some(type_name) = Self::referenced_member_type_name(name, &child) {
				return Some(type_name);
			}
		}
		None
	}

	fn accessor_type_name(left: &besl::NodeReference) -> Option<String> {
		match left.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				let binding = Self::hlsl_buffer_binding_source(source)?;
				let flattened = binding.flattened_member.as_deref() == Some(name.as_str());
				if flattened {
					return None;
				}
				Self::hlsl_buffer_member_type(source, name)
			}
			_ => Self::node_type_name(left),
		}
	}

	fn is_matrix_type(type_name: Option<String>) -> bool {
		type_name.is_some_and(|name| matches!(name.as_str(), "mat2f" | "mat3f" | "mat4f"))
	}

	fn hlsl_name_likely_matrix_operand(name: &str) -> bool {
		name.contains("projection") || name.contains("matrix") || name == "model" || name == "view"
	}

	fn hlsl_array_type(source: &str) -> Option<(&str, &str)> {
		let (element_type, count) = source.split_once('[')?;
		Some((element_type, count.trim_end_matches(']')))
	}

	fn push_constant_space(order: &[besl::NodeReference]) -> u32 {
		order
			.iter()
			.filter_map(|node| match node.borrow().node() {
				besl::Nodes::Binding { set, .. } => Some(*set),
				_ => None,
			})
			.max()
			.map_or(0, |set| set + 1)
	}

	fn atomic_add_arguments(expression: &besl::NodeReference) -> Option<Vec<besl::NodeReference>> {
		let expression = expression.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = expression.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, .. } = intrinsic.node() else {
			return None;
		};
		(name == "atomic_add").then(|| arguments.clone())
	}

	fn image_size_arguments(expression: &besl::NodeReference) -> Option<Vec<besl::NodeReference>> {
		let expression = expression.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = expression.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, .. } = intrinsic.node() else {
			return None;
		};
		matches!(name.as_str(), "image_size" | "texture_size").then(|| arguments.clone())
	}

	fn emit_atomic_add_call(&mut self, string: &mut String, arguments: &[besl::NodeReference], previous_value: Option<&str>) {
		string.push_str("InterlockedAdd(");
		self.emit_node_string(string, &arguments[0]);
		string.push_str(", ");
		self.emit_node_string(string, &arguments[1]);
		if let Some(previous_value) = previous_value {
			string.push_str(", ");
			string.push_str(previous_value);
		}
		string.push(')');
	}

	fn emit_atomic_add_assignment(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> bool {
		let Some(arguments) = Self::atomic_add_arguments(right) else {
			return false;
		};
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::VariableDeclaration { name, r#type }) = left.node() else {
			return false;
		};

		// HLSL InterlockedAdd returns the previous value through an out parameter instead of as an expression.
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
		string.push(';');
		self.emit_atomic_add_call(string, &arguments, Some(name));
		true
	}

	fn emit_image_size_assignment(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> bool {
		let Some(arguments) = Self::image_size_arguments(right) else {
			return false;
		};
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::VariableDeclaration { name, r#type }) = left.node() else {
			return false;
		};

		// HLSL exposes texture dimensions through an out-parameter method instead of an expression value.
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
		string.push(';');
		self.emit_node_string(string, &arguments[0]);
		string.push_str(".GetDimensions(");
		string.push_str(name);
		string.push_str(".x, ");
		string.push_str(name);
		string.push_str(".y)");
		true
	}

	fn emit_array_initializer(&mut self, string: &mut String, value: &besl::NodeReference) -> bool {
		let value = value.borrow();
		let besl::Nodes::Expression(besl::Expressions::FunctionCall { parameters, .. }) = value.node() else {
			return false;
		};

		// HLSL array constants use brace initializers rather than constructor syntax like float[3](...).
		string.push('{');
		emit_comma_separated_nodes(
			string,
			ShaderFormatting::new(self.minified),
			parameters,
			|string, parameter| self.emit_node_string(string, parameter),
		);
		string.push('}');
		true
	}

	fn emit_const_node(&mut self, string: &mut String, name: &str, r#type: &besl::NodeReference, value: &besl::NodeReference) {
		let type_node = r#type.borrow();
		let type_name = type_node.get_name().unwrap();
		string.push_str("static const ");
		if let Some((element_type, count)) = Self::hlsl_array_type(type_name) {
			string.push_str(Self::translate_type(element_type));
			string.push(' ');
			string.push_str(name);
			string.push('[');
			string.push_str(count);
			string.push_str("] = ");
			if !self.emit_array_initializer(string, value) {
				self.emit_node_string(string, value);
			}
		} else {
			Self::emit_type_name(string, type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" = ");
			self.emit_node_string(string, value);
		}
		string.push(';');
		if !self.minified {
			string.push('\n');
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
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "round" | "fwidth"
			| "step" | "radians" | "smoothstep" | "dot" | "cross" | "normalize" | "reflect" | "length" => {
				string.push_str(name);
				string.push('(');
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"fract" => {
				string.push_str("frac(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"mix" => {
				string.push_str("lerp(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"f32" => {
				string.push_str("float(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"u32" => {
				string.push_str("uint(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"inversesqrt" => {
				string.push_str("rsqrt(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"fetch" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Load(int3(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", 0))");
			}
			"fetch_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Load(int3(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", 0)).x");
			}
			"image_load" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push(']');
			}
			"texture_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", ");
				if let Some(lod) = arguments.get(2) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
			}
			"image_atomic_or" => {
				string.push_str("({ uint _previous; InterlockedOr(");
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push_str("], ");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(", _previous); _previous; })");
			}
			"image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push(']');
			}
			"guard_image_bounds" => {
				// HLSL has no portable image bounds guard intrinsic, so emit the guard inline at the call site.
				string.push_str("uint2 _besl_image_size; ");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".GetDimensions(_besl_image_size.x, _besl_image_size.y); if (any(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(" >= _besl_image_size)) { return; }");
			}
			"image_size" | "texture_size" => {
				string.push_str("/* image_size requires assignment lowering for HLSL */");
				self.emit_node_string(string, &arguments[0]);
			}
			"write" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push_str("] = ");
				self.emit_node_string(string, &arguments[2]);
			}
			"atomic_add" => {
				self.emit_atomic_add_call(string, arguments, None);
			}
			"atomic_load" => self.emit_node_string(string, &arguments[0]),
			"atomic_store" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"thread_id" => {
				string.push_str("dispatch_thread_id.xy");
			}
			"thread_idx" => {
				string.push_str("group_thread_index");
			}
			"threadgroup_position" => {
				string.push_str("group_id");
			}
			_ => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
		}
	}

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
		self.current_stage_is_compute = matches!(shader_compilation_settings.stage, Stages::Compute { .. });
		self.current_compute_local_size = match shader_compilation_settings.stage {
			Stages::Compute { local_size } => Some(local_size),
			_ => None,
		};
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "HLSL");
		self.current_push_constant_space = Self::push_constant_space(&order);

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
			"vec2u16" => "uint2",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f32" => "float",
			"u8" => "uint",
			"u16" => "uint",
			"u32" => "uint32_t",
			"atomicu32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "Texture2D",
			"Texture3D" => "Texture3D",
			"ArrayTexture2D" => "Texture2DArray<float4>",
			_ => source,
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
			} => {
				let hlsl_name = if name == "main" { "besl_main" } else { name };
				self.emit_function_node(string, this_node, hlsl_name, statements, return_type, params);
			}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_struct_node(string, name, fields, template),
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_atomic_add_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_image_size_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Multiply
					&& (Self::is_matrix_type(Self::node_type_name(left))
						|| Self::is_matrix_type(Self::node_type_name(right))) =>
			{
				// HLSL matrix-vector multiplication is best expressed through mul so row-major operands type-check.
				string.push_str("mul(");
				self.emit_node_string(string, left);
				string.push_str(", ");
				self.emit_node_string(string, right);
				string.push(')');
			}
			besl::Nodes::PushConstant { members } => {
				// DX12 root constants are exposed to HLSL as a constant buffer in the space after descriptor sets.
				if self.minified {
					string.push_str("struct PushConstant{");
				} else {
					string.push_str("// Root constants\n");
					string.push_str("struct PushConstant {\n");
				}

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, member);
					formatting.push_statement_end(string);
				}

				if self.minified {
					string.push_str("};ConstantBuffer<PushConstant> push_constant : register(b0, space");
					string.push_str(&self.current_push_constant_space.to_string());
					string.push_str(");");
				} else {
					string.push_str("};\n");
					string.push_str("ConstantBuffer<PushConstant> push_constant : register(b0, space");
					string.push_str(&self.current_push_constant_space.to_string());
					string.push_str(");\n");
				}
			}
			besl::Nodes::Specialization { name, r#type } => {
				// DXC treats Vulkan specialization attributes as resource metadata, so use plain HLSL constants.
				let mut members = Vec::new();

				let r#type = r#type.borrow();

				let t = r#type.get_name().unwrap();
				let type_name = Self::translate_type(t);

				if let besl::Nodes::Struct { fields, .. } = r#type.node() {
					for field in fields.iter() {
						if let besl::Nodes::Member {
							name: member_name,
							r#type,
							..
						} = field.borrow().node()
						{
							let member_name = format!("{}_{}", name, { member_name });
							string.push_str("static const ");
							string.push_str(Self::translate_type(r#type.borrow().get_name().unwrap()));
							string.push(' ');
							string.push_str(&member_name);
							string.push_str("=1.0f;");
							if !self.minified {
								string.push('\n');
							}
							members.push(member_name);
						}
					}
				}

				string.push_str("static const ");
				string.push_str(type_name);
				string.push(' ');
				string.push_str(name);
				string.push('=');
				string.push_str(&format!("{}({})", type_name, members.join(",")));
				string.push(';');
				if !self.minified {
					string.push('\n');
				}
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
			besl::Nodes::Parameter { name, r#type } => self.emit_parameter_node(string, name, r#type),
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
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
						String::new()
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
					Self::translate_type(format.borrow().get_name().unwrap()),
					name,
					location
				));
			}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_atomic_add_assignment(string, left, right) => {}
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
				set,
				binding,
				read,
				write,
				r#type,
				count,
				..
			} => {
				// HLSL uses the binding as the register index and the descriptor set as the register space.
				let register_index = *binding;
				let read_only = *read && !*write;
				let buffer_type = if read_only { "StructuredBuffer" } else { "RWStructuredBuffer" };
				let register_type = if read_only { "t" } else { "u" };

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						if let Some((member_name, element_type)) = Self::hlsl_flattened_array_member(members) {
							string.push_str(buffer_type);
							string.push('<');
							string.push_str(Self::translate_type(&element_type));
							string.push_str("> ");
							string.push_str(name);
							if let Some(count) = count {
								string.push('[');
								string.push_str(count.to_string().as_str());
								string.push(']');
							}
							string.push_str(&format!(" : register({}{}, space{});", register_type, register_index, set));
							if !self.minified {
								string.push('\n');
							}
							let _ = member_name;
							return;
						}

						self.emit_named_struct_start(string, &format!("_{name}"));

						for member in members.iter() {
							self.emit_indentation(string, 1);
							self.emit_node_string(string, member);
							self.emit_statement_end(string);
						}

						if self.minified {
							string.push_str("};");
						} else {
							string.push_str("};\n");
						}

						string.push_str(&format!("{buffer_type}<_{name}> "));
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register({}{}, space{});", register_type, register_index, set));
						if !self.minified {
							string.push('\n');
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
						string.push_str(name);

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
							"Texture3D" => "Texture3D",
							"ArrayTexture2D" => "Texture2DArray",
							_ => "Texture2D",
						};

						string.push_str(texture_type);
						string.push_str(match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "<uint>",
							_ => "<float4>",
						});
						string.push(' ');
						string.push_str(name);

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
						string.push_str(name);
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
					self.emit_node_string(string, element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, value);
			}
			besl::Nodes::Const { name, r#type, value } => {
				self.emit_const_node(string, name, r#type, value);
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
			Stages::Mesh { .. } => {
				hlsl_block.push_str("// Requires: Mesh shader support\n");
				hlsl_block.push_str("[outputtopology(\"triangle\")]\n");
				hlsl_block.push_str("[numthreads(1, 1, 1)]\n");
				hlsl_block.push_str("// Note: Mesh shader configuration needs manual setup\n");
			}
			_ => {}
		}

		// Local size for mesh shaders. Compute local size is a function attribute in HLSL.
		if let Stages::Mesh { .. } = compilation_settings.stage {
			// Already added above in mesh-specific section
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

impl crate::shader::generator::NodeEmitter for Generator {
	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn supports_atomic_u32(&self) -> bool {
		true
	}
	fn emit_function_attributes(&mut self, string: &mut String, _node: &besl::NodeReference, name: &str) {
		let Some(local_size) = self.current_compute_local_size else {
			return;
		};
		if name != "besl_main" {
			return;
		}

		// HLSL requires compute thread-group size attributes to be attached to the entry function.
		string.push_str(&format!(
			"[numthreads({}, {}, {})]",
			local_size.width().max(1),
			local_size.height().max(1),
			local_size.depth().max(1)
		));
		if !self.minified {
			string.push('\n');
		}
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		_node: &besl::NodeReference,
		name: &str,
		has_previous_parameter: bool,
	) {
		if !self.current_stage_is_compute || name != "besl_main" {
			return;
		}

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint3 dispatch_thread_id : SV_DispatchThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_thread_id : SV_GroupThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_id : SV_GroupID");
		self.emit_separator(string);
		string.push_str("uint group_thread_index : SV_GroupIndex");
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		let Some(binding) = Self::hlsl_buffer_binding_source(source) else {
			return false;
		};
		if name == binding.name || binding.flattened_member.as_deref() == Some(name) {
			string.push_str(&binding.name);
			return true;
		}

		// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
		string.push_str(&binding.name);
		string.push_str("[0].");
		string.push_str(name);
		true
	}
	fn emit_expression_node(&mut self, string: &mut String, expression: &besl::Expressions) {
		if let besl::Expressions::Operator { operator, left, right } = expression {
			if *operator == besl::Operators::Multiply
				&& (Self::is_matrix_type(Self::node_type_name(left)) || Self::is_matrix_type(Self::node_type_name(right)))
			{
				// HLSL matrix-vector multiplication is best expressed through mul so row-major operands type-check.
				string.push_str("mul(");
				self.emit_node_string(string, left);
				string.push_str(", ");
				self.emit_node_string(string, right);
				string.push(')');
				return;
			}
			if *operator == besl::Operators::Multiply {
				let left_name = left.borrow().get_name().map(str::to_string);
				if left_name.as_deref().is_some_and(Self::hlsl_name_likely_matrix_operand) {
					// Some expression references do not retain resolved types, so preserve known matrix operand names.
					string.push_str("mul(");
					self.emit_node_string(string, left);
					string.push_str(", ");
					self.emit_node_string(string, right);
					string.push(')');
					return;
				}
				let mut left_operand = String::new();
				self.emit_node_string(&mut left_operand, left);
				if Self::hlsl_name_likely_matrix_operand(&left_operand) {
					// Buffer member references can lose their source type but still expose matrix field names.
					string.push_str("mul(");
					string.push_str(&left_operand);
					string.push_str(", ");
					self.emit_node_string(string, right);
					string.push(')');
					return;
				}
			}
		}

		let formatting = ShaderFormatting::new(self.minified);
		match expression {
			besl::Expressions::Operator { operator, left, right } => {
				self.emit_wrapped_expression(string, left);
				let operator = operator_token(operator);
				if self.minified {
					string.push_str(operator)
				} else {
					string.push(' ');
					string.push_str(operator);
					string.push(' ');
				}
				self.emit_wrapped_expression(string, right);
			}
			besl::Expressions::FunctionCall {
				parameters, function, ..
			} => {
				let function_ref = function.clone();
				let function = RefCell::borrow(&function_ref);
				let name = function.get_name().unwrap();
				Self::emit_type_name(string, name);
				string.push('(');
				emit_comma_separated_nodes(string, formatting, parameters, |string, parameter| {
					self.emit_node(string, parameter)
				});
				self.emit_function_call_extra_arguments(string, &function_ref, !parameters.is_empty());
				string.push(')');
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
					self.emit_node(string, element);
				}
			}
			besl::Expressions::Macro { .. } => {}
			besl::Expressions::Member { name, source, .. } => {
				if self.emit_expression_member(string, name, source) {
					return;
				}
				match source.borrow().node() {
					besl::Nodes::Literal { value, .. } => self.emit_node(string, value),
					_ => string.push_str(name),
				}
			}
			besl::Expressions::VariableDeclaration { name, r#type } => {
				Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
				string.push(' ');
				string.push_str(name);
			}
			besl::Expressions::Literal { value } => string.push_str(value),
			besl::Expressions::Return { value } => {
				string.push_str("return");
				if let Some(value) = value {
					string.push(' ');
					self.emit_node(string, value);
				}
			}
			besl::Expressions::Continue => string.push_str("continue"),
			besl::Expressions::Accessor { left, right } => self.emit_accessor_expression(string, left, right),
		}
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		if let (Some(binding), Some(field_name)) = (Self::hlsl_buffer_binding_source(left), Self::hlsl_member_name(right)) {
			if binding.flattened_member.as_deref() == Some(&field_name) {
				string.push_str(&binding.name);
			} else {
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str(&binding.name);
				string.push_str("[0].");
				string.push_str(&field_name);
			}
			return;
		}

		if let Some((binding_name, field_name, write)) = Self::hlsl_buffer_member_target(left) {
			if field_name == binding_name {
				string.push_str(&field_name);
			} else {
				let _ = write;
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str(&binding_name);
				string.push_str("[0].");
				string.push_str(&field_name);
			}
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		self.emit_node_string(string, left);
		if left.borrow().node().is_indexable() {
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node_string(string, right);
		}
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

	macro_rules! assert_string_does_not_contain {
		($haystack:expr, $needle:expr) => {
			assert!(
				!$haystack.contains($needle),
				"Expected string not to contain '{}', but it did. String: '{}'",
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

		// The test sets read=true, write=true for buff, which makes it a RWStructuredBuffer
		// Check for structured buffer (writable buffer)
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "RWStructuredBuffer<_buff> buff : register(u0, space0);");

		// Check for RWTexture2D (image)
		assert_string_contains!(shader, "RWTexture2D<float4> image : register(u1, space0);");

		// Check for Texture2D and SamplerState (combined image sampler)
		assert_string_contains!(shader, "Texture2D<float4> texture : register(t0, space1);");
		assert_string_contains!(shader, "SamplerState texture_sampler : register(s0, space1);");

		// Check main function
		assert_string_contains!(shader, "void besl_main(){buff;image;texture;}");
	}

	#[test]
	fn array_texture_binding_declares_single_hlsl_template_argument() {
		let mut root =
			besl::parse("main: fn () -> void { shadow_map; }").expect("Expected array texture binding shader source to parse");
		root.add(vec![besl::parser::Node::binding(
			"shadow_map",
			besl::parser::Node::combined_array_image_sampler(),
			2,
			11,
			true,
			false,
		)]);

		let root = besl::lex(root).expect("Expected array texture binding shader source to lex");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected array texture binding shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected array texture binding shader source to generate HLSL");

		assert_string_contains!(shader, "Texture2DArray<float4> shadow_map : register(t11, space2);");
		assert_string_does_not_contain!(shader, "Texture2DArray<float4><float4>");
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "static const float color_x=1.0f;");
		assert_string_contains!(shader, "static const float color_y=1.0f;");
		assert_string_contains!(shader, "static const float color_z=1.0f;");
		assert_string_contains!(shader, "static const float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void besl_main(){color;}");
		assert_string_does_not_contain!(shader, "vk::constant_id");
	}

	#[test]
	fn input() {
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color : TEXCOORD0;");
		assert_string_contains!(shader, "void besl_main(){color;}");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float3 color : SV_Target0;");
		assert_string_contains!(shader, "void besl_main(){color;}");
	}

	#[test]
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void besl_main(){float3 albedo=float3(1.0,0.0,0.0);}");
	}

	#[test]
	fn fetch_intrinsic_lowers_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
		}
		"#;

		let mut root = besl::Node::root();
		root.add_child(
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
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

		assert_string_contains!(shader, "float4 texel=texture.Load(int3(coord, 0));");
	}

	#[test]
	fn storage_image_intrinsics_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			guard_image_bounds(image, coord);
			let texel: u32 = image_load_u32(image, coord);
			let color: vec4f = image_load(color_image, coord);
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
		let void_type = root.get_child("void").expect("Expected void type");
		let image_type = root.get_child("Texture2D").expect("Expected Texture2D type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");

		root.add_children(vec![
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r32ui".to_string(),
				},
				0,
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"color_image",
				besl::BindingTypes::Image { format: String::new() },
				0,
				1,
				true,
				false,
			)
			.into(),
		]);
		let guard_image_bounds = root.add_child(besl::Node::intrinsic("guard_image_bounds", Vec::new(), void_type).into());
		guard_image_bounds.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type.clone(),
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type.clone(),
			})
			.into(),
		]);
		let image_load_u32 = root.add_child(besl::Node::intrinsic("image_load_u32", Vec::new(), u32_type).into());
		image_load_u32.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type.clone(),
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type.clone(),
			})
			.into(),
		]);
		let image_load = root.add_child(besl::Node::intrinsic("image_load", Vec::new(), vec4f_type).into());
		image_load.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected storage-image shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint2 _besl_image_size;");
		assert_string_contains!(shader, "image.GetDimensions(_besl_image_size.x, _besl_image_size.y);");
		assert_string_contains!(shader, "if (any(coord >= _besl_image_size)) { return; }");
		assert_string_contains!(shader, "uint32_t texel=image[coord];");
		assert_string_contains!(shader, "float4 color=color_image[coord];");
		assert_string_does_not_contain!(shader, "imagecoord");
		assert_string_does_not_contain!(shader, "color_imagecoord");
		assert_string_does_not_contain!(shader, "image[coord].x");
	}

	#[test]
	fn compute_image_math_and_storage_buffers_lower_to_dx12_hlsl() {
		let script = r#"
		main: fn (inverse_projection: mat4f, clip_space: vec4f) -> void {
			let coord: vec2u = thread_id();
			let extent: vec2u = image_size(output_image);
			let noise: f32 = fract(1.25);
			let projected: vec4f = inverse_projection * clip_space;
			let item_index: u32 = item_data.items[0].counter_index;
			write(output_image, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			atomic_store(counter_buffer.count[item_index], 2);
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
		let void_type = root.get_child("void").expect("Expected void type");
		let texture_2d_type = root.get_child("Texture2D").expect("Expected Texture2D type");
		let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
		let item =
			root.add_child(besl::Node::r#struct("Item", vec![besl::Node::member("counter_index", u32_type).into()]).into());

		root.add_children(vec![
			besl::Node::binding(
				"item_data",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", item, 8)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
				0,
				1,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"output_image",
				besl::BindingTypes::Image { format: String::new() },
				0,
				2,
				true,
				true,
			)
			.into(),
		]);

		let image_size = root.add_child(besl::Node::intrinsic("image_size", Vec::new(), vec2u_type.clone()).into());
		image_size
			.borrow_mut()
			.add_children(vec![besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d_type.clone(),
			})
			.into()]);
		let write = root.add_child(besl::Node::intrinsic("write", Vec::new(), void_type.clone()).into());
		write.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: vec4f_type,
			})
			.into(),
		]);
		let atomic_store = root.add_child(besl::Node::intrinsic("atomic_store", Vec::new(), void_type.clone()).into());
		atomic_store.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: atomic_u32,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "stored".to_string(),
				r#type: root.get_child("u32").expect("Expected u32 type"),
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected compute shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "StructuredBuffer<Item> item_data : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter_buffer : register(u1, space0);");
		assert_string_contains!(shader, "uint2 extent;output_image.GetDimensions(extent.x, extent.y);");
		assert_string_contains!(shader, "float noise=frac(1.25);");
		assert_string_contains!(shader, "float4 projected=(mul(inverse_projection, clip_space));");
		assert_string_contains!(shader, "uint32_t item_index=item_data[0].counter_index;");
		assert_string_contains!(shader, "output_image[coord] = float4(1.0,1.0,1.0,1.0);");
		assert_string_contains!(shader, "counter_buffer[item_index] = 2;");
		assert_string_does_not_contain!(shader, "fract(");
		assert_string_does_not_contain!(shader, "item_data : register(u0");
		assert_string_does_not_contain!(shader, "item_data.items");
		assert_string_does_not_contain!(shader, "_besl_atomic_store");
	}

	#[test]
	fn compute_entry_attributes_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::new(32, 16, 1)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "[numthreads(32, 16, 1)]void besl_main(");
		assert_string_does_not_contain!(shader, "[numthreads(32, 16, 1)]#pragma");
	}

	#[test]
	fn buffer_member_access_lowers_to_hlsl_binding_model() {
		let script = r#"
		main: fn () -> void {
			let instance_index: u32 = meshes.meshes[0];
			counter.count[instance_index] = counter.count[instance_index] + 1;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_children(vec![
			besl::Node::binding(
				"meshes",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("meshes", u32_type.clone(), 2)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", u32_type, 2)],
				},
				0,
				1,
				false,
				true,
			)
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "StructuredBuffer<uint32_t> meshes : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter : register(u1, space0);");
		assert_string_contains!(shader, "uint32_t instance_index=meshes[0];");
		assert_string_contains!(shader, "counter[instance_index]=(counter[instance_index]+1);");
		assert_string_does_not_contain!(shader, "meshes.meshes");
		assert_string_does_not_contain!(shader, "counter.count");
		assert_string_does_not_contain!(shader, "struct _counter");
	}

	#[test]
	fn structured_buffer_and_cbuffer_access_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = thread_id();
			let item_index: u32 = image_load_u32(index_image, coord);
			let counter_index: u32 = item_data.items[item_index].counter_index;
			atomic_add(counter_buffer.count[counter_index], 1);
			let previous_count: u32 = atomic_add(counter_buffer.count[counter_index], 1);
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
		let item = root
			.add_child(besl::Node::r#struct("Item", vec![besl::Node::member("counter_index", u32_type.clone()).into()]).into());

		root.add_children(vec![
			besl::Node::binding(
				"item_data",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", item, 8)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
				0,
				1,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"index_image",
				besl::BindingTypes::Image {
					format: "r32ui".to_string(),
				},
				0,
				2,
				true,
				false,
			)
			.into(),
		]);

		let texture_2d = root.get_child("Texture2D").expect("Expected Texture2D type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");
		let image_load_u32 = root.add_child(besl::Node::intrinsic("image_load_u32", Vec::new(), u32_type.clone()).into());
		image_load_u32.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
		]);
		let atomic_add = root.add_child(besl::Node::intrinsic("atomic_add", Vec::new(), u32_type).into());
		atomic_add.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: atomic_u32,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "increment".to_string(),
				r#type: root.get_child("u32").expect("Expected u32 type"),
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "[numthreads(8, 8, 1)]void besl_main(");
		assert_string_contains!(shader, "uint32_t item_index=index_image[coord];");
		assert_string_contains!(shader, "StructuredBuffer<Item> item_data : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter_buffer : register(u1, space0);");
		assert_string_contains!(shader, "uint32_t counter_index=item_data[item_index].counter_index;");
		assert_string_contains!(shader, "InterlockedAdd(counter_buffer[counter_index], 1);");
		assert_string_contains!(
			shader,
			"uint32_t previous_count;InterlockedAdd(counter_buffer[counter_index], 1, previous_count);"
		);
		assert_string_does_not_contain!(shader, "item_data.items");
		assert_string_does_not_contain!(shader, "counter_buffer.count");
		assert_string_does_not_contain!(shader, "struct _counter_buffer");
		assert_string_does_not_contain!(shader, "index_image[coord].x");
		assert_string_does_not_contain!(shader, "_besl_atomic_add");
	}

	#[test]
	fn parameter_buffer_and_texture_lod_lower_to_dx12_hlsl() {
		let script = r#"
		main: fn () -> void {
			let uv: vec2f = vec2f(0.5, 0.5);
			let texel: vec4f = texture_lod(depth_texture, uv);
			let projected: vec4f = parameters.inverse_view_projection * texel;
			let sun: vec4f = parameters.sun_direction;
		}
		"#;

		let mut root = besl::Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let texture_2d = root.get_child("Texture2D").expect("Expected Texture2D type");

		root.add_children(vec![
			besl::Node::binding(
				"depth_texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"parameters",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("inverse_view_projection", mat4f).into(),
						besl::Node::member("sun_direction", vec4f.clone()).into(),
					],
				},
				0,
				2,
				true,
				false,
			)
			.into(),
		]);

		let texture_lod = root.add_child(besl::Node::intrinsic("texture_lod", Vec::new(), vec4f.clone()).into());
		texture_lod.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "texture".to_string(),
				r#type: texture_2d,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "uv".to_string(),
				r#type: vec2f,
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected parameter-buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct _parameters{float4x4 inverse_view_projection;float4 sun_direction;};"
		);
		assert_string_contains!(shader, "StructuredBuffer<_parameters> parameters : register(t2, space0);");
		assert_string_contains!(
			shader,
			"float4 texel=depth_texture.SampleLevel(depth_texture_sampler, uv, 0.0);"
		);
		assert_string_contains!(
			shader,
			"float4 projected=(mul(parameters[0].inverse_view_projection, texel));"
		);
		assert_string_contains!(shader, "float4 sun=parameters[0].sun_direction;");
		assert_string_does_not_contain!(shader, "cbuffer parameters");
		assert_string_does_not_contain!(shader, "depth_textureuv");
		assert_string_does_not_contain!(shader, "parameters.inverse_view_projection");
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
			"void used_by_used(){}void used(){used_by_used();}void besl_main(){used();}"
		);
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
			"struct Vertex{float3 position;float3 normal;};Vertex use_vertex(){}void besl_main(){use_vertex();}"
		);
	}

	#[test]
	fn push_constant() {
		let main = generator::tests::push_constant();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct PushConstant{uint32_t material_id;};");
		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
		assert_string_contains!(shader, "void besl_main(){push_constant;}");
		assert_string_does_not_contain!(shader, "vk::push_constant");
	}

	#[test]
	fn push_constant_space_follows_descriptor_sets() {
		let script = r#"
		main: fn () -> void {
			push_constant;
			values;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_children(vec![
			besl::Node::push_constant(vec![besl::Node::member("material_id", u32_type.clone()).into()]).into(),
			besl::Node::binding(
				"values",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", u32_type, 4)],
				},
				2,
				7,
				true,
				false,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected push-constant shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected push-constant shader source to generate HLSL");

		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space3);");
		assert_string_contains!(shader, "StructuredBuffer<uint32_t> values : register(t7, space2);");
		assert_string_does_not_contain!(shader, "vk::push_constant");
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

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "output.position = float4(0, 0, 0, 1)");
	}

	#[test]
	fn test_instrinsic() {
		let main = generator::tests::intrinsic();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void besl_main(){0 + 1.0 * 2;}");
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

		// HLSL generator should use the HLSL code
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void besl_main(){output.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "HLSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "static const float PI = 3.14;");
		assert_string_contains!(shader, "void besl_main(){PI;}");
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

		let shader = Generator::new()
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

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint32_t packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_hlsl() {
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
	fn scalar_max_and_clamp_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
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
	fn const_array_variable_lowers_to_hlsl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "static const float WEIGHTS[3] = {0.5,0.25,0.125};");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
		assert_string_does_not_contain!(shader, "float[3] WEIGHTS");
		assert_string_does_not_contain!(shader, "float[3](");
	}

	#[test]
	fn mix_intrinsic_lowers_to_hlsl_lerp() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = mix(0.0, 1.0, 0.5);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected mix shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float value=lerp(0.0,1.0,0.5);");
		assert_string_does_not_contain!(shader, "mix(");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_hlsl() {
		let main = generator::tests::return_value();

		let minified_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(minified_shader, "float besl_main(){return 1.0;}");

		let pretty_shader = Generator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(pretty_shader, "float besl_main() {\n\treturn 1.0;\n}\n");
	}
}

pub use Generator as HLSLShaderGenerator;
