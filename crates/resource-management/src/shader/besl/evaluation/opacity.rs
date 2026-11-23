use std::collections::HashSet;

/// The `OpacityEvaluation` enum classifies whether a shader writes an opaque output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpacityEvaluation {
	Opaque,
	NonOpaque,
	Unknown,
}

/// Evaluates whether the main shader output is statically opaque.
pub(super) fn evaluate_opacity(program: &besl::Program, main_function_node: &besl::NodeId) -> OpacityEvaluation {
	let mut local_output_symbols = HashSet::new();

	{
		let node_borrow = &program[main_function_node];
		let node_ref = node_borrow.node();

		if let besl::Nodes::Function { statements, params, .. } = node_ref {
			for param in params {
				let param_borrow = &program[param];
				if let besl::Nodes::Parameter {
					name: parameter_name, ..
				} = param_borrow.node()
					&& parameter_name == "output"
				{
					local_output_symbols.insert(*param);
				}
			}

			for statement in statements {
				collect_local_output_symbols(program, statement, &mut local_output_symbols);
			}
		}
	}

	if writes_non_opaque_vec4f_to_non_local_output(program, main_function_node, &local_output_symbols) {
		return OpacityEvaluation::NonOpaque;
	}

	if references_non_local_output(program, main_function_node, &local_output_symbols) {
		OpacityEvaluation::Opaque
	} else {
		OpacityEvaluation::Unknown
	}
}

fn collect_local_output_symbols(
	program: &besl::Program,
	node: &besl::NodeId,
	local_output_symbols: &mut HashSet<besl::NodeId>,
) {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, params, .. } => {
			for param in params {
				collect_local_output_symbols(program, param, local_output_symbols);
			}
			for statement in statements {
				collect_local_output_symbols(program, statement, local_output_symbols);
			}
		}
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
			for child in branch.branch_children() {
				collect_local_output_symbols(program, child, local_output_symbols);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_local_output_symbols(program, initializer, local_output_symbols);
			collect_local_output_symbols(program, condition, local_output_symbols);
			collect_local_output_symbols(program, update, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(program, statement, local_output_symbols);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::VariableDeclaration { name, .. } => {
				if name == "output" {
					local_output_symbols.insert(*node);
				}
			}
			besl::Expressions::FunctionCall {
				function,
				parameters: arguments,
			} => {
				collect_local_output_symbols(program, function, local_output_symbols);
				for argument in arguments {
					collect_local_output_symbols(program, argument, local_output_symbols);
				}
			}
			besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				collect_local_output_symbols(program, callable, local_output_symbols);
				for argument in arguments {
					collect_local_output_symbols(program, argument, local_output_symbols);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				collect_local_output_symbols(program, left, local_output_symbols);
				collect_local_output_symbols(program, right, local_output_symbols);
			}
			besl::Expressions::Unary { operand, .. } => collect_local_output_symbols(program, operand, local_output_symbols),
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => {
				for part in [condition, if_true, if_false] {
					collect_local_output_symbols(program, part, local_output_symbols);
				}
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					collect_local_output_symbols(program, element, local_output_symbols);
				}
			}
			besl::Expressions::Member { source, .. } => {
				collect_local_output_symbols(program, source, local_output_symbols);
			}
			besl::Expressions::Macro { body, .. } => {
				collect_local_output_symbols(program, body, local_output_symbols);
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Break
			| besl::Expressions::Discard => {}
		},
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				collect_local_output_symbols(program, element, local_output_symbols);
			}
			collect_local_output_symbols(program, r#return, local_output_symbols);
		}
		besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			collect_local_output_symbols(program, nested, local_output_symbols);
		}
		besl::Nodes::Parameter {
			name: parameter_name,
			r#type: parameter_type,
		} => {
			if parameter_name == "output" {
				local_output_symbols.insert(*node);
			}
			collect_local_output_symbols(program, parameter_type, local_output_symbols);
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				collect_local_output_symbols(program, child, local_output_symbols);
			}
		}
		besl::Nodes::Binding { .. } | besl::Nodes::Use { .. } | besl::Nodes::Layer { .. } | besl::Nodes::Signature { .. } => {}
		besl::Nodes::Const { r#type, value, .. } => {
			collect_local_output_symbols(program, r#type, local_output_symbols);
			collect_local_output_symbols(program, value, local_output_symbols);
		}
	}
}

