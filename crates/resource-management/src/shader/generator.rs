use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	vec::Vec as AllocVec,
};

use utils::Extent;

use crate::shader::besl::{
	evaluation::BindingKind,
	graph::{build_graph_in, topological_sort_in},
};

/// The `Generator` trait provides graphics-API shader generation from a BESL program definition.
pub trait Generator {}

/// The `CompiledShaderBinding` struct preserves the flat resource interface required to create a backend shader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledShaderBinding {
	pub slot: u32,
	pub kind: BindingKind,
	pub count: u32,
	pub buffer_stride: Option<u32>,
	pub read: bool,
	pub write: bool,
}

impl CompiledShaderBinding {
	/// Builds one validated compiled resource requirement.
	pub fn new(slot: u32, kind: BindingKind, count: u32, buffer_stride: Option<u32>, read: bool, write: bool) -> Self {

		assert!(
			count > 0,
			"Invalid resource count. The most likely cause is that a compiled shader resource was declared with an empty array."
		);
		assert!(
			slot.checked_add(count).is_some(),
			"Invalid resource slot range. The most likely cause is that a compiled shader resource array extends beyond the flat slot space."
		);
		match (kind, buffer_stride) {
			(BindingKind::StorageBuffer, Some(stride)) => assert!(
				stride > 0,
				"Invalid storage-buffer stride. The most likely cause is that compiled reflection produced a zero-byte element."
			),
			(BindingKind::StorageBuffer, None) => panic!(
				"Missing storage-buffer stride. The most likely cause is that compiled reflection dropped the element layout."
			),
			(_, Some(_)) => panic!(
				"Unexpected buffer stride. The most likely cause is that compiled reflection attached buffer metadata to a non-buffer resource."
			),
			(_, None) => {}
		}
		Self {
			slot,
			kind,
			count,
			buffer_stride,
			read,
			write,
		}
	}
}

/// The `CompiledShader` struct provides compiled bytes and reflection metadata across compiler backends.
pub struct CompiledShader {
	binary: Box<[u8]>,
	bindings: Vec<CompiledShaderBinding>,
	extent: Option<Extent>,
}

impl CompiledShader {
	pub fn new(binary: Box<[u8]>, bindings: Vec<CompiledShaderBinding>, extent: Option<Extent>) -> Self {
		Self {
			binary,
			bindings,
			extent,
		}
	}

	pub fn extent(&self) -> Option<Extent> {
		self.extent
	}

	pub fn binary(&self) -> &[u8] {
		&self.binary
	}

	pub fn into_binary(self) -> Box<[u8]> {
		self.binary
	}

	pub fn into_parts(self) -> (Box<[u8]>, Vec<CompiledShaderBinding>, Option<Extent>) {
		(self.binary, self.bindings, self.extent)
	}

	pub fn bindings(&self) -> &[CompiledShaderBinding] {
		&self.bindings
	}
}

pub enum Stages {
	Vertex,
	Compute {
		local_size: Extent,
	},
	Task {
		local_size: Extent,
		maximum_mesh_threadgroups: u32,
	},
	Mesh {
		maximum_vertices: u32,
		maximum_primitives: u32,
		local_size: Extent,
	},
	Fragment,
}

pub enum MatrixLayouts {
	RowMajor,
	ColumnMajor,
}

pub struct GLSLSettings {
	pub(crate) version: String,
}

impl Default for GLSLSettings {
	fn default() -> Self {
		Self {
			version: "450".to_string(),
		}
	}
}

pub struct Settings {
	pub(crate) glsl: GLSLSettings,
	pub(crate) stage: Stages,
	pub(crate) matrix_layout: MatrixLayouts,
	pub(crate) name: String,
}

/// The `ShaderFormatting` struct provides shared text formatting rules for shader generators.
#[derive(Clone, Copy)]
pub(crate) struct ShaderFormatting {
	minified: bool,
}

