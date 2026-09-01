use super::*;

pub(crate) fn parse_var_decl<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("let")?;
	let variable_name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let variable_type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find a type for variable {}", variable_name),
		},
		_ => e,
	})?;
	let (variable_type, iterator) = parse_type_name(iterator, variable_type)?;

	expressions.push(Atoms::VariableDeclaration {
		name: variable_name,
		r#type: variable_type,
	});

	let possible_following_expressions: Vec<ExpressionParser<'i, 'a>> = vec![parse_operator];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, expressions)?;

	Ok(expressions)
}
pub(crate) fn parse_keywords<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("return")?;

	expressions.push(Atoms::Keyword);

	if **iterator
		.clone()
		.peekable()
		.peek()
		.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
		== ";"
	{
		return Ok((expressions, iterator));
	}

	try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

pub(crate) fn parse_continue<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("continue")?;
	expressions.push(Atoms::Continue);
	Ok((expressions, iterator))
}

pub(crate) fn parse_discard<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("discard")?;
	expressions.push(Atoms::Discard);
	Ok((expressions, iterator))
}

pub(crate) fn parse_variable<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	expressions.push(Atoms::Member { name });

	let lexers = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

pub(crate) fn parse_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str(".")?;

	expressions.push(Atoms::Accessor);

	let lexers: Vec<ExpressionParser<'i, 'a>> = vec![parse_variable];

	execute_expression_parsers(&lexers, iterator, expressions)
}

pub(crate) fn parse_index_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str("[")?;
	expressions.push(Atoms::Accessor);
	let (inner_expressions, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	expressions.push(Atoms::GroupedExpression(inner_expressions));
	iterator.next_str("]")?;

	let lexers = vec![parse_operator, parse_accessor, parse_index_accessor];
	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

pub(crate) fn is_literal(s: &str) -> bool {
	matches!(s, "true" | "false") || s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

pub(crate) fn parse_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let value = iterator.next_is(is_literal)?;

	expressions.push(Atoms::Literal { value });

	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

/// Parses a parenthesized sub-expression like `(a + b)`.
pub(crate) fn parse_grouped_expression<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("(")?;

	// Parse the inner expression
	let (inner_expressions, mut inner_iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;

	inner_iterator.next_str(")").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected closing ')' for grouped expression".to_string(),
	})?;

	// Keep grouped expressions intact so later lowering can preserve precedence.
	expressions.push(Atoms::GroupedExpression(inner_expressions));

	// Check for following expressions (operators, accessors, etc.)
	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, inner_iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, inner_iterator)))
}

/// Parses an anonymous record value with named or shorthand fields.
pub(crate) fn parse_record_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("{")?;
	let invalid = || ParsingFailReasons::BadSyntax {
		message: "Invalid record literal. The most likely cause is a missing field value, comma, or closing }.".to_string(),
	};
	let mut fields = Vec::new();
	loop {
		if iterator.clone().next().copied() == Some("}") {
			iterator.next();
			break;
		}
		let field_name = iterator.next_identifier().map_err(|_| invalid())?;
		let value = if iterator.clone().next().copied() == Some(":") {
			iterator.next();
			let (value, next_iterator) =
				execute_expression_parsers(&[parse_rvalue], iterator, Vec::new()).map_err(|_| invalid())?;
			iterator = next_iterator;
			Some(value)
		} else {
			None
		};
		fields.push(AtomRecordField { name: field_name, value });
		match iterator.clone().next().copied() {
			Some(",") => {
				iterator.next();
			}
			Some("}") => {
				iterator.next();
				break;
			}
			_ => return Err(invalid()),
		}
	}

	expressions.push(Atoms::RecordLiteral { fields });
	Ok((expressions, iterator))
}

pub(crate) fn parse_rvalue<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let parsers = vec![
		parse_record_literal,
		parse_function_call,
		parse_grouped_expression,
		parse_literal,
		parse_variable,
	];

	execute_expression_parsers(&parsers, iterator.clone(), expressions)
}

