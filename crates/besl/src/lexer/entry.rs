use super::*;
use crate::parser;

const INTERFACE_SYMBOL_PREFIX: &str = "_besl_interface_";
const OUTPUT_SYMBOL_PREFIX: &str = "_besl_output_";

/// The `EntryContext` struct keeps contextual source names available while structural entry syntax is erased.
struct EntryContext<'tree, 'source> {
	stage_input_parameter: Option<&'source str>,
	interface_parameter: Option<(&'source str, &'tree [parser::TypeField<'source>])>,
	return_fields: Option<(parser::RecordRole, &'tree [parser::TypeField<'source>])>,
}

/// Rewrites one structural `main` into the flat, parameterless ABI shared by the VM and backends.
pub(super) fn normalize_entry<'a>(node: &mut parser::Node<'a>, root: &NodeReference) -> Result<Vec<Node>, LexError> {
	let parser::Nodes::Function {
		name,
		params,
		return_type,
		statements,
		..
	} = node.node_mut()
	else {
		return Ok(Vec::new());
	};
	if *name != "main" {
		return Ok(Vec::new());
	}

	let has_structural_parameter = params.iter().any(|parameter| {
		matches!(
			parameter.node(),
			parser::Nodes::Parameter {
				r#type: parser::TypeName::Named("StageInput") | parser::TypeName::Record { .. },
				..
			}
		)
	});
	let has_structural_return = matches!(return_type, parser::TypeName::Record { .. });
	if !has_structural_parameter && !has_structural_return {
		return Ok(Vec::new());
	}

	let mut declarations = Vec::new();
	let mut context = normalize_parameters(params, root, &mut declarations)?;
	context.return_fields = normalize_return_type(return_type, root, &mut declarations)?;

	let old_statements = std::mem::take(statements);
	*statements = rewrite_statements(old_statements, &context, true)?;
	params.clear();
	*return_type = parser::TypeName::Named("void");

	Ok(declarations)
}

/// Converts structural parameters into flat input declarations and their rewrite lookup tables.
fn normalize_parameters<'tree, 'source>(
	params: &'tree [parser::Node<'source>],
	root: &NodeReference,
	declarations: &mut Vec<Node>,
) -> Result<EntryContext<'tree, 'source>, LexError> {
	let mut context = EntryContext {
		stage_input_parameter: None,
		interface_parameter: None,
		return_fields: None,
	};
	for (index, parameter) in params.iter().enumerate() {
		let parser::Nodes::Parameter { name, r#type } = parameter.node() else {
			return Err(entry_error("Entry-point parameters must be named values"));
		};
		if params[..index].iter().any(
			|previous| matches!(previous.node(), parser::Nodes::Parameter { name: previous_name, .. } if previous_name == name),
		) {
			return Err(entry_error("main contains duplicate parameter names"));
		}
		match r#type {
			parser::TypeName::Named("StageInput") => {
				if context.stage_input_parameter.replace(*name).is_some() {
					return Err(entry_error("main can declare only one StageInput parameter"));
				}
			}
			parser::TypeName::Record {
				role: parser::RecordRole::Interface,
				fields,
			} => {
				if context.interface_parameter.is_some() {
					return Err(entry_error("main can declare only one interface parameter"));
				}
				validate_unique_type_fields(fields, "interface")?;
				if fields.iter().any(|field| field.name == "position") {
					return Err(entry_error(
						"position is a vertex interface output and cannot be an interface parameter",
					));
				}
				for field in fields {
					let symbol = interface_symbol(field.name);
					declarations.push(Node::input(
						&symbol,
						resolve_entry_field_type(root, &field.type_name)?,
						interface_location(fields, field.name, false)?,
					));
				}
				context.interface_parameter = Some((*name, fields.as_slice()));
			}
			parser::TypeName::Record {
				role: parser::RecordRole::Output,
				..
			} => {
				return Err(entry_error(
					"output records can be returned from main but cannot be parameters",
				));
			}
			_ => {
				return Err(entry_error(
					"Structural main accepts only StageInput and interface parameters",
				));
			}
		}
	}
	Ok(context)
}