impl ShaderFormatting {
	pub(crate) fn new(minified: bool) -> Self {
		Self { minified }
	}

	pub(crate) fn break_str(&self) -> &'static str {
		if self.minified {
			""
		} else {
			"\n"
		}
	}

	pub(crate) fn space_str(&self) -> &'static str {
		if self.minified {
			""
		} else {
			" "
		}
	}

	pub(crate) fn comma_str(&self) -> &'static str {
		if self.minified {
			","
		} else {
			", "
		}
	}

	pub(crate) fn push_indentation(&self, string: &mut String, indent: usize) {
		if !self.minified {
			for _ in 0..indent {
				string.push('\t');
			}
		}
	}

	pub(crate) fn push_block_start(&self, string: &mut String) {
		if self.minified {
			string.push_str("){");
		} else {
			string.push_str(") {\n");
		}
	}

	pub(crate) fn push_statement_end(&self, string: &mut String) {
		if self.minified {
			string.push(';');
		} else {
			string.push_str(";\n");
		}
	}
}

/// Returns the reachable non-leaf shader nodes in emission order.
pub(crate) fn ordered_shader_nodes(main_function_node: &besl::NodeReference, backend_name: &str) -> Vec<besl::NodeReference> {
	ordered_shader_nodes_in(main_function_node, backend_name, Global)
}

/// Rejects shared storage in stages that do not have workgroup execution semantics.
pub(crate) fn validate_workgroup_storage_stage(stage: &Stages, order: &[besl::NodeReference]) -> Result<(), ()> {
	if matches!(stage, Stages::Compute { .. } | Stages::Task { .. })
		|| !order
			.iter()
			.any(|node| matches!(node.borrow().node(), besl::Nodes::Workgroup { .. }))
	{
		Ok(())
	} else {
		Err(())
	}
}

/// Returns the reachable non-leaf shader nodes in emission order using the provided allocator for transient graph storage.
pub(crate) fn ordered_shader_nodes_in<A: Allocator + Clone>(
	main_function_node: &besl::NodeReference,
	backend_name: &str,
	allocator: A,
) -> AllocVec<besl::NodeReference, A> {
	if !matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }) {
		panic!(
			"{backend_name} shader generation requires a function node as the main function. The provided node was not a function."
		);
	}

	besl::optimization::optimize(main_function_node);

	let graph = build_graph_in(main_function_node.clone(), allocator.clone());

	let mut ordered = AllocVec::new_in(allocator.clone());
	for node in topological_sort_in(&graph, allocator) {
		let include = {
			let borrowed = node.borrow();
			!borrowed.node().is_leaf()
				&& !matches!(borrowed.node(), besl::Nodes::Conditional { .. } | besl::Nodes::ForLoop { .. })
		};
		if include {
			ordered.push(node);
		}
	}
	ordered
}

pub(crate) fn emit_comma_separated_nodes<F>(
	string: &mut String,
	formatting: ShaderFormatting,
	nodes: &[besl::NodeReference],
	mut emit_node: F,
) where
	F: FnMut(&mut String, &besl::NodeReference),
{
	for (i, node) in nodes.iter().enumerate() {
		if i > 0 {
			string.push_str(formatting.comma_str());
		}

		emit_node(string, node);
	}
}

pub(crate) fn emit_statement_block<F>(
	string: &mut String,
	formatting: ShaderFormatting,
	statements: &[besl::NodeReference],
	indent: usize,
	mut emit_statement: F,
) where
	F: FnMut(&mut String, &besl::NodeReference),
{
	for statement in statements {
		formatting.push_indentation(string, indent);
		emit_statement(string, statement);
		formatting.push_statement_end(string);
	}
}

pub(crate) fn operator_token(operator: &besl::Operators) -> &'static str {
	match operator {
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
		besl::Operators::Inequality => "!=",
		besl::Operators::GreaterThan => ">",
		besl::Operators::LessThanOrEqual => "<=",
		besl::Operators::GreaterThanOrEqual => ">=",
		besl::Operators::LogicalAnd => "&&",
		besl::Operators::LogicalOr => "||",
	}
}

