use super::*;

/// Parses the optional operator, ternary, member access, or index after an operand, as in `a + b`, `a ? b : c`,
/// `a.b`, or `a[i]`. Without one, it returns `iterator` unchanged.
fn parse_followers<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> std::slice::Iter<'i, &'a str> {
	let followers: [ExpressionParser<'i, 'a>; 4] = [parse_operator, parse_ternary, parse_accessor, parse_index_accessor];
	try_expression_parsers(&followers, &iterator, expressions).unwrap_or(iterator)
}

pub(crate) fn parse_var_decl<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("let")?;
	let variable_name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let variable_type = iterator
		.next_identifier()
		.map_err(|error| error.claimed(|| format!("Expected to find a type for variable {variable_name}")))?;
	let (variable_type, iterator) = parse_type_name(iterator, variable_type)?;

	expressions.push(Atoms::VariableDeclaration {
		name: variable_name,
		r#type: variable_type,
	});

	execute_expression_parsers(&[parse_operator], iterator, expressions)
}

pub(crate) fn parse_keywords<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("return")?;

	expressions.push(Atoms::Keyword);

	if *iterator
		.as_slice()
		.first()
		.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
		== ";"
	{
		return Ok(iterator);
	}

	Ok(try_expression_parsers(&[parse_rvalue], &iterator, expressions).unwrap_or(iterator))
}

pub(crate) fn parse_continue<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("continue")?;
	expressions.push(Atoms::Continue);
	Ok(iterator)
}

pub(crate) fn parse_break<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("break")?;
	expressions.push(Atoms::Break);
	Ok(iterator)
}

pub(crate) fn parse_discard<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("discard")?;
	expressions.push(Atoms::Discard);
	Ok(iterator)
}

pub(crate) fn parse_variable<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	expressions.push(Atoms::Member { name });

	Ok(parse_followers(iterator, expressions))
}

pub(crate) fn parse_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str(".")?;

	expressions.push(Atoms::Accessor);

	execute_expression_parsers(&[parse_variable], iterator, expressions)
}

pub(crate) fn parse_index_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str("[")?;
	expressions.push(Atoms::Accessor);
	let mut inner_expressions = Vec::new();
	let mut iterator = execute_expression_parsers(&[parse_rvalue], iterator, &mut inner_expressions)?;
	expressions.push(Atoms::GroupedExpression(inner_expressions));
	iterator.next_str("]")?;

	Ok(parse_followers(iterator, expressions))
}

pub(crate) fn is_literal(s: &str) -> bool {
	matches!(s, "true" | "false") || s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

pub(crate) fn parse_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let value = iterator.next_is(is_literal)?;

	expressions.push(Atoms::Literal { value });

	Ok(parse_followers(iterator, expressions))
}

/// Parses a parenthesized sub-expression like `(a + b)`.
pub(crate) fn parse_grouped_expression<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("(")?;

	let mut inner_expressions = Vec::new();
	let mut inner_iterator = execute_expression_parsers(&[parse_rvalue], iterator, &mut inner_expressions)?;

	inner_iterator.next_str(")").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected closing ')' for grouped expression".to_string(),
	})?;

	// Keep grouped expressions intact so later lowering can preserve precedence.
	expressions.push(Atoms::GroupedExpression(inner_expressions));

	Ok(parse_followers(inner_iterator, expressions))
}

/// Parses an anonymous record value with named or shorthand fields.
pub(crate) fn parse_record_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
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
			let mut value = Vec::new();
			iterator = execute_expression_parsers(&[parse_rvalue], iterator, &mut value).map_err(|_| invalid())?;
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
	Ok(iterator)
}

pub(crate) fn parse_rvalue<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	execute_expression_parsers(
		&[
			parse_unary,
			parse_record_literal,
			parse_function_call,
			parse_grouped_expression,
			parse_literal,
			parse_variable,
		],
		iterator,
		expressions,
	)
}

pub(crate) fn parse_operator<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let token = iterator.next().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
	let operator = crate::Operators::from_token(token).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Operator { operator });

	execute_expression_parsers(&[parse_rvalue], iterator, expressions)
}

/// Parses a prefix operator and the operand it applies to, such as `-x`, `!flag`, or `~mask`.
pub(crate) fn parse_unary<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let token = iterator.next().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
	let operator = crate::UnaryOperators::from_token(token).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Unary { operator });

	execute_expression_parsers(&[parse_rvalue], iterator, expressions)
		.map_err(|error| error.claimed(|| format!("Expected a value after the prefix operator `{token}`.")))
}