pub(crate) fn parse_operator<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let operator =
		iterator.next_is(|v| {
			v == "*"
				|| v == "+" || v == "-"
				|| v == "/" || v == "%"
				|| v == "=" || v == "<"
				|| v == ">" || v == "=="
				|| v == "!=" || v == "<="
				|| v == ">=" || v == "&&"
				|| v == "||" || v == "<<"
				|| v == ">>" || v == "&"
				|| v == "|"
		})?;

	expressions.push(Atoms::Operator { name: operator });

	let possible_following_expressions: Vec<ExpressionParser<'i, 'a>> = vec![parse_rvalue];

	execute_expression_parsers(&possible_following_expressions, iterator, expressions)
}

pub(crate) fn expression_atoms_to_node<'a>(atoms: &[Atoms<'a>]) -> Node<'a> {
	if matches!(atoms.first(), Some(Atoms::Keyword)) {
		return Node {
			node: Nodes::Expression(Expressions::Return {
				value: atoms
					.get(1..)
					.filter(|remaining| !remaining.is_empty())
					.map(|remaining| Box::new(expression_atoms_to_node(remaining))),
			}),
		};
	}

	if matches!(atoms.first(), Some(Atoms::Continue)) {
		return Node {
			node: Nodes::Expression(Expressions::Continue),
		};
	}
	if matches!(atoms.first(), Some(Atoms::Discard)) {
		return Node {
			node: Nodes::Expression(Expressions::Discard),
		};
	}

	let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

	if let Some((i, e)) = max_precedence_item {
		match e {
			Atoms::Keyword => Node {
				node: Nodes::Expression(Expressions::Return { value: None }),
			},
			Atoms::Continue => Node {
				node: Nodes::Expression(Expressions::Continue),
			},
			Atoms::Discard => Node {
				node: Nodes::Expression(Expressions::Discard),
			},
			Atoms::Operator { name } => {
				let left = expression_atoms_to_node(&atoms[..i]);
				let right = expression_atoms_to_node(&atoms[i + 1..]);

				Node {
					node: Nodes::Expression(Expressions::Operator {
						name,
						left: Box::new(left),
						right: Box::new(right),
					}),
				}
			}
			Atoms::Accessor => {
				let left = expression_atoms_to_node(&atoms[..i]);
				let right = expression_atoms_to_node(&atoms[i + 1..]);

				Node {
					node: Nodes::Expression(Expressions::Accessor {
						left: Box::new(left),
						right: Box::new(right),
					}),
				}
			}
			Atoms::GroupedExpression(inner) => Node::sentence(vec![expression_atoms_to_node(inner)]),
			Atoms::FunctionCall { name, parameters } => {
				let parameters = parameters.iter().map(|v| expression_atoms_to_node(v)).collect::<Vec<_>>();

				Node {
					node: Nodes::Expression(Expressions::Call {
						name: name.clone(),
						parameters,
					}),
				}
			}
			Atoms::Literal { value } => Node {
				node: Nodes::Expression(Expressions::Literal { value: (*value).into() }),
			},
			Atoms::RecordLiteral { fields } => Node::record_literal(
				fields
					.iter()
					.map(|field| RecordField {
						name: field.name,
						value: field
							.value
							.as_deref()
							.map_or_else(|| Node::member_expression(field.name), expression_atoms_to_node),
					})
					.collect(),
			),
			Atoms::Member { name } => Node {
				node: Nodes::Expression(Expressions::Member { name: (*name).into() }),
			},
			Atoms::VariableDeclaration { name, r#type } => Node {
				node: Nodes::Expression(Expressions::VariableDeclaration {
					name: (*name).into(),
					r#type: r#type.clone(),
				}),
			},
		}
	} else {
		panic!("No max precedence item");
	}
}

pub(crate) fn parse_conditional<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("if")?;
	iterator.next_str("(")?;

	let (condition_atoms, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	let condition = expression_atoms_to_node(&condition_atoms);

	iterator.next_str(")")?;
	iterator.next_str("{")?;

	let mut statements = vec![];
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== "}"
		{
			iterator.next();
			break;
		}

		let (statement, new_iterator) = parse_statement(iterator)?;
		statements.push(statement);
		iterator = new_iterator;
	}

	Ok((Node::conditional(condition, statements), iterator))
}