/// Converts one structural return type into flat output declarations and a return-value rewrite table.
fn normalize_return_type<'tree, 'source>(
	return_type: &'tree parser::TypeName<'source>,
	root: &NodeReference,
	declarations: &mut Vec<Node>,
) -> Result<Option<(parser::RecordRole, &'tree [parser::TypeField<'source>])>, LexError> {
	let parser::TypeName::Record { role, fields } = return_type else {
		return match return_type {
			parser::TypeName::Named("void") => Ok(None),
			_ => Err(entry_error("Structural main must return interface, output, or void")),
		};
	};

	validate_unique_type_fields(fields, "entry-point return")?;
	for (attachment, field) in fields.iter().enumerate() {
		let location = match role {
			parser::RecordRole::Interface if field.name == "position" => 0,
			parser::RecordRole::Interface => interface_location(fields, field.name, true)?,
			parser::RecordRole::Output => {
				u8::try_from(attachment).map_err(|_| entry_error("An output record cannot contain more than 256 fields"))?
			}
		};
		let symbol = output_symbol(*role, field.name);
		declarations.push(Node::output(
			&symbol,
			resolve_entry_field_type(root, &field.type_name)?,
			location,
		));
	}
	Ok(Some((*role, fields.as_slice())))
}

fn resolve_entry_field_type(root: &NodeReference, type_name: &parser::TypeName<'_>) -> Result<NodeReference, LexError> {
	if !matches!(type_name, parser::TypeName::Named(_)) {
		return Err(entry_error(
			"Structural fields must use named types; use dedicated declarations for arrays",
		));
	}
	super::resolution::resolve_type_name(std::slice::from_ref(root), type_name)
}

/// Assigns one interface location by name so declaration order cannot break stage linkage.
fn interface_location(fields: &[parser::TypeField<'_>], name: &str, exclude_position: bool) -> Result<u8, LexError> {
	let location = fields
		.iter()
		.filter(|field| (!exclude_position || field.name != "position") && field.name < name)
		.count();
	u8::try_from(location).map_err(|_| entry_error("An interface cannot contain more than 256 fields"))
}

fn validate_unique_type_fields(fields: &[parser::TypeField<'_>], context: &str) -> Result<(), LexError> {
	for (index, field) in fields.iter().enumerate() {
		if fields[..index].iter().any(|previous| previous.name == field.name) {
			return Err(entry_error(&format!("Duplicate field `{}` in {context}", field.name)));
		}
	}
	Ok(())
}

/// Expands contextual record returns into assignments to flat semantic outputs.
fn rewrite_statements<'a>(
	statements: Vec<parser::Node<'a>>,
	context: &EntryContext<'_, 'a>,
	allow_record_return: bool,
) -> Result<Vec<parser::Node<'a>>, LexError> {
	let mut rewritten = Vec::with_capacity(statements.len());
	for mut statement in statements {
		let record_fields = match statement.node_mut() {
			parser::Nodes::Expression(parser::Expressions::Return { value: Some(value) }) => match value.node_mut() {
				parser::Nodes::Expression(parser::Expressions::RecordLiteral { fields }) => Some(std::mem::take(fields)),
				_ => None,
			},
			_ => None,
		};

		if let Some(record_fields) = record_fields {
			if !allow_record_return {
				return Err(entry_error(
					"Record returns must be top-level main statements so every backend can finalize its stage output",
				));
			}
			let Some((return_role, expected_fields)) = context.return_fields else {
				return Err(entry_error("A record value can be returned only from a structural main"));
			};
			validate_record_literal(&record_fields, expected_fields)?;
			for field in record_fields {
				let mut value = field.value;
				rewrite_node(&mut value, context)?;
				rewritten.push(parser::Node::assignment(
					parser::Node::member_expression(output_symbol(return_role, field.name)),
					value,
				));
			}
			// A source return terminates main. Dropping later statements here preserves
			// that control flow after the contextual return value itself is erased.
			return Ok(rewritten);
		}

		match statement.node() {
			parser::Nodes::Expression(parser::Expressions::Return { value: Some(_) }) if context.return_fields.is_some() => {
				return Err(entry_error("A structural main must return a record literal"));
			}
			parser::Nodes::Expression(parser::Expressions::Return { value: None }) if context.return_fields.is_some() => {
				return Err(entry_error("A structural main cannot return without all declared fields"));
			}
			_ => {}
		}

		rewrite_node(&mut statement, context)?;
		rewritten.push(statement);
	}
	if allow_record_return && context.return_fields.is_some() {
		return Err(entry_error("A structural main return type requires a record return value"));
	}
	Ok(rewritten)
}