/// Parses the `? if_true : if_false` that follows a ternary's condition. The true branch is parsed on its own, like a
/// parenthesized expression, because `?` and `:` enclose it. The condition is whatever precedes `?` and the false
/// branch whatever follows `:`, so [`expression_atoms_to_node`] decides how far each reaches by precedence.
pub(crate) fn parse_ternary<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("?")?;

	let mut if_true = Vec::new();
	let mut iterator = execute_expression_parsers(&[parse_rvalue], iterator, &mut if_true)
		.map_err(|error| error.claimed(|| "Expected a value after `?` in a ternary expression.".to_string()))?;
	iterator.next_str(":").map_err(|_| {
		ParsingFailReasons::BadSyntax {
		message: "Expected `:` after the true branch of a ternary expression. The most likely cause is a missing `:` or false branch."
			.to_string(),
	}
	})?;

	expressions.push(Atoms::Ternary { if_true });

	execute_expression_parsers(&[parse_rvalue], iterator, expressions)
		.map_err(|error| error.claimed(|| "Expected a value after `:` in a ternary expression.".to_string()))
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

	// The expression splits at its loosest atom. Binary operators group left to right, so a tie splits at the last of
	// them. Prefix operators and the ternary group right to left, so `- -x` is `-(-x)` and `a ? b : c ? d : e` is
	// `a ? b : (c ? d : e)`, and a tie splits at the first.
	let precedence = |atom: &Atoms<'a>| match atom {
		Atoms::Accessor => 1,
		Atoms::Unary { .. } => crate::lexer::UNARY_PRECEDENCE,
		Atoms::Ternary { .. } => crate::lexer::TERNARY_PRECEDENCE,
		Atoms::Operator { operator } => operator.precedence(),
		_ => 0,
	};
	let Some(loosest) = atoms.iter().map(precedence).max() else {
		panic!("No max precedence item");
	};
	let groups_right_to_left = loosest == crate::lexer::UNARY_PRECEDENCE || loosest == crate::lexer::TERNARY_PRECEDENCE;
	let split = if groups_right_to_left {
		atoms.iter().position(|atom| precedence(atom) == loosest)
	} else {
		atoms.iter().rposition(|atom| precedence(atom) == loosest)
	};
	let i = split.expect("The loosest atom exists");
	let atom = &atoms[i];

	match atom {
		Atoms::Keyword => Node {
			node: Nodes::Expression(Expressions::Return { value: None }),
		},
		Atoms::Continue => Node {
			node: Nodes::Expression(Expressions::Continue),
		},
		Atoms::Break => Node {
			node: Nodes::Expression(Expressions::Break),
		},
		Atoms::Discard => Node {
			node: Nodes::Expression(Expressions::Discard),
		},
		Atoms::Operator { operator } => Node {
			node: Nodes::Expression(Expressions::Operator {
				operator: *operator,
				left: Box::new(expression_atoms_to_node(&atoms[..i])),
				right: Box::new(expression_atoms_to_node(&atoms[i + 1..])),
			}),
		},
		Atoms::Unary { operator } => {
			// The parser reads a prefix operator only where an operand starts, so it is the first atom of its operand.
			debug_assert_eq!(i, 0, "A prefix operator must start the expression it applies to.");
			Node {
				node: Nodes::Expression(Expressions::Unary {
					operator: *operator,
					operand: Box::new(expression_atoms_to_node(&atoms[i + 1..])),
				}),
			}
		}
		Atoms::Ternary { if_true } => Node {
			node: Nodes::Expression(Expressions::Ternary {
				condition: Box::new(expression_atoms_to_node(&atoms[..i])),
				if_true: Box::new(expression_atoms_to_node(if_true)),
				if_false: Box::new(expression_atoms_to_node(&atoms[i + 1..])),
			}),
		},
		Atoms::Accessor => Node::accessor(
			expression_atoms_to_node(&atoms[..i]),
			expression_atoms_to_node(&atoms[i + 1..]),
		),
		Atoms::GroupedExpression(inner) => Node::sentence(vec![expression_atoms_to_node(inner)]),
		Atoms::FunctionCall { name, parameters } => Node {
			node: Nodes::Expression(Expressions::Call {
				name: name.clone(),
				parameters: parameters.iter().map(|v| expression_atoms_to_node(v)).collect(),
			}),
		},
		Atoms::Literal { value } => Node::literal_expression(*value),
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
		Atoms::Member { name } => Node::member_expression(*name),
		Atoms::VariableDeclaration { name, r#type } => Node {
			node: Nodes::Expression(Expressions::VariableDeclaration {
				name: (*name).into(),
				r#type: r#type.clone(),
			}),
		},
	}
}

