use std::{cell::RefCell, fmt::Write as _};

use utils::Extent;

use crate::shader::besl::{
	evaluation::{BindingKind, BindingUsage},
	graph::dependency_order,
};

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

impl From<BindingUsage> for CompiledShaderBinding {
	/// Drops the reflection-only binding name and keeps the flat interface a backend shader needs.
	fn from(binding: BindingUsage) -> Self {
		Self {
			slot: binding.slot,
			kind: binding.kind,
			count: binding.count,
			buffer_stride: binding.buffer_stride,
			read: binding.read,
			write: binding.write,
		}
	}
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
	pub binary: Box<[u8]>,
	pub bindings: Vec<CompiledShaderBinding>,
	pub extent: Option<Extent>,
}

#[derive(Clone, Copy)]
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

impl Stages {
	/// Returns the workgroup size of a compute, task, or mesh stage, or `None` for raster stages.
	pub(crate) fn local_size(self) -> Option<Extent> {
		match self {
			Stages::Compute { local_size } | Stages::Task { local_size, .. } | Stages::Mesh { local_size, .. } => {
				Some(local_size)
			}
			Stages::Vertex | Stages::Fragment => None,
		}
	}

	/// Reports whether the stage reads interpolated inputs, so backends mark integer inputs as flat.
	///
	/// Only fragment inputs and raster-producing outputs participate in interpolation.
	pub(crate) fn interpolates_inputs(self) -> bool {
		matches!(self, Stages::Fragment)
	}

	/// Reports whether the stage writes interpolated outputs, so backends mark integer outputs as flat.
	pub(crate) fn interpolates_outputs(self) -> bool {
		matches!(self, Stages::Vertex | Stages::Mesh { .. })
	}
}

pub struct Settings {
	pub(crate) stage: Stages,
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
		if self.minified { "" } else { "\n" }
	}

	pub(crate) fn space_str(&self) -> &'static str {
		if self.minified { "" } else { " " }
	}

	pub(crate) fn comma_str(&self) -> &'static str {
		if self.minified { "," } else { ", " }
	}

	pub(crate) fn push_indentation(&self, string: &mut String, indent: usize) {
		if !self.minified {
			for _ in 0..indent {
				string.push('\t');
			}
		}
	}

	pub(crate) fn push_block_start(&self, string: &mut String) {
		string.push(')');
		string.push_str(self.space_str());
		string.push('{');
		string.push_str(self.break_str());
	}

	pub(crate) fn push_statement_end(&self, string: &mut String) {
		string.push(';');
		string.push_str(self.break_str());
	}
}

/// The prefix a backend adds to a BESL name that would collide with its target shader language.
///
/// BESL already rejects its own keywords, but names such as `float`, `sampler`, or `half` are valid BESL
/// identifiers and reserved words in GLSL, HLSL, or MSL. See [`Identifier`] for how backends apply it.
pub(crate) const RESERVED_IDENTIFIER_PREFIX: &str = "besl_";

/// The `Identifier` struct exists so every backend writes a BESL name the same way at its declaration and at
/// each use, keeping names that collide with the target language's reserved words valid after lowering.
///
/// Backends build it through [`NodeEmitter::identifier`] and write it with [`Identifier::push_to`] or
/// `format!`. Names that already start with [`RESERVED_IDENTIFIER_PREFIX`] are prefixed too, so a BESL name
/// such as `besl_float` cannot collide with the escaped form of `float`.
#[derive(Clone, Copy)]
pub(crate) struct Identifier<'a> {
	name: &'a str,
	prefixed: bool,
}

impl Identifier<'_> {
	/// Appends the backend-safe name to `string` without allocating.
	pub(crate) fn push_to(self, string: &mut String) {
		if self.prefixed {
			string.push_str(RESERVED_IDENTIFIER_PREFIX);
		}
		string.push_str(self.name);
	}
}

impl std::fmt::Display for Identifier<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if self.prefixed {
			f.write_str(RESERVED_IDENTIFIER_PREFIX)?;
		}
		f.write_str(self.name)
	}
}

