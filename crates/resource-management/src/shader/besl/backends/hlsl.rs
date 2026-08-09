use std::{cell::RefCell, fmt::Write as _};

use crate::shader::generator::{
	emit_comma_separated_nodes, ordered_shader_nodes, MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings,
	ShaderGenerator, Stages,
};

/// The `Generator` struct exists to produce HLSL source for DirectX-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	minified: bool,
	current_stage: HlslStage,
	current_stage_interpolates_inputs: bool,
	current_stage_interpolates_outputs: bool,
	current_local_size: Option<utils::Extent>,
	current_mesh_maximum_vertices: u32,
	current_mesh_maximum_primitives: u32,
	mesh_uses_render_target_array_index: bool,
	task_payloads: Vec<besl::NodeReference>,
	mesh_outputs: Vec<besl::NodeReference>,
	raster_inputs: Vec<besl::NodeReference>,
	raster_outputs: Vec<besl::NodeReference>,
	user_struct_constructors: Vec<besl::NodeReference>,
	packed_write_counter: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HlslStage {
	Vertex,
	Fragment,
	Compute,
	Task,
	Mesh,
}

/// The `HlslBufferBindingSource` struct preserves the binding metadata needed while flattening BESL buffers for HLSL.
struct HlslBufferBindingSource {
	name: String,
	write: bool,
	flattened_member: Option<String>,
	flattened_element_type: Option<String>,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates an HLSL generator with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage: HlslStage::Vertex,
			current_stage_interpolates_inputs: false,
			current_stage_interpolates_outputs: false,
			current_local_size: None,
			current_mesh_maximum_vertices: 0,
			current_mesh_maximum_primitives: 0,
			mesh_uses_render_target_array_index: false,
			task_payloads: Vec::new(),
			mesh_outputs: Vec::new(),
			raster_inputs: Vec::new(),
			raster_outputs: Vec::new(),
			user_struct_constructors: Vec::new(),
			packed_write_counter: 0,
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
			} => {
				let (flattened_member, flattened_element_type) = Self::hlsl_flattened_array_member(members)
					.map_or((None, None), |(name, element_type)| (Some(name), Some(element_type)));
				Some(HlslBufferBindingSource {
					name: name.to_string(),
					write: *write,
					flattened_member,
					flattened_element_type,
				})
			}
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => Self::hlsl_buffer_binding_source(source),
			_ => None,
		}
	}

	/// Recovers a buffer member name and its source from either BESL member representation.
	fn hlsl_buffer_member_reference(member: &besl::NodeReference) -> Option<(String, besl::NodeReference)> {
		let member = member.borrow();
		match member.node() {
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => Some((name.to_string(), source.clone())),
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				Some((Self::hlsl_member_name(right)?, left.clone()))
			}
			_ => None,
		}
	}

	/// Recovers the underlying HLSL buffer and flattened-field metadata for an indexed BESL member expression.
	fn hlsl_buffer_member_target(member: &besl::NodeReference) -> Option<(String, String, bool, Option<String>, bool)> {
		// Lexed buffer-member access can retain its dot operation as an accessor,
		// so recover both sides before indexing it.
		let (name, source) = Self::hlsl_buffer_member_reference(member)?;
		let binding = Self::hlsl_buffer_binding_source(&source)?;
		let flattened = binding.flattened_member.as_deref() == Some(name.as_str());
		Some((binding.name, name, binding.write, binding.flattened_element_type, flattened))
	}

	/// Reports whether an accessor selects one element from a declared buffer-member array.
	fn hlsl_buffer_member_is_array(member: &besl::NodeReference) -> bool {
		let Some((name, source)) = Self::hlsl_buffer_member_reference(member) else {
			return false;
		};
		Self::hlsl_buffer_source_member_is_array(&source, &name)
	}

	/// Finds whether the named member is an array in the underlying buffer declaration.
	fn hlsl_buffer_source_member_is_array(source: &besl::NodeReference, member_name: &str) -> bool {
		match source.borrow().node() {
			besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} => members.iter().any(|member| {
				matches!(
					member.borrow().node(),
					besl::Nodes::Member {
						name,
						count: Some(_),
						..
					} if name == member_name
				)
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				Self::hlsl_buffer_source_member_is_array(source, member_name)
			}
			_ => false,
		}
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
			| besl::Nodes::TaskPayload { format: r#type, .. }
			| besl::Nodes::Workgroup { format: r#type, .. }
			| besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Const { r#type, .. }
			| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => {
				r#type.borrow().get_name().map(str::to_string)
			}
			besl::Nodes::Struct { name, .. } => Some(name.to_string()),
			besl::Nodes::Function { return_type, .. }
			| besl::Nodes::Intrinsic {
				r#return: return_type, ..
			} => return_type.borrow().get_name().map(str::to_string),
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => Self::node_type_name(function),
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => Self::node_type_name(intrinsic),
			besl::Nodes::Expression(besl::Expressions::Literal { value }) => Some(
				if matches!(value.as_str(), "true" | "false") {
					"bool"
				} else if value.contains(['.', 'e', 'E']) {
					"f32"
				} else {
					"u32"
				}
				.to_string(),
			),
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				Self::referenced_member_type_name(name, source)
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				if matches!(
					right.borrow().node(),
					besl::Nodes::Expression(besl::Expressions::Member { .. })
				) {
					Self::node_type_name(right)
				} else {
					Self::accessor_type_name(left)
				}
			}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right }) => {
				Self::operator_result_type_name(operator, left, right)
			}
			besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
				Self::node_type_name(&elements[0])
			}
			_ => None,
		}
	}

	fn referenced_member_type_name(name: &str, source: &besl::NodeReference) -> Option<String> {
		match source.borrow().node() {
			besl::Nodes::Parameter { .. }
			| besl::Nodes::Member { .. }
			| besl::Nodes::Input { .. }
			| besl::Nodes::Output { .. }
			| besl::Nodes::TaskPayload { .. }
			| besl::Nodes::Workgroup { .. }
			| besl::Nodes::Specialization { .. }
			| besl::Nodes::Const { .. }
			| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { .. }) => {
				return Self::node_type_name(source);
			}
			besl::Nodes::Function { params, .. } => {
				return params
					.iter()
					.find(|parameter| parameter.borrow().get_name() == Some(name))
					.and_then(Self::node_type_name);
			}
			_ => {}
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
		if let Some((name, source)) = Self::hlsl_buffer_member_reference(left) {
			if let Some(binding) = Self::hlsl_buffer_binding_source(&source) {
				let flattened = binding.flattened_member.as_deref() == Some(name.as_str());
				let member_type = if flattened {
					binding.flattened_element_type
				} else {
					Self::hlsl_buffer_member_type(&source, &name)
				}?;
				// The first index on an array member selects its declared element.
				// Only a later index into that element selects a matrix column or vector component.
				return if Self::hlsl_buffer_member_is_array(left) {
					Some(member_type)
				} else {
					Some(Self::indexed_value_type_name(&member_type).to_string())
				};
			}
		}

		// Local and parameter references do not carry buffer metadata. Their
		// resolved value type still determines the result of one index operation.
		Self::node_type_name(left).map(|type_name| Self::indexed_value_type_name(&type_name).to_string())
	}

	/// Returns the BESL value type produced by indexing one matrix, vector, or scalar-like value.
	fn indexed_value_type_name(type_name: &str) -> &str {
		if let Some((element_type, _)) = Self::hlsl_array_type(type_name) {
			return element_type;
		}
		match type_name {
			"mat2f" => "vec2f",
			"mat3f" => "vec3f",
			"mat4f" => "vec4f",
			"mat4x3f" => "vec3f",
			"vec2u16" | "vec4u16" => "u16",
			"vec2i" => "i32",
			"vec2u" | "vec3u" | "vec4u" => "u32",
			"vec2f16" | "vec3f16" | "vec4f16" => "f16",
			"vec2f" | "vec3f" | "vec4f" | "packed_vec4f" => "f32",
			_ => type_name,
		}
	}

	/// Recovers matrix result types without mistaking matrix-vector products for matrices.
	fn operator_result_type_name(
		operator: &besl::Operators,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> Option<String> {
		let left_type = Self::node_type_name(left);
		let right_type = Self::node_type_name(right);
		match operator {
			besl::Operators::Plus
			| besl::Operators::Minus
			| besl::Operators::Multiply
			| besl::Operators::Divide
			| besl::Operators::Modulo => Self::matrix_arithmetic_result_type(left_type.as_deref(), right_type.as_deref()),
			_ => None,
		}
	}

	/// Mirrors the matrix and f32-broadcast result rules used by the BESL VM.
	fn matrix_arithmetic_result_type(left_type: Option<&str>, right_type: Option<&str>) -> Option<String> {
		match (left_type, right_type) {
			(Some(left), Some(right)) if left == right && Self::is_matrix_type(Some(left)) => Some(left.to_string()),
			(Some(matrix), Some("f32")) if Self::is_matrix_type(Some(matrix)) => Some(matrix.to_string()),
			(Some("f32"), Some(matrix)) if Self::is_matrix_type(Some(matrix)) => Some(matrix.to_string()),
			_ => None,
		}
	}

	fn is_matrix_type(type_name: Option<&str>) -> bool {
		type_name.is_some_and(|name| matches!(name, "mat2f" | "mat3f" | "mat4f" | "mat4x3f"))
	}

	fn hlsl_square_matrix_column_type(type_name: &str) -> Option<(&'static str, usize)> {
		match type_name {
			"mat2f" => Some(("vec2f", 2)),
			"mat3f" => Some(("vec3f", 3)),
			"mat4f" => Some(("vec4f", 4)),
			_ => None,
		}
	}

	fn is_square_column_vector_matrix_constructor(type_name: &str, parameters: &[besl::NodeReference]) -> bool {
		let Some((column_type, column_count)) = Self::hlsl_square_matrix_column_type(type_name) else {
			return false;
		};

		parameters.len() == column_count
			&& parameters
				.iter()
				.all(|parameter| Self::node_type_name(parameter).as_deref() == Some(column_type))
	}

	fn hlsl_name_likely_matrix_operand(name: &str) -> bool {
		name.contains("projection")
			|| name.contains("matrix")
			|| name == "model"
			|| name.ends_with(".model")
			|| name == "view"
			|| name.ends_with(".view")
	}

	fn emit_visibility_texture_sample(
		&mut self,
		string: &mut String,
		texture_index: &besl::NodeReference,
		uv: &besl::NodeReference,
		xy_only: bool,
	) {
		string.push_str("textures[");
		self.emit_node_string(string, texture_index);
		string.push_str("].SampleLevel(textures_sampler,");
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv);
		string.push_str(", 0.0)");
		if xy_only {
			string.push_str(".xy");
		}
	}

	fn hlsl_array_type(source: &str) -> Option<(&str, &str)> {
		let (element_type, count) = source.split_once('[')?;
		Some((element_type, count.trim_end_matches(']')))
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

	fn atomic_compare_exchange_arguments(expression: &besl::NodeReference) -> Option<Vec<besl::NodeReference>> {
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
		(name == "atomic_compare_exchange").then(|| arguments.clone())
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

	fn emit_atomic_compare_exchange_call(
		&mut self,
		string: &mut String,
		arguments: &[besl::NodeReference],
		previous_value: Option<&str>,
	) {
		string.push_str("InterlockedCompareExchange(");
		self.emit_node_string(string, &arguments[0]);
		string.push_str(", ");
		self.emit_node_string(string, &arguments[1]);
		string.push_str(", ");
		self.emit_node_string(string, &arguments[2]);
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

	fn emit_atomic_compare_exchange_assignment(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> bool {
		let Some(arguments) = Self::atomic_compare_exchange_arguments(right) else {
			return false;
		};
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::VariableDeclaration { name, r#type }) = left.node() else {
			return false;
		};

		// HLSL exposes the previous value through an out parameter.
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
		string.push(';');
		self.emit_atomic_compare_exchange_call(string, &arguments, Some(name));
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
		let array_texture = Self::node_type_name(&arguments[0]).as_deref() == Some("ArrayTexture2D");
		if array_texture {
			string.push_str("uint ");
			string.push_str(name);
			string.push_str("_layers;");
		}
		self.emit_node_string(string, &arguments[0]);
		string.push_str(".GetDimensions(");
		string.push_str(name);
		string.push_str(".x, ");
		string.push_str(name);
		string.push_str(".y");
		if array_texture {
			string.push_str(", ");
			string.push_str(name);
			string.push_str("_layers");
		}
		string.push(')');
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
		if let Some(vector_type) = crate::shader::generator::scalar_array_vector_type(type_name) {
			string.push_str(Self::translate_type(vector_type));
			string.push(' ');
			string.push_str(name);
			string.push_str(" = ");
			self.emit_node_string(string, value);
		} else if let Some((element_type, count)) = Self::hlsl_array_type(type_name) {
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

	/// Emits the amplification-to-mesh payload shared by both Shader Model 6.5 stages.
	fn emit_object_payload_struct(&self, string: &mut String) {
		if self.task_payloads.is_empty() {
			return;
		}

		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "ObjectPayload");
		for payload in &self.task_payloads {
			let payload = payload.borrow();
			let besl::Nodes::TaskPayload { name, format, count } = payload.node() else {
				continue;
			};

			formatting.push_indentation(string, 1);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			string.push('[');
			string.push_str(&count.get().to_string());
			string.push(']');
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	/// Emits the fixed vertex output and the authored per-primitive mesh outputs.
	fn emit_mesh_output_structs(&self, string: &mut String) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position : SV_Position");
		formatting.push_statement_end(string);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		if self.mesh_uses_render_target_array_index {
			formatting.push_indentation(string, 1);
			string.push_str("uint32_t render_target_array_index : SV_RenderTargetArrayIndex");
			formatting.push_statement_end(string);
		}
		for output in &self.mesh_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: Some(_),
			} = output.node()
			else {
				continue;
			};

			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" : TEXCOORD");
			string.push_str(&location.to_string());
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
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
		const SUBGROUP_INTRINSICS: [&str; 8] = [
			"subgroup_lane_index",
			"subgroup_ballot",
			"subgroup_ballot_any",
			"subgroup_ballot_find_lsb",
			"subgroup_ballot_count",
			"subgroup_ballot_and_not",
			"subgroup_broadcast_u32",
			"subgroup_broadcast_f32",
		];
		order.iter().any(|node| {
			SUBGROUP_INTRINSICS
				.iter()
				.any(|intrinsic| Self::uses_intrinsic(node, intrinsic))
		})
	}

	/// Recovers an indexed mesh-output declaration so HLSL can address its primitive structure field.
	fn hlsl_mesh_output_target(left: &besl::NodeReference) -> Option<String> {
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = left.node() else {
			return None;
		};
		let source = source.borrow();
		let besl::Nodes::Output {
			name, count: Some(_), ..
		} = source.node()
		else {
			return None;
		};
		Some(name.clone())
	}

	/// Finds a lane-guarded BESL mesh-count statement that HLSL must execute uniformly.
	fn mesh_output_count_arguments(statements: &[besl::NodeReference]) -> Option<(besl::NodeReference, besl::NodeReference)> {
		let [statement] = statements else {
			return None;
		};
		let statement = statement.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = statement.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, .. } = intrinsic.node() else {
			return None;
		};
		let [vertices, primitives] = arguments.as_slice() else {
			return None;
		};
		(name == "set_mesh_output_counts").then(|| (vertices.clone(), primitives.clone()))
	}

	/// Emits raster stage I/O as mutable entry-point parameters because HLSL semantic globals are immutable.
	fn emit_raster_entry_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let mut has_previous_parameter = has_previous_parameter;
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, location, format } = input.node() else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if self.current_stage_interpolates_inputs && Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" : TEXCOORD");
			string.push_str(&location.to_string());
			has_previous_parameter = true;
		}

		for output in &self.raster_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: None,
			} = output.node()
			else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if self.current_stage_interpolates_outputs && Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str("out ");
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(if self.current_stage == HlslStage::Fragment {
				" : SV_Target"
			} else {
				" : TEXCOORD"
			});
			string.push_str(&location.to_string());
			has_previous_parameter = true;
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
			"sample" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
				return;
			}
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
			"pow" if arguments.len() == 2 && super::is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "atan2"
			| "floor" | "round" | "fwidth" | "step" | "radians" | "smoothstep" | "dot" | "cross" | "normalize" | "reflect"
			| "length" => {
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
			"f16" => {
				string.push_str("float16_t(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"u16" => {
				string.push_str("uint(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" | "packed_vec4f" => {
				string.push_str(Self::translate_type(name));
				string.push('(');
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
				if arguments.len() == 3 {
					string.push_str(".Load(int4(");
				} else {
					string.push_str(".Load(int3(");
				}
				self.emit_node_string(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str(", int(");
					self.emit_node_string(string, layer);
					string.push(')');
				}
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
			"texture_lod" | "downsample_min" | "downsample_max" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				if arguments.len() == 4 {
					string.push_str("float3(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", float(");
					self.emit_node_string(string, &arguments[2]);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				string.push_str(", ");
				if let Some(lod) = arguments.get(if arguments.len() == 4 { 3 } else { 2 }) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
				if name != "texture_lod" {
					string.push_str(".x");
				}
			}
			"texture_cube_array_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, float4(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", float(");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(")), ");
				self.emit_node_string(string, &arguments[3]);
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
			"atomic_compare_exchange" => {
				// HLSL requires an out parameter even when BESL discards the previous value.
				string.push_str("{ uint _besl_atomic_previous; ");
				self.emit_atomic_compare_exchange_call(string, arguments, Some("_besl_atomic_previous"));
				string.push_str("; }");
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
			"thread_position" => {
				string.push_str("dispatch_thread_id.x");
			}
			"thread_idx" => {
				string.push_str("group_thread_index");
			}
			"subgroup_lane_index" => string.push_str("WaveGetLaneIndex()"),
			"threadgroup_position" => {
				string.push_str("group_id.x");
			}
			"subgroup_ballot" => {
				string.push_str("WaveActiveBallot(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_any" => {
				string.push_str("_besl_subgroup_ballot_any(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_find_lsb" => {
				string.push_str("_besl_subgroup_ballot_find_lsb(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_count" => {
				string.push_str("_besl_subgroup_ballot_count(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_and_not" => {
				string.push_str("_besl_subgroup_ballot_and_not(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => {
				string.push_str("WaveReadLaneAt(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"fma" => {
				string.push_str("mad(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"sincos" => {
				string.push_str("float2(sin(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("), cos(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"round_to_i32" => {
				string.push_str("int2(round(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"workgroup_barrier" => {
				string.push_str("GroupMemoryBarrierWithGroupSync()");
			}
			"set_task_mesh_output_count" => {
				string.push_str("besl_mesh_output_count = ");
				self.emit_node_string(string, &arguments[0]);
			}
			"set_mesh_output_counts" => {
				string.push_str("SetMeshOutputCounts(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"set_mesh_vertex_position" => {
				string.push_str("besl_vertices[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].position = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"set_mesh_triangle" => {
				string.push_str("besl_triangles[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("] = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("besl_primitives[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].render_target_array_index = ");
				self.emit_node_string(string, &arguments[1]);
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
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
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
		self.current_stage = match shader_compilation_settings.stage {
			Stages::Vertex => HlslStage::Vertex,
			Stages::Fragment => HlslStage::Fragment,
			Stages::Compute { .. } => HlslStage::Compute,
			Stages::Task { .. } => HlslStage::Task,
			Stages::Mesh { .. } => HlslStage::Mesh,
		};
		// Only fragment inputs and raster-producing outputs participate in interpolation.
		self.current_stage_interpolates_inputs = matches!(shader_compilation_settings.stage, Stages::Fragment);
		self.current_stage_interpolates_outputs =
			matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Mesh { .. });
		self.current_local_size = match shader_compilation_settings.stage {
			Stages::Compute { local_size } | Stages::Task { local_size, .. } | Stages::Mesh { local_size, .. } => {
				Some(local_size)
			}
			_ => None,
		};
		(self.current_mesh_maximum_vertices, self.current_mesh_maximum_primitives) = match shader_compilation_settings.stage {
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => (maximum_vertices, maximum_primitives),
			_ => (0, 0),
		};
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "HLSL");
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		let uses_subgroup_intrinsics = Self::uses_subgroup_intrinsics(&order);
		if uses_subgroup_intrinsics && self.current_stage != HlslStage::Compute {
			return Err(());
		}
		self.mesh_uses_render_target_array_index = order
			.iter()
			.any(|node| Self::uses_intrinsic(node, "set_mesh_primitive_render_target_array_index"));
		self.task_payloads.clear();
		self.mesh_outputs.clear();
		self.raster_inputs.clear();
		self.raster_outputs.clear();
		self.packed_write_counter = 0;
		for node in &order {
			match node.borrow().node() {
				besl::Nodes::TaskPayload { .. } => self.task_payloads.push(node.clone()),
				besl::Nodes::Output { count: Some(_), .. } => self.mesh_outputs.push(node.clone()),
				besl::Nodes::Input { .. } => self.raster_inputs.push(node.clone()),
				besl::Nodes::Output { count: None, .. } => self.raster_outputs.push(node.clone()),
				_ => {}
			}
		}
		self.user_struct_constructors.clear();
		// Discover constructor calls before declarations are emitted so their HLSL factories can stay next to each struct.
		for node in &order {
			self.emit_node_string(&mut string, node);
		}
		string.clear();

		self.generate_hlsl_header_block(&mut string, shader_compilation_settings, uses_subgroup_intrinsics);
		if self.current_stage == HlslStage::Task {
			string.push_str("groupshared uint32_t besl_mesh_output_count;");
			if !self.minified {
				string.push('\n');
			}
		}
		if self.current_stage == HlslStage::Mesh {
			self.emit_mesh_output_structs(&mut string);
		}

		for node in order {
			self.emit_node_string(&mut string, &node);
		}

		Ok(string)
	}

	/// Emits one user struct and its factory when the program constructs that type.
	fn emit_hlsl_struct_node(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		name: &str,
		fields: &[besl::NodeReference],
		template: &Option<besl::NodeReference>,
	) {
		self.emit_struct_node(string, name, fields, template);
		if template.is_none()
			&& !crate::shader::generator::is_builtin_struct_type(name, self.supports_atomic_u32())
			&& self.user_struct_constructors.contains(node)
		{
			self.emit_hlsl_struct_factory(string, name, fields);
		}
	}

	/// Emits an amplification entry point with the group-shared payload required by `DispatchMesh`.
	fn emit_hlsl_task_entry(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		statements: &[besl::NodeReference],
		return_type: &besl::NodeReference,
		params: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified);
		if !self.task_payloads.is_empty() {
			// Every amplification lane contributes to one payload, so it must use group-shared storage.
			string.push_str("groupshared ObjectPayload payload;");
			if !self.minified {
				string.push('\n');
			}
		}
		self.emit_function_attributes(string, node, "besl_main");
		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push_str(" besl_main(");
		emit_comma_separated_nodes(string, formatting, params, |string, parameter| {
			self.emit_node_string(string, parameter)
		});
		self.emit_function_extra_parameters(string, node, "besl_main", !params.is_empty());
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		if !self.task_payloads.is_empty() {
			// DXIL requires DispatchMesh to dominate the entry point, so every lane converges after BESL selects the count.
			formatting.push_indentation(string, 1);
			string.push_str("GroupMemoryBarrierWithGroupSync()");
			formatting.push_statement_end(string);
			formatting.push_indentation(string, 1);
			string.push_str("DispatchMesh(besl_mesh_output_count, 1, 1, payload)");
			formatting.push_statement_end(string);
		}
		self.emit_block_end(string);
	}

	/// Emits a field-by-field factory because DXC does not support user-defined struct constructor expressions.
	fn emit_hlsl_struct_factory(&mut self, string: &mut String, name: &str, fields: &[besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		string.push_str(name);
		string.push_str(" besl_construct_");
		string.push_str(name);
		string.push('(');
		for (index, field) in fields.iter().enumerate() {
			let field = field.borrow();
			let besl::Nodes::Member {
				name: field_name,
				r#type,
				count,
			} = field.node()
			else {
				continue;
			};
			if index > 0 {
				string.push_str(formatting.comma_str());
			}
			Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
			string.push_str(" besl_argument_");
			string.push_str(field_name);
			if let Some(count) = count {
				string.push('[');
				string.push_str(&count.to_string());
				string.push(']');
			}
		}
		formatting.push_block_start(string);

		formatting.push_indentation(string, 1);
		string.push_str(name);
		string.push_str(" besl_value");
		formatting.push_statement_end(string);
		for field in fields {
			let field = field.borrow();
			let besl::Nodes::Member {
				name: field_name, count, ..
			} = field.node()
			else {
				continue;
			};

			if let Some(count) = count {
				formatting.push_indentation(string, 1);
				string.push_str("[unroll] for(uint besl_index=0;besl_index<");
				string.push_str(&count.to_string());
				string.push_str(";++besl_index){");
				string.push_str("besl_value.");
				string.push_str(field_name);
				string.push_str("[besl_index]=besl_argument_");
				string.push_str(field_name);
				string.push_str("[besl_index];}");
				if !self.minified {
					string.push('\n');
				}
			} else {
				formatting.push_indentation(string, 1);
				string.push_str("besl_value.");
				string.push_str(field_name);
				string.push_str("=besl_argument_");
				string.push_str(field_name);
				formatting.push_statement_end(string);
			}
		}
		formatting.push_indentation(string, 1);
		string.push_str("return besl_value");
		formatting.push_statement_end(string);
		self.emit_block_end(string);
	}

	/// Translates BESL intrinsic type names to HLSL type names, such as `vec2f` to `float2`.
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f16" => "float16_t2",
			"vec3f16" => "float16_t3",
			"vec4f16" => "float16_t4",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "uint16_t2",
			"vec3u16" => "uint16_t3",
			"vec4u16" => "uint16_t4",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"packed_vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint",
			"u16" => "uint",
			"u32" => "uint32_t",
			"atomicu32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "Texture2D",
			"Texture3D" => "Texture3D",
			"TextureCube" => "TextureCube<float4>",
			"TextureCubeArray" => "TextureCubeArray<float4>",
			"ArrayTexture2D" => "Texture2DArray<float4>",
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
				| "uint64_t" | "int2"
				| "uint2" | "uint3"
				| "uint4" | "uint16_t2"
				| "uint16_t4"
		)
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();

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
				if hlsl_name == "besl_main" && self.current_stage == HlslStage::Task {
					self.emit_hlsl_task_entry(string, this_node, statements, return_type, params);
				} else {
					self.emit_function_node(string, this_node, hlsl_name, statements, return_type, params);
				}
			}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_hlsl_struct_node(string, this_node, name, fields, template),
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_atomic_add_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment
					&& self.emit_atomic_compare_exchange_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_image_size_assignment(string, left, right) => {}
			besl::Nodes::PushConstant { members } => {
				// Root constants use the constant-buffer namespace, while flat resources use t/u/s registers in space 0.
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
					string.push_str("};ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
				} else {
					string.push_str("};\n");
					string.push_str("ConstantBuffer<PushConstant> push_constant : register(b0, space0);\n");
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
				if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
					return;
				}
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());

				// HLSL uses semantics like TEXCOORD0, TEXCOORD1, etc.
				string.push_str(&format!(
					"{}{} {} : TEXCOORD{};{break_char}",
					if self.current_stage_interpolates_inputs && Self::is_integer_type(type_name) {
						"nointerpolation "
					} else {
						""
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
				if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
					return;
				}
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());

				// HLSL uses SV_Target0, SV_Target1, etc. for render targets
				string.push_str(&format!(
					"{}{} {} : SV_Target{};{break_char}",
					if self.current_stage_interpolates_outputs && Self::is_integer_type(type_name) {
						"nointerpolation "
					} else {
						""
					},
					type_name,
					name,
					location
				));
			}
			besl::Nodes::TaskPayload { .. } => {
				if self.task_payloads.first() == Some(this_node) {
					self.emit_object_payload_struct(string);
				}
			}
			besl::Nodes::Workgroup { name, format, count } => {
				string.push_str("groupshared ");
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
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional { statements, .. }
				if self.current_stage == HlslStage::Mesh && Self::mesh_output_count_arguments(statements).is_some() =>
			{
				let (vertices, primitives) = Self::mesh_output_count_arguments(statements).unwrap();
				// DXIL requires SetMeshOutputCounts to dominate every mesh output, so remove BESL's portable lane-zero guard.
				string.push_str("SetMeshOutputCounts(");
				self.emit_node_string(string, &vertices);
				self.emit_separator(string);
				self.emit_node_string(string, &primitives);
				string.push(')');
			}
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
				// HLSL preserves the flat slot in the matching register namespace and always uses space 0.
				let register_index = *slot;
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
							string.push_str(&format!(" : register({register_type}{register_index}, space0);"));
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

						string.push_str(&format!(" : register({register_type}{register_index}, space0);"));
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

						string.push_str(&format!(" : register(u{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						// HLSL separates textures and samplers, but for combined sampler we use Texture2D
						let texture_type = match format.as_str() {
							"Texture3D" => "Texture3D",
							"TextureCube" => "TextureCube",
							"TextureCubeArray" => "TextureCubeArray",
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

						string.push_str(&format!(" : register(t{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}

						// Also declare a sampler with the same name + _sampler suffix
						string.push_str("SamplerState ");
						string.push_str(name);
						string.push_str("_sampler");
						string.push_str(&format!(" : register(s{register_index}, space0);"));
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

	fn generate_hlsl_header_block(
		&self,
		hlsl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		uses_subgroup_intrinsics: bool,
	) {
		// HLSL doesn't use #version, but we can add shader model target as a comment
		hlsl_block.push_str("// Shader Model 6.0+\n");

		// Shader type as comment (user preference: Option B)
		match compilation_settings.stage {
			Stages::Vertex => hlsl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => hlsl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => hlsl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task { .. } => hlsl_block.push_str("// #pragma shader_stage(amplification)\n"),
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
			}
			Stages::Task { .. } => hlsl_block.push_str("// Requires: Amplification shader support\n"),
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
		if uses_subgroup_intrinsics {
			hlsl_block.push_str(
				"bool _besl_subgroup_ballot_any(uint4 mask) { return any(mask); }\n\
				 uint _besl_subgroup_ballot_find_lsb(uint4 mask) { if (mask.x != 0u) { return firstbitlow(mask.x); } if (mask.y != 0u) { return 32u + firstbitlow(mask.y); } if (mask.z != 0u) { return 64u + firstbitlow(mask.z); } if (mask.w != 0u) { return 96u + firstbitlow(mask.w); } return 0xffffffffu; }\n\
				 uint _besl_subgroup_ballot_count(uint4 mask) { return countbits(mask.x) + countbits(mask.y) + countbits(mask.z) + countbits(mask.w); }\n\
				 uint4 _besl_subgroup_ballot_and_not(uint4 mask, uint4 removed) { return mask & ~removed; }\n",
			);
		}
	}

	/// Emits the 32-bit word containing one packed logical narrow-buffer element.
	fn emit_packed_word_access_by_name(
		&self,
		string: &mut String,
		binding_name: &str,
		index_name: &str,
		elements_per_word: u32,
	) {
		string.push_str(binding_name);
		string.push('[');
		string.push_str(index_name);
		let _ = write!(string, "/{elements_per_word}u]");
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
		if name != "besl_main" {
			return;
		}

		if self.current_stage == HlslStage::Mesh {
			string.push_str("[outputtopology(\"triangle\")]");
			if !self.minified {
				string.push('\n');
			}
		}

		let Some(local_size) = self.current_local_size else {
			return;
		};
		// HLSL attaches compute-like stage thread-group sizes directly to their entry functions.
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
		if name != "besl_main" {
			return;
		}
		if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
			self.emit_raster_entry_parameters(string, has_previous_parameter);
			return;
		}
		if !matches!(self.current_stage, HlslStage::Compute | HlslStage::Task | HlslStage::Mesh) {
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

		if self.current_stage == HlslStage::Mesh {
			if !self.task_payloads.is_empty() {
				self.emit_separator(string);
				string.push_str("in payload ObjectPayload payload");
			}
			self.emit_separator(string);
			string.push_str("out vertices VertexOutput besl_vertices[");
			string.push_str(&self.current_mesh_maximum_vertices.to_string());
			string.push(']');
			self.emit_separator(string);
			string.push_str("out primitives PrimitiveOutput besl_primitives[");
			string.push_str(&self.current_mesh_maximum_primitives.to_string());
			string.push(']');
			self.emit_separator(string);
			string.push_str("out indices uint3 besl_triangles[");
			string.push_str(&self.current_mesh_maximum_primitives.to_string());
			string.push(']');
		}
	}
	fn emit_function_call(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		parameters: &[besl::NodeReference],
	) -> bool {
		let function_node = function.borrow();
		let besl::Nodes::Struct {
			name, template: None, ..
		} = function_node.node()
		else {
			return false;
		};
		if Self::is_square_column_vector_matrix_constructor(name, parameters) {
			// Square BESL matrix constructors take columns, while their HLSL equivalents take rows.
			string.push_str("transpose(");
			string.push_str(Self::translate_type(name));
			string.push('(');
			self.emit_call_arguments(string, parameters);
			string.push_str("))");
			return true;
		}
		if crate::shader::generator::is_builtin_struct_type(name, self.supports_atomic_u32()) {
			return false;
		}
		if !self.user_struct_constructors.contains(function) {
			self.user_struct_constructors.push(function.clone());
		}

		// Route portable BESL construction through the field-by-field factory emitted with the struct.
		string.push_str("besl_construct_");
		string.push_str(name);
		string.push('(');
		self.emit_call_arguments(string, parameters);
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		match source.borrow().node() {
			besl::Nodes::TaskPayload { .. } => {
				string.push_str("payload.");
				string.push_str(name);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				string.push_str(name);
				return true;
			}
			_ => {}
		}

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
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		if let besl::Expressions::Operator { operator, left, right } = expression {
			if *operator == besl::Operators::Assignment {
				let indexed_target = {
					let left = left.borrow();
					let besl::Nodes::Expression(besl::Expressions::Accessor {
						left: member,
						right: index,
					}) = left.node()
					else {
						return false;
					};
					Some((member.clone(), index.clone()))
				};
				if let Some((member, index)) = indexed_target {
					if let Some((binding_name, _, write, element_type, flattened)) = Self::hlsl_buffer_member_target(&member) {
						if write && flattened && matches!(element_type.as_deref(), Some("u8" | "u16")) {
							let (elements_per_word, bits_per_element, element_mask) = if element_type.as_deref() == Some("u8") {
								(4u32, 8u32, "0xffu")
							} else {
								(2u32, 16u32, "0xffffu")
							};
							let temporary_id = self.packed_write_counter;
							self.packed_write_counter = self.packed_write_counter.checked_add(1).expect(
								"Packed narrow-buffer write count overflowed. The most likely cause is an invalid shader with billions of assignment nodes.",
							);
							let index_name = format!("besl_packed_index_{temporary_id}");
							let value_name = format!("besl_packed_value_{temporary_id}");

							// Adjacent logical narrow elements share one DX12 word. Clear
							// and set only this lane so concurrent writes preserve neighbors.
							// Evaluate both source expressions before changing the shared word.
							string.push_str("uint ");
							string.push_str(&index_name);
							string.push('=');
							self.emit_node_string(string, &index);
							string.push_str(";uint ");
							string.push_str(&value_name);
							string.push_str("=(uint(");
							self.emit_node_string(string, right);
							string.push_str(")&");
							string.push_str(element_mask);
							string.push_str(");InterlockedAnd(");
							self.emit_packed_word_access_by_name(string, &binding_name, &index_name, elements_per_word);
							string.push_str(",~(");
							string.push_str(element_mask);
							string.push_str("<<((");
							string.push_str(&index_name);
							let _ = write!(string, "%{elements_per_word}u)*{bits_per_element}u)));InterlockedOr(");
							self.emit_packed_word_access_by_name(string, &binding_name, &index_name, elements_per_word);
							string.push(',');
							string.push_str(&value_name);
							string.push_str("<<((");
							string.push_str(&index_name);
							let _ = write!(string, "%{elements_per_word}u)*{bits_per_element}u))");
							return true;
						}
					}
				}
			}

			let left_type = Self::node_type_name(left);
			let right_type = Self::node_type_name(right);
			if *operator == besl::Operators::Multiply
				&& left_type.as_deref() == Some("mat4x3f")
				&& right_type.as_deref() == Some("vec4f")
			{
				// HLSL float4x3 stores the four BESL columns as rows, so the vector must be the left mul operand.
				string.push_str("mul(");
				self.emit_node_string(string, right);
				string.push_str(", ");
				self.emit_node_string(string, left);
				string.push(')');
				return true;
			}
			if *operator == besl::Operators::Multiply
				&& matches!(
					(left_type.as_deref(), right_type.as_deref()),
					(Some("mat4f"), Some("mat4f" | "vec4f"))
						| (Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"), Some("f32"))
						| (Some("f32"), Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"))
				) {
				// BESL reserves algebraic multiplication for these matrix
				// shapes. Same-shaped mat4x3 values use component-wise `*`.
				string.push_str("mul(");
				self.emit_node_string(string, left);
				string.push_str(", ");
				self.emit_node_string(string, right);
				string.push(')');
				return true;
			}
			if *operator == besl::Operators::Multiply
				&& !matches!(
					(left_type.as_deref(), right_type.as_deref()),
					(Some(left), Some(right))
						if Self::is_matrix_type(Some(left)) && Self::is_matrix_type(Some(right))
				) {
				let left_name = left.borrow().get_name().map(str::to_string);
				if left_name.as_deref().is_some_and(Self::hlsl_name_likely_matrix_operand) {
					// Some expression references do not retain resolved types, so preserve known matrix operand names.
					string.push_str("mul(");
					self.emit_node_string(string, left);
					string.push_str(", ");
					self.emit_node_string(string, right);
					string.push(')');
					return true;
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
					return true;
				}
			}
		}

		false
	}

	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		let right_is_member = matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		);
		if right_is_member {
			if let Some((binding_name, field_name, _, _, flattened)) = Self::hlsl_buffer_member_target(left) {
				if field_name != binding_name {
					// A component selected from a buffer field remains an HLSL swizzle after the buffer access itself is lowered.
					string.push_str(&binding_name);
					if !flattened {
						string.push_str("[0].");
						string.push_str(&field_name);
					}
					string.push('.');
					self.emit_node_string(string, right);
					return;
				}
			}
		}

		if let Some(field_name) = Self::hlsl_mesh_output_target(left) {
			// Mesh primitive attributes live in the native per-primitive output array rather than module globals.
			string.push_str("besl_primitives[");
			self.emit_node_string(string, right);
			string.push_str("].");
			string.push_str(&field_name);
			return;
		}

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

		if !right_is_member
			&& !Self::hlsl_buffer_member_is_array(left)
			&& Self::node_type_name(left)
				.as_deref()
				.and_then(Self::hlsl_square_matrix_column_type)
				.is_some()
		{
			// BESL indexes square matrices by column. HLSL indexes them by row,
			// regardless of the storage packing selected for the shader.
			string.push_str("transpose(");
			self.emit_node_string(string, left);
			string.push_str(")[");
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		if let Some((binding_name, field_name, _, element_type, flattened)) = Self::hlsl_buffer_member_target(left) {
			if flattened && matches!(element_type.as_deref(), Some("u8" | "u16")) {
				let (word_index, bit_offset, element_mask) = if element_type.as_deref() == Some("u8") {
					(") / 4u] >> (((", ") % 4u) * 8u)) & ", "0xffu")
				} else {
					(") / 2u] >> (((", ") % 2u) * 16u)) & ", "0xffffu")
				};

				// DX12 exposes packed narrow-index buffers as 32-bit structured words, so recover the logical element here.
				string.push_str("((");
				string.push_str(&binding_name);
				string.push_str("[(");
				self.emit_node_string(string, right);
				string.push_str(word_index);
				self.emit_node_string(string, right);
				string.push_str(bit_offset);
				string.push_str(element_mask);
				string.push(')');
				return;
			}

			if field_name == binding_name || flattened {
				string.push_str(&binding_name);
			} else {
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
		// BESL numeric access always remains an HLSL subscript, including when
		// its left side is itself an array-element expression.
		if !right_is_member {
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
	fn power_of_two_uses_exp2() {
		let root = besl::compile_to_besl(
			"main: fn () -> void { let full: f32 = pow(2.0, 3.0); let half: f16 = pow(f16(2.0), f16(3.0)); full; half; }",
			None,
		)
		.expect("Expected power source to link.");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main."),
			)
			.expect("Expected HLSL power lowering.");

		assert_eq!(shader.matches("exp2(").count(), 2);
		assert!(!shader.contains("pow("));
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
		assert_string_contains!(shader, "Texture2D<float4> texture : register(t2, space0);");
		assert_string_contains!(shader, "SamplerState texture_sampler : register(s2, space0);");

		// Check main function
		assert_string_contains!(shader, "void besl_main(){buff;image;texture;}");
	}

	#[test]
	fn compute_subgroup_intrinsics_lower_to_hlsl_wave_operations() {
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
			.expect("Expected subgroup fixture to lower to HLSL");

		assert_string_contains!(shader, "WaveActiveBallot(group_thread_index<4)");
		assert_string_contains!(shader, "WaveReadLaneAt(group_thread_index,leader)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_find_lsb(mask)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_count(remaining)");
	}

	#[test]
	fn vec4u16_uses_the_native_eight_byte_hlsl_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 HLSL generation");

		assert_string_contains!(shader, "uint16_t4 value;");
		assert_string_does_not_contain!(shader, "struct vec4u16");
	}

	#[test]
	fn packed_vec4f_uses_native_hlsl_vectors_in_nested_records() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::packed_vec4f_meshlet_binding(),
			)
			.expect("Expected packed_vec4f HLSL generation");

		assert_string_contains!(shader, "float4 center_radius;float4 cone_apex_cutoff;");
		assert_string_does_not_contain!(shader, "struct packed_vec4f");
	}

	#[test]
	fn vec2u16_array_uses_the_native_four_byte_hlsl_vector_type() {
		let main = generator::tests::vec2u16_array_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec2u16 HLSL generation");

		assert_string_contains!(shader, "RWStructuredBuffer<uint16_t2> buff : register(u0, space0);");
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<uint2> buff");
	}

	#[test]
	fn vec2f16_array_uses_the_native_four_byte_hlsl_vector_type() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2f16_array_binding(),
			)
			.expect("Expected vec2f16 HLSL generation");

		assert_string_contains!(shader, "RWStructuredBuffer<float16_t2> buff : register(u0, space0);");
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<float2> buff");
	}

	#[test]
	fn f16_storage_types_use_native_hlsl_types() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_f16_storage_binding(),
			)
			.expect("Expected f16 HLSL generation");

		assert_string_contains!(shader, "float16_t scalar;");
		assert_string_contains!(shader, "float16_t2 uv;");
		assert_string_contains!(shader, "float16_t3 normal;");
		assert_string_contains!(shader, "float16_t4 color;");
		assert_string_contains!(shader, "float16_t2(uv32)");
		assert_string_contains!(shader, "float2(uv16)");
		assert_string_contains!(shader, "float16_t(0.5)");
		assert_string_contains!(shader, "float(weight16)");
		assert_string_contains!(shader, "float16_t literal=float16_t(0.25);");
		assert_string_contains!(shader, "weight16*float16_t(2.0)");
		assert_string_contains!(shader, "uv16*float16_t(2.0)");
		assert_string_does_not_contain!(shader, "struct vec2f16");
	}

	#[test]
	fn vector_components_use_hlsl_members_and_numeric_indices_use_subscripts() {
		let root = besl::parse(
			r#"
			main: fn() -> void {
				let vector: vec4f = vec4f(1.0, 2.0, 3.0, 4.0);
				let component: f32 = vector.x;
				let indexed_component: f32 = vector[1];
				let joints: vec4u16 = vec4u16(0, 1, 2, 3);
				let joint_component: u16 = joints.x;
				let indexed_joint: u16 = joints[1];
				if (component > indexed_component) {
					return;
				}
				if (joint_component > indexed_joint) {
					return;
				}
			}
			"#,
		)
		.expect("Expected vector access shader source to parse");
		let root = besl::lex(root).expect("Expected vector access shader source to lex");
		let main = root
			.borrow()
			.get_child("main")
			.expect("Expected vector access shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vector access shader source to generate HLSL");

		assert_string_contains!(shader, "float component=vector.x;");
		assert_string_contains!(shader, "float indexed_component=vector[1];");
		assert_string_contains!(shader, "uint joint_component=joints.x;");
		assert_string_contains!(shader, "uint indexed_joint=joints[1];");
		assert_string_does_not_contain!(shader, "vector[x]");
		assert_string_does_not_contain!(shader, "joints[x]");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"vector-access-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected vector access HLSL to compile to DXIL");
	}

	#[test]
	fn user_struct_constructors_lower_to_hlsl_factories() {
		let root = besl::compile_to_besl(
			r#"
			Pair: struct {
				left: vec4f,
				right: vec4f,
			}

			main: fn () -> void {
				let pair: Pair = Pair(
					vec4f(1.0, 1.0, 1.0, 1.0),
					vec4f(2.0, 2.0, 2.0, 2.0)
				);
				pair;
			}
			"#,
			None,
		)
		.expect("Expected user struct constructor shader source to compile");
		let main = root
			.get_main()
			.expect("Expected user struct constructor shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected user struct constructor shader source to generate HLSL");

		assert_string_contains!(
			shader,
			"Pair pair=besl_construct_Pair(float4(1.0,1.0,1.0,1.0),float4(2.0,2.0,2.0,2.0));"
		);
		assert_string_does_not_contain!(shader, "Pair pair=Pair(");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"user-struct-constructor-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected user struct constructor HLSL to compile to DXIL");
	}

	#[test]
	fn affine_matrix_columns_and_mat4x3_multiplication_preserve_besl_semantics_in_dxil() {
		let mut root = besl::Node::root();
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_child(
			besl::Node::binding(
				"results",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec4f, 4)],
				},
				0,
				false,
				true,
			)
			.into(),
		);
		let root = besl::compile_to_besl(
			r#"
			extend_vec3f: fn (value: vec3f, w: f32) -> vec4f {
				return vec4f(value.x, value.y, value.z, w);
			}

			expand_affine: fn (model: mat4x3f) -> mat4f {
				return mat4f(
					extend_vec3f(model[0], 0.0),
					extend_vec3f(model[1], 0.0),
					extend_vec3f(model[2], 0.0),
					extend_vec3f(model[3], 1.0)
				);
			}

			transform_affine: fn (model: mat4x3f, position: vec4f) -> vec3f {
				return model * position;
			}

			componentwise_affine: fn (left: mat4x3f, right: mat4x3f) -> mat4x3f {
				return left * right;
			}

			main: fn () -> void {
				let model: mat4x3f = mat4x3f(
					vec3f(1.0, 0.0, 0.0),
					vec3f(0.0, 1.0, 0.0),
					vec3f(0.0, 0.0, 1.0),
					vec3f(10.0, 20.0, 30.0)
				);
				let position: vec4f = vec4f(2.0, 3.0, 4.0, 1.0);
				let compact_result: vec3f = transform_affine(model, position);
				let expanded_model: mat4f = expand_affine(model);
				let expanded_result: vec4f = expanded_model * position;
				let componentwise_result: mat4x3f = componentwise_affine(model, model);
				results.values[0] = extend_vec3f(compact_result, 1.0);
				results.values[1] = expanded_result;
				results.values[2] = expanded_model[3];
				results.values[3] = extend_vec3f(componentwise_result[3], 1.0);
			}
			"#,
			Some(root),
		)
		.expect("Expected affine-matrix shader source to compile");
		let main = root.get_main().expect("Expected affine-matrix shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected affine-matrix shader source to generate HLSL");

		assert_string_contains!(shader, "return mul(position, model);");
		assert_string_contains!(
			shader,
			"return transpose(float4x4(extend_vec3f(model[0],0.0),extend_vec3f(model[1],0.0),extend_vec3f(model[2],0.0),extend_vec3f(model[3],1.0)));"
		);
		assert_string_contains!(
			shader,
			"float4x3 model=float4x3(float3(1.0,0.0,0.0),float3(0.0,1.0,0.0),float3(0.0,0.0,1.0),float3(10.0,20.0,30.0));"
		);
		assert_string_contains!(shader, "return left*right;");
		assert_string_contains!(shader, "results[2]=transpose(expanded_model)[3];");
		assert_string_contains!(shader, "float4x3 componentwise_result=componentwise_affine(model,model);");
		assert_string_contains!(shader, "model[3]");
		assert_string_does_not_contain!(shader, "mul(model, position)");
		assert_string_does_not_contain!(shader, "mul(left, right)");
		assert_string_does_not_contain!(shader, "return float4x4(extend_vec3f(model[0]");
		assert_string_does_not_contain!(shader, "transpose(model)[3]");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"affine-matrix-semantics-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected affine-matrix HLSL to compile to DXIL");
	}

	#[test]
	fn square_matrix_columns_survive_buffer_and_expression_access_in_dxil() {
		let mut root = besl::Node::root();
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_children(vec![
			besl::Node::binding(
				"wrapped",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("matrix", mat4f.clone()).into()],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"matrices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", mat4f, 2)],
				},
				1,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"results",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec4f, 6)],
				},
				2,
				false,
				true,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(
			r#"
			copy_matrix_columns: fn (matrix: mat4f) -> mat4f {
				return mat4f(matrix[0], matrix[1], matrix[2], matrix[3]);
			}

			direct_constructed_column: fn (matrix: mat4f) -> vec4f {
				return mat4f(matrix[0], matrix[1], matrix[2], matrix[3])[2];
			}

			matrix_arithmetic_columns: fn (matrix: mat4f, scale: f32) -> vec4f {
				let multiplied: vec4f = (matrix * 2.0)[0];
				let added: vec4f = (matrix + scale)[1];
				let divided: vec4f = (matrix / scale)[2];
				let subtracted: vec4f = (scale - matrix)[3];
				let remainder: vec4f = (matrix % scale)[0];
				return multiplied + added + divided + subtracted + remainder;
			}

			main: fn () -> void {
				results.values[0] = wrapped.matrix[1];
				results.values[1] = matrices.values[0][2];
				results.values[2] = (wrapped.matrix + matrices.values[0])[3];
				results.values[3] = copy_matrix_columns(wrapped.matrix)[2];
				results.values[4] = direct_constructed_column(matrices.values[1]);
				results.values[5] = matrix_arithmetic_columns(wrapped.matrix, 2.0);
			}
			"#,
			Some(root),
		)
		.expect("Expected buffered matrix-column shader source to compile");
		let main = root
			.get_main()
			.expect("Expected buffered matrix-column shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected buffered matrix-column shader source to generate HLSL");

		assert_string_contains!(shader, "results[0]=transpose(wrapped[0].matrix)[1];");
		assert_string_contains!(shader, "results[1]=transpose(matrices[0])[2];");
		assert_string_contains!(shader, "results[2]=transpose(wrapped[0].matrix+matrices[0])[3];");
		assert_string_contains!(
			shader,
			"return transpose(float4x4(transpose(matrix)[0],transpose(matrix)[1],transpose(matrix)[2],transpose(matrix)[3]));"
		);
		assert_string_contains!(
			shader,
			"return transpose(transpose(float4x4(transpose(matrix)[0],transpose(matrix)[1],transpose(matrix)[2],transpose(matrix)[3])))[2];"
		);
		assert_string_contains!(shader, "results[3]=transpose(copy_matrix_columns(wrapped[0].matrix))[2];");
		assert_string_contains!(shader, "results[4]=direct_constructed_column(matrices[1]);");
		assert_string_contains!(shader, "float4 multiplied=transpose(mul(matrix, 2.0))[0];");
		assert_string_contains!(shader, "float4 added=transpose(matrix+scale)[1];");
		assert_string_contains!(shader, "float4 divided=transpose(matrix/scale)[2];");
		assert_string_contains!(shader, "float4 subtracted=transpose(scale-matrix)[3];");
		assert_string_contains!(shader, "float4 remainder=transpose(matrix%scale)[0];");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"buffered-matrix-column-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected buffered matrix-column HLSL to compile to DXIL");
	}

	#[test]
	fn task_payload_compaction_uses_groupshared_storage_and_compiles_as_dxil_amplification_shader() {
		let root = besl::compile_to_besl(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			visible_count: workgroup<atomicu32>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				if (lane == 0) {
					atomic_store(visible_count, 0);
				}
				workgroup_barrier();
				if (thread_position() < 32) {
					let payload_index: u32 = atomic_add(visible_count, 1);
					meshlet_indices[payload_index] = thread_position();
				}
				workgroup_barrier();
				if (lane == 0) {
					set_task_mesh_output_count(atomic_load(visible_count));
				}
			}
			"#,
			None,
		)
		.expect("Expected task shader source to compile");
		let main = root.get_main().expect("Expected task shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::task(utils::Extent::line(32), 32), &main)
			.expect("Expected task shader source to generate HLSL");

		assert_string_contains!(shader, "struct ObjectPayload{uint32_t meshlet_indices[32];};");
		assert_string_contains!(shader, "groupshared uint32_t visible_count;");
		assert_string_contains!(shader, "[numthreads(32, 1, 1)]");
		assert_string_contains!(shader, "groupshared ObjectPayload payload;");
		assert_string_contains!(shader, "besl_mesh_output_count = visible_count;");
		assert_string_contains!(shader, "DispatchMesh(besl_mesh_output_count, 1, 1, payload);");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"task-payload-regression",
			"besl_main",
			crate::types::ShaderTypes::Task,
		)
		.expect("Expected task HLSL to compile to amplification DXIL");
	}

	#[test]
	fn mesh_payload_and_primitive_outputs_compile_as_dxil_mesh_shader() {
		let root = besl::compile_to_besl(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			out_instance_index: output<u32, 0, 1>;
			out_primitive_index: output<u32, 1, 1>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				let meshlet_index: u32 = meshlet_indices[threadgroup_position()];
				if (lane == 0) {
					set_mesh_output_counts(3, 1);
				}
				if (lane < 3) {
					set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
				}
				if (lane < 1) {
					set_mesh_triangle(0, vec3u(0, 1, 2));
					set_mesh_primitive_render_target_array_index(0, 2);
					out_instance_index[0] = meshlet_index;
					out_primitive_index[0] = meshlet_index;
				}
			}
			"#,
			None,
		)
		.expect("Expected mesh shader source to compile");
		let main = root.get_main().expect("Expected mesh shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)), &main)
			.expect("Expected mesh shader source to generate HLSL");

		assert_string_contains!(shader, "struct ObjectPayload{uint32_t meshlet_indices[32];};");
		assert_string_contains!(shader, "struct VertexOutput{float4 position : SV_Position;};");
		assert_string_contains!(shader, "struct PrimitiveOutput{");
		assert_string_contains!(shader, "uint32_t render_target_array_index : SV_RenderTargetArrayIndex;");
		assert_string_contains!(shader, "nointerpolation uint32_t out_instance_index : TEXCOORD0;");
		assert_string_contains!(shader, "nointerpolation uint32_t out_primitive_index : TEXCOORD1;");
		assert_string_contains!(shader, "[outputtopology(\"triangle\")][numthreads(32, 1, 1)]");
		assert_string_contains!(shader, "in payload ObjectPayload payload");
		assert_string_contains!(shader, "SetMeshOutputCounts(3,1);");
		assert_string_contains!(shader, "besl_vertices[lane].position = float4(float(lane),0.0,0.0,1.0)");
		assert_string_contains!(shader, "besl_triangles[0] = uint3(0,1,2)");
		assert_string_contains!(shader, "besl_primitives[0].render_target_array_index = 2");
		assert_string_contains!(shader, "besl_primitives[0].out_instance_index=meshlet_index");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"mesh-output-regression",
			"besl_main",
			crate::types::ShaderTypes::Mesh,
		)
		.expect("Expected mesh HLSL to compile to mesh DXIL");
	}

	#[test]
	fn array_texture_binding_declares_single_hlsl_template_argument() {
		let mut root =
			besl::parse("main: fn () -> void { shadow_map; }").expect("Expected array texture binding shader source to parse");
		root.add(vec![besl::parser::Node::binding(
			"shadow_map",
			besl::parser::Node::combined_array_image_sampler(),
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

		assert_string_contains!(shader, "Texture2DArray<float4> shadow_map : register(t11, space0);");
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

		assert_string_contains!(shader, "void besl_main(float3 color : TEXCOORD0){color;}");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void besl_main(out float3 color : TEXCOORD0){color;}");
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_nointerpolation_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected packed integer vertex HLSL generation");
		let fragment_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Expected packed integer fragment HLSL generation");

		assert_string_contains!(vertex_shader, "uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(vertex_shader, "nointerpolation out uint16_t4 packed_output : TEXCOORD1");
		assert_string_contains!(fragment_shader, "nointerpolation uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(fragment_shader, "out uint16_t4 packed_output : SV_Target1");
		assert_string_does_not_contain!(vertex_shader, "nointerpolation uint16_t2 packed_input");
		assert_string_does_not_contain!(fragment_shader, "nointerpolation uint16_t4 packed_output");
	}

	#[test]
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void besl_main(){float3 albedo=float3(1.0,0.0,0.0);albedo;}");
	}

	#[test]
	fn fetch_intrinsic_lowers_to_hlsl() {
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
			texel;
			color;
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
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"color_image",
				besl::BindingTypes::Image { format: String::new() },
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
			extent;
			noise;
			projected;
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
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
				1,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"output_image",
				besl::BindingTypes::Image { format: String::new() },
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
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", u32_type, 2)],
				},
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

	/// Verifies logical narrow indices are recovered from the packed words exposed by DX12.
	#[test]
	fn packed_narrow_buffer_elements_are_extracted_from_u32_words() {
		let script = r#"
		main: fn () -> void {
			let vertex_index: u16 = vertex_indices.vertex_indices[3];
			let primitive_index: u8 = primitive_indices.primitive_indices[5];
			vertex_index;
			primitive_index;
		}
		"#;
		let mut root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8 type");
		let u16_type = root.get_child("u16").expect("Expected u16 type");
		root.add_children(vec![
			besl::Node::binding(
				"vertex_indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("vertex_indices", u16_type, 8)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"primitive_indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("primitive_indices", u8_type, 8)],
				},
				1,
				true,
				false,
			)
			.into(),
		]);
		let main = besl::compile_to_besl(script, Some(root))
			.expect("Failed to compile packed narrow-buffer BESL. The most likely cause is invalid test source.")
			.get_main()
			.expect("Expected packed narrow-buffer main function");
		let shader = Generator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Failed to generate HLSL for packed narrow buffers. The most likely cause is unsupported buffer access.");

		assert_string_contains!(shader, "vertex_indices[(3) / 2u] >> (((3) % 2u) * 16u)) & 0xffffu");
		assert_string_contains!(shader, "primitive_indices[(5) / 4u] >> (((5) % 4u) * 8u)) & 0xffu");
	}

	/// Verifies read-write narrow buffers preserve packed neighbors when one logical element changes.
	#[test]
	fn packed_narrow_buffer_writes_use_atomic_word_updates() {
		let script = r#"
		next_index: fn () -> u32 {
			return 5;
		}

		main: fn () -> void {
			bytes.values[next_index()] = bytes.values[5];
			shorts.values[3] = shorts.values[3];
		}
		"#;
		let mut root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8 type");
		let u16_type = root.get_child("u16").expect("Expected u16 type");
		root.add_children(vec![
			besl::Node::binding(
				"bytes",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", u8_type, 8)],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"shorts",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", u16_type, 8)],
				},
				1,
				true,
				true,
			)
			.into(),
		]);
		let main = besl::compile_to_besl(script, Some(root))
			.expect("Failed to compile read-write narrow-buffer BESL. The most likely cause is invalid test source.")
			.get_main()
			.expect("Expected read-write narrow-buffer main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect(
				"Failed to generate HLSL for read-write narrow buffers. The most likely cause is unsupported packed assignment.",
			);

		assert_string_contains!(shader, "RWStructuredBuffer<uint> bytes : register(u0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint> shorts : register(u1, space0);");
		assert_string_contains!(shader, "bytes[(5) / 4u] >> (((5) % 4u) * 8u)) & 0xffu");
		assert_string_contains!(shader, "shorts[(3) / 2u] >> (((3) % 2u) * 16u)) & 0xffffu");
		assert_string_contains!(shader, "uint besl_packed_index_");
		assert_string_contains!(shader, "uint besl_packed_value_");
		assert_string_contains!(shader, "InterlockedAnd(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedOr(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedAnd(shorts[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedOr(shorts[besl_packed_index_");
		assert_eq!(
			shader.matches("=next_index();").count(),
			1,
			"Packed writes must evaluate their index expression exactly once."
		);
		let value_position = shader
			.find("uint besl_packed_value_")
			.expect("Expected packed value temporary");
		let clear_position = shader.find("InterlockedAnd(bytes").expect("Expected packed byte clear");
		assert!(
			value_position < clear_position,
			"Packed writes must evaluate a self-reading right-hand side before clearing its destination lane."
		);

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"packed-narrow-buffer-write-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected read-write narrow-buffer HLSL to compile to DXIL");
	}

	#[test]
	fn atomic_compare_exchange_lowers_to_hlsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
			atomic_compare_exchange(shared_keys[thread_idx()], 7, 9);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected compare-exchange shader source to lex");
		let main = root.get_main().expect("Expected compare-exchange main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Expected compare-exchange source to lower to HLSL");

		assert_string_contains!(
			shader,
			"uint32_t previous;InterlockedCompareExchange(shared_keys[group_thread_index], 4294967295, 7, previous);"
		);
		assert_string_contains!(
			shader,
			"{ uint _besl_atomic_previous; InterlockedCompareExchange(shared_keys[group_thread_index], 7, 9, _besl_atomic_previous); }"
		);
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
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
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
			projected;
			sun;
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
	fn push_constants_and_flat_resources_use_space_zero() {
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

		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
		assert_string_contains!(shader, "StructuredBuffer<uint32_t> values : register(t7, space0);");
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
	fn const_array_variable_lowers_to_hlsl() {
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

		assert_string_contains!(shader, "static const float3 WEIGHTS = float3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
		assert_string_does_not_contain!(shader, "WEIGHTS[3]");
	}

	#[test]
	fn short_scalar_arrays_lower_to_hlsl_vectors() {
		let script = r#"
		scalar_f32: fn () -> f32[3] {
			return f32[3](0.5, 0.25, 0.125);
		}
		scalar_u16: fn () -> u16[3] {
			return u16[3](1, 2, 3);
		}
		scalar_u32: fn () -> u32[3] {
			return u32[3](4, 5, 6);
		}
		mirror_indices: fn (indices: u32[3]) -> u32[3] {
			return indices;
		}
		main: fn () -> void {
			let floats: f32[3] = scalar_f32();
			let shorts: u16[3] = scalar_u16();
			let indices: u32[3] = mirror_indices(scalar_u32());
			let sum: f32 = floats[1] + f32(shorts[1]) + f32(indices[1]);
			sum;
		}
		"#;
		let root = besl::compile_to_besl(script, None).expect("Expected scalar-array shader source to lex");
		let main = root.get_main().expect("Expected scalar-array main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected scalar arrays to lower to HLSL vectors");

		assert_string_contains!(shader, "float3 scalar_f32()");
		assert_string_contains!(shader, "uint16_t3 scalar_u16()");
		assert_string_contains!(shader, "uint3 scalar_u32()");
		assert_string_contains!(shader, "uint3 mirror_indices(uint3 indices)");
		assert_string_contains!(shader, "float3 floats=scalar_f32();");
		assert_string_contains!(shader, "uint16_t3 shorts=scalar_u16();");
		assert_string_contains!(shader, "uint3 indices=mirror_indices(scalar_u32());");
	}

	#[test]
	fn mix_intrinsic_lowers_to_hlsl_lerp() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = mix(0.0, 1.0, 0.5);
			value;
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