pub(crate) fn parse_conditional<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("if")?;
	iterator.next_str("(")?;

	let (condition, mut iterator) = parse_expression_node(&[parse_rvalue], iterator)?;

	iterator.next_str(")")?;

	let (statements, mut iterator) = parse_block(iterator)?;

	let (else_branch, iterator) = match iterator.as_slice() {
		["else", "if", ..] => {
			iterator.next();
			let (conditional, iterator) = parse_conditional(iterator)?;
			(Some(ElseBranch::If(Box::new(conditional))), iterator)
		}
		["else", ..] => {
			iterator.next();
			let (statements, iterator) = parse_block(iterator)?;
			(Some(ElseBranch::Block(statements)), iterator)
		}
		_ => (None, iterator),
	};

	Ok((Node::conditional(condition, statements, else_branch), iterator))
}

/// Parses a braced statement block, such as the body of an `if` or `else` branch.
fn parse_block<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
) -> Result<(Vec<Node<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	iterator.next_str("{")?;

	let mut statements = vec![];
	loop {
		if *iterator
			.as_slice()
			.first()
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

	Ok((statements, iterator))
}

pub(crate) fn parse_for_loop<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("for")?;
	iterator.next_str("(")?;

	let statement_parsers = statement_expression_parsers();
	let (initializer, mut iterator) = parse_expression_node(&statement_parsers, iterator)?;

	iterator.next_str(";")?;

	let (condition, mut iterator) = parse_expression_node(&[parse_rvalue], iterator)?;

	iterator.next_str(";")?;

	let (update, mut iterator) = parse_expression_node(&statement_parsers, iterator)?;

	iterator.next_str(")")?;

	let (statements, iterator) = parse_block(iterator)?;

	Ok((Node::for_loop(initializer, condition, update, statements), iterator))
}

pub(crate) fn parse_function_call<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let function_name = iterator.next_identifier()?;
	let (function_name, mut iterator) = parse_type_name(iterator, function_name)?;
	iterator.next_str("(")?;

	let mut parameters = vec![];

	loop {
		let iter_before = iterator.clone();

		let mut parameter = Vec::new();
		if let Some(new_iterator) = try_expression_parsers(&[parse_rvalue], &iterator, &mut parameter) {
			parameters.push(parameter);
			iterator = new_iterator;
		}

		// Check if iter is comma
		if *iterator
			.as_slice()
			.first()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}

		// check if iter is close brace
		if *iterator
			.as_slice()
			.first()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		// Safety: if no progress was made, break to avoid infinite loop
		if iterator.len() == iter_before.len() {
			let token = iterator.as_slice().first().copied().unwrap_or("<eof>");
			return Err(ParsingFailReasons::BadSyntax {
				message: format!("Unexpected token '{}' in function call {}", token, function_name),
			});
		}
	}

	expressions.push(Atoms::FunctionCall {
		name: function_name,
		parameters,
	});

	Ok(parse_followers(iterator, expressions))
}

/// Lists the parsers for expressions that can stand alone as a statement, such as assignments and calls.
fn statement_expression_parsers<'i, 'a: 'i>() -> [ExpressionParser<'i, 'a>; 7] {
	[
		parse_keywords,
		parse_continue,
		parse_break,
		parse_discard,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	]
}

pub(crate) fn parse_statement<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	// `match` is a keyword, so syntax errors inside a match are reported instead of retried as an expression.
	if iterator.as_slice().first() == Some(&"match") {
		return parse_match(iterator);
	}

	if let Ok(result) = parse_conditional(iterator.clone()) {
		return Ok(result);
	}

	if let Ok(result) = parse_for_loop(iterator.clone()) {
		return Ok(result);
	}

	let (statement, mut iterator) = parse_expression_node(&statement_expression_parsers(), iterator)?;

	iterator.next_str(";")?; // Skip semicolon

	Ok((statement, iterator))
}