/// Returns the reachable non-leaf shader nodes in emission order.
pub(crate) fn ordered_shader_nodes(main_function_node: &besl::NodeReference, backend_name: &str) -> Vec<besl::NodeReference> {
	assert!(
		matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }),
		"{backend_name} shader generation requires a function node as the main function. The provided node was not a function."
	);

	besl::optimization::optimize(main_function_node);

	let mut ordered = dependency_order(main_function_node);
	ordered.retain(|node| {
		let node = node.borrow();
		!node.node().is_leaf()
			&& !matches!(
				node.node(),
				besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. } | besl::Nodes::ForLoop { .. }
			)
	});
	ordered
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

/// Recovers the indexed mesh output that a member expression names, so backends can address its vertex or primitive
/// structure field.
///
/// Passes the output name and whether the output is per-vertex to `target`, so each backend copies only the name it
/// emits.
pub(crate) fn mesh_output_target<R>(member: &besl::NodeReference, target: impl FnOnce(&str, bool) -> R) -> Option<R> {
	let member = member.borrow();
	let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = member.node() else {
		return None;
	};
	let source = source.borrow();
	let besl::Nodes::Output {
		name,
		count: Some(_),
		per_vertex,
		..
	} = source.node()
	else {
		return None;
	};
	Some(target(name, *per_vertex))
}

/// Reports whether a BESL input is one of the implicit vertex invocation indices.
pub(crate) fn is_vertex_builtin_input(name: &str) -> bool {
	matches!(name, besl::VERTEX_INDEX_BUILTIN | besl::INSTANCE_INDEX_BUILTIN)
}

/// Rejects vertex invocation indices outside the vertex stage or with a non-`u32` representation.
pub(crate) fn validate_vertex_builtin_inputs(stage: &Stages, order: &[besl::NodeReference]) -> Result<(), ()> {
	for node in order {
		let node = node.borrow();
		let besl::Nodes::Input { name, format, .. } = node.node() else {
			continue;
		};
		if is_vertex_builtin_input(name) && (!matches!(stage, Stages::Vertex) || format.borrow().get_name() != Some("u32")) {
			return Err(());
		}
	}
	Ok(())
}

/// Writes the name of the flag that records a loop `break` inside the arms of the flagged match at `depth`.
fn push_match_break_flag(string: &mut String, depth: usize) {
	let _ = write!(string, "{RESERVED_IDENTIFIER_PREFIX}match_break_{depth}");
}

/// Reports whether `statement` holds a `break` that leaves a loop around it, outside any nested loop.
fn breaks_enclosing_loop(statement: &besl::NodeReference) -> bool {
	match statement.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Break) => true,
		// A condition or scrutinee can't hold a `break`, so walking every branch child only finds branch statements.
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
			branch.branch_children().any(breaks_enclosing_loop)
		}
		_ => false,
	}
}

/// Writes one `switch` case label for a match value. `i32` scrutinees use signed labels and every other type
/// uses unsigned labels, matching the 32-bit value the `switch` tests.
fn push_switch_label(string: &mut String, value: i64, signed: bool) {
	string.push_str("case ");
	let _ = match (signed, value) {
		// `2147483648` doesn't fit a signed literal, so the minimum is spelled as an expression.
		(true, value) if value == i64::from(i32::MIN) => write!(string, "({}-1)", i32::MIN + 1),
		(true, value) => write!(string, "{value}"),
		(false, value) => write!(string, "{value}u"),
	};
	string.push(':');
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
		besl::Operators::BitwiseXor => "^",
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

/// Reports whether the BESL type `name` is a texture, which a function parameter can take. Backends whose textures
/// carry no sampler of their own pair each such parameter with one.
pub(crate) fn is_texture_besl_type(name: &str) -> bool {
	matches!(
		name,
		"Texture2D" | "Texture3D" | "TextureCube" | "TextureCubeArray" | "ArrayTexture2D"
	)
}

pub(crate) fn is_builtin_struct_type(name: &str) -> bool {
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
			| "vec4f" | "mat2f"
			| "mat3f" | "mat4f"
			| "mat4x3f"
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
			| "atomicu32"
			| "atomici32"
	)
}

/// Returns the name of `parameter` when it is a function parameter of a texture type.
fn texture_parameter_name(parameter: &besl::NodeReference) -> Option<String> {
	let parameter = parameter.borrow();
	let besl::Nodes::Parameter { name, r#type } = parameter.node() else {
		return None;
	};
	r#type
		.borrow()
		.get_name()
		.filter(|type_name| is_texture_besl_type(type_name))
		.map(|_| name.clone())
}

