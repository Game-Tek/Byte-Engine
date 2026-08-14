use std::fmt::Write as _;
use std::ops::Deref;

use super::lowering::lex_parsed_node;
use super::*;
use crate::parser;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DescendantSearch {
	Any,
	NonIntrinsic,
}

/// Resolves a node reference by searching the current lexical scope chain.
pub(super) fn get_reference(chain: &[NodeReference], name: &str) -> Option<NodeReference> {
	for node in chain.iter().rev() {
		let reference = match node.borrow().node() {
			Nodes::Intrinsic { .. } => find_descendant(node, name, DescendantSearch::Any),
			_ => find_descendant(node, name, DescendantSearch::NonIntrinsic),
		};

		if let Some(c) = reference {
			return Some(c);
		}
	}

	None
}

pub(super) fn resolve_type(chain: &[NodeReference], type_name: &str) -> Result<NodeReference, LexError> {
	if let Some(existing) = get_reference(chain, type_name) {
		return Ok(existing);
	}

	if type_name.contains('[') {
		let mut parts = type_name.split(['[', ']']);
		let element_type_name = parts.next().ok_or(LexError::Undefined {
			message: Some("No type name".to_string()),
		})?;
		let count = parts
			.next()
			.ok_or(LexError::Undefined {
				message: Some("No count".to_string()),
			})?
			.parse::<usize>()
			.map_err(|_| LexError::Undefined {
				message: Some("Invalid count".to_string()),
			})?;

		let element_type = parser::TypeName::Named(element_type_name);
		return resolve_array_type(chain, &element_type, count);
	}

	get_reference(chain, type_name).ok_or(LexError::ReferenceToUndefinedType {
		type_name: type_name.to_string(),
	})
}

/// Resolves a source descriptor's resource type into the existing semantic binding representation.
pub(super) fn resolve_descriptor_type(
	chain: &[NodeReference],
	resource_type: &str,
	format: Option<&str>,
) -> Result<BindingTypes, LexError> {
	if format.is_some() && resource_type != "StorageImage" {
		return Err(LexError::Undefined {
			message: Some(format!(
				"Resource type {resource_type} cannot declare a storage image format. The most likely cause is that a format was attached to a non-StorageImage descriptor."
			)),
		});
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
	let array_type = Node::internal_new(Node {
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
	}
}

/// Resolves a parser type without flattening its array structure into source text.
pub(super) fn resolve_type_name(chain: &[NodeReference], type_name: &parser::TypeName) -> Result<NodeReference, LexError> {
	match type_name {
		parser::TypeName::Named(type_name) => resolve_type(chain, type_name),
		parser::TypeName::Array { element, count } => {
			let count = usize::try_from(*count).map_err(|_| LexError::Undefined {
				message: Some("Invalid count".to_string()),
			})?;
			resolve_array_type(chain, element, count)
		}
	}
}
pub(super) fn resolve_member(chain: &[NodeReference], name: &str) -> Result<NodeReference, LexError> {
	// After the left side of an accessor has resolved a buffer binding, the next identifier
	// belongs to that buffer's member namespace even when the binding and member share a name.
	if let Some(left) = chain.last() {
		let source = match left.borrow().node() {
			Nodes::Expression(Expressions::Member { source, .. }) => Some(source.clone()),
			_ => None,
		};
		if let Some(source) = source {
			if let Nodes::Binding {
				r#type: BindingTypes::Buffer { members },
				..
			} = source.borrow().node()
			{
				if let Some(member) = find_named_child(members, name) {
					return Ok(member);
				}
			}
		}
	}
	get_reference(chain, name).ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })
}

/// Clones the lexical scope chain and appends the current parent node.
pub(super) fn extend_chain(chain: &[NodeReference], parent: &NodeReference) -> Vec<NodeReference> {
	let mut extended = chain.to_vec();
	extended.push(parent.clone());
	extended
}

/// Lexes one parser child in the scope of its parent node.
pub(super) fn lex_child_with_parent(
	chain: &[NodeReference],
	parent: &NodeReference,
	parser_node: &parser::Node,
) -> Result<NodeReference, LexError> {
	lex_parsed_node(extend_chain(chain, parent), parser_node)
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
		Nodes::Conditional { condition, statements } if mode == DescendantSearch::NonIntrinsic => {
			find_descendant(condition, child_name, mode).or_else(|| find_in_descendants(statements, child_name, mode))
		}
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} if mode == DescendantSearch::NonIntrinsic => find_descendant(initializer, child_name, mode)
			.or_else(|| find_descendant(condition, child_name, mode))
			.or_else(|| find_descendant(update, child_name, mode))
			.or_else(|| find_in_descendants(statements, child_name, mode)),
		Nodes::Expression(expression) => find_in_expression(expression, child_name, mode),
		Nodes::Raw { output, .. } => find_in_descendants(output, child_name, mode),
		Nodes::Binding {
			r#type: BindingTypes::Buffer { members },
			..
		} => find_in_descendants(members, child_name, mode),
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
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Accessor { left, right } => {
				find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
			}
			Expressions::Return { value } => value.as_ref().and_then(|value| find_descendant(value, child_name, mode)),
			_ => None,
		},
		DescendantSearch::NonIntrinsic => match expression {
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Operator { left, .. } => find_descendant(left, child_name, mode),
			_ => None,
		},
	}
}

