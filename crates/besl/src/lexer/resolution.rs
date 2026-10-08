use std::{collections::HashMap, fmt::Write as _};

use super::*;
use crate::parser;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DescendantSearch {
	Any,
	NonIntrinsic,
}

/// Resolves a node reference by searching the current lexical scope chain.
///
/// A function in the chain is one the lookup runs inside, so its parameters are visible. Its earlier locals are visible
/// too: the lexer pushes each statement onto the chain after the function, so the chain searches them before it
/// reaches the function. Functions reached while searching an enclosing scope keep theirs private. See
/// [`find_descendant`].
pub(super) fn get_reference(chain: &[NodeReference], name: &str) -> Option<NodeReference> {
	for node in chain.iter().rev() {
		let reference = match node.borrow().node() {
			Nodes::Intrinsic { .. } => find_descendant(node, name, DescendantSearch::Any),
			Nodes::Function {
				name: function_name,
				params,
				..
			} => (function_name == name)
				.then(|| node.clone())
				.or_else(|| find_named_child(params, name)),
			_ => find_descendant(node, name, DescendantSearch::NonIntrinsic),
		};

		if let Some(c) = reference {
			return Some(c);
		}
	}

	None
}

/// Resolves a type name to its struct declaration.
///
/// Only struct nodes declare types. Accepting any named node would let a type name such as `root` or a function's own
/// name point back at an ancestor, which forms an `Rc` cycle that is never freed.
pub(super) fn resolve_type(chain: &[NodeReference], type_name: &str) -> Result<NodeReference, LexError> {
	let existing = get_reference(chain, type_name);
	if let Some(existing) = existing.filter(|node| matches!(node.borrow().node(), Nodes::Struct { .. })) {
		return Ok(existing);
	}

	if type_name.contains('[') {
		let mut parts = type_name.split(['[', ']']);
		let element_type_name = parts.next().ok_or_else(|| LexError::invalid("No type name"))?;
		let count = parts
			.next()
			.ok_or_else(|| LexError::invalid("No count"))?
			.parse::<usize>()
			.map_err(|_| LexError::invalid("Invalid count"))?;

		let element_type = parser::TypeName::Named(element_type_name);
		return resolve_array_type(chain, &element_type, count);
	}

	Err(LexError::ReferenceToUndefinedType {
		type_name: type_name.to_string(),
	})
}

/// Resolves a source descriptor's resource type into the existing semantic binding representation.
pub(super) fn resolve_descriptor_type(
	chain: &[NodeReference],
	resource_type: &str,
	format: Option<&str>,
	runtime_array: bool,
) -> Result<BindingTypes, LexError> {
	if runtime_array {
		if format.is_some() {
			return Err(LexError::invalid(
				"Runtime buffer arrays cannot declare an image format. The most likely cause is that [] was attached to a formatted storage image.",
			));
		}
		let element = resolve_type(chain, resource_type)?;
		// Records (user structs and built-in vectors) have fields; numeric scalars are field-less built-ins that still
		// have a byte representation. Empty structs, `bool`, `void`, and resource handles have none.
		let storable = match element.borrow().node() {
			Nodes::Struct { fields, .. } if !fields.is_empty() => true,
			Nodes::Struct { name, .. } => super::ast::STORABLE_SCALAR_TYPES.contains(&name.as_str()),
			_ => false,
		};
		if !storable {
			return Err(LexError::invalid(format!(
				"Runtime buffer element `{resource_type}` has no storable buffer representation. The most likely cause is that [] was attached to a boolean, empty, or resource-handle type."
			)));
		}
		return Ok(BindingTypes::BufferArray { element, fixed: None });
	}

	if format.is_some() && resource_type != "StorageImage" {
		return Err(LexError::invalid(format!(
			"Resource type {resource_type} cannot declare a storage image format. The most likely cause is that a format was attached to a non-StorageImage descriptor."
		)));
	}

	match resource_type {
		"Texture2D" => Ok(BindingTypes::CombinedImageSampler { format: String::new() }),
		"Texture2DArray" => Ok(BindingTypes::CombinedImageSampler {
			format: "ArrayTexture2D".to_string(),
		}),
		"Texture3D" => Ok(BindingTypes::CombinedImageSampler {
			format: "Texture3D".to_string(),
		}),
		"TextureCube" => Ok(BindingTypes::CombinedImageSampler {
			format: "TextureCube".to_string(),
		}),
		"TextureCubeArray" => Ok(BindingTypes::CombinedImageSampler {
			format: "TextureCubeArray".to_string(),
		}),
		"StorageImage" => Ok(BindingTypes::Image {
			format: format.unwrap_or("unknown").to_string(),
		}),
		struct_name => {
			let r#struct = resolve_type(chain, struct_name)?;
			let members = match r#struct.borrow().node() {
				Nodes::Struct { fields, .. } => fields.clone(),
				_ => {
					return Err(LexError::ReferenceToUndefinedType {
						type_name: struct_name.to_string(),
					});
				}
			};
			Ok(BindingTypes::Buffer { members })
		}
	}
}