pub(crate) fn is_builtin_struct_type(name: &str, supports_atomic_u32: bool) -> bool {
	matches!(
		name,
		"void"
			| "bool" | "vec2u16"
			| "vec4u16"
			| "vec2u" | "vec3u"
			| "vec4u" | "vec2i"
			| "vec2f16"
			| "vec3f16"
			| "vec4f16"
			| "vec2f" | "vec3f"
			| "vec4f" | "packed_vec4f"
			| "mat2f" | "mat3f"
			| "mat4f" | "mat4x3f"
			| "f16" | "f32"
			| "u8" | "u16"
			| "u32" | "i32"
			| "Texture2D"
			| "Texture3D"
			| "TextureCube"
			| "TextureCubeArray"
			| "ArrayTexture2D"
			| "VertexOutput"
			| "PrimitiveOutput"
	) || supports_atomic_u32 && name == "atomicu32"
}

/// Returns the vector that carries a short scalar array through backends that cannot return native arrays.
pub(crate) fn scalar_array_vector_type(source: &str) -> Option<&'static str> {
	match source {
		"f32[2]" => Some("vec2f"),
		"f32[3]" => Some("vec3f"),
		"f32[4]" => Some("vec4f"),
		"u16[2]" => Some("vec2u16"),
		"u16[3]" => Some("vec3u16"),
		"u16[4]" => Some("vec4u16"),
		"u32[2]" => Some("vec2u"),
		"u32[3]" => Some("vec3u"),
		"u32[4]" => Some("vec4u"),
		_ => None,
	}
}

impl Settings {
	fn normalize_local_size(extent: Extent) -> Extent {
		Extent::new(extent.width().max(1), extent.height().max(1), extent.depth().max(1))
	}

	pub fn compute(extent: Extent) -> Settings {
		Self::from_stage(Stages::Compute {
			local_size: Self::normalize_local_size(extent),
		})
	}

	pub fn task(local_size: Extent, maximum_mesh_threadgroups: u32) -> Settings {

		assert!(
			maximum_mesh_threadgroups > 0,
			"Invalid task mesh-threadgroup limit. The most likely cause is that a task shader was configured to emit zero mesh threadgroups."
		);
		Self::from_stage(Stages::Task {
			local_size: Self::normalize_local_size(local_size),
			maximum_mesh_threadgroups,
		})
	}

	pub fn mesh(maximum_vertices: u32, maximum_primitives: u32, local_size: Extent) -> Settings {
		Self::from_stage(Stages::Mesh {
			maximum_vertices,
			maximum_primitives,
			local_size: Self::normalize_local_size(local_size),
		})
	}

	pub fn fragment() -> Settings {
		Self::from_stage(Stages::Fragment)
	}

	pub fn vertex() -> Settings {
		Self::from_stage(Stages::Vertex)
	}

	fn from_stage(stage: Stages) -> Self {
		Settings {
			glsl: GLSLSettings::default(),
			stage,
			matrix_layout: MatrixLayouts::RowMajor,
			name: "shader".to_string(),
		}
	}

	pub fn name(mut self, name: String) -> Self {
		self.name = name;
		self
	}
}