pub(crate) fn parse_for_loop<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("for")?;
	iterator.next_str("(")?;

	let statement_parsers = vec![
		parse_keywords,
		parse_continue,
		parse_discard,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];
	let (initializer_atoms, mut iterator) = execute_expression_parsers(&statement_parsers, iterator, Vec::new())?;
	let initializer = expression_atoms_to_node(&initializer_atoms);

	iterator.next_str(";")?;

	let (condition_atoms, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	let condition = expression_atoms_to_node(&condition_atoms);

	iterator.next_str(";")?;

	let (update_atoms, mut iterator) = execute_expression_parsers(&statement_parsers, iterator, Vec::new())?;
	let update = expression_atoms_to_node(&update_atoms);

	iterator.next_str(")")?;
	iterator.next_str("{")?;

	let mut statements = vec![];
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== "}"
		{
			iterator.next();
			break;
		}

		let (statement, new_iterator) = parse_statement(iterator)?;
		statements.push(statement);
		iterator = new_iterator;
	}

	Ok((Node::for_loop(initializer, condition, update, statements), iterator))
}

pub(crate) fn parse_function_call<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let function_name = iterator.next_identifier()?;
	let (function_name, mut iterator) = parse_type_name(iterator, function_name)?;
	iterator.next_str("(")?;

	let mut parameters = vec![];

	loop {
		let iter_before = iterator.clone();

		if let Some(a) = try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), Vec::new()) {
			let (expressions, new_iterator) = a?;
			parameters.push(expressions);
			iterator = new_iterator;
		}

		// Check if iter is comma
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}

		// check if iter is close brace
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		// Safety: if no progress was made, break to avoid infinite loop
		if iterator.len() == iter_before.len() {
			let token = iterator.clone().peekable().peek().copied().copied().unwrap_or("<eof>");
			return Err(ParsingFailReasons::BadSyntax {
				message: format!("Unexpected token '{}' in function call {}", token, function_name),
			});
		}
	}

	expressions.push(Atoms::FunctionCall {
		name: function_name,
		parameters,
	});

	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

pub(crate) fn parse_statement<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	if let Some(result) = try_execute_parsers(&[parse_conditional], iterator.clone()) {
		return result;
	}

	if let Some(result) = try_execute_parsers(&[parse_for_loop], iterator.clone()) {
		return result;
	}

	let parsers = vec![
		parse_keywords,
		parse_continue,
		parse_discard,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];

	let (expressions, mut iterator) = execute_expression_parsers(&parsers, iterator, Vec::new())?;

	iterator.next_str(";")?; // Skip semicolon

	Ok((expression_atoms_to_node(&expressions), iterator))
}

pub(crate) fn parse_function<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	iterator.next_str(":")?;
	iterator.next_str("fn")?;
	iterator.next_str("(")?;

	let mut params = Vec::new();
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		let param_name = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected a parameter name for function {}.", name),
			},
			_ => e,
		})?;
		iterator.next_str(":")?;
		let param_type = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected a parameter type for function {}.", name),
			},
			_ => e,
		})?;
		let (param_type, next_iterator) = parse_type_name(iterator, param_type)?;
		params.push(Node::parameter(param_name, param_type));
		iterator = next_iterator;

		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}
	}
	iterator.next_str("->")?;

	let return_type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected a return type for function {} declaration.", name),
		},
		_ => e,
	})?;
	let (return_type, mut iterator) = parse_type_name(iterator, return_type)?;

	iterator.next_str("{").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected a {{ after function {} declaration.", name),
		},
		_ => e,
	})?;

	let mut statements = vec![];

	loop {
		if let Some(Ok((expression, new_iterator))) = try_execute_parsers(&[parse_statement], iterator.clone()) {
			iterator = new_iterator;

			statements.push(expression);
		} else {
			// A failed statement parser at EOF means the function body was truncated.
			let Some(token) = iterator.clone().next().copied() else {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!(
						"Function `{}` is missing a closing `}}`. The source most likely ended before the function body was complete.",
						name
					),
				});
			};

			if token == "}" {
				iterator.next();
				break;
			} else {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!("Expected a }} after function {} declaration, found `{}`.", name, token),
				});
			}
		}

		// check if iter is close brace
		if **iterator.clone().peekable().peek().ok_or(ParsingFailReasons::BadSyntax {
			message: "Expected a '}' after function body".to_string(),
		})? == "}"
		{
			iterator.next();
			break;
		}
	}

	let node = Node::function(name, params, return_type, statements);

	Ok((node, iterator))
}
