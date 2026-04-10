use std::{cell::RefCell, collections::BTreeMap};

use crate::shader_generator::{
	emit_comma_separated_nodes, emit_statement_block as emit_shared_statement_block, is_builtin_struct_type, operator_token,
	ordered_shader_nodes, MatrixLayouts, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
};

/// The `MSLShaderGenerator` struct generates Metal Shading Language shaders from BESL ASTs while preserving the engine's row-major matrix semantics in Metal.
///
/// # Parameters
///
/// - *minified*: Controls whether the shader string output is minified. Is `true` by default in release builds.
///
/// Matrix multiplications are emitted with reversed operands whenever a matrix participates in the `*` expression so the generated MSL matches the engine's row-major conventions against Metal's column-major multiplication rules.
pub struct MSLShaderGenerator {
	minified: bool,
	compute_binding_mode: ComputeBindingMode,
	in_compute_body: bool,
	compute_stage_context: Option<ComputeStageContext>,
	mesh_stage_context: Option<MeshStageContext>,
}

const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeBindingMode {
	ArgumentBuffers,
	BareResources,
}

#[derive(Clone, Debug)]
struct MeshStageContext {
	binding_sets: Vec<u32>,
	has_push_constant: bool,
	maximum_vertices: u32,
	maximum_primitives: u32,
}

#[derive(Clone, Debug)]
struct ComputeStageContext {
	binding_sets: Vec<u32>,
	has_push_constant: bool,
}

impl ShaderGenerator for MSLShaderGenerator {}