/// Parses a Rust-style `match` statement, such as `match n { 0 => a = 1, 1 | 2 => { a = 2; } _ => {} }`.
/// See [`crate::lexer`] for the type and exhaustiveness checks that follow parsing.
pub(crate) fn parse_match<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("match")?;

	let (scrutinee, mut iterator) = parse_expression_node(&[parse_rvalue], iterator)?;

	iterator.next_str("{").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected `{` after the match scrutinee. The most likely cause is a missing brace before the match arms."
			.to_string(),
	})?;

	let mut arms = Vec::new();
	loop {
		if iterator.as_slice().first() == Some(&"}") {
			iterator.next();
			break;
		}

		let (patterns, next_iterator) = parse_match_patterns(iterator)?;
		iterator = next_iterator;

		iterator.next_str("=>").map_err(|_| ParsingFailReasons::BadSyntax {
			message: "Expected `=>` after a match pattern. The most likely cause is a match guard or a pattern BESL can't match, such as a range or a binding."
				.to_string(),
		})?;

		let (statements, next_iterator) = parse_match_arm_body(iterator)?;
		iterator = next_iterator;
		arms.push(MatchArm { patterns, statements });
	}

	Ok((Node::r#match(scrutinee, arms), iterator))
}

/// Parses the or-pattern of one match arm, such as `1 | 2`. A leading `|` is allowed, as in Rust.
fn parse_match_patterns<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
) -> Result<(Vec<MatchPattern<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	if iterator.as_slice().first() == Some(&"|") {
		iterator.next();
	}

	let mut patterns = Vec::new();
	loop {
		let pattern = match iterator.as_slice() {
			["_", ..] => MatchPattern::Wildcard,
			["-", value, ..] if is_literal(value) => {
				iterator.next();
				MatchPattern::Literal { value, negative: true }
			}
			[value, ..] if is_literal(value) => MatchPattern::Literal { value, negative: false },
			[token, ..] => {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!(
						"Unsupported match pattern `{token}`. The most likely cause is a pattern other than a literal, `_`, or an or-pattern of those."
					),
				});
			}
			[] => return Err(ParsingFailReasons::StreamEndedPrematurely),
		};
		iterator.next();
		patterns.push(pattern);

		if iterator.as_slice().first() != Some(&"|") {
			return Ok((patterns, iterator));
		}
		iterator.next();
	}
}

/// Parses the body of one match arm and the comma after it. A block or nested `match` body doesn't need a comma,
/// and neither does the last arm, as in Rust.
fn parse_match_arm_body<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
) -> Result<(Vec<Node<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	let (statements, block_like, mut iterator) = match iterator.as_slice().first() {
		Some(&"{") => {
			let (statements, iterator) = parse_block(iterator)?;
			(statements, true, iterator)
		}
		Some(&"match") => {
			let (statement, iterator) = parse_match(iterator)?;
			(vec![statement], true, iterator)
		}
		_ => {
			let (statement, iterator) = parse_expression_node(&statement_expression_parsers(), iterator)?;
			(vec![statement], false, iterator)
		}
	};

	match iterator.as_slice().first() {
		Some(&",") => {
			iterator.next();
		}
		Some(&"}") => {}
		_ if block_like => {}
		_ => {
			return Err(ParsingFailReasons::BadSyntax {
				message: "Expected `,` after a match arm. The most likely cause is a missing comma between two arms."
					.to_string(),
			});
		}
	}

	Ok((statements, iterator))
}

pub(crate) fn parse_function<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	iterator.next_str(":")?;
	iterator.next_str("fn")?;
	iterator.next_str("(")?;

	let mut params = Vec::new();
	loop {
		if *iterator
			.as_slice()
			.first()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		let param_name = iterator
			.next_identifier()
			.map_err(|error| error.claimed(|| format!("Expected a parameter name for function {name}.")))?;
		iterator.next_str(":")?;
		let param_type = iterator
			.next_identifier()
			.map_err(|error| error.claimed(|| format!("Expected a parameter type for function {name}.")))?;
		let (param_type, next_iterator) = parse_type_name(iterator, param_type)?;
		params.push(Node::parameter(param_name, param_type));
		iterator = next_iterator;

		if *iterator
			.as_slice()
			.first()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}
	}
	iterator.next_str("->")?;

	let return_type = iterator
		.next_identifier()
		.map_err(|error| error.claimed(|| format!("Expected a return type for function {name} declaration.")))?;
	let (return_type, mut iterator) = parse_type_name(iterator, return_type)?;

	iterator
		.next_str("{")
		.map_err(|error| error.claimed(|| format!("Expected a {{ after function {name} declaration.")))?;

	let mut statements = vec![];

	loop {
		if let Ok((expression, new_iterator)) = parse_statement(iterator.clone()) {
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
		if *iterator.as_slice().first().ok_or_else(|| ParsingFailReasons::BadSyntax {
			message: "Expected a '}' after function body".to_string(),
		})? == "}"
		{
			iterator.next();
			break;
		}
	}

	Ok((Node::function(name, params, return_type, statements), iterator))
}