fn type_uses_f16(r#type: &besl::NodeReference) -> bool {
	matches!(r#type.borrow().get_name(), Some("f16" | "vec2f16" | "vec3f16" | "vec4f16"))
}

/// Reports whether a node resolves to a value that uses f16 components.
fn expression_uses_f16(node: &besl::NodeReference) -> bool {
	match node.borrow().node() {
		besl::Nodes::Member { r#type, .. }
		| besl::Nodes::Parameter { r#type, .. }
		| besl::Nodes::Input { format: r#type, .. }
		| besl::Nodes::Output { format: r#type, .. }
		| besl::Nodes::TaskPayload { format: r#type, .. }
		| besl::Nodes::Workgroup { format: r#type, .. }
		| besl::Nodes::Specialization { r#type, .. }
		| besl::Nodes::Const { r#type, .. }
		| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => type_uses_f16(r#type),
		besl::Nodes::Struct { name, .. } => matches!(name.as_str(), "f16" | "vec2f16" | "vec3f16" | "vec4f16"),
		besl::Nodes::Function { return_type, .. }
		| besl::Nodes::Intrinsic {
			r#return: return_type, ..
		} => type_uses_f16(return_type),
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Member { source, .. } => expression_uses_f16(source),
			besl::Expressions::FunctionCall { function, .. } => expression_uses_f16(function),
			besl::Expressions::IntrinsicCall { intrinsic, .. } => expression_uses_f16(intrinsic),
			besl::Expressions::Operator { operator, left, right } => {
				if *operator == besl::Operators::Assignment {
					expression_uses_f16(left)
				} else {
					expression_uses_f16(left) || expression_uses_f16(right)
				}
			}
			besl::Expressions::Expression { elements } if elements.len() == 1 => expression_uses_f16(&elements[0]),
			besl::Expressions::Accessor { left, right } => expression_uses_f16(left) || expression_uses_f16(right),
			_ => false,
		},
		_ => false,
	}
}

/// Reports whether a node contains one numeric literal that can require explicit narrowing.
fn is_numeric_literal(node: &besl::NodeReference) -> bool {
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse::<f32>().is_ok(),
		besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
			is_numeric_literal(&elements[0])
		}
		_ => false,
	}
}

/// The `NodeEmitter` trait provides shared code generation helpers for shader language backends.
///
/// Backends implement the required methods and inherit default implementations for
/// common emit operations like `emit_wrapped_expression`, `emit_type_name`, and
/// `emit_call_arguments`.
pub(crate) trait NodeEmitter {
	/// Maps a BESL type name to the backend's native type name.
	fn type_from_besl(source: &str) -> &str;

	/// Whether the backend uses minified output.
	fn minified(&self) -> bool;

	/// Appends the string representation of a BESL node to the output buffer.
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeReference);

	/// Emits a backend intrinsic call.
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	);

	fn supports_atomic_u32(&self) -> bool {
		true
	}

	fn emit_separator(&self, string: &mut String) {
		string.push_str(ShaderFormatting::new(self.minified()).comma_str());
	}

	fn emit_named_struct_start(&self, string: &mut String, name: &str) {
		string.push_str("struct ");
		string.push_str(name);
		if self.minified() {
			string.push('{');
		} else {
			string.push_str(" {\n");
		}
	}

	fn emit_struct_declaration_end(&self, string: &mut String) {
		string.push_str("};");
		if !self.minified() {
			string.push('\n');
		}
	}

	fn emit_block_end(&self, string: &mut String) {
		string.push('}');
		if !self.minified() {
			string.push('\n');
		}
	}

	fn emit_indentation(&self, string: &mut String, indent: usize) {
		ShaderFormatting::new(self.minified()).push_indentation(string, indent);
	}

	fn emit_statement_end(&self, string: &mut String) {
		ShaderFormatting::new(self.minified()).push_statement_end(string);
	}

	fn emit_discard(&mut self, string: &mut String) {
		string.push_str("discard");
	}

	fn emit_function_extra_parameters(
		&mut self,
		_string: &mut String,
		_node: &besl::NodeReference,
		_name: &str,
		_has_previous_parameter: bool,
	) {
	}

	fn emit_function_attributes(&mut self, _string: &mut String, _node: &besl::NodeReference, _name: &str) {}

	fn emit_function_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified());
		emit_statement_block(string, formatting, statements, indent, |string, statement| {
			self.emit_node(string, statement)
		});
	}

	fn emit_function_call_extra_arguments(
		&mut self,
		_string: &mut String,
		_function: &besl::NodeReference,
		_has_previous_argument: bool,
	) {
	}

	/// Gives a backend the opportunity to replace call syntax for callable types such as aggregate structs.
	fn emit_function_call(
		&mut self,
		_string: &mut String,
		_function: &besl::NodeReference,
		_parameters: &[besl::NodeReference],
	) -> bool {
		false
	}

	fn emit_expression_member(&mut self, _string: &mut String, _name: &str, _source: &besl::NodeReference) -> bool {
		false
	}

	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		self.emit_node(string, left);
		if left.borrow().node().is_indexable() {
			string.push('[');
			self.emit_node(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node(string, right);
		}
	}

	fn emit_function_node(
		&mut self,
		string: &mut String,
		this_node: &besl::NodeReference,
		name: &str,
		statements: &[besl::NodeReference],
		return_type: &besl::NodeReference,
		params: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified());
		self.emit_function_attributes(string, this_node, name);
		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
		string.push('(');
		emit_comma_separated_nodes(string, formatting, params, |string, param| self.emit_node(string, param));
		self.emit_function_extra_parameters(string, this_node, name, !params.is_empty());
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	fn emit_struct_node(
		&mut self,
		string: &mut String,
		name: &str,
		fields: &[besl::NodeReference],
		template: &Option<besl::NodeReference>,
	) {
		if template.is_some() || is_builtin_struct_type(name, self.supports_atomic_u32()) {
			return;
		}

		let formatting = ShaderFormatting::new(self.minified());
		self.emit_named_struct_start(string, name);
		for field in fields {
			formatting.push_indentation(string, 1);
			self.emit_node(string, field);
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	fn emit_parameter_node(&mut self, string: &mut String, name: &str, r#type: &besl::NodeReference) {
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
	}

	/// Gives a backend the opportunity to replace expression syntax before portable lowering.
	fn emit_expression_override(&mut self, _string: &mut String, _expression: &besl::Expressions) -> bool {
		false
	}

	fn emit_expression_node(&mut self, string: &mut String, expression: &besl::Expressions) {
		if self.emit_expression_override(string, expression) {
			return;
		}

		let formatting = ShaderFormatting::new(self.minified());
		match expression {
			besl::Expressions::Operator { operator, left, right } => {
				let left_uses_f16 = expression_uses_f16(left);
				let right_uses_f16 = expression_uses_f16(right);
				let emit_value = |emitter: &mut Self, string: &mut String, value: &besl::NodeReference, as_f16: bool| {
					if as_f16 && is_numeric_literal(value) {
						// GLSL does not implicitly narrow float literals to float16_t.
						Self::emit_type_name(string, "f16");
						string.push('(');
						emitter.emit_node(string, value);
						string.push(')');
					} else {
						emitter.emit_wrapped_expression(string, value);
					}
				};

				let left_needs_f16 = *operator != besl::Operators::Assignment && right_uses_f16;
				emit_value(self, string, left, left_needs_f16);
				let operator = operator_token(operator);
				if self.minified() {
					string.push_str(operator)
				} else {
					string.push(' ');
					string.push_str(operator);
					string.push(' ');
				}
				emit_value(self, string, right, left_uses_f16);
			}
			besl::Expressions::FunctionCall {
				parameters, function, ..
			} => {
				let function_ref = function.clone();
				if self.emit_function_call(string, &function_ref, parameters) {
					return;
				}
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
			besl::Expressions::Discard => self.emit_discard(string),
			besl::Expressions::Accessor { left, right } => self.emit_accessor_expression(string, left, right),
		}
	}

	fn emit_conditional_node(
		&mut self,
		string: &mut String,
		condition: &besl::NodeReference,
		statements: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified());
		string.push_str("if(");
		self.emit_node(string, condition);
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	fn emit_for_loop_node(
		&mut self,
		string: &mut String,
		initializer: &besl::NodeReference,
		condition: &besl::NodeReference,
		update: &besl::NodeReference,
		statements: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified());
		string.push_str("for(");
		self.emit_node(string, initializer);
		string.push(';');
		self.emit_node(string, condition);
		string.push(';');
		self.emit_node(string, update);
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	/// Wraps a node's string representation in parentheses when the node is an operator or
	/// expression, otherwise emits it directly.
	fn emit_wrapped_expression(&mut self, string: &mut String, node: &besl::NodeReference) {
		match node.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::Operator { .. } | besl::Expressions::Expression { .. }) => {
				string.push('(');
				self.emit_node(string, node);
				string.push(')');
			}
			_ => self.emit_node(string, node),
		}
	}

	/// Emits a type name with optional array dimension suffix, delegating type mapping to
	/// [`Self::type_from_besl`].
	fn emit_type_name(string: &mut String, source: &str) {
		if let Some(vector_type) = scalar_array_vector_type(source) {
			string.push_str(Self::type_from_besl(vector_type));
		} else if let Some((element_type, count)) = source.split_once('[') {
			string.push_str(Self::type_from_besl(element_type));
			string.push('[');
			string.push_str(count.trim_end_matches(']'));
			string.push(']');
		} else {
			string.push_str(Self::type_from_besl(source));
		}
	}

	/// Emits comma-separated call arguments with the backend's formatting rules.
	fn emit_call_arguments(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified());
		emit_comma_separated_nodes(string, formatting, arguments, |string, argument| {
			self.emit_node(string, argument);
		});
	}
}

#[cfg(test)]
pub mod tests {
	use std::cell::RefCell;

	use utils::Extent;

	use crate::shader::besl::evaluation::BindingKind;

	#[test]
	#[should_panic(expected = "Invalid resource slot range")]
	fn compiled_shader_binding_rejects_flat_slot_overflow() {
		super::CompiledShaderBinding::new(u32::MAX, BindingKind::StorageBuffer, 1, Some(4), true, false);
	}

	#[test]
	fn task_settings_preserve_workgroup_and_mesh_threadgroup_limit() {
		let settings = super::ShaderGenerationSettings::task(Extent::new(32, 0, 0), 32);

		assert!(matches!(
			settings.stage,
			super::Stages::Task {
				local_size,
				maximum_mesh_threadgroups: 32,
			} if local_size == Extent::new(32, 1, 1)
		));
	}

	#[test]
	fn workgroup_storage_is_limited_to_compute_and_task_stages() {
		let program = besl::compile_to_besl(
			r#"
			scratch: workgroup<f32, 64>;
			main: fn () -> void {
				scratch[thread_idx()] = 1.0;
			}
			"#,
			None,
		)
		.expect("workgroup fixture should link");
		let main = program.get_main().expect("workgroup fixture should contain main");
		let order = super::ordered_shader_nodes(&main, "stage validation");

		assert!(super::validate_workgroup_storage_stage(
			&super::Stages::Compute {
				local_size: Extent::square(8)
			},
			&order
		)
		.is_ok());
		assert!(super::validate_workgroup_storage_stage(
			&super::Stages::Task {
				local_size: Extent::line(32),
				maximum_mesh_threadgroups: 32,
			},
			&order,
		)
		.is_ok());
		assert!(super::validate_workgroup_storage_stage(&super::Stages::Fragment, &order).is_err());
	}

	pub fn bindings() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let mut root_node = besl::Node::root();

		let float_type = root_node.get_child("f32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("member", float_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r8".to_string(),
				},
				1,
				false,
				true,
			)
			.into(),
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: "".to_string() },
				2,
				true,
				false,
			)
			.into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	/// Builds a buffer access that verifies packed four-component u16 vectors remain intrinsic backend types.
	pub fn vec4u16_binding() -> besl::NodeReference {
		let script = "main: fn () -> void { buff.value; }";
		let mut root_node = besl::Node::root();
		let vec4u16_type = root_node.get_child("vec4u16").expect("Expected vec4u16 type");
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("value", vec4u16_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected vec4u16 shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds the 52-byte meshlet record used to verify explicit packed-float storage across backends.
	pub fn packed_vec4f_meshlet_binding() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let center: vec4f = vec4f(buff.meshlets[1].center_radius);
			let packed: packed_vec4f = packed_vec4f(center);
			packed.x;
			buff.meshlets[0].cone_apex_cutoff.w;
		}
		"#;
		let mut root_node = besl::Node::root();
		let u32_type = root_node.get_child("u32").expect("Expected u32 type");
		let packed_vec4f_type = root_node.get_child("packed_vec4f").expect("Expected packed_vec4f type");
		let vec2u16_type = root_node.get_child("vec2u16").expect("Expected vec2u16 type");
		let meshlet = root_node.add_child(
			besl::Node::r#struct(
				"Meshlet",
				vec![
					besl::Node::member("primitive_offset", u32_type.clone()).into(),
					besl::Node::member("triangle_offset", u32_type.clone()).into(),
					besl::Node::member("primitive_count", u32_type.clone()).into(),
					besl::Node::member("triangle_count", u32_type).into(),
					besl::Node::member("center_radius", packed_vec4f_type.clone()).into(),
					besl::Node::member("cone_apex_cutoff", packed_vec4f_type).into(),
					besl::Node::member("cone_axis", vec2u16_type).into(),
				],
			)
			.into(),
		);
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("meshlets", meshlet, 2)],
				},
				0,
				true,
				false,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected packed meshlet shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds a flattened vec2u16 array binding used to verify native-width backend storage strides.
	pub fn vec2u16_array_binding() -> besl::NodeReference {
		let script = "main: fn () -> void { buff.values[1]; }";
		let mut root_node = besl::Node::root();
		let vec2u16_type = root_node.get_child("vec2u16").expect("Expected vec2u16 type");
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec2u16_type, 2)],
				},
				0,
				true,
				true,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected vec2u16 array shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds a flattened vec2f16 array binding used to verify native-width backend storage strides.
	pub fn vec2f16_array_binding() -> besl::NodeReference {
		let script = "main: fn () -> void { buff.values[1]; }";
		let mut root_node = besl::Node::root();
		let vec2f16_type = root_node.get_child("vec2f16").expect("Expected vec2f16 type");
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec2f16_type, 2)],
				},
				0,
				true,
				true,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected vec2f16 array shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds mixed packed-u16 storage members used to verify backend alignment against the VM layout.
	pub fn mixed_vec4u16_binding() -> besl::NodeReference {
		let script = "main: fn () -> void { buff.value; buff.tail; }";
		let mut root_node = besl::Node::root();
		let vec4u16_type = root_node.get_child("vec4u16").expect("Expected vec4u16 type");
		let u16_type = root_node.get_child("u16").expect("Expected u16 type");
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("value", vec4u16_type).into(),
						besl::Node::member("tail", u16_type).into(),
					],
				},
				0,
				true,
				true,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected mixed vec4u16 shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds mixed f16 storage members used to verify native backend type and packing mappings.
	pub fn mixed_f16_storage_binding() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let uv32: vec2f = vec2f(0.25, 0.75);
			let uv16: vec2f16 = vec2f16(uv32);
			let sampled_uv: vec2f = vec2f(uv16);
			let weight16: f16 = f16(0.5);
			let weight32: f32 = f32(weight16);
			let literal: f16 = 0.25;
			let doubled: f16 = weight16 * 2.0;
			let scaled_uv: vec2f16 = uv16 * 2.0;
			let buffer_uv: vec2f16 = buff.uv;
			let buffer_scaled_uv: vec2f16 = buffer_uv * 2.0;
			buff.scalar;
			buff.uv;
			buff.normal;
			buff.color;
			sampled_uv;
			weight32;
			literal;
			doubled;
			scaled_uv;
			buffer_scaled_uv;
		}
		"#;
		let mut root_node = besl::Node::root();
		let f16_type = root_node.get_child("f16").expect("Expected f16 type");
		let vec2f16_type = root_node.get_child("vec2f16").expect("Expected vec2f16 type");
		let vec3f16_type = root_node.get_child("vec3f16").expect("Expected vec3f16 type");
		let vec4f16_type = root_node.get_child("vec4f16").expect("Expected vec4f16 type");
		root_node.add_child(
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("scalar", f16_type).into(),
						besl::Node::member("uv", vec2f16_type).into(),
						besl::Node::member("normal", vec3f16_type).into(),
						besl::Node::member("color", vec4f16_type).into(),
					],
				},
				0,
				true,
				true,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected f16 storage shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	/// Builds packed integer vector inputs and outputs used to verify interpolation qualifiers.
	pub fn packed_u16_stage_io() -> besl::NodeReference {
		let script = "main: fn () -> void { packed_input; packed_output; }";
		let mut root_node = besl::Node::root();
		let vec2u16_type = root_node.get_child("vec2u16").expect("Expected vec2u16 type");
		let vec4u16_type = root_node.get_child("vec4u16").expect("Expected vec4u16 type");
		root_node.add_children(vec![
			besl::Node::input("packed_input", vec2u16_type, 0).into(),
			besl::Node::output("packed_output", vec4u16_type, 1).into(),
		]);

		let root = besl::compile_to_besl(script, Some(root_node)).expect("Expected packed stage I/O shader to compile");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		main
	}

	pub fn same_named_buffer_member_access() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			pixel_mapping.pixel_mapping[0] = meshes.meshes[1];
		}
		"#;

		let mut root_node = besl::Node::root();
		let u32_type = root_node.get_child("u32").unwrap();

		root_node.add_children(vec![
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
				"pixel_mapping",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("pixel_mapping", u32_type, 2)],
				},
				1,
				false,
				true,
			)
			.into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn specializations() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![besl::Node::specialization("color", vec3f_type).into()]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn input() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![besl::Node::input("color", vec3f_type, 0).into()]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn output() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![besl::Node::output("color", vec3f_type, 0).into()]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn fragment_program() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
			albedo;
		}
		"#;

		besl::compile_to_besl(&script, None).unwrap()
	}

	pub fn fragment_shader() -> besl::NodeReference {
		RefCell::borrow(&fragment_program()).get_child("main").unwrap()
	}

	pub fn cull_unused_functions() -> besl::NodeReference {
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

		let main_function_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		main
	}

	pub fn structure() -> besl::NodeReference {
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

		let main_function_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		main
	}

	pub fn push_constant() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			push_constant;
		}
		"#;

		let mut root_node = besl::Node::root();

		let u32_t = root_node.get_child("u32").unwrap();
		root_node.add_child(besl::Node::push_constant(vec![besl::Node::member("material_id", u32_t.clone()).into()]).into());

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&program_node).get_child("main").unwrap();

		main
	}

	pub fn intrinsic() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			sample_user(1.0);
		}
		"#;

		use besl::parser::Node;

		let sample_function = Node::intrinsic(
			"sample_user",
			Node::parameter("num", "f32"),
			Node::sentence(vec![
				Node::glsl("0 + ", &[], &[]),
				Node::member_expression("num"),
				Node::glsl(" * 2", &[], &[]),
			]),
			"f32",
		);

		let mut root = besl::parse(&script).unwrap();

		root.add(vec![sample_function]);

		let root = besl::lex(root).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		main
	}

	pub fn const_variable() -> besl::NodeReference {
		let script = r#"
		PI: const f32 = 3.14;

		main: fn () -> void {
			PI;
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn return_value() -> besl::NodeReference {
		let script = r#"
		main: fn () -> f32 {
			return 1.0;
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();
		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}
}

pub use Generator as ShaderGenerator;
pub use Settings as ShaderGenerationSettings;