fn validate_record_literal(
	fields: &[parser::RecordField<'_>],
	expected_fields: &[parser::TypeField<'_>],
) -> Result<(), LexError> {
	for (index, field) in fields.iter().enumerate() {
		if !expected_fields.iter().any(|expected| expected.name == field.name) {
			return Err(entry_error(&format!(
				"Record return contains undeclared field `{}`",
				field.name
			)));
		}
		if fields[..index].iter().any(|previous| previous.name == field.name) {
			return Err(entry_error(&format!(
				"Record return contains duplicate field `{}`",
				field.name
			)));
		}
	}
	if fields.len() != expected_fields.len() {
		return Err(entry_error("Record return does not provide every declared field"));
	}
	Ok(())
}

/// Rewrites contextual parameter member access throughout one non-record-return statement.
fn rewrite_node<'a>(node: &mut parser::Node<'a>, context: &EntryContext<'_, 'a>) -> Result<(), LexError> {
	if let Some(replacement) = contextual_access_symbol(node, context)? {
		*node = parser::Node::member_expression(replacement);
		return Ok(());
	}

	match node.node_mut() {
		parser::Nodes::Conditional { condition, statements } => {
			rewrite_node(condition, context)?;
			*statements = rewrite_statements(std::mem::take(statements), context, false)?;
			Ok(())
		}
		parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			rewrite_node(initializer, context)?;
			rewrite_node(condition, context)?;
			rewrite_node(update, context)?;
			*statements = rewrite_statements(std::mem::take(statements), context, false)?;
			Ok(())
		}
		parser::Nodes::Expression(expression) => rewrite_expression(expression, context),
		parser::Nodes::Const { value, .. } | parser::Nodes::Literal { body: value, .. } => rewrite_node(value, context),
		_ => Ok(()),
	}
}

fn rewrite_expression<'a>(expression: &mut parser::Expressions<'a>, context: &EntryContext<'_, 'a>) -> Result<(), LexError> {
	match expression {
		parser::Expressions::Expression(elements) => {
			for element in elements {
				rewrite_node(element, context)?;
			}
			Ok(())
		}
		parser::Expressions::Accessor { left, right } | parser::Expressions::Operator { left, right, .. } => {
			rewrite_node(left, context)?;
			rewrite_node(right, context)
		}
		parser::Expressions::Call { parameters, .. } => {
			for parameter in parameters {
				rewrite_node(parameter, context)?;
			}
			Ok(())
		}
		parser::Expressions::RecordLiteral { .. } => Err(entry_error(
			"Record literals are contextual values and can appear only directly after return",
		)),
		parser::Expressions::Macro { body, .. } | parser::Expressions::Return { value: Some(body) } => {
			rewrite_node(body, context)
		}
		_ => Ok(()),
	}
}

/// Resolves a direct `parameter.field` access before its component nodes enter normal name lookup.
fn contextual_access_symbol<'a>(node: &parser::Node<'a>, context: &EntryContext<'_, 'a>) -> Result<Option<String>, LexError> {
	let parser::Nodes::Expression(parser::Expressions::Accessor { left, right }) = node.node() else {
		return Ok(None);
	};
	let parser::Nodes::Expression(parser::Expressions::Member { name: parameter }) = left.node() else {
		return Ok(None);
	};
	let parser::Nodes::Expression(parser::Expressions::Member { name: field }) = right.node() else {
		return Ok(None);
	};
	if context.stage_input_parameter == Some(parameter.as_ref()) {
		return match field.as_ref() {
			crate::VERTEX_INDEX_BUILTIN | crate::INSTANCE_INDEX_BUILTIN => Ok(Some(field.to_string())),
			field => Err(entry_error(&format!("StageInput does not define `{field}`"))),
		};
	}
	let Some((interface_parameter, fields)) = &context.interface_parameter else {
		return Ok(None);
	};
	if *interface_parameter != parameter.as_ref() {
		return Ok(None);
	}
	if fields.iter().any(|declared| declared.name == field.as_ref()) {
		Ok(Some(interface_symbol(field)))
	} else {
		Err(entry_error(&format!(
			"Interface parameter `{parameter}` does not define `{field}`"
		)))
	}
}

fn interface_symbol(field_name: &str) -> String {
	format!("{INTERFACE_SYMBOL_PREFIX}{field_name}")
}

fn output_symbol(role: parser::RecordRole, field_name: &str) -> String {
	match (role, field_name) {
		(parser::RecordRole::Interface, "position") => crate::STRUCTURAL_POSITION_OUTPUT.to_string(),
		(parser::RecordRole::Interface, _) => interface_symbol(field_name),
		(parser::RecordRole::Output, _) => format!("{OUTPUT_SYMBOL_PREFIX}{field_name}"),
	}
}

fn entry_error(message: &str) -> LexError {
	LexError::Undefined {
		message: Some(format!(
			"Invalid structural entry point: {message}. The most likely cause is that main's declared record shape and its parameter or return value do not match."
		)),
	}
}