impl MSLShaderGenerator {
	/// Creates a new MSLShaderGenerator.
	pub fn new() -> Self {
		MSLShaderGenerator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			compute_binding_mode: ComputeBindingMode::ArgumentBuffers,
			in_compute_body: false,
			compute_stage_context: None,
			mesh_stage_context: None,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	pub fn compute_binding_mode(mut self, compute_binding_mode: ComputeBindingMode) -> Self {
		self.compute_binding_mode = compute_binding_mode;
		self
	}

	fn emit_wrapped_expression(&mut self, string: &mut String, node: &besl::NodeReference) {
		match node.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::Operator { .. } | besl::Expressions::Expression { .. }) => {
				string.push('(');
				self.emit_node_string(string, node);
				string.push(')');
			}
			_ => self.emit_node_string(string, node),
		}
	}

	fn is_matrix_type_name(type_name: &str) -> bool {
		matches!(type_name, "mat2f" | "mat3f" | "mat4f")
	}

	fn is_vector_type_name(type_name: &str) -> bool {
		matches!(
			type_name,
			"vec2f" | "vec3f" | "vec4f" | "vec2u" | "vec3u" | "vec4u" | "vec2i" | "vec2u16"
		)
	}

	fn is_scalar_type_name(type_name: &str) -> bool {
		matches!(type_name, "f32" | "u8" | "u16" | "u32" | "i32")
	}

	fn find_named_member_type(parent_type: &besl::NodeReference, member_name: &str) -> Option<besl::NodeReference> {
		match parent_type.borrow().node() {
			besl::Nodes::Struct { fields, .. } => fields.iter().find_map(|field| match field.borrow().node() {
				besl::Nodes::Member { name, r#type, .. } if name == member_name => Some(r#type.clone()),
				_ => None,
			}),
			_ => None,
		}
	}

	fn infer_member_type(source: &besl::NodeReference) -> Option<besl::NodeReference> {
		match source.borrow().node() {
			besl::Nodes::Member { r#type, .. }
			| besl::Nodes::Parameter { r#type, .. }
			| besl::Nodes::Input { format: r#type, .. }
			| besl::Nodes::Output { format: r#type, .. }
			| besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Const { r#type, .. } => Some(r#type.clone()),
			besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
			besl::Nodes::Expression(besl::Expressions::Member { source, name }) => {
				let parent_type = Self::infer_member_type(source)?;
				Self::find_named_member_type(&parent_type, name)
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { right, .. }) => Self::infer_expression_type(right),
			_ => None,
		}
	}

	fn infer_callable_return_type(callable: &besl::NodeReference) -> Option<besl::NodeReference> {
		match callable.borrow().node() {
			besl::Nodes::Function { return_type, .. } => Some(return_type.clone()),
			besl::Nodes::Struct { .. } => Some(callable.clone()),
			besl::Nodes::Intrinsic { r#return, .. } => Some(r#return.clone()),
			_ => None,
		}
	}

	fn infer_multiply_result_type(left: &besl::NodeReference, right: &besl::NodeReference) -> Option<besl::NodeReference> {
		let left_type = Self::infer_expression_type(left)?;
		let right_type = Self::infer_expression_type(right)?;
		let left_name = left_type.borrow().get_name().map(str::to_owned)?;
		let right_name = right_type.borrow().get_name().map(str::to_owned)?;

		if Self::is_matrix_type_name(&left_name) && Self::is_vector_type_name(&right_name) {
			return Some(right_type.clone());
		}

		if Self::is_vector_type_name(&left_name) && Self::is_matrix_type_name(&right_name) {
			return Some(left_type.clone());
		}

		if Self::is_matrix_type_name(&left_name) && Self::is_scalar_type_name(&right_name) {
			return Some(left_type.clone());
		}

		if Self::is_scalar_type_name(&left_name) && Self::is_matrix_type_name(&right_name) {
			return Some(right_type.clone());
		}

		if Self::is_vector_type_name(&left_name) && Self::is_scalar_type_name(&right_name) {
			return Some(left_type.clone());
		}

		if Self::is_scalar_type_name(&left_name) && Self::is_vector_type_name(&right_name) {
			return Some(right_type.clone());
		}

		if Self::is_matrix_type_name(&left_name) && Self::is_matrix_type_name(&right_name) {
			return Some(right_type.clone());
		}

		Some(left_type)
	}

	fn infer_expression_type(expression: &besl::NodeReference) -> Option<besl::NodeReference> {
		match expression.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => Self::infer_member_type(source),
			besl::Nodes::Expression(besl::Expressions::Accessor { right, .. }) => Self::infer_expression_type(right),
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => {
				Self::infer_callable_return_type(function)
			}
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => {
				Self::infer_callable_return_type(intrinsic)
			}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right }) => match operator {
				besl::Operators::Assignment => Self::infer_expression_type(left),
				besl::Operators::Multiply => Self::infer_multiply_result_type(left, right),
				besl::Operators::Equality
				| besl::Operators::LessThan
				| besl::Operators::Inequality
				| besl::Operators::GreaterThan
				| besl::Operators::LessThanOrEqual
				| besl::Operators::GreaterThanOrEqual
				| besl::Operators::LogicalAnd
				| besl::Operators::LogicalOr => None,
				_ => Self::infer_expression_type(left).or_else(|| Self::infer_expression_type(right)),
			},
			_ => None,
		}
	}

	fn expression_is_matrix_type(expression: &besl::NodeReference) -> bool {
		Self::infer_expression_type(expression)
			.and_then(|r#type| r#type.borrow().get_name().map(str::to_owned))
			.is_some_and(|type_name| Self::is_matrix_type_name(&type_name))
	}

	fn function_requires_compute_context(&self, function_node: &besl::NodeReference) -> bool {
		fn node_requires_compute_context(node: &besl::NodeReference, visited: &mut Vec<besl::NodeReference>) -> bool {
			if visited.iter().any(|visited_node| visited_node == node) {
				return false;
			}

			visited.push(node.clone());

			let result = match node.borrow().node() {
				besl::Nodes::Binding { .. } | besl::Nodes::PushConstant { .. } => true,
				besl::Nodes::Scope { children, .. } => {
					children.iter().any(|child| node_requires_compute_context(child, visited))
				}
				besl::Nodes::Function {
					params,
					return_type,
					statements,
					..
				} => {
					params.iter().any(|param| node_requires_compute_context(param, visited))
						|| node_requires_compute_context(return_type, visited)
						|| statements
							.iter()
							.any(|statement| node_requires_compute_context(statement, visited))
				}
				besl::Nodes::Conditional { condition, statements } => {
					node_requires_compute_context(condition, visited)
						|| statements
							.iter()
							.any(|statement| node_requires_compute_context(statement, visited))
				}
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					node_requires_compute_context(initializer, visited)
						|| node_requires_compute_context(condition, visited)
						|| node_requires_compute_context(update, visited)
						|| statements
							.iter()
							.any(|statement| node_requires_compute_context(statement, visited))
				}
				besl::Nodes::Struct { fields, .. } => fields.iter().any(|field| node_requires_compute_context(field, visited)),
				besl::Nodes::Raw { input, output, .. } => {
					input.iter().any(|input| node_requires_compute_context(input, visited))
						|| output.iter().any(|output| node_requires_compute_context(output, visited))
				}
				besl::Nodes::Parameter { r#type, .. }
				| besl::Nodes::Member { r#type, .. }
				| besl::Nodes::Specialization { r#type, .. }
				| besl::Nodes::Input { format: r#type, .. }
				| besl::Nodes::Output { format: r#type, .. } => node_requires_compute_context(r#type, visited),
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::Operator { left, right, .. } => {
						node_requires_compute_context(left, visited) || node_requires_compute_context(right, visited)
					}
					besl::Expressions::FunctionCall {
						function, parameters, ..
					} => {
						node_requires_compute_context(function, visited)
							|| parameters
								.iter()
								.any(|parameter| node_requires_compute_context(parameter, visited))
					}
					besl::Expressions::IntrinsicCall { elements, .. } | besl::Expressions::Expression { elements } => {
						elements.iter().any(|element| node_requires_compute_context(element, visited))
					}
					besl::Expressions::Macro { body, .. } => node_requires_compute_context(body, visited),
					besl::Expressions::Member { source, .. } => node_requires_compute_context(source, visited),
					besl::Expressions::VariableDeclaration { r#type, .. } => node_requires_compute_context(r#type, visited),
					besl::Expressions::Return { value } => value
						.as_ref()
						.is_some_and(|value| node_requires_compute_context(value, visited)),
					besl::Expressions::Accessor { left, right } => {
						node_requires_compute_context(left, visited) || node_requires_compute_context(right, visited)
					}
					besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
				},
				_ => false,
			};

			visited.pop();
			result
		}

		node_requires_compute_context(function_node, &mut Vec::new())
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
		let order = ordered_shader_nodes(main_function_node, "MSL");

		self.generate_msl_header_block(&mut string, shader_compilation_settings);

		match shader_compilation_settings.stage {
			Stages::Compute { .. } => self.generate_compute_shader(&mut string, &order, main_function_node),
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => self.generate_mesh_shader(&mut string, &order, main_function_node, maximum_vertices, maximum_primitives),
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
		let mut declaration_nodes = Vec::new();
		let mut function_nodes = Vec::new();

		for node in order {
			match node.borrow().node() {
				besl::Nodes::Binding { r#type, .. } => {
					bindings.push(node.clone());
				}
				besl::Nodes::PushConstant { .. } => {
					if push_constant.is_none() {
						push_constant = Some(node.clone());
					}
				}
				besl::Nodes::Function { name, .. } if name == "main" => {}
				besl::Nodes::Function { .. } => function_nodes.push(node.clone()),
				besl::Nodes::Struct { .. }
				| besl::Nodes::Raw { .. }
				| besl::Nodes::Output { .. }
				| besl::Nodes::Input { .. }
				| besl::Nodes::Intrinsic { .. }
				| besl::Nodes::Const { .. }
				| besl::Nodes::Specialization { .. } => declaration_nodes.push(node.clone()),
				_ => {}
			}
		}

		for node in declaration_nodes {
			self.emit_node_string(string, &node);
		}

		if let Some(push_constant) = push_constant.as_ref() {
			self.emit_push_constant_struct(string, push_constant);
		}

		let binding_sets = self.group_bindings_by_set(bindings.as_slice());
		let previous_compute_stage_context = self.compute_stage_context.replace(ComputeStageContext {
			binding_sets: binding_sets.keys().copied().collect(),
			has_push_constant: push_constant.is_some(),
		});
		let previous_in_compute_body = self.in_compute_body;
		self.in_compute_body = true;

		for binding in &bindings {
			if let besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} = binding.borrow().node()
			{
				self.emit_buffer_binding_struct(string, binding, members.as_slice());
			}
		}

		if matches!(self.compute_binding_mode, ComputeBindingMode::ArgumentBuffers) {
			for (&set, bindings) in &binding_sets {
				self.emit_argument_buffer_struct(string, set, bindings);
			}
		}

		for node in function_nodes.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in function_nodes.into_iter().rev() {
			self.emit_node_string(string, &node);
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers => {
				self.emit_compute_entry_point_argument_buffers(
					string,
					main_function_node,
					&binding_sets,
					push_constant.as_ref(),
				);
			}
			ComputeBindingMode::BareResources => {
				self.emit_compute_entry_point_bare_resources(
					string,
					main_function_node,
					bindings.as_slice(),
					push_constant.as_ref(),
				);
			}
		}

		self.in_compute_body = previous_in_compute_body;
		self.compute_stage_context = previous_compute_stage_context;
	}

	fn generate_mesh_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
		maximum_vertices: u32,
		maximum_primitives: u32,
	) {
		let mut bindings = Vec::new();
		let mut push_constant = None;
		let mut declaration_nodes = Vec::new();
		let mut function_nodes = Vec::new();

		for node in order {
			match node.borrow().node() {
				besl::Nodes::Binding { r#type, .. } => {
					bindings.push(node.clone());
				}
				besl::Nodes::PushConstant { .. } => {
					if push_constant.is_none() {
						self.emit_push_constant_struct(string, node);
						push_constant = Some(node.clone());
					}
				}
				besl::Nodes::Function { name, .. } if name == "main" => {}
				besl::Nodes::Function { .. } => function_nodes.push(node.clone()),
				besl::Nodes::Struct { .. }
				| besl::Nodes::Raw { .. }
				| besl::Nodes::Output { .. }
				| besl::Nodes::Input { .. }
				| besl::Nodes::Intrinsic { .. }
				| besl::Nodes::Const { .. }
				| besl::Nodes::Specialization { .. } => declaration_nodes.push(node.clone()),
				_ => {}
			}
		}

		let binding_sets = self.group_bindings_by_set(bindings.as_slice());
		let previous_mesh_stage_context = self.mesh_stage_context.replace(MeshStageContext {
			binding_sets: binding_sets.keys().copied().collect(),
			has_push_constant: push_constant.is_some(),
			maximum_vertices,
			maximum_primitives,
		});
		for node in declaration_nodes {
			self.emit_node_string(string, &node);
		}

		for binding in &bindings {
			if let besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} = binding.borrow().node()
			{
				self.emit_buffer_binding_struct(string, binding, members.as_slice());
			}
		}

		for (&set, bindings) in &binding_sets {
			self.emit_argument_buffer_struct(string, set, bindings);
		}

		for node in function_nodes.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in function_nodes.into_iter().rev() {
			self.emit_node_string(string, &node);
		}

		self.emit_mesh_entry_point_argument_buffers(
			string,
			main_function_node,
			&binding_sets,
			push_constant.as_ref(),
			maximum_vertices,
			maximum_primitives,
		);

		self.mesh_stage_context = previous_mesh_stage_context;
	}

	fn group_bindings_by_set(&self, bindings: &[besl::NodeReference]) -> BTreeMap<u32, Vec<besl::NodeReference>> {
		let mut binding_sets = BTreeMap::<u32, Vec<besl::NodeReference>>::new();

		for binding in bindings {
			let set = match binding.borrow().node() {
				besl::Nodes::Binding { set, .. } => *set,
				_ => continue,
			};

			binding_sets.entry(set).or_default().push(binding.clone());
		}

		for bindings in binding_sets.values_mut() {
			bindings.sort_by_key(|binding| match binding.borrow().node() {
				besl::Nodes::Binding { binding, .. } => *binding,
				_ => u32::MAX,
			});
		}

		binding_sets
	}

	fn emit_push_constant_struct(&mut self, string: &mut String, push_constant: &besl::NodeReference) {
		let node = push_constant.borrow();
		let besl::Nodes::PushConstant { members } = node.node() else {
			return;
		};

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

	fn emit_argument_buffer_struct(&mut self, string: &mut String, set: u32, bindings: &[besl::NodeReference]) {
		string.push_str("struct _set");
		string.push_str(set.to_string().as_str());
		if self.minified {
			string.push('{');
		} else {
			string.push_str(" {\n");
		}

		let mut next_id = 0u32;
		for binding in bindings {
			self.emit_argument_buffer_field(string, binding, &mut next_id);
		}

		string.push_str("};");
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_argument_buffer_field(&mut self, string: &mut String, binding_node: &besl::NodeReference, next_id: &mut u32) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			read,
			write,
			r#type,
			count,
			..
		} = node.node()
		else {
			return;
		};

		let emit_suffix = |string: &mut String, next_id: &mut u32| {
			if let Some(count) = count {
				string.push('[');
				string.push_str(count.to_string().as_str());
				string.push(']');
			}
			string.push_str(" [[id(");
			string.push_str(next_id.to_string().as_str());
			string.push_str(")]];");
			if !self.minified {
				string.push('\n');
			}
			*next_id += 1;
		};

		if !self.minified {
			string.push('\t');
		}

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = if *write { "device" } else { "constant" };
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {}", name, name));
				emit_suffix(string, next_id);
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
				emit_suffix(string, next_id);
			}
			besl::BindingTypes::CombinedImageSampler { format } => {
				let texture_type = match format.as_str() {
					"ArrayTexture2D" => "texture2d_array<float>",
					"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
					_ => "texture2d<float>",
				};
				string.push_str(texture_type);
				string.push(' ');
				string.push_str(name);
				emit_suffix(string, next_id);

				if !self.minified {
					string.push('\t');
				}
				string.push_str("sampler ");
				string.push_str(&format!("{}_sampler", name));
				emit_suffix(string, next_id);
			}
		}
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

	fn emit_compute_entry_point_bare_resources(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		bindings: &[besl::NodeReference],
		push_constant: Option<&besl::NodeReference>,
	) {
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

		self.emit_statement_block(string, statements, 1);

		string.push('}');
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_compute_entry_point_argument_buffers(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		binding_sets: &BTreeMap<u32, Vec<besl::NodeReference>>,
		push_constant: Option<&besl::NodeReference>,
	) {
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

		for &set in binding_sets.keys() {
			if self.minified {
				string.push(',');
			} else {
				string.push_str(", ");
			}
			string.push_str("constant _set");
			string.push_str(set.to_string().as_str());
			string.push_str("& set");
			string.push_str(set.to_string().as_str());
			string.push_str(" [[buffer(");
			string.push_str((16 + set).to_string().as_str());
			string.push_str(")]]");
		}

		if self.minified {
			string.push_str("){");
		} else {
			string.push_str(") {\n");
		}

		self.emit_statement_block(string, statements, 1);

		string.push('}');
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_mesh_entry_point_argument_buffers(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		binding_sets: &BTreeMap<u32, Vec<besl::NodeReference>>,
		push_constant: Option<&besl::NodeReference>,
		maximum_vertices: u32,
		maximum_primitives: u32,
	) {
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

		string.push_str("[[mesh]] void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');

		let mut has_previous_parameter = false;
		for param in params {
			if has_previous_parameter {
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
			}
			self.emit_node_string(string, param);
			has_previous_parameter = true;
		}

		if let Some(push_constant) = push_constant {
			if has_previous_parameter {
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
			}
			self.emit_mesh_push_constant_parameter(string, push_constant);
			has_previous_parameter = true;
		}

		for &set in binding_sets.keys() {
			if has_previous_parameter {
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
			}
			string.push_str("constant _set");
			string.push_str(set.to_string().as_str());
			string.push_str("& set");
			string.push_str(set.to_string().as_str());
			string.push_str(" [[buffer(");
			string.push_str((16 + set).to_string().as_str());
			string.push_str(")]]");
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			if self.minified {
				string.push(',');
			} else {
				string.push_str(", ");
			}
		}
		string.push_str("uint threadgroup_position [[threadgroup_position_in_grid]]");
		if self.minified {
			string.push(',');
		} else {
			string.push_str(", ");
		}
		string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		if self.minified {
			string.push(',');
		} else {
			string.push_str(", ");
		}
		string.push_str(&format!(
			"metal::mesh<VertexOutput, PrimitiveOutput, {}, {}, topology::triangle> out_mesh",
			maximum_vertices, maximum_primitives
		));

		if self.minified {
			string.push_str("){");
		} else {
			string.push_str(") {\n");
		}

		self.emit_statement_block(string, statements, 1);

		string.push('}');
		if !self.minified {
			string.push('\n');
		}
	}

	fn emit_mesh_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
	}

	fn emit_compute_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
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

	fn emit_compute_binding_reference(&self, string: &mut String, set: u32, name: &str) {
		if self.mesh_stage_context.is_some() {
			string.push_str("set");
			string.push_str(set.to_string().as_str());
			string.push('.');
			string.push_str(name);
			return;
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers => {
				string.push_str("set");
				string.push_str(set.to_string().as_str());
				string.push('.');
				string.push_str(name);
			}
			ComputeBindingMode::BareResources => string.push_str(name),
		}
	}

	fn emit_mesh_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(mesh_stage_context) = &self.mesh_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		let separator = if self.minified { "," } else { ", " };

		if mesh_stage_context.has_push_constant {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("constant PushConstant& push_constant");
			has_previous_parameter = true;
		}

		for &set in &mesh_stage_context.binding_sets {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("constant _set");
			string.push_str(set.to_string().as_str());
			string.push_str("& set");
			string.push_str(set.to_string().as_str());
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			string.push_str(separator);
		}
		string.push_str("uint threadgroup_position");
		string.push_str(separator);
		string.push_str("uint thread_index");
		string.push_str(separator);
		string.push_str(&format!(
			"metal::mesh<VertexOutput, PrimitiveOutput, {}, {}, topology::triangle> out_mesh",
			mesh_stage_context.maximum_vertices, mesh_stage_context.maximum_primitives
		));
	}

	fn emit_mesh_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(mesh_stage_context) = &self.mesh_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		let separator = if self.minified { "," } else { ", " };

		if mesh_stage_context.has_push_constant {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("push_constant");
			has_previous_parameter = true;
		}

		for &set in &mesh_stage_context.binding_sets {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("set");
			string.push_str(set.to_string().as_str());
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			string.push_str(separator);
		}
		string.push_str("threadgroup_position");
		string.push_str(separator);
		string.push_str("thread_index");
		string.push_str(separator);
		string.push_str("out_mesh");
	}

	fn emit_compute_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		if self.mesh_stage_context.is_some() {
			self.emit_mesh_hidden_parameters(string, has_previous_parameter);
			return;
		}

		let Some(compute_stage_context) = &self.compute_stage_context else {
			return;
		};

		if !self.in_compute_body {
			return;
		}

		let mut has_previous_parameter = has_previous_parameter;
		let separator = if self.minified { "," } else { ", " };

		if has_previous_parameter {
			string.push_str(separator);
		}
		string.push_str("uint2 gid");
		has_previous_parameter = true;

		if compute_stage_context.has_push_constant {
			string.push_str(separator);
			string.push_str("constant PushConstant& push_constant");
			has_previous_parameter = true;
		}

		for &set in &compute_stage_context.binding_sets {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("constant _set");
			string.push_str(set.to_string().as_str());
			string.push_str("& set");
			string.push_str(set.to_string().as_str());
			has_previous_parameter = true;
		}
	}

	fn emit_compute_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		if self.mesh_stage_context.is_some() {
			self.emit_mesh_hidden_call_arguments(string, has_previous_parameter);
			return;
		}

		let Some(compute_stage_context) = &self.compute_stage_context else {
			return;
		};

		if !self.in_compute_body {
			return;
		}

		let mut has_previous_parameter = has_previous_parameter;
		let separator = if self.minified { "," } else { ", " };

		if has_previous_parameter {
			string.push_str(separator);
		}
		string.push_str("gid");
		has_previous_parameter = true;

		if compute_stage_context.has_push_constant {
			string.push_str(separator);
			string.push_str("push_constant");
			has_previous_parameter = true;
		}

		for &set in &compute_stage_context.binding_sets {
			if has_previous_parameter {
				string.push_str(separator);
			}
			string.push_str("set");
			string.push_str(set.to_string().as_str());
			has_previous_parameter = true;
		}
	}

	fn emit_function_prototype(&mut self, string: &mut String, function_node: &besl::NodeReference) {
		let node = RefCell::borrow(function_node);
		let besl::Nodes::Function {
			name,
			return_type,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str(Self::translate_type(&return_type.borrow().get_name().unwrap()));
		string.push(' ');
		string.push_str(name);
		string.push('(');

		let formatting = ShaderFormatting::new(self.minified);
		emit_comma_separated_nodes(string, formatting, params, |string, param| {
			self.emit_node_string(string, param)
		});

		if self.in_compute_body && self.function_requires_compute_context(function_node) {
			self.emit_compute_hidden_parameters(string, !params.is_empty());
		}

		string.push(')');
		string.push(';');
		if !self.minified {
			string.push('\n');
		}
	}

	fn mesh_output_assignment_parts(
		&mut self,
		statement: &besl::NodeReference,
	) -> Option<(String, besl::NodeReference, besl::NodeReference)> {
		let node = statement.borrow();
		let besl::Nodes::Expression(besl::Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		}) = node.node()
		else {
			return None;
		};

		let left_node = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor {
			left: output,
			right: index,
		}) = left_node.node()
		else {
			return None;
		};

		let output_node = output.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = output_node.node() else {
			return None;
		};

		let source = source.borrow();
		let besl::Nodes::Output { name, count, .. } = source.node() else {
			return None;
		};

		if count.is_none() {
			return None;
		}

		if name != "out_instance_index" && name != "out_primitive_index" {
			return None;
		}

		Some((name.clone(), index.clone(), right.clone()))
	}

	fn emit_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		let mut i = 0;

		while i < statements.len() {
			if self.mesh_stage_context.is_some() && i + 1 < statements.len() {
				let current = self.mesh_output_assignment_parts(&statements[i]);
				let next = self.mesh_output_assignment_parts(&statements[i + 1]);

				if let (Some((current_name, current_index, current_value)), Some((next_name, next_index, next_value))) =
					(current, next)
				{
					let mut current_index_string = String::new();
					self.emit_node_string(&mut current_index_string, &current_index);
					let mut next_index_string = String::new();
					self.emit_node_string(&mut next_index_string, &next_index);

					if current_index_string == next_index_string
						&& current_name != next_name
						&& ((current_name == "out_instance_index" && next_name == "out_primitive_index")
							|| (current_name == "out_primitive_index" && next_name == "out_instance_index"))
					{
						let (instance_value, primitive_value) = if current_name == "out_instance_index" {
							(current_value, next_value)
						} else {
							(next_value, current_value)
						};

						formatting.push_indentation(string, indent);

						string.push_str("out_mesh.set_primitive(");
						self.emit_node_string(string, &current_index);
						string.push_str(", PrimitiveOutput{.instance_index = ");
						self.emit_node_string(string, &instance_value);
						string.push_str(", .primitive_index = ");
						self.emit_node_string(string, &primitive_value);
						string.push_str("})");
						formatting.push_statement_end(string);
						i += 2;
						continue;
					}
				}
			}

			emit_shared_statement_block(string, formatting, &statements[i..i + 1], indent, |string, statement| {
				self.emit_node_string(string, statement)
			});
			i += 1;
		}
	}

	/// Translates BESL intrinsic type names to MSL type names.
	/// Example: `vec2f` -> `float2`
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"atomicu32" => "atomic_uint",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "ushort2",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
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

	fn emit_type_name(string: &mut String, source: &str) {
		if let Some((element_type, count)) = source.split_once('[') {
			string.push_str(Self::translate_type(element_type));
			string.push('[');
			string.push_str(count.trim_end_matches(']'));
			string.push(']');
		} else {
			string.push_str(Self::translate_type(source));
		}
	}

	fn emit_call_arguments(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		emit_comma_separated_nodes(string, formatting, arguments, |string, argument| {
			self.emit_node_string(string, argument)
		});
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
			"max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "round" | "fract"
			| "smoothstep" | "mix" => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"radians" => {
				string.push('(');
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push_str("*(PI/180.0))");
				} else {
					string.push_str(" * (PI / 180.0))");
				}
			}
			"inversesqrt" => {
				string.push_str("rsqrt(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
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
				string.push_str("atomic_fetch_add_explicit(&");
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", memory_order_relaxed)");
			}
			"thread_id" => {
				string.push_str("gid");
			}
			"thread_idx" => {
				string.push_str("thread_index");
			}
			"threadgroup_position" => {
				string.push_str("threadgroup_position");
			}
			"set_mesh_output_counts" => {
				string.push_str("if(thread_index==0){out_mesh.set_primitive_count(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(");}");
			}
			"set_mesh_vertex_position" => {
				string.push_str("out_mesh.set_vertex(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", VertexOutput{.position = ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str("})");
			}
			"set_mesh_triangle" => {
				string.push_str("out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 0, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".x);out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 1, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".y);out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 2, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".z)");
			}
			"image_load" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(").x");
			}
			"fetch" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"texture_size" | "image_size" => {
				string.push_str("uint2(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_width(),");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_height())");
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
				Self::emit_type_name(string, &return_type.borrow().get_name().unwrap());

				string.push(' ');

				string.push_str(name);

				string.push('(');

				emit_comma_separated_nodes(string, formatting, params, |string, param| {
					self.emit_node_string(string, param)
				});

				if self.in_compute_body && self.function_requires_compute_context(this_node) {
					self.emit_compute_hidden_parameters(string, !params.is_empty());
				}

				if self.mesh_stage_context.is_some() && name == "main" {
					self.emit_mesh_hidden_parameters(string, !params.is_empty());
				}

				formatting.push_block_start(string);

				self.emit_statement_block(string, statements, 1);

				if self.minified {
					string.push('}')
				} else {
					string.push_str("}\n");
				}
			}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => {
				if template.is_some() {
					return;
				}

				if is_builtin_struct_type(name, true) {
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
				string.push_str("struct PushConstant");
				if self.minified {
					string.push('{');
				} else {
					string.push_str(" {\n");
				}

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, &member);
					formatting.push_statement_end(string);
				}

				string.push_str("};");
				if !self.minified {
					string.push('\n');
				}

				// TODO: Confirm push constant mapping for Metal argument buffers.
				if self.minified {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];",
						PUSH_CONSTANT_BINDING_INDEX
					));
				} else {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];\n",
						PUSH_CONSTANT_BINDING_INDEX
					));
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
										"constant {} {} [[function_constant({})]];{}",
										Self::translate_type(&r#type.borrow().get_name().unwrap()),
										&member_name,
										i,
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
			besl::Nodes::Raw { glsl, hlsl, msl, .. } => {
				if let Some(code) = msl.as_ref().or(hlsl.as_ref()).or(glsl.as_ref()) {
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
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} => {
				if count.is_some() {
					return;
				}

				let format = format.borrow();
				let type_name = Self::translate_type(&format.get_name().unwrap());
				string.push_str(&format!("{} {} [[color({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { operator, left, right } => {
					let swap_matrix_multiply = matches!(operator, besl::Operators::Multiply)
						&& (Self::expression_is_matrix_type(left) || Self::expression_is_matrix_type(right));
					if swap_matrix_multiply {
						self.emit_wrapped_expression(string, &right);
					} else {
						self.emit_wrapped_expression(string, &left);
					}
					let operator = operator_token(operator);
					if self.minified {
						string.push_str(operator);
					} else {
						string.push(' ');
						string.push_str(operator);
						string.push(' ');
					}
					if swap_matrix_multiply {
						self.emit_wrapped_expression(string, &left);
					} else {
						self.emit_wrapped_expression(string, &right);
					}
				}
				besl::Expressions::FunctionCall {
					parameters, function, ..
				} => {
					let function_ref = function.clone();
					let function = RefCell::borrow(&function_ref);
					let name = function.get_name().unwrap();
					let append_hidden_context = self.in_compute_body
						&& matches!(function.node(), besl::Nodes::Function { name, .. } if name != "main")
						&& self.function_requires_compute_context(&function_ref);

					Self::emit_type_name(string, &name);
					string.push('(');
					emit_comma_separated_nodes(string, formatting, parameters, |string, parameter| {
						self.emit_node_string(string, parameter)
					});
					if append_hidden_context {
						self.emit_compute_hidden_call_arguments(string, !parameters.is_empty());
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
					besl::Nodes::Binding { set, .. } if self.in_compute_body || self.mesh_stage_context.is_some() => {
						self.emit_compute_binding_reference(string, *set, name);
					}
					_ => {
						string.push_str(name);
					}
				},
				besl::Expressions::VariableDeclaration { name, r#type } => {
					Self::emit_type_name(string, &r#type.borrow().get_name().unwrap());
					string.push(' ');
					string.push_str(name);
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
				besl::Expressions::Continue => {
					string.push_str("continue");
				}
				besl::Expressions::Accessor { left, right } => {
					self.emit_node_string(string, &left);
					if left.borrow().node().is_buffer_binding() {
						string.push_str("->");
						self.emit_node_string(string, &right);
					} else if !matches!(
						right.borrow().node(),
						besl::Nodes::Expression(besl::Expressions::Member { .. })
					) && left.borrow().node().is_indexable()
					{
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

				self.emit_statement_block(string, statements, 1);

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

				self.emit_statement_block(string, statements, 1);

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
				if self.in_compute_body || self.mesh_stage_context.is_some() {
					self.emit_compute_binding_reference(string, *set, name);
					return;
				}

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
							"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
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
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("constant ");
				let type_name = r#type.borrow().get_name().unwrap().to_string();
				if let Some((element_type, count)) = type_name.split_once('[') {
					string.push_str(Self::translate_type(element_type));
					string.push(' ');
					string.push_str(name);
					string.push('[');
					string.push_str(count.trim_end_matches(']'));
					string.push(']');
				} else {
					Self::emit_type_name(string, &type_name);
					string.push(' ');
					string.push_str(name);
				}
				string.push_str(" = ");
				if let besl::Nodes::Expression(besl::Expressions::FunctionCall {
					parameters, function, ..
				}) = value.borrow().node()
				{
					if function.borrow().get_name() == Some(type_name.as_str()) {
						string.push('{');
						self.emit_call_arguments(string, parameters);
						string.push('}');
					} else {
						self.emit_node_string(string, &value);
					}
				} else {
					self.emit_node_string(string, &value);
				}
				string.push_str(&format!(";{break_char}"));
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
			Stages::Compute { local_size } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width(),
					local_size.height(),
					local_size.depth()
				));
				msl_block.push_str("// Note: Metal threadgroup sizes are set on the pipeline state.\n");
			}
			Stages::Mesh { local_size, .. } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width(),
					local_size.height(),
					local_size.depth()
				));
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
	fn compute_bindings_use_argument_buffers_by_default() {
		let main = shader_generator::tests::bindings();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct _set0{device _buff* buff [[id(0)]];texture2d<float, access::write> image [[id(1)]];};"
		);
		assert_string_contains!(
			shader,
			"struct _set1{texture2d<float> texture [[id(0)]];sampler texture_sampler [[id(1)]];};"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint2 gid [[thread_position_in_grid]],constant _set0& set0 [[buffer(16)]],constant _set1& set1 [[buffer(17)]])"
		);
		assert_string_contains!(shader, "set0.buff;set0.image;set1.texture;");
	}

	#[test]
	fn compute_bindings_can_use_bare_resources() {
		let main = shader_generator::tests::bindings();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "kernel void besl_main(uint2 gid [[thread_position_in_grid]],");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]]");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]]");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(100)]]");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(100)]]");
		assert_string_contains!(shader, "buff;image;texture;");
	}

	#[test]
	fn same_named_buffer_members_lower_to_msl() {
		let main = shader_generator::tests::same_named_buffer_member_access();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "set0.pixel_mapping->pixel_mapping[0]");
		assert_string_contains!(shader, "set0.meshes->meshes[1]");
	}

	#[test]
	fn intrinsics_lower_to_valid_msl_names() {
		let source = r#"
		main: fn () -> void {
			let angle: f32 = radians(180.0);
			let inverse: f32 = inversesqrt(4.0);
			angle;
			inverse;
		}
		"#;

		let root = besl::compile_to_besl(source, None).expect(
			"Expected intrinsic test shader source to compile. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float angle=(180.0*(PI/180.0));");
		assert_string_contains!(shader, "rsqrt(4.0)");
	}

	#[test]
	fn mesh_stage_uses_mesh_entry_point_and_mesh_push_constants() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let main = besl::parser::Node::function(
			"main",
			Vec::new(),
			"void",
			vec![besl::parser::Node::raw_code(
				Some("".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				&["push_constant", "VertexOutput", "PrimitiveOutput"],
				&[],
			)],
		);
		let shader = besl::parser::Node::scope("Shader", vec![push_constant, mesh_output_types, main]);
		let mut root = besl::parser::Node::root();
		root.add(vec![shader]);

		let root_node = besl::lex(root).unwrap();
		let main_node = root_node.get_main().unwrap();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
		assert_string_contains!(shader, "[[mesh]] void besl_main(");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, "uint threadgroup_position [[threadgroup_position_in_grid]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(
			shader,
			"metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh"
		);
	}

	#[test]
	fn compute_shaders_emit_threadgroup_metadata() {
		let source = "main: fn () -> void { let coord: vec3u = thread_id(); }";
		let root = besl::parse(source).unwrap();
		let root = besl::lex(root).unwrap();
		let main_node = root.get_main().unwrap();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
	}

	#[test]
	fn specializtions() {
		let main = shader_generator::tests::specializations();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float color_x [[function_constant(0)]];");
		assert_string_contains!(shader, "constant float color_y [[function_constant(1)]];");
		assert_string_contains!(shader, "constant float color_z [[function_constant(2)]];");
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
	fn fetch_intrinsic_lowers_to_msl() {
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float4 texel=set0.texture.read(coord);");
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
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]];");
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
	fn matrix_multiplication_operands_are_flipped_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f, position: vec4f) -> vec4f {
			return projection * model * position;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4x4 model,float4 position){return position*(model*projection);}"
		);
	}

	#[test]
	fn matrix_on_both_sides_is_flipped_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f) -> mat4f {
			return projection * model;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-matrix shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4x4 main(float4x4 projection,float4x4 model){return model*projection;}"
		);
	}

	#[test]
	fn matrix_and_vector_multiplication_is_flipped_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f) -> vec4f {
			return projection * position;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-vector shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position){return position*projection;}"
		);
	}

	#[test]
	fn chained_matrix_vector_scalar_multiplication_only_flips_matrix_step() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f, scale: f32) -> vec4f {
			return projection * position * scale;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected chained multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position,float scale){return (position*projection)*scale;}"
		);
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// MSL generator should use the explicit MSL code
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void main(){out.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "MSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = shader_generator::tests::const_variable();

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn mesh_intrinsics_emit_msl_mesh_commands() {
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![mesh_output_types]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(thread_index==0){out_mesh.set_primitive_count(2);}");
		assert_string_contains!(
			shader,
			"out_mesh.set_vertex(0, VertexOutput{.position = float4(1.0,2.0,3.0,1.0)})"
		);
		assert_string_contains!(
			shader,
			"out_mesh.set_index(0 * 3 + 0, uint3(0,1,2).x);out_mesh.set_index(0 * 3 + 1, uint3(0,1,2).y);out_mesh.set_index(0 * 3 + 2, uint3(0,1,2).z)"
		);
	}

	#[test]
	fn mesh_output_assignments_lower_to_msl_primitive_outputs() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let out_instance_index = besl::parser::Node::output_array("out_instance_index", "u32", 0, 126);
		let out_primitive_index = besl::parser::Node::output_array("out_primitive_index", "u32", 1, 126);
		let script = r#"
		main: fn () -> void {
			out_instance_index[0] = 7;
			out_primitive_index[0] = 9;
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![
			push_constant,
			mesh_output_types,
			out_instance_index,
			out_primitive_index,
		]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"out_mesh.set_primitive(0, PrimitiveOutput{.instance_index = 7, .primitive_index = 9});"
		);
	}

	#[test]
	fn mesh_stage_user_functions_do_not_receive_hidden_context_parameters() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let meshlets = besl::parser::Node::binding(
			"meshlets",
			besl::parser::Node::buffer("MeshletBuffer", vec![besl::parser::Node::member("count", "u32")]),
			0,
			0,
			true,
			false,
		);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let mut parsed_shader = besl::parse(
			r#"
			helper: fn () -> void {
				meshlets.count;
				threadgroup_position();
				thread_idx();
				set_mesh_output_counts(3, 1);
			}

			main: fn () -> void {
				helper();
			}
			"#,
		)
		.expect("Expected mesh helper shader to parse");
		let parsed_children = match parsed_shader.node_mut() {
			besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
			_ => panic!(
				"Expected mesh helper shader to parse into a scope. The most likely cause is invalid BESL syntax in the mesh helper shader test."
			),
		};
		let mut shader = besl::parser::Node::root();
		shader.add(vec![meshlets, push_constant, mesh_output_types]);
		shader.add(parsed_children);
		let root = besl::lex(shader).expect("Expected mesh helper shader to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void helper()");
		assert_string_contains!(shader, "helper();");
		assert!(!shader.contains("void helper(constant _set0& set0"));
		assert!(!shader.contains("helper(set0,threadgroup_position,thread_index,out_mesh);"));
	}

	#[test]
	fn conditional_blocks_lower_to_msl() {
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(n<1){n=2;}");
	}

	#[test]
	fn bitwise_operators_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_msl() {
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

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "for(uint i=0;i<=4;i=(i+1)){if(i>=2){continue;};};");
	}

	#[test]
	fn scalar_max_and_clamp_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "max(1.0,2.0)");
		assert_string_contains!(shader, "clamp(1.5,0.0,1.0)");
	}

	#[test]
	fn const_array_variable_lowers_to_msl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float WEIGHTS[3] = {0.5,0.25,0.125};");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_msl() {
		let main = shader_generator::tests::return_value();

		let minified_shader = MSLShaderGenerator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(minified_shader, "float main(){return 1.0;}");

		let pretty_shader = MSLShaderGenerator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(pretty_shader, "float main() {\n\treturn 1.0;\n}\n");
	}
}