/// Reports whether the BESL type `name` holds integers, which stage interfaces must pass without interpolation.
///
/// Backends call it with the BESL type name, before translating it, to add `flat`, `nointerpolation`, or `[[flat]]`
/// to integer stage inputs and outputs.
pub(crate) fn is_integer_besl_type(name: &str) -> bool {
	matches!(
		name,
		"u8" | "u16" | "u32" | "i32" | "vec2u" | "vec2u16" | "vec4u16" | "vec2i" | "vec3u" | "vec4u"
	)
}

/// Splits an array type name into its element type and its element count.
///
/// BESL writes an array type as `element[count]`, so `vec4f[3]` splits into `vec4f` and `3`. A backend whose
/// language declares arrays in C position uses this to place the count after the variable name. Returns
/// `None` for a type that is not an array.
pub(crate) fn array_type_parts(source: &str) -> Option<(&str, &str)> {
	let (element_type, count) = source.split_once('[')?;
	Some((element_type, count.trim_end_matches(']')))
}

/// Splits an array type that stays an array in generated code into its element type and element count.
///
/// Returns `None` for types that aren't arrays and for short scalar arrays, which lower to vectors through
/// [`scalar_array_vector_type`]. Backends use it to spell local, parameter, and constructed arrays.
pub(crate) fn value_array_parts(source: &str) -> Option<(&str, &str)> {
	if scalar_array_vector_type(source).is_some() {
		return None;
	}
	array_type_parts(source)
}