/// Resolves a structural array type and creates its semantic indexed members.
pub(super) fn resolve_array_type(
	chain: &[NodeReference],
	element_type_name: &parser::TypeName,
	count: usize,
) -> Result<NodeReference, LexError> {
	let mut array_name = String::new();
	append_type_name(&mut array_name, element_type_name);
	let _ = write!(array_name, "[{count}]");
	if let Some(existing) = get_reference(chain, &array_name) {
		return Ok(existing);
	}

	let element_type = resolve_type_name(chain, element_type_name)?;
	let array_type = NodeReference::from(Node {
		node: Nodes::Struct {
			name: array_name,
			template: Some(element_type.clone()),
			fields: (0..count)
				.map(|index| Node::member(&format!("value_{index}"), element_type.clone()).into())
				.collect(),
			types: Vec::new(),
		},
	});

	Ok(array_type)
}

/// Appends a structural parser type's canonical spelling to an owned name.
pub(super) fn append_type_name(name: &mut String, type_name: &parser::TypeName) {
	match type_name {
		parser::TypeName::Named(type_name) => name.push_str(type_name),
		parser::TypeName::Array { element, count } => {
			append_type_name(name, element);
			let _ = write!(name, "[{count}]");
		}
		parser::TypeName::Record { role, .. } => {
			let _ = write!(name, "{role}");
		}
	}
}

/// Resolves a parser type without flattening its array structure into source text.
pub(super) fn resolve_type_name(chain: &[NodeReference], type_name: &parser::TypeName) -> Result<NodeReference, LexError> {
	match type_name {
		parser::TypeName::Named(type_name) => resolve_type(chain, type_name),
		parser::TypeName::Array { element, count } => {
			let count = usize::try_from(*count).map_err(|_| LexError::invalid("Invalid count"))?;
			resolve_array_type(chain, element, count)
		}
		parser::TypeName::Record { role, .. } => Err(LexError::invalid(format!(
			"Anonymous {role} types are valid only on main. The most likely cause is that a structural stage type was used as an ordinary value type."
		))),
	}
}

/// Resolves a bare name through the lexical scope chain.
pub(super) fn resolve_member(chain: &[NodeReference], name: &str) -> Result<NodeReference, LexError> {
	get_reference(chain, name).ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })
}

/// Resolves the name after `.` in `left.name` to a member of `left`'s type.
///
/// Member names form their own namespace, so a local, binding, or field named `name` elsewhere in scope never
/// shadows the member.
pub(super) fn resolve_accessed_member(left: &NodeReference, name: &str) -> Result<NodeReference, LexError> {
	let undeclared = || LexError::AccessingUndeclaredMember { name: name.to_string() };

	// Buffer bindings and push constants declare their members on themselves rather than on a value type.
	if let Some(source) = expression_source(left) {
		match source.borrow().node() {
			Nodes::Binding {
				r#type: BindingTypes::Buffer { members },
				..
			}
			| Nodes::PushConstant { members } => return find_named_child(members, name).ok_or_else(undeclared),
			_ => {}
		}
	}

	let r#type = infer_expression_type(left).ok_or_else(undeclared)?;
	let r#type = r#type.borrow();
	match r#type.node() {
		Nodes::Struct { fields, .. } => find_named_child(fields, name).ok_or_else(undeclared),
		_ => Err(undeclared()),
	}
}

/// Resolves raw-code IO references and lowers them into a lexer node.
pub(super) fn lex_raw_code(
	chain: &[NodeReference],
	glsl: Option<&str>,
	hlsl: Option<&str>,
	msl: Option<&str>,
	input: &[&str],
	output: &[&str],
) -> Result<Node, LexError> {
	let inputs = input
		.iter()
		.map(|name| resolve_member(chain, name))
		.collect::<Result<Vec<_>, _>>()?;

	let vec3f = resolve_member(chain, "vec3f")?;
	let outputs = output
		.iter()
		.map(|name| {
			Node::expression(Expressions::VariableDeclaration {
				name: (*name).to_string(),
				r#type: vec3f.clone(),
			})
			.into()
		})
		.collect();

	Ok(Node::raw(
		glsl.map(str::to_string),
		hlsl.map(str::to_string),
		msl.map(str::to_string),
		inputs,
		outputs,
	))
}