fn references_non_local_output(
	program: &besl::Program,
	node: &besl::NodeId,
	local_output_symbols: &HashSet<besl::NodeId>,
) -> bool {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| references_non_local_output(program, statement, local_output_symbols)),
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => branch
			.branch_children()
			.any(|child| references_non_local_output(program, child, local_output_symbols)),
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			references_non_local_output(program, initializer, local_output_symbols)
				|| references_non_local_output(program, condition, local_output_symbols)
				|| references_non_local_output(program, update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(program, statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Member { name, source } => {
				if name == "output" && !local_output_symbols.contains(source) {
					return true;
				}

				references_non_local_output(program, source, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| references_non_local_output(program, element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function,
				parameters: arguments,
			} => {
				references_non_local_output(program, function, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| references_non_local_output(program, argument, local_output_symbols))
			}
			besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				references_non_local_output(program, callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| references_non_local_output(program, argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				references_non_local_output(program, left, local_output_symbols)
					|| references_non_local_output(program, right, local_output_symbols)
			}
			besl::Expressions::Unary { operand, .. } => references_non_local_output(program, operand, local_output_symbols),
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => [condition, if_true, if_false]
				.into_iter()
				.any(|part| references_non_local_output(program, part, local_output_symbols)),
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				references_non_local_output(program, nested, local_output_symbols)
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Break
			| besl::Expressions::Discard => false,
		},
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| references_non_local_output(program, element, local_output_symbols))
				|| references_non_local_output(program, r#return, local_output_symbols)
		}
		besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => references_non_local_output(program, nested, local_output_symbols),
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| references_non_local_output(program, child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Use { .. } | besl::Nodes::Layer { .. } | besl::Nodes::Signature { .. } => {
			false
		}
		besl::Nodes::Const { r#type, value, .. } => {
			references_non_local_output(program, r#type, local_output_symbols)
				|| references_non_local_output(program, value, local_output_symbols)
		}
	}
}

fn writes_non_opaque_vec4f_to_non_local_output(
	program: &besl::Program,
	node: &besl::NodeId,
	local_output_symbols: &HashSet<besl::NodeId>,
) -> bool {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| writes_non_opaque_vec4f_to_non_local_output(program, statement, local_output_symbols)),
		branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => branch
			.branch_children()
			.any(|child| writes_non_opaque_vec4f_to_non_local_output(program, child, local_output_symbols)),
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			writes_non_opaque_vec4f_to_non_local_output(program, initializer, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(program, condition, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(program, update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(program, statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Operator { operator, left, right } => {
				if operator == &besl::Operators::Assignment
					&& is_non_local_output_target(program, left, local_output_symbols)
					&& is_non_opaque_vec4f_constructor(program, right)
				{
					return true;
				}

				writes_non_opaque_vec4f_to_non_local_output(program, left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(program, right, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(program, element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function,
				parameters: arguments,
			} => {
				writes_non_opaque_vec4f_to_non_local_output(program, function, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| writes_non_opaque_vec4f_to_non_local_output(program, argument, local_output_symbols))
			}
			besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				writes_non_opaque_vec4f_to_non_local_output(program, callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| writes_non_opaque_vec4f_to_non_local_output(program, argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } => {
				writes_non_opaque_vec4f_to_non_local_output(program, left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(program, right, local_output_symbols)
			}
			besl::Expressions::Unary { operand, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(program, operand, local_output_symbols)
			}
			besl::Expressions::Ternary {
				condition,
				if_true,
				if_false,
			} => [condition, if_true, if_false]
				.into_iter()
				.any(|part| writes_non_opaque_vec4f_to_non_local_output(program, part, local_output_symbols)),
			besl::Expressions::Member { source, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(program, source, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(program, nested, local_output_symbols)
			}
			besl::Expressions::Return { .. }
			| besl::Expressions::Literal { .. }
			| besl::Expressions::Continue
			| besl::Expressions::Break
			| besl::Expressions::Discard => false,
		},
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(program, element, local_output_symbols))
				|| writes_non_opaque_vec4f_to_non_local_output(program, r#return, local_output_symbols)
		}
		besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(program, nested, local_output_symbols)
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| writes_non_opaque_vec4f_to_non_local_output(program, child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Use { .. } | besl::Nodes::Layer { .. } | besl::Nodes::Signature { .. } => {
			false
		}
		besl::Nodes::Const { r#type, value, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(program, r#type, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(program, value, local_output_symbols)
		}
	}
}

fn is_non_local_output_target(
	program: &besl::Program,
	node: &besl::NodeId,
	local_output_symbols: &HashSet<besl::NodeId>,
) -> bool {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Member {
			name: member_name,
			source: member_source,
		}) => member_name == "output" && !local_output_symbols.contains(member_source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => {
			is_non_local_output_target(program, left, local_output_symbols)
		}
		_ => false,
	}
}

fn is_non_opaque_vec4f_constructor(program: &besl::Program, node: &besl::NodeId) -> bool {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function = *function;
			let function_borrow = &program[function];
			if function_borrow.get_name() != Some("vec4f") {
				return false;
			}

			let w_parameter = match parameters.len() {
				4 => Some(&parameters[3]),
				2 if is_vec3f_constructor(program, &parameters[0]) => Some(&parameters[1]),
				_ => None,
			};

			let Some(w_parameter) = w_parameter else {
				return false;
			};

			match parse_literal_number(program, w_parameter) {
				Some(w) => w != 1.0,
				None => false,
			}
		}
		// Either branch of a ternary can be the value written.
		besl::Nodes::Expression(besl::Expressions::Ternary { if_true, if_false, .. }) => {
			is_non_opaque_vec4f_constructor(program, if_true) || is_non_opaque_vec4f_constructor(program, if_false)
		}
		_ => false,
	}
}

fn is_vec3f_constructor(program: &besl::Program, node: &besl::NodeId) -> bool {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function = *function;
			let function_borrow = &program[function];
			function_borrow.get_name() == Some("vec3f") && parameters.len() == 3
		}
		_ => false,
	}
}

fn parse_literal_number(program: &besl::Program, node: &besl::NodeId) -> Option<f64> {
	let node_borrow = &program[node];
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse().ok(),
		besl::Nodes::Expression(besl::Expressions::Unary {
			operator: besl::UnaryOperators::Negate,
			operand,
		}) => parse_literal_number(program, operand).map(|value| -value),
		_ => None,
	}
}
