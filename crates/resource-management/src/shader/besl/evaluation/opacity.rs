use std::{cell::RefCell, collections::HashSet};

/// The `OpacityEvaluation` enum classifies whether a shader writes an opaque output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpacityEvaluation {
	Opaque,
	NonOpaque,
	Unknown,
}

/// Evaluates whether the main shader output is statically opaque.
pub(super) fn evaluate_opacity(main_function_node: &besl::NodeReference) -> OpacityEvaluation {
	let mut main_contains_raw_code = false;
	let mut local_output_symbols = HashSet::new();

	{
		let node_borrow = RefCell::borrow(main_function_node);
		let node_ref = node_borrow.node();

		if let besl::Nodes::Function { statements, params, .. } = node_ref {
			for param in params {
				let param_borrow = RefCell::borrow(param);
				if let besl::Nodes::Parameter {
					name: parameter_name, ..
				} = param_borrow.node()
				{
					if parameter_name == "output" {
						local_output_symbols.insert(param.clone());
					}
				}
			}

			for statement in statements {
				let statement_borrow = RefCell::borrow(statement);
				match statement_borrow.node() {
					besl::Nodes::Raw { .. } => {
						main_contains_raw_code = true;
					}
					_ => collect_local_output_symbols(statement, &mut local_output_symbols),
				}
			}
		}
	}

	if main_contains_raw_code {
		return OpacityEvaluation::Unknown;
	}

	if writes_non_opaque_vec4f_to_non_local_output(main_function_node, &local_output_symbols) {
		return OpacityEvaluation::NonOpaque;
	}

	if references_non_local_output(main_function_node, &local_output_symbols) {
		OpacityEvaluation::Opaque
	} else {
		OpacityEvaluation::Unknown
	}
}

fn collect_local_output_symbols(node: &besl::NodeReference, local_output_symbols: &mut HashSet<besl::NodeReference>) {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, params, .. } => {
			for param in params {
				collect_local_output_symbols(param, local_output_symbols);
			}
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Conditional { condition, statements } => {
			collect_local_output_symbols(condition, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_local_output_symbols(initializer, local_output_symbols);
			collect_local_output_symbols(condition, local_output_symbols);
			collect_local_output_symbols(update, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::VariableDeclaration { name, .. } => {
				if name == "output" {
					local_output_symbols.insert(node.clone());
				}
			}
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				collect_local_output_symbols(callable, local_output_symbols);
				for argument in arguments {
					collect_local_output_symbols(argument, local_output_symbols);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				collect_local_output_symbols(left, local_output_symbols);
				collect_local_output_symbols(right, local_output_symbols);
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					collect_local_output_symbols(element, local_output_symbols);
				}
			}
			besl::Expressions::Member { source, .. } => {
				collect_local_output_symbols(source, local_output_symbols);
			}
			besl::Expressions::Macro { body, .. } => {
				collect_local_output_symbols(body, local_output_symbols);
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Discard => {}
		},
		besl::Nodes::Raw { input, output, .. } => {
			for value in input.iter().chain(output.iter()) {
				collect_local_output_symbols(value, local_output_symbols);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				collect_local_output_symbols(element, local_output_symbols);
			}
			collect_local_output_symbols(r#return, local_output_symbols);
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			collect_local_output_symbols(nested, local_output_symbols);
		}
		besl::Nodes::Parameter {
			name: parameter_name,
			r#type: parameter_type,
		} => {
			if parameter_name == "output" {
				local_output_symbols.insert(node.clone());
			}
			collect_local_output_symbols(parameter_type, local_output_symbols);
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				collect_local_output_symbols(child, local_output_symbols);
			}
		}
		besl::Nodes::Binding { .. } | besl::Nodes::Null => {}
		besl::Nodes::Const { r#type, value, .. } => {
			collect_local_output_symbols(r#type, local_output_symbols);
			collect_local_output_symbols(value, local_output_symbols);
		}
	}
}

fn references_non_local_output(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| references_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			references_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			references_non_local_output(initializer, local_output_symbols)
				|| references_non_local_output(condition, local_output_symbols)
				|| references_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Member { name, source } => {
				if name == "output" && !local_output_symbols.contains(source) {
					return true;
				}

				references_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				references_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| references_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				references_non_local_output(left, local_output_symbols)
					|| references_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				references_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Discard => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| references_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols))
				|| references_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => references_non_local_output(nested, local_output_symbols),
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| references_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			references_non_local_output(r#type, local_output_symbols)
				|| references_non_local_output(value, local_output_symbols)
		}
	}
}

fn writes_non_opaque_vec4f_to_non_local_output(
	node: &besl::NodeReference,
	local_output_symbols: &HashSet<besl::NodeReference>,
) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			writes_non_opaque_vec4f_to_non_local_output(initializer, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Operator { operator, left, right } => {
				if operator == &besl::Operators::Assignment
					&& is_non_local_output_target(left, local_output_symbols)
					&& is_non_opaque_vec4f_constructor(right)
				{
					return true;
				}

				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				writes_non_opaque_vec4f_to_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| writes_non_opaque_vec4f_to_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } => {
				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Member { source, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Discard => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| writes_non_opaque_vec4f_to_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols))
				|| writes_non_opaque_vec4f_to_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| writes_non_opaque_vec4f_to_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(r#type, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(value, local_output_symbols)
		}
	}
}

fn is_non_local_output_target(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Member {
			name: member_name,
			source: member_source,
		}) => member_name == "output" && !local_output_symbols.contains(member_source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => {
			is_non_local_output_target(left, local_output_symbols)
		}
		_ => false,
	}
}

fn is_non_opaque_vec4f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			if function_borrow.get_name() != Some("vec4f") {
				return false;
			}

			let w_parameter = match parameters.len() {
				4 => Some(&parameters[3]),
				2 if is_vec3f_constructor(&parameters[0]) => Some(&parameters[1]),
				_ => None,
			};

			let Some(w_parameter) = w_parameter else {
				return false;
			};

			match parse_literal_number(w_parameter) {
				Some(w) => w != 1.0,
				None => false,
			}
		}
		_ => false,
	}
}

fn is_vec3f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			function_borrow.get_name() == Some("vec3f") && parameters.len() == 3
		}
		_ => false,
	}
}

fn parse_literal_number(node: &besl::NodeReference) -> Option<f64> {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse().ok(),
		_ => None,
	}
}