pub(super) fn find_descendant(node: &NodeReference, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	let prefer_descendants_before_self = mode == DescendantSearch::NonIntrinsic
		&& matches!(
			node.borrow().node(),
			Nodes::PushConstant { .. }
				| Nodes::Member { .. }
				| Nodes::Parameter { .. }
				| Nodes::Input { .. }
				| Nodes::Output { .. }
				| Nodes::TaskPayload { .. }
				| Nodes::Workgroup { .. }
				| Nodes::Expression(Expressions::Member { .. })
		);

	if !prefer_descendants_before_self && node.borrow().get_name() == Some(child_name) {
		return Some(node.clone());
	}

	let result = match node.borrow().node() {
		// Lexical lookup sees only declared names. Struct fields, a value's type members, and resource members are
		// reached through `value.member` (see `resolve_accessed_member`), and a function's parameters and locals are
		// private to it.
		Nodes::Struct { .. }
		| Nodes::Member { .. }
		| Nodes::Parameter { .. }
		| Nodes::Function { .. }
		| Nodes::PushConstant { .. }
		| Nodes::Binding { .. }
			if mode == DescendantSearch::NonIntrinsic =>
		{
			None
		}
		Nodes::Scope { children, .. } | Nodes::Struct { fields: children, .. } | Nodes::PushConstant { members: children } => {
			find_in_children(children, child_name, mode == DescendantSearch::NonIntrinsic, mode)
		}
		Nodes::Intrinsic { elements, .. } => {
			if mode == DescendantSearch::Any {
				find_in_children(elements, child_name, false, mode)
			} else {
				None
			}
		}
		Nodes::Member { r#type, .. } | Nodes::Parameter { r#type, .. } => find_descendant(r#type, child_name, mode),
		Nodes::Function { params, statements, .. } => find_in_function(params, statements, child_name, mode),
		// Control-flow statements own their block scopes, so later statements never see declarations inside them.
		Nodes::Conditional { .. } | Nodes::Match { .. } | Nodes::ForLoop { .. } => None,
		Nodes::Expression(expression) => find_in_expression(expression, child_name, mode),
		Nodes::Raw { output, .. } => find_in_descendants(output, child_name, mode),
		Nodes::Binding {
			r#type: BindingTypes::Buffer { members },
			..
		} => find_in_descendants(members, child_name, mode),
		Nodes::Binding {
			r#type: BindingTypes::BufferArray { element, .. },
			..
		} => find_descendant(element, child_name, mode),
		Nodes::Input { format, .. }
		| Nodes::Output { format, .. }
		| Nodes::TaskPayload { format, .. }
		| Nodes::Workgroup { format, .. } => find_descendant(format, child_name, mode),
		_ => None,
	};

	result.or_else(|| {
		if prefer_descendants_before_self && node.borrow().get_name() == Some(child_name) {
			Some(node.clone())
		} else {
			None
		}
	})
}

pub(super) fn find_in_children(
	children: &[NodeReference],
	child_name: &str,
	prefer_direct_children: bool,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	if prefer_direct_children {
		find_named_child(children, child_name).or_else(|| find_in_descendants(children, child_name, mode))
	} else {
		find_in_descendants(children, child_name, mode)
	}
}

pub(super) fn find_named_child(children: &[NodeReference], child_name: &str) -> Option<NodeReference> {
	children
		.iter()
		.find(|child| child.borrow().get_name() == Some(child_name))
		.cloned()
}

pub(super) fn find_in_descendants(
	children: &[NodeReference],
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	children.iter().find_map(|child| find_descendant(child, child_name, mode))
}

pub(super) fn find_in_function(
	params: &[NodeReference],
	statements: &[NodeReference],
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	find_named_child(params, child_name).or_else(|| {
		statements
			.iter()
			.find_map(|statement| find_in_function_statement(statement, child_name, mode))
	})
}

pub(super) fn find_in_function_statement(
	statement: &NodeReference,
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	match statement.borrow().node() {
		Nodes::Expression(expression) => find_in_function_expression(statement, expression, child_name, mode),
		Nodes::Raw { output, .. } if mode == DescendantSearch::Any => find_in_descendants(output, child_name, mode),
		_ => None,
	}
}

pub(super) fn find_in_function_expression(
	statement: &NodeReference,
	expression: &Expressions,
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	match mode {
		DescendantSearch::Any => match expression {
			Expressions::Operator { left, right, .. } => {
				find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
			}
			Expressions::Unary { operand, .. } => find_descendant(operand, child_name, mode),
			Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => find_descendant(condition, child_name, mode)
				.or_else(|| find_descendant(if_true, child_name, mode))
				.or_else(|| find_descendant(if_false, child_name, mode)),
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Accessor { left, right } => {
				find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
			}
			Expressions::Return { value } => value.as_ref().and_then(|value| find_descendant(value, child_name, mode)),
			_ => None,
		},
		DescendantSearch::NonIntrinsic => match expression {
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Operator { left, .. } if is_declaration(left) => find_descendant(left, child_name, mode),
			_ => None,
		},
	}
}

/// Reports whether an assignment's left side declares a local, the only kind of statement that adds a name to scope.
fn is_declaration(left: &NodeReference) -> bool {
	matches!(
		left.borrow().node(),
		Nodes::Expression(Expressions::VariableDeclaration { .. })
	)
}

pub(super) fn find_in_expression(expression: &Expressions, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	match expression {
		// Only assignment declarations on the left enter the surrounding lexical scope. A store such as
		// `views.views[i] = …` declares nothing, so its accesses must not shadow later names.
		Expressions::Operator { left, .. } if mode == DescendantSearch::NonIntrinsic => is_declaration(left)
			.then(|| find_descendant(left, child_name, mode))
			.flatten(),
		Expressions::Operator { left, right, .. } => {
			find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
		}
		// Neither a prefix operator nor a ternary declares a name, so only a search of every descendant enters them.
		Expressions::Unary { operand, .. } if mode == DescendantSearch::Any => find_descendant(operand, child_name, mode),
		Expressions::Ternary {
			condition,
			if_true,
			if_false,
		} if mode == DescendantSearch::Any => find_descendant(condition, child_name, mode)
			.or_else(|| find_descendant(if_true, child_name, mode))
			.or_else(|| find_descendant(if_false, child_name, mode)),
		Expressions::Member { source, .. } => find_descendant(source, child_name, mode),
		Expressions::Expression { elements } => find_in_descendants(elements, child_name, mode),
		// A local exposes only its own name to the scope. Its type's fields belong to `local.field` accesses.
		Expressions::VariableDeclaration { r#type, .. } if mode == DescendantSearch::Any => {
			find_descendant(r#type, child_name, mode)
		}
		Expressions::Accessor { left, right } => {
			find_descendant(right, child_name, mode).or_else(|| find_descendant(left, child_name, mode))
		}
		Expressions::IntrinsicCall { intrinsic, .. } => {
			let intrinsic = intrinsic.borrow();
			if let Nodes::Intrinsic { r#return, .. } = intrinsic.node() {
				find_descendant(r#return, child_name, mode)
			} else {
				None
			}
		}
		Expressions::Return { value } => value.as_ref().and_then(|value| find_descendant(value, child_name, mode)),
		_ => None,
	}
}

/// The `IntrinsicInstantiation` struct keeps one intrinsic expansion separate from its caller's scope.
struct IntrinsicInstantiation {
	arguments: HashMap<usize, NodeReference>,
	locals: HashMap<usize, IntrinsicLocal>,
}

/// The `IntrinsicLocal` struct preserves one renamed intrinsic-local declaration and its emitted name.
#[derive(Clone)]
struct IntrinsicLocal {
	declaration: NodeReference,
	name: String,
}

/// Instantiates an intrinsic body with its call arguments and fresh local declaration names.
pub(super) fn build_intrinsic(
	elements: &[NodeReference],
	parameters: &[NodeReference],
	expansion_id: usize,
) -> Result<Vec<NodeReference>, LexError> {
	let is_parameter = |element: &&NodeReference| matches!(element.borrow().node(), Nodes::Parameter { .. });
	if elements.iter().filter(is_parameter).count() != parameters.len() {
		return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
	}

	let body = elements.iter().filter(|element| !is_parameter(element)).collect::<Vec<_>>();
	if body.is_empty() {
		return Ok(parameters.to_vec());
	}

	let mut locals = Vec::new();
	for element in &body {
		collect_intrinsic_local_declarations(element, &mut locals);
	}

	let mut local_replacements = HashMap::with_capacity(locals.len());
	for declaration in locals {
		let (name, r#type) = match declaration.borrow().node() {
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => (name.clone(), r#type.clone()),
			_ => unreachable!("Intrinsic local collection must return variable declarations"),
		};
		let name = format!("_besl_intrinsic_{expansion_id}_{name}");
		let replacement = Node::expression(Expressions::VariableDeclaration {
			name: name.clone(),
			r#type,
		})
		.into();
		local_replacements.insert(
			declaration.identity(),
			IntrinsicLocal {
				declaration: replacement,
				name,
			},
		);
	}

	let instantiation = IntrinsicInstantiation {
		arguments: elements
			.iter()
			.filter(is_parameter)
			.map(|parameter| parameter.identity())
			.zip(parameters.iter().cloned())
			.collect(),
		locals: local_replacements,
	};

	Ok(body
		.into_iter()
		.map(|element| instantiate_intrinsic_node(element, &instantiation))
		.collect())
}

pub(super) fn intrinsic_matches_parameters(intrinsic: &NodeReference, parameters: &[NodeReference]) -> bool {
	let intrinsic = intrinsic.borrow();
	let Nodes::Intrinsic { elements, .. } = intrinsic.node() else {
		return false;
	};

	// Walk the parameter types twice, to check the arity before any type, without collecting them.
	let expected_parameters = || {
		elements.iter().filter_map(|element| match element.borrow().node() {
			Nodes::Parameter { r#type, .. } => Some(r#type.clone()),
			_ => None,
		})
	};
	expected_parameters().count() == parameters.len()
		&& expected_parameters()
			.zip(parameters)
			.all(|(expected, parameter)| expression_matches_type(parameter, &expected))
}

pub(super) fn expression_matches_type(expression: &NodeReference, expected_type: &NodeReference) -> bool {
	infer_expression_type(expression)
		.map(|actual_type| actual_type.borrow().get_name() == expected_type.borrow().get_name())
		// Resource bindings do not expose a value type until backend lowering. Their
		// arity still selects the correct overload, and known value arguments remain checked.
		.unwrap_or(true)
}

/// Returns the type node of the value `expression` produces, or `None` when linking cannot know it, such as for a
/// resource binding.
///
/// Overload resolution uses it to pick an intrinsic, and shader backends use it to choose type-dependent spellings,
/// such as a matrix product. It accepts linked expressions and the declarations they reference. Built-in types of
/// literals and comparisons come from a shared [`Node::root`] registry, so compare the result by
/// [`Node::get_name`] rather than by node identity.
pub fn infer_expression_type(expression: &NodeReference) -> Option<NodeReference> {
	match expression.borrow().node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => infer_expression_type(&elements[0]),
		Nodes::Expression(Expressions::Literal { value }) => infer_literal_type(value),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => infer_member_type(source),
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			if let Some(element) = runtime_buffer_array_element(left) {
				Some(element)
			} else if is_array_texture_reference(left) {
				// The binding stores its native texture shape rather than a linked `Texture2D`
				// handle. Leaving this contextual layer view unknown keeps overload matching
				// permissive without allocating a second built-in registry.
				None
			} else if matches!(left.borrow().node(), Nodes::Workgroup { .. } | Nodes::TaskPayload { .. }) {
				infer_member_type(left)
			} else if is_index(right) {
				indexed_element_type(left)
			} else {
				infer_expression_type(right)
			}
		}
		Nodes::Expression(Expressions::FunctionCall { function, .. }) => infer_callable_return_type(&function.get()),
		Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => infer_callable_return_type(intrinsic),
		Nodes::Expression(Expressions::Operator { operator, left, right }) => infer_operator_result_type(operator, left, right),
		Nodes::Expression(Expressions::Unary {
			operator: UnaryOperators::LogicalNot,
			..
		}) => builtin_type("bool"),
		// Negation and bitwise not keep their operand's type.
		Nodes::Expression(Expressions::Unary { operand, .. }) => infer_expression_type(operand),
		// Both branches of a ternary have the same type; a branch that linking cannot type defers to the other one.
		Nodes::Expression(Expressions::Ternary { if_true, if_false, .. }) => {
			infer_expression_type(if_true).or_else(|| infer_expression_type(if_false))
		}
		// A declaration read in place, such as the workgroup array under an index, has its declared type.
		Nodes::Member { .. }
		| Nodes::Parameter { .. }
		| Nodes::Input { .. }
		| Nodes::Output { .. }
		| Nodes::TaskPayload { .. }
		| Nodes::Workgroup { .. }
		| Nodes::Specialization { .. }
		| Nodes::Const { .. } => infer_member_type(expression),
		_ => None,
	}
}

/// Returns the runtime-array element type selected by an indexed binding expression.
fn runtime_buffer_array_element(expression: &NodeReference) -> Option<NodeReference> {
	let source = expression_source(expression)?;
	let borrowed = source.borrow();
	match borrowed.node() {
		Nodes::Binding {
			r#type: BindingTypes::BufferArray { element, .. },
			..
		} => Some(element.clone()),
		_ => None,
	}
}

/// Reports whether indexing this expression selects one layer of a layered 2D texture.
pub(super) fn is_array_texture_reference(expression: &NodeReference) -> bool {
	let Some(source) = expression_source(expression) else {
		return false;
	};
	let borrowed = source.borrow();
	matches!(
		borrowed.node(),
		Nodes::Binding {
			r#type: BindingTypes::CombinedImageSampler { format },
			count: None,
			..
		} if format == "ArrayTexture2D"
	)
}

/// Peels member wrappers and named accesses to the declaration that supplies their value.
fn expression_source(expression: &NodeReference) -> Option<NodeReference> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Binding { .. } => Some(expression.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => Some(source.clone()),
		Nodes::Expression(Expressions::Accessor { right, .. }) if !is_index(right) => expression_source(right),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => expression_source(&elements[0]),
		_ => None,
	}
}

/// Infers arithmetic results so overload selection sees the value produced by an expression, not merely its left operand.
pub(super) fn infer_operator_result_type(
	operator: &Operators,
	left: &NodeReference,
	right: &NodeReference,
) -> Option<NodeReference> {
	if *operator == Operators::Assignment {
		return infer_expression_type(left);
	}
	if matches!(
		operator,
		Operators::Equality
			| Operators::LessThan
			| Operators::Inequality
			| Operators::GreaterThan
			| Operators::LessThanOrEqual
			| Operators::GreaterThanOrEqual
			| Operators::LogicalAnd
			| Operators::LogicalOr
	) {
		return builtin_type("bool");
	}

	let left_type = infer_expression_type(left);
	let right_type = infer_expression_type(right);
	// Compare the borrowed type names. The borrows end with this block, before either type is returned.
	let right_first = {
		let left_node = left_type.as_ref().map(|r#type| r#type.borrow());
		let right_node = right_type.as_ref().map(|r#type| r#type.borrow());
		let left_name = left_node.as_deref().and_then(Node::get_name);
		let right_name = right_node.as_deref().and_then(Node::get_name);
		if *operator == Operators::Multiply {
			match (left_name, right_name) {
				(Some("mat4x3f"), Some("vec4f")) => return builtin_type("vec3f"),
				(Some("mat4f"), Some("vec4f")) => return builtin_type("vec4f"),
				_ => {}
			}
		}
		// An `f32` operand takes the other operand's type, so `f32 * vec3f` is a `vec3f`.
		left_name != right_name && left_name == Some("f32")
	};

	if right_first {
		right_type.or(left_type)
	} else {
		left_type.or(right_type)
	}
}

pub(super) fn infer_literal_type(value: &str) -> Option<NodeReference> {
	if matches!(value, "true" | "false") {
		builtin_type("bool")
	} else if value.contains(['.', 'e', 'E']) {
		builtin_type("f32")
	} else {
		builtin_type("u32")
	}
}

thread_local! {
	/// One built-in registry per thread, so inference names `bool` or `vec4f` without rebuilding [`Node::root`] for
	/// every literal and comparison it types.
	static BUILTIN_REGISTRY: Node = Node::root();
}

/// Returns the built-in type named `name` from the shared registry.
fn builtin_type(name: &str) -> Option<NodeReference> {
	BUILTIN_REGISTRY.with(|root| match root.node() {
		Nodes::Scope { children, .. } => find_named_child(children, name),
		_ => None,
	})
}

pub(super) fn infer_member_type(source: &NodeReference) -> Option<NodeReference> {
	match source.borrow().node() {
		Nodes::Member { r#type, .. }
		| Nodes::Parameter { r#type, .. }
		| Nodes::Input { format: r#type, .. }
		| Nodes::Output { format: r#type, .. }
		| Nodes::TaskPayload { format: r#type, .. }
		| Nodes::Workgroup { format: r#type, .. }
		| Nodes::Specialization { r#type, .. }
		| Nodes::Const { r#type, .. } => Some(r#type.clone()),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
		Nodes::Expression(Expressions::Member { source, name }) => {
			let parent_type = infer_member_type(source)?;
			find_named_member_type(&parent_type, name)
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			if matches!(left.borrow().node(), Nodes::Workgroup { .. } | Nodes::TaskPayload { .. }) {
				infer_member_type(left)
			} else if is_index(right) {
				indexed_element_type(left)
			} else {
				infer_expression_type(right)
			}
		}
		_ => None,
	}
}

/// Reports whether the right side of an accessor is a bracketed index rather than a member name.
pub(super) fn is_index(right: &NodeReference) -> bool {
	matches!(right.borrow().node(), Nodes::Expression(Expressions::Expression { .. }))
}

/// Returns the element type that indexing `indexed` selects, or `None` when it is unknown.
///
/// The index expression's own type says nothing about the element, so overload selection must not use it.
fn indexed_element_type(indexed: &NodeReference) -> Option<NodeReference> {
	let source = expression_source(indexed).unwrap_or_else(|| indexed.clone());
	let indexed_type = match source.borrow().node() {
		Nodes::Workgroup { format, count, .. } | Nodes::Output { format, count, .. } if count.is_some() => {
			return Some(format.clone());
		}
		Nodes::Member {
			r#type, count: Some(_), ..
		} => return Some(r#type.clone()),
		Nodes::TaskPayload { format, .. } => return Some(format.clone()),
		_ => infer_member_type(&source)?,
	};
	// Array types are structs whose template is the element type. Matrices index their columns and vectors their
	// components, and both declare those as same-typed fields.
	match indexed_type.borrow().node() {
		Nodes::Struct {
			template: Some(element), ..
		} => Some(element.clone()),
		Nodes::Struct { name, fields, .. } if name.starts_with("vec") || name.starts_with("mat") => {
			fields.first().and_then(infer_member_type)
		}
		_ => None,
	}
}

pub(super) fn find_named_member_type(parent_type: &NodeReference, member_name: &str) -> Option<NodeReference> {
	match parent_type.borrow().node() {
		Nodes::Struct { fields, .. } => fields.iter().find_map(|field| match field.borrow().node() {
			Nodes::Member { name, r#type, .. } if name == member_name => Some(r#type.clone()),
			_ => None,
		}),
		_ => None,
	}
}

pub(super) fn infer_callable_return_type(callable: &NodeReference) -> Option<NodeReference> {
	match callable.borrow().node() {
		Nodes::Function { return_type, .. } => Some(return_type.clone()),
		Nodes::Struct { .. } => Some(callable.clone()),
		Nodes::Intrinsic { r#return, .. } => Some(r#return.clone()),
		_ => None,
	}
}

pub(super) fn resolve_call_target(
	chain: &[NodeReference],
	name: &parser::TypeName,
	parameters: &[NodeReference],
) -> Result<NodeReference, LexError> {
	let parser::TypeName::Named(name) = name else {
		return resolve_type_name(chain, name);
	};

	for node in chain.iter().rev() {
		if let Some(candidate) = resolve_call_target_in_node(node, name, parameters) {
			return Ok(candidate);
		}
	}

	// Calls may name an intrinsic or a type constructor, so this fallback accepts any declaration, not only types.
	if let Some(r#type) = get_reference(chain, name) {
		let mismatched_intrinsic_with_known_types =
			matches!(r#type.borrow().node(), Nodes::Intrinsic { .. }) && parameters.iter().all(expression_has_reliable_type);
		// Resource expressions do not always expose a value type during linking, so keep the established fallback only when
		// overload matching lacked enough information. Fully known intrinsic arguments must match their declared types.
		if !mismatched_intrinsic_with_known_types {
			return Ok(r#type);
		}
	}
	Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters)
}

/// Reports whether overload resolution can trust the expression's linked value type.
pub(super) fn expression_has_reliable_type(expression: &NodeReference) -> bool {
	match expression.borrow().node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			expression_has_reliable_type(&elements[0])
		}
		Nodes::Expression(
			Expressions::Literal { .. }
			| Expressions::VariableDeclaration { .. }
			| Expressions::FunctionCall { .. }
			| Expressions::IntrinsicCall { .. },
		) => true,
		Nodes::Expression(Expressions::Member { source, .. }) => !matches!(
			source.borrow().node(),
			Nodes::Binding { .. } | Nodes::Input { .. } | Nodes::Output { .. }
		),
		Nodes::Expression(Expressions::Operator { left, right, .. }) => {
			expression_has_reliable_type(left) && expression_has_reliable_type(right)
		}
		Nodes::Expression(Expressions::Unary {
			operator: UnaryOperators::LogicalNot,
			..
		}) => true,
		Nodes::Expression(Expressions::Unary { operand, .. }) => expression_has_reliable_type(operand),
		Nodes::Expression(Expressions::Ternary { if_true, if_false, .. }) => {
			expression_has_reliable_type(if_true) && expression_has_reliable_type(if_false)
		}
		_ => false,
	}
}

pub(super) fn resolve_call_target_in_node(
	node: &NodeReference,
	name: &str,
	parameters: &[NodeReference],
) -> Option<NodeReference> {
	match node.borrow().node() {
		Nodes::Scope { children, .. } | Nodes::Struct { fields: children, .. } | Nodes::PushConstant { members: children } => {
			children.iter().find_map(|child| match child.borrow().node() {
				Nodes::Intrinsic {
					name: candidate_name, ..
				} if candidate_name == name && intrinsic_matches_parameters(child, parameters) => Some(child.clone()),
				Nodes::Function {
					name: candidate_name,
					params,
					..
				} if candidate_name == name && params.len() == parameters.len() => Some(child.clone()),
				Nodes::Struct {
					name: candidate_name,
					fields,
					..
				} if candidate_name == name && fields.len() == parameters.len() => Some(child.clone()),
				_ => resolve_call_target_in_node(child, name, parameters),
			})
		}
		_ => None,
	}
}

/// Collects local declarations that must be renamed before an intrinsic body is inlined.
fn collect_intrinsic_local_declarations(node: &NodeReference, declarations: &mut Vec<NodeReference>) {
	match node.borrow().node() {
		Nodes::Expression(Expressions::VariableDeclaration { .. }) => declarations.push(node.clone()),
		Nodes::Scope { children, .. } => {
			for child in children {
				collect_intrinsic_local_declarations(child, declarations);
			}
		}
		// Raw backend source is opaque, so its declared textual names must remain unchanged. Functions and constants
		// are declarations the body refers to, not part of it.
		Nodes::Raw { .. } | Nodes::Function { .. } | Nodes::Const { .. } => {}
		other => {
			for child in other.children() {
				collect_intrinsic_local_declarations(child, declarations);
			}
		}
	}
}

/// Instantiates every statement of a control-flow block. See [`instantiate_intrinsic_node`].
fn instantiate_intrinsic_block(statements: &[NodeReference], instantiation: &IntrinsicInstantiation) -> Vec<NodeReference> {
	statements
		.iter()
		.map(|statement| instantiate_intrinsic_node(statement, instantiation))
		.collect()
}

/// Clones one structured intrinsic-body node while preserving links to caller and outer-scope nodes.
fn instantiate_intrinsic_node(node: &NodeReference, instantiation: &IntrinsicInstantiation) -> NodeReference {
	let argument = {
		let node = node.borrow();
		match node.node() {
			Nodes::Expression(Expressions::Member { source, .. }) => instantiation.arguments.get(&source.identity()).cloned(),
			_ => None,
		}
	};
	if let Some(argument) = argument {
		return argument;
	}
	if let Some(local) = instantiation.locals.get(&node.identity()) {
		return local.declaration.clone();
	}

	let node = node.borrow();
	match node.node() {
		Nodes::Scope { name, children } => {
			let mut scope = Node::scope(name.clone());
			for child in children {
				scope.add_child(instantiate_intrinsic_node(child, instantiation));
			}
			scope.into()
		}
		Nodes::Expression(expression) => Node::expression(instantiate_intrinsic_expression(expression, instantiation)).into(),
		Nodes::Conditional {
			condition,
			statements,
			else_branch,
		} => {
			Node::conditional(
				instantiate_intrinsic_node(condition, instantiation),
				instantiate_intrinsic_block(statements, instantiation),
				else_branch.as_ref().map(|else_branch| match else_branch {
					ElseBranch::Block(statements) => ElseBranch::Block(instantiate_intrinsic_block(statements, instantiation)),
					ElseBranch::If(conditional) => ElseBranch::If(instantiate_intrinsic_node(conditional, instantiation)),
				}),
			)
		}
		.into(),
		Nodes::Match {
			scrutinee,
			r#type,
			arms,
			default,
		} => Node::r#match(
			instantiate_intrinsic_node(scrutinee, instantiation),
			r#type.clone(),
			arms.iter()
				.map(|arm| MatchArm {
					values: arm.values.clone(),
					statements: instantiate_intrinsic_block(&arm.statements, instantiation),
				})
				.collect(),
			instantiate_intrinsic_block(default, instantiation),
		)
		.into(),
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => Node::for_loop(
			instantiate_intrinsic_node(initializer, instantiation),
			instantiate_intrinsic_node(condition, instantiation),
			instantiate_intrinsic_node(update, instantiation),
			instantiate_intrinsic_block(statements, instantiation),
		)
		.into(),
		Nodes::Raw {
			glsl,
			hlsl,
			msl,
			input,
			output,
		} => Node::raw(
			glsl.clone(),
			hlsl.clone(),
			msl.clone(),
			input
				.iter()
				.map(|input| instantiate_intrinsic_node(input, instantiation))
				.collect(),
			output.clone(),
		)
		.into(),
		// Definition nodes and outer-scope references remain shared. Only structured body nodes can contain
		// implementation-local values that need a fresh declaration per intrinsic call.
		_ => node.clone().into(),
	}
}

/// Clones one intrinsic-body expression and substitutes linked parameters and locals by identity.
fn instantiate_intrinsic_expression(expression: &Expressions, instantiation: &IntrinsicInstantiation) -> Expressions {
	match expression {
		Expressions::Operator { operator, left, right } => Expressions::Operator {
			operator: *operator,
			left: instantiate_intrinsic_node(left, instantiation),
			right: instantiate_intrinsic_node(right, instantiation),
		},
		Expressions::Unary { operator, operand } => Expressions::Unary {
			operator: *operator,
			operand: instantiate_intrinsic_node(operand, instantiation),
		},
		Expressions::Ternary {
			condition,
			if_true,
			if_false,
		} => Expressions::Ternary {
			condition: instantiate_intrinsic_node(condition, instantiation),
			if_true: instantiate_intrinsic_node(if_true, instantiation),
			if_false: instantiate_intrinsic_node(if_false, instantiation),
		},
		Expressions::FunctionCall { function, parameters } => Expressions::FunctionCall {
			function: function.clone(),
			parameters: parameters
				.iter()
				.map(|parameter| instantiate_intrinsic_node(parameter, instantiation))
				.collect(),
		},
		Expressions::IntrinsicCall {
			intrinsic,
			arguments,
			elements,
		} => Expressions::IntrinsicCall {
			intrinsic: intrinsic.clone(),
			arguments: arguments
				.iter()
				.map(|argument| instantiate_intrinsic_node(argument, instantiation))
				.collect(),
			elements: elements
				.iter()
				.map(|element| instantiate_intrinsic_node(element, instantiation))
				.collect(),
		},
		Expressions::Expression { elements } => Expressions::Expression {
			elements: elements
				.iter()
				.map(|element| instantiate_intrinsic_node(element, instantiation))
				.collect(),
		},
		Expressions::Macro { name, body } => Expressions::Macro {
			name: name.clone(),
			body: instantiate_intrinsic_node(body, instantiation),
		},
		Expressions::Member { source, name } => {
			if let Some(local) = instantiation.locals.get(&source.identity()) {
				Expressions::Member {
					source: local.declaration.clone(),
					name: local.name.clone(),
				}
			} else {
				Expressions::Member {
					source: source.clone(),
					name: name.clone(),
				}
			}
		}
		Expressions::VariableDeclaration { name, r#type } => Expressions::VariableDeclaration {
			name: name.clone(),
			r#type: r#type.clone(),
		},
		Expressions::Literal { value } => Expressions::Literal { value: value.clone() },
		Expressions::Return { value } => Expressions::Return {
			value: value.as_ref().map(|value| instantiate_intrinsic_node(value, instantiation)),
		},
		Expressions::Continue => Expressions::Continue,
		Expressions::Break => Expressions::Break,
		Expressions::Discard => Expressions::Discard,
		Expressions::Accessor { left, right } => Expressions::Accessor {
			left: instantiate_intrinsic_node(left, instantiation),
			right: instantiate_intrinsic_node(right, instantiation),
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parameter(name: &str, r#type: NodeReference) -> NodeReference {
		Node::new(Nodes::Parameter {
			name: name.to_string(),
			r#type,
		})
		.into()
	}

	fn member(name: &str, source: NodeReference) -> NodeReference {
		Node::expression(Expressions::Member {
			name: name.to_string(),
			source,
		})
		.into()
	}

	fn literal(value: &str) -> NodeReference {
		Node::expression(Expressions::Literal {
			value: value.to_string(),
		})
		.into()
	}

	#[test]
	fn intrinsic_expansion_substitutes_each_parameter_by_identity() {
		let f32_type = Node::root().get_child("f32").expect("The standard BESL scope defines f32");
		let left = parameter("left", f32_type.clone());
		let right = parameter("right", f32_type);
		let body: NodeReference = Node::expression(Expressions::Operator {
			operator: Operators::Plus,
			left: member("left", left.clone()),
			right: member("left", left.clone()),
		})
		.into();
		let first_argument = literal("1.0");
		let second_argument = literal("2.0");

		let expanded = build_intrinsic(&[left, right, body], &[first_argument.clone(), second_argument], 0)
			.expect("The intrinsic arguments match its declaration");
		let expression = expanded[0].borrow();
		let Nodes::Expression(Expressions::Operator { left, right, .. }) = expression.node() else {
			panic!("Expected an expanded operator expression");
		};

		assert_eq!(left, &first_argument);
		assert_eq!(right, &first_argument);
	}

	#[test]
	fn intrinsic_expansion_mangles_template_locals_per_call() {
		let f32_type = Node::root().get_child("f32").expect("The standard BESL scope defines f32");
		let value = parameter("value", f32_type.clone());
		let template_local: NodeReference = Node::expression(Expressions::VariableDeclaration {
			name: "temporary".to_string(),
			r#type: f32_type,
		})
		.into();
		let assignment: NodeReference = Node::expression(Expressions::Operator {
			operator: Operators::Assignment,
			left: template_local.clone(),
			right: member("value", value.clone()),
		})
		.into();
		let body: NodeReference = Node::expression(Expressions::Expression {
			elements: vec![assignment, member("temporary", template_local)],
		})
		.into();
		let definition = [value, body];

		let first = build_intrinsic(&definition, &[literal("1.0")], 0).expect("The first intrinsic call should expand");
		let second = build_intrinsic(&definition, &[literal("2.0")], 1).expect("The second intrinsic call should expand");
		let (first_name, first_declaration, first_reference_source) = expanded_local(&first[0]);
		let (second_name, ..) = expanded_local(&second[0]);

		assert!(first_name.starts_with("_besl_intrinsic_"));
		assert_ne!(first_name, "temporary");
		assert_ne!(first_name, second_name);
		assert_eq!(first_declaration, first_reference_source);
	}

	fn expanded_local(body: &NodeReference) -> (String, NodeReference, NodeReference) {
		let body = body.borrow();
		let Nodes::Expression(Expressions::Expression { elements }) = body.node() else {
			panic!("Expected the expanded intrinsic body");
		};
		let assignment = elements[0].borrow();
		let Nodes::Expression(Expressions::Operator { left, .. }) = assignment.node() else {
			panic!("Expected the expanded intrinsic local assignment");
		};
		let name = {
			let declaration = left.borrow();
			let Nodes::Expression(Expressions::VariableDeclaration { name, .. }) = declaration.node() else {
				panic!("Expected a local declaration");
			};
			name.clone()
		};
		let reference = elements[1].borrow();
		let Nodes::Expression(Expressions::Member { source, .. }) = reference.node() else {
			panic!("Expected a reference to the expanded intrinsic local");
		};

		(name, left.clone(), source.clone())
	}
}