/// Returns the element expressions of a call that constructs an array kept as an array, or `None` for any other node.
pub(crate) fn array_constructor_elements(node: &besl::NodeReference) -> Option<std::cell::Ref<'_, [besl::NodeReference]>> {
	std::cell::Ref::filter_map(node.borrow(), |node| match node.node() {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters })
			if function.get().borrow().get_name().and_then(value_array_parts).is_some() =>
		{
			Some(parameters.as_slice())
		}
		_ => None,
	})
	.ok()
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
			stage,
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
			besl::Expressions::FunctionCall { function, .. } => expression_uses_f16(&function.get()),
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
			besl::Expressions::Unary { operand, .. } => expression_uses_f16(operand),
			besl::Expressions::Ternary { if_true, if_false, .. } => {
				expression_uses_f16(if_true) || expression_uses_f16(if_false)
			}
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
		// A negative literal such as `-1.0` narrows like the literal it negates.
		besl::Nodes::Expression(besl::Expressions::Unary {
			operator: besl::UnaryOperators::Negate,
			operand,
		}) => is_numeric_literal(operand),
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

	/// Reports whether a BESL name collides with a reserved word of the target shader language.
	///
	/// Include keywords, reserved words, built-in type names, and names the backend itself emits into user
	/// scopes. BESL keywords never reach this check because the BESL compiler rejects them. Backends call
	/// [`Self::identifier`] instead of this method when they write a name.
	fn is_reserved_identifier(name: &str) -> bool;

	/// Wraps a BESL name so it is written without colliding with the target language.
	///
	/// Use it at every declaration and every reference of a user name so both sides stay in sync.
	fn identifier(name: &str) -> Identifier<'_> {
		let prefixed = name.starts_with(RESERVED_IDENTIFIER_PREFIX) || Self::is_reserved_identifier(name);
		Identifier { name, prefixed }
	}

	/// Maps a non-array BESL type name to its backend spelling.
	///
	/// Built-in BESL types translate through [`Self::type_from_besl`]. User structs, and user functions reached
	/// through call syntax, go through [`Self::identifier`] so they match their escaped declarations.
	fn type_identifier(source: &str) -> Identifier<'_> {
		let translated = Self::type_from_besl(source);
		if translated != source || is_builtin_struct_type(source) {
			Identifier {
				name: translated,
				prefixed: false,
			}
		} else {
			Self::identifier(source)
		}
	}

	/// Returns the depth of the flagged match whose arm is being emitted, or `None` inside a loop body.
	///
	/// A `switch` case can't `break` its enclosing loop, so a `break` in a flagged match arm sets that match's
	/// flag instead. Backends store an `Option<usize>` field, starting at `None`, and return it here.
	/// [`Self::emit_match_node`] and [`Self::emit_for_loop_node`] update it.
	fn match_break_depth(&mut self) -> &mut Option<usize>;

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

	fn emit_separator(&self, string: &mut String) {
		string.push_str(ShaderFormatting::new(self.minified()).comma_str());
	}

	/// Opens a struct declaration. Pass an [`Identifier`] for user structs so the name is backend-safe.
	fn emit_named_struct_start(&self, string: &mut String, name: impl std::fmt::Display) {
		let formatting = ShaderFormatting::new(self.minified());
		let _ = write!(string, "struct {name}{}{{{}", formatting.space_str(), formatting.break_str());
	}

	fn emit_struct_declaration_end(&self, string: &mut String) {
		string.push_str("};");
		string.push_str(ShaderFormatting::new(self.minified()).break_str());
	}

	fn emit_block_end(&self, string: &mut String) {
		string.push('}');
		string.push_str(ShaderFormatting::new(self.minified()).break_str());
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

	/// Emits the sampler a backend pairs with the texture parameter `name`, right after that parameter. Backends
	/// whose texture types carry their sampler emit nothing.
	fn emit_texture_parameter_sampler(&mut self, _string: &mut String, _name: &str) {}

	/// Emits the sampler paired with `argument`, a texture passed to a function, right after that argument, so it
	/// matches what [`Self::emit_texture_parameter_sampler`] declared.
	fn emit_texture_argument_sampler(&mut self, _string: &mut String, _argument: &besl::NodeReference) {}

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
		self.emit_function_attributes(string, this_node, name);
		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push(' ');
		Self::identifier(name).push_to(string);
		string.push('(');
		self.emit_function_parameters(string, params);
		self.emit_function_extra_parameters(string, this_node, name, !params.is_empty());
		ShaderFormatting::new(self.minified()).push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	/// The qualifier that declares a specialization aggregate, such as `const` or `static const`.
	const SPECIALIZATION_QUALIFIER: &'static str;

	/// Writes the declaration of one specialization constant, without the line break.
	///
	/// `type_name` is already translated, `name` is the constant's name, and `index` is its field position, which
	/// backends with pipeline specialization use as the constant ID. [`Self::emit_specialization_node`] calls it.
	fn emit_specialization_constant(&self, string: &mut String, type_name: &str, name: std::fmt::Arguments<'_>, index: usize);

	/// Writes a specialization block: one constant per field of its struct type, named `<name>_<field>`, then an
	/// aggregate of that type named `name` that collects them.
	///
	/// Backends call it for [`besl::Nodes::Specialization`] and customize it through
	/// [`Self::emit_specialization_constant`] and [`Self::SPECIALIZATION_QUALIFIER`].
	fn emit_specialization_node(&self, string: &mut String, name: &str, r#type: &besl::NodeReference) {
		let r#type = r#type.borrow();
		let type_name = Self::type_identifier(r#type.get_name().unwrap());
		let fields = match r#type.node() {
			besl::Nodes::Struct { fields, .. } => fields.as_slice(),
			_ => &[],
		};
		let break_str = ShaderFormatting::new(self.minified()).break_str();

		// Declare every constant, keeping the field position as its specialization ID.
		for (index, field) in fields.iter().enumerate() {
			let field = field.borrow();
			let besl::Nodes::Member {
				name: member_name,
				r#type,
				..
			} = field.node()
			else {
				continue;
			};
			self.emit_specialization_constant(
				string,
				Self::type_from_besl(r#type.borrow().get_name().unwrap()),
				format_args!("{name}_{member_name}"),
				index,
			);
			string.push_str(break_str);
		}

		// Collect the constants into the aggregate the shader reads.
		let _ = write!(
			string,
			"{} {type_name} {}={type_name}(",
			Self::SPECIALIZATION_QUALIFIER,
			Self::identifier(name)
		);
		let mut separator = "";
		for field in fields {
			if let besl::Nodes::Member { name: member_name, .. } = field.borrow().node() {
				let _ = write!(string, "{separator}{name}_{member_name}");
				separator = ",";
			}
		}
		string.push_str(");");
		string.push_str(break_str);
	}

	fn emit_struct_node(
		&mut self,
		string: &mut String,
		name: &str,
		fields: &[besl::NodeReference],
		template: &Option<besl::NodeReference>,
	) {
		if template.is_some() || is_builtin_struct_type(name) {
			return;
		}

		let formatting = ShaderFormatting::new(self.minified());
		self.emit_named_struct_start(string, Self::identifier(name));
		emit_statement_block(string, formatting, fields, 1, |string, field| self.emit_node(string, field));
		self.emit_struct_declaration_end(string);
	}

	/// Emits a local variable's or a parameter's type and name.
	///
	/// The default places an array dimension on the type, as GLSL writes `float[3] values`. A backend whose
	/// language declares arrays in C position overrides this to write `float values[3]` instead.
	fn emit_variable_declaration(&mut self, string: &mut String, name: &str, type_name: &str) {
		Self::emit_type_name(string, type_name);
		string.push(' ');
		Self::identifier(name).push_to(string);
	}

	/// Writes a type and a name with an array count after the name, as C-like languages declare arrays.
	///
	/// HLSL declares every array this way, and MSL declares module constants this way. Short scalar arrays are
	/// vectors, so they keep their vector type before the name.
	fn emit_c_declaration(string: &mut String, name: &str, type_name: &str) {
		if let Some((element_type, count)) = value_array_parts(type_name) {
			let _ = write!(
				string,
				"{} {}[{count}]",
				Self::type_identifier(element_type),
				Self::identifier(name)
			);
		} else {
			Self::emit_type_name(string, type_name);
			string.push(' ');
			Self::identifier(name).push_to(string);
		}
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
				// A numeric literal beside an f16 value is cast, because GLSL does not implicitly narrow float literals to
				// float16_t. The f16 walks run only for literal operands.
				let left_as_f16 =
					*operator != besl::Operators::Assignment && is_numeric_literal(left) && expression_uses_f16(right);
				let right_as_f16 = is_numeric_literal(right) && expression_uses_f16(left);
				let emit_value = |emitter: &mut Self, string: &mut String, value: &besl::NodeReference, as_f16: bool| {
					if as_f16 {
						Self::emit_type_name(string, "f16");
						string.push('(');
						emitter.emit_node(string, value);
						string.push(')');
					} else {
						emitter.emit_wrapped_expression(string, value);
					}
				};

				emit_value(self, string, left, left_as_f16);
				string.push_str(formatting.space_str());
				string.push_str(operator_token(operator));
				string.push_str(formatting.space_str());
				emit_value(self, string, right, right_as_f16);
			}
			besl::Expressions::Unary { operator, operand } => {
				string.push_str(operator.token());
				self.emit_wrapped_expression(string, operand);
			}
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => {
				// As with binary operators, a numeric literal branch beside an f16 branch is cast, because GLSL does not
				// implicitly narrow float literals to float16_t and both branches must have one type.
				let true_as_f16 = is_numeric_literal(if_true) && expression_uses_f16(if_false);
				let false_as_f16 = is_numeric_literal(if_false) && expression_uses_f16(if_true);
				let emit_branch = |emitter: &mut Self, string: &mut String, value: &besl::NodeReference, as_f16: bool| {
					if as_f16 {
						Self::emit_type_name(string, "f16");
						string.push('(');
						emitter.emit_node(string, value);
						string.push(')');
					} else {
						emitter.emit_wrapped_expression(string, value);
					}
				};
				self.emit_wrapped_expression(string, condition);
				string.push_str(formatting.space_str());
				string.push('?');
				string.push_str(formatting.space_str());
				emit_branch(self, string, if_true, true_as_f16);
				string.push_str(formatting.space_str());
				string.push(':');
				string.push_str(formatting.space_str());
				emit_branch(self, string, if_false, false_as_f16);
			}
			besl::Expressions::FunctionCall {
				parameters, function, ..
			} => {
				let function_ref = function.get();
				if self.emit_function_call(string, &function_ref, parameters) {
					return;
				}
				let function = RefCell::borrow(&function_ref);
				let name = function.get_name().unwrap();
				Self::emit_type_name(string, name);
				let texture_parameters: Vec<bool> = match function.node() {
					besl::Nodes::Function { params, .. } => params
						.iter()
						.map(|parameter| texture_parameter_name(parameter).is_some())
						.collect(),
					_ => Vec::new(),
				};
				drop(function);
				string.push('(');
				for (index, argument) in parameters.iter().enumerate() {
					if index > 0 {
						self.emit_separator(string);
					}
					self.emit_node(string, argument);
					if texture_parameters.get(index).copied().unwrap_or(false) {
						self.emit_texture_argument_sampler(string, argument);
					}
				}
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
				Self::identifier(name).push_to(string);
			}
			besl::Expressions::VariableDeclaration { name, r#type } => {
				self.emit_variable_declaration(string, name, r#type.borrow().get_name().unwrap());
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
			besl::Expressions::Break => self.emit_break(string),
			besl::Expressions::Discard => self.emit_discard(string),
			besl::Expressions::Accessor { left, right } => self.emit_accessor_expression(string, left, right),
		}
	}

	fn emit_conditional_node(
		&mut self,
		string: &mut String,
		condition: &besl::NodeReference,
		statements: &[besl::NodeReference],
		else_branch: Option<&besl::ElseBranch>,
	) {
		let formatting = ShaderFormatting::new(self.minified());
		string.push_str("if(");
		self.emit_node(string, condition);
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);

		let Some(else_branch) = else_branch else {
			self.emit_block_end(string);
			return;
		};

		string.push('}');
		string.push_str(formatting.space_str());
		string.push_str("else");

		match else_branch {
			besl::ElseBranch::If(conditional) if !self.else_if_needs_block(conditional) => {
				// Emit the link directly so backend-specific conditional rewrites never apply to an `else if`.
				let conditional = conditional.borrow();
				let besl::Nodes::Conditional {
					condition,
					statements,
					else_branch,
				} = conditional.node()
				else {
					unreachable!("An `else if` link always holds a conditional node");
				};
				string.push(' ');
				self.emit_conditional_node(string, condition, statements, else_branch.as_ref());
			}
			// A block, or an `else if` link the backend emits as `else { if ... }`.
			else_branch => {
				string.push_str(formatting.space_str());
				string.push('{');
				string.push_str(formatting.break_str());
				self.emit_function_statement_block(string, else_branch.statements(), 1);
				self.emit_block_end(string);
			}
		}
	}

	/// Reports whether an `else if` link must be emitted as `else { if ... }`, so that the
	/// nested conditional goes through [`NodeEmitter::emit_function_statement_block`].
	/// Override it when the backend emits extra statements before some conditionals.
	fn else_if_needs_block(&self, _conditional: &besl::NodeReference) -> bool {
		false
	}

	/// Emits a BESL `break`, which always leaves the innermost loop. See [`Self::match_break_depth`].
	fn emit_break(&mut self, string: &mut String) {
		if let Some(depth) = *self.match_break_depth() {
			push_match_break_flag(string, depth);
			string.push_str("=true");
			self.emit_statement_end(string);
		}
		string.push_str("break");
	}

	/// Emits a `match` as a `switch` whose cases never fall through.
	///
	/// When an arm holds a `break` for an enclosing loop, the `switch` is wrapped in a block with a flag.
	/// The arm sets the flag and leaves the `switch`, and the code after it breaks the loop, as Rust does.
	fn emit_match_node(
		&mut self,
		string: &mut String,
		scrutinee: &besl::NodeReference,
		r#type: &besl::NodeReference,
		arms: &[besl::MatchArm],
		default: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified());
		let outer_depth = *self.match_break_depth();
		let cases = arms
			.iter()
			.map(|arm| (&arm.values[..], &arm.statements[..]))
			.chain([(&[][..], default)]);
		let flag_depth = cases
			.clone()
			.flat_map(|(_, statements)| statements)
			.any(breaks_enclosing_loop)
			.then(|| outer_depth.map_or(0, |depth| depth + 1));

		if let Some(depth) = flag_depth {
			string.push('{');
			string.push_str("bool ");
			push_match_break_flag(string, depth);
			string.push_str("=false");
			self.emit_statement_end(string);
		}

		// Every target switches on a 32-bit integer, so narrower scalars and `bool` widen to `uint`.
		let (signed, widen) = match r#type.borrow().get_name() {
			Some("i32") => (true, false),
			Some("u32") => (false, false),
			_ => (false, true),
		};
		string.push_str(if widen { "switch(uint(" } else { "switch(" });
		self.emit_node(string, scrutinee);
		if widen {
			string.push(')');
		}
		formatting.push_block_start(string);

		*self.match_break_depth() = flag_depth.or(outer_depth);
		for (values, statements) in cases {
			for &value in values {
				push_switch_label(string, value, signed);
			}
			// Only the default case has no labels.
			if values.is_empty() {
				string.push_str("default:");
			}
			// Braces scope each arm's declarations, which C++-based targets require inside a case.
			string.push('{');
			string.push_str(formatting.break_str());
			self.emit_function_statement_block(string, statements, 1);
			string.push_str("break");
			self.emit_statement_end(string);
			self.emit_block_end(string);
		}
		*self.match_break_depth() = outer_depth;
		string.push('}');

		if let Some(depth) = flag_depth {
			string.push_str("if(");
			push_match_break_flag(string, depth);
			formatting.push_block_start(string);
			// Emitted at the outer depth, so a match nested in another flagged arm forwards the break to its flag.
			self.emit_break(string);
			self.emit_statement_end(string);
			string.push_str("}}");
		}
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
		// A `break` in the body leaves this loop, even when the loop sits inside a match arm.
		let outer_depth = self.match_break_depth().take();
		self.emit_function_statement_block(string, statements, 1);
		*self.match_break_depth() = outer_depth;
		self.emit_block_end(string);
	}

	/// Wraps a node's string representation in parentheses when the node is an operator or
	/// expression, otherwise emits it directly.
	fn emit_wrapped_expression(&mut self, string: &mut String, node: &besl::NodeReference) {
		match node.borrow().node() {
			// A nested prefix operator is wrapped too, so `-(-x)` never prints as the decrement `--x`.
			besl::Nodes::Expression(
				besl::Expressions::Operator { .. }
				| besl::Expressions::Expression { .. }
				| besl::Expressions::Unary { .. }
				| besl::Expressions::Ternary { .. },
			) => {
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
		} else if let Some((element_type, count)) = array_type_parts(source) {
			let _ = write!(string, "{}[{count}]", Self::type_identifier(element_type));
		} else {
			Self::type_identifier(source).push_to(string);
		}
	}

	/// Emits a function's comma-separated parameter declarations, each texture parameter followed by the sampler the
	/// backend pairs with it.
	fn emit_function_parameters(&mut self, string: &mut String, params: &[besl::NodeReference]) {
		for (index, parameter) in params.iter().enumerate() {
			if index > 0 {
				self.emit_separator(string);
			}
			self.emit_node(string, parameter);
			if let Some(name) = texture_parameter_name(parameter) {
				self.emit_texture_parameter_sampler(string, &name);
			}
		}
	}

	/// Emits comma-separated call arguments with the backend's formatting rules.
	fn emit_call_arguments(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		for (i, argument) in arguments.iter().enumerate() {
			if i > 0 {
				self.emit_separator(string);
			}
			self.emit_node(string, argument);
		}
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

		assert!(
			super::validate_workgroup_storage_stage(
				&super::Stages::Compute {
					local_size: Extent::square(8)
				},
				&order
			)
			.is_ok()
		);
		assert!(
			super::validate_workgroup_storage_stage(
				&super::Stages::Task {
					local_size: Extent::line(32),
					maximum_mesh_threadgroups: 32,
				},
				&order,
			)
			.is_ok()
		);
		assert!(super::validate_workgroup_storage_stage(&super::Stages::Fragment, &order).is_err());
	}

	/// Builds one sampled-image binding for resource-interface tests.
	pub fn sampled_binding(name: &str, slot: u32, read: bool, write: bool) -> besl::NodeReference {
		besl::Node::binding(
			name,
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			slot,
			read,
			write,
		)
		.into()
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

		besl::compile_to_besl(&script, Some(root_node)).unwrap().get_main().unwrap()
	}

	/// Builds the 52-byte meshlet record, whose `vec4f` members follow 16 bytes of scalars and are followed by a
	/// `vec2u16`, to verify scalar-aligned vector storage across backends.
	pub fn vec4f_meshlet_binding() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let center: vec4f = buff.meshlets[1].center_radius;
			center.x;
			buff.meshlets[0].cone_apex_cutoff.w;
		}
		"#;
		let mut root_node = besl::Node::root();
		let u32_type = root_node.get_child("u32").expect("Expected u32 type");
		let vec4f_type = root_node.get_child("vec4f").expect("Expected vec4f type");
		let vec2u16_type = root_node.get_child("vec2u16").expect("Expected vec2u16 type");
		let meshlet = root_node.add_child(
			besl::Node::r#struct(
				"Meshlet",
				vec![
					besl::Node::member("primitive_offset", u32_type.clone()).into(),
					besl::Node::member("triangle_offset", u32_type.clone()).into(),
					besl::Node::member("primitive_count", u32_type.clone()).into(),
					besl::Node::member("triangle_count", u32_type).into(),
					besl::Node::member("center_radius", vec4f_type.clone()).into(),
					besl::Node::member("cone_apex_cutoff", vec4f_type).into(),
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
		root.get_main().expect("Expected main function")
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
		root.get_main().expect("Expected main function")
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
		root.get_main().expect("Expected main function")
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
		root.get_main().expect("Expected main function")
	}

	/// Builds a mesh shader that writes one per-vertex and one per-primitive output array, used to verify where each
	/// backend declares mesh outputs.
	pub fn vertex_and_primitive_mesh_outputs() -> besl::NodeReference {
		let root = besl::compile_to_besl(
			r#"
			out_primitive_index: output<u32, 1, 1>;
			out_uv: vertex_output<vec2f, 2, 3>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				if (lane == 0) {
					set_mesh_output_counts(3, 1);
				}
				if (lane < 3) {
					set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
					out_uv[lane] = vec2f(f32(lane), 1.0);
				}
				if (lane < 1) {
					set_mesh_triangle(0, vec3u(0, 1, 2));
					out_primitive_index[0] = lane;
				}
			}
			"#,
			None,
		)
		.expect("Expected mesh shader source to compile");

		root.get_main().expect("Expected mesh shader source to contain main")
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
		root.get_main().expect("Expected main function")
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

		besl::compile_to_besl(&script, Some(root_node)).unwrap().get_main().unwrap()
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

		besl::compile_to_besl(&script, Some(root_node)).unwrap().get_main().unwrap()
	}

	/// Builds a vertex `main` whose body is one raw statement with GLSL, HLSL, and MSL variants that reads a user struct.
	pub fn multi_language_raw_code() -> besl::NodeReference {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		main: fn () -> void {}
		"#;
		let root = besl::compile_to_besl(script, None).unwrap();
		let main = root.get_main().unwrap();
		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();
		main.borrow_mut().add_child(
			besl::Node::raw(
				Some("gl_Position = vec4(0)".to_string()),
				Some("output.position = float4(0, 0, 0, 1)".to_string()),
				Some("out.position = float4(0, 0, 0, 1)".to_string()),
				vec![vertex_struct],
				vec![],
			)
			.into(),
		);
		main
	}

	/// Builds a compute `main` that reads one exact texel with `fetch`.
	pub fn texel_fetch() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
			texel;
		}
		"#;
		let mut root = besl::Node::root();
		root.add_child(sampled_binding("texture", 0, true, false));
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to link");
		root.get_main().expect("Expected main")
	}

	/// Returns the linked program. Keep it alive while you use `main`: it owns the functions `main` calls.
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

		besl::compile_to_besl(&script, None).unwrap()
	}

	pub fn push_constant() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			push_constant;
		}
		"#;

		let mut root_node = besl::Node::root();

		let u32_t = root_node.get_child("u32").unwrap();
		root_node.add_child(besl::Node::push_constant(vec![besl::Node::member("material_id", u32_t).into()]).into());

		besl::compile_to_besl(&script, Some(root_node)).unwrap().get_main().unwrap()
	}

	pub fn const_variable() -> besl::NodeReference {
		let script = r#"
		PI: const f32 = 3.14;

		main: fn () -> void {
			PI;
		}
		"#;

		besl::compile_to_besl(&script, None).unwrap().get_main().unwrap()
	}
}

pub use Settings as ShaderGenerationSettings;