pub(super) fn find_in_expression(expression: &Expressions, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	match expression {
		// Only assignment declarations on the left enter the surrounding lexical scope.
		Expressions::Operator { left, .. } if mode == DescendantSearch::NonIntrinsic => find_descendant(left, child_name, mode),
		Expressions::Operator { left, right, .. } => {
			find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
		}
		Expressions::Member { source, .. } => find_descendant(source, child_name, mode),
		Expressions::Expression { elements } => find_in_descendants(elements, child_name, mode),
		Expressions::VariableDeclaration { r#type, .. } => find_descendant(r#type, child_name, mode),
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

pub(super) fn build_intrinsic(
	elements: &[NodeReference],
	parameters: &[NodeReference],
) -> Result<Vec<NodeReference>, LexError> {
	let expected_parameter_count = elements
		.iter()
		.filter(|element| matches!(element.borrow().node(), Nodes::Parameter { .. }))
		.count();

	if expected_parameter_count != parameters.len() {
		return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
	}

	let has_body = elements
		.iter()
		.any(|element| !matches!(element.borrow().node(), Nodes::Parameter { .. }));

	if !has_body {
		return Ok(parameters.to_vec());
	}

	build_intrinsic_elements(elements, &mut parameters.iter())
}

pub(super) fn intrinsic_matches_parameters(intrinsic: &NodeReference, parameters: &[NodeReference]) -> bool {
	let intrinsic = intrinsic.borrow();
	let Nodes::Intrinsic { elements, .. } = intrinsic.node() else {
		return false;
	};

	let expected_parameters = elements
		.iter()
		.filter_map(|element| match element.borrow().node() {
			Nodes::Parameter { r#type, .. } => Some(r#type.clone()),
			_ => None,
		})
		.collect::<Vec<_>>();

	if expected_parameters.len() != parameters.len() {
		return false;
	}

	expected_parameters
		.iter()
		.zip(parameters.iter())
		.all(|(expected, parameter)| expression_matches_type(parameter, expected))
}

pub(super) fn expression_matches_type(expression: &NodeReference, expected_type: &NodeReference) -> bool {
	infer_expression_type(expression)
		.map(|actual_type| actual_type.borrow().get_name() == expected_type.borrow().get_name())
		// Resource bindings do not expose a value type until backend lowering. Their
		// arity still selects the correct overload, and known value arguments remain checked.
		.unwrap_or(true)
}

pub(super) fn infer_expression_type(expression: &NodeReference) -> Option<NodeReference> {
	match expression.borrow().node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => infer_expression_type(&elements[0]),
		Nodes::Expression(Expressions::Literal { value }) => infer_literal_type(value),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => infer_member_type(source),
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			if matches!(left.borrow().node(), Nodes::Workgroup { .. } | Nodes::TaskPayload { .. }) {
				infer_member_type(left)
			} else {
				infer_expression_type(right)
			}
		}
		Nodes::Expression(Expressions::FunctionCall { function, .. }) => infer_callable_return_type(function),
		Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => infer_callable_return_type(intrinsic),
		Nodes::Expression(Expressions::Operator { operator, left, right }) => infer_operator_result_type(operator, left, right),
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
		return None;
	}

	let left_type = infer_expression_type(left);
	let right_type = infer_expression_type(right);
	let left_name = left_type
		.as_ref()
		.and_then(|r#type| r#type.borrow().get_name().map(str::to_owned));
	let right_name = right_type
		.as_ref()
		.and_then(|r#type| r#type.borrow().get_name().map(str::to_owned));

	if *operator == Operators::Multiply {
		let product_type = match (left_name.as_deref(), right_name.as_deref()) {
			(Some("mat4x3f"), Some("vec4f")) => Some("vec3f"),
			(Some("mat4f"), Some("vec4f")) => Some("vec4f"),
			_ => None,
		};
		if let Some(product_type) = product_type {
			return Node::root().get_child(product_type);
		}
	}

	if left_name == right_name {
		return left_type.or(right_type);
	}
	if left_name.as_deref() == Some("f32") {
		return right_type.or(left_type);
	}
	if right_name.as_deref() == Some("f32") {
		return left_type.or(right_type);
	}

	left_type.or(right_type)
}

pub(super) fn infer_literal_type(value: &str) -> Option<NodeReference> {
	let root = Node::root();
	if matches!(value, "true" | "false") {
		root.get_child("bool")
	} else if value.contains(['.', 'e', 'E']) {
		root.get_child("f32")
	} else {
		root.get_child("u32")
	}
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
			} else {
				infer_expression_type(right)
			}
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

	if let Ok(r#type) = resolve_type(chain, name) {
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

pub(super) fn build_intrinsic_elements<'a>(
	elements: &[NodeReference],
	parameters: &mut impl Iterator<Item = &'a NodeReference>,
) -> Result<Vec<NodeReference>, LexError> {
	let mut ret = Vec::new();

	for e in elements
		.iter()
		.filter(|e| !matches!(e.borrow().node(), Nodes::Parameter { .. }))
	{
		let f = e.borrow();
		let e = match f.node() {
			Nodes::Expression(expression) => match expression {
				Expressions::Member { source, .. } => match source.deref().borrow().node() {
					Nodes::Parameter { .. } => parameters
						.next()
						.ok_or(LexError::Undefined {
							message: Some("Expected parameter".to_string()),
						})?
						.clone(),
					_ => e.clone(),
				},
				Expressions::Expression { elements } => NodeReference::from(Node::expression(Expressions::Expression {
					elements: build_intrinsic_elements(elements, parameters)?,
				})),
				_ => e.clone(),
			},
			_ => e.clone(),
		};

		ret.push(e);
	}

	Ok(ret)
}
