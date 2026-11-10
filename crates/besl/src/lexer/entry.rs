use super::*;
use crate::parser;

const INTERFACE_SYMBOL_PREFIX: &str = "_besl_interface_";
const OUTPUT_SYMBOL_PREFIX: &str = "_besl_output_";
const RAY_TRACING_ENTRY_DOCS_PATH: &str = "reference/besl/interface#ray-tracing-entry-points";
const RASTER_ENTRY_DOCS_PATH: &str = "reference/besl/interface#raster-entry-points";

/// The `StageInput` fields of ray tracing stages. Each reads the zero-argument intrinsic of the same name.
const RAY_TRACING_STAGE_INPUTS: [&str; 9] = [
	"launch_index",
	"launch_size",
	"hit_distance",
	"hit_custom_index",
	"hit_geometry_index",
	"hit_primitive_index",
	"hit_barycentrics",
	"ray_origin",
	"ray_direction",
];

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
		types,
	} = node.node_mut()
	else {
		return Ok(Vec::new());
	};
	if *name != "main" {
		return Ok(Vec::new());
	}
	// A `main` that implements another module's signature names that module's types.
	let types = match types {
		Some(path) => super::resolution::module_chain(std::slice::from_ref(root), path)
			.ok_or_else(|| entry_error("main implements a signature whose module isn't linked"))?,
		None => vec![root.clone()],
	};
	let types = types.as_slice();

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
	let payload = payload_parameter(params, return_type, types);
	if !has_structural_parameter && !has_structural_return {
		return Ok(Vec::new());
	}

	let mut declarations = Vec::new();
	let mut context = normalize_parameters(params, payload, types, &mut declarations)?;
	// A closest hit returns its payload as a plain value, which needs no output declarations.
	if payload.is_none() {
		context.return_fields = normalize_return_type(return_type, types, &mut declarations)?;
	}

	let old_statements = std::mem::take(statements);
	*statements = rewrite_statements(old_statements, &context, true)?;
	// A closest hit keeps its payload parameter and return type: they are its native signature on every backend. Layer
	// parameters stay until the program links, so `main` reaches their members; linking then erases them.
	let mut index = 0;
	params.retain(|parameter| {
		let keep = payload == Some(index) || is_layer_parameter(parameter, types);
		index += 1;
		keep
	});
	if payload.is_none() {
		*return_type = parser::TypeName::Named("void");
	}

	Ok(declarations)
}

/// Returns the index of the payload parameter when `main` is a closest hit: its only parameter with a named type other
/// than `StageInput` or a layer comes last, and `main` returns that type. Every other `main` has no payload.
fn payload_parameter(
	params: &[parser::Node<'_>],
	return_type: &parser::TypeName<'_>,
	types: &[NodeReference],
) -> Option<usize> {
	let mut values = params
		.iter()
		.enumerate()
		.filter(|(_, parameter)| !is_layer_parameter(parameter, types))
		.filter_map(|(index, parameter)| match parameter.node() {
			parser::Nodes::Parameter {
				r#type: parser::TypeName::Named(r#type),
				..
			} if *r#type != "StageInput" => Some((index, *r#type)),
			_ => None,
		});
	let (index, payload_type) = values.next()?;
	let is_payload = values.next().is_none()
		&& params[index + 1..]
			.iter()
			.all(|parameter| is_layer_parameter(parameter, types))
		&& matches!(return_type, parser::TypeName::Named(returned) if *returned == payload_type);
	is_payload.then_some(index)
}

/// Converts structural parameters into flat input declarations and their rewrite lookup tables.
fn normalize_parameters<'tree, 'source>(
	params: &'tree [parser::Node<'source>],
	payload: Option<usize>,
	types: &[NodeReference],
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
		// The payload stays a parameter. Lexing checks its type.
		if payload == Some(index) {
			continue;
		}
		if params[..index].iter().any(
			|previous| matches!(previous.node(), parser::Nodes::Parameter { name: previous_name, .. } if previous_name == name),
		) {
			return Err(entry_error("main contains duplicate parameter names"));
		}
		match r#type {
			// A layer names resources the shader binds; its parameter stays until linking erases it.
			parser::TypeName::Named(_) if is_layer_parameter(parameter, types) => {}
			parser::TypeName::Named("StageInput") => {
				if context.stage_input_parameter.replace(*name).is_some() {
					return Err(entry_error("main can declare only one StageInput parameter"));
				}
			}
			parser::TypeName::Record {
				role: parser::RecordRole::Interface,
				..
			} if payload.is_some() => {
				return Err(LexError::invalid(format!(
					"Closest-hit main declares interface parameter `{name}`. The most likely cause is a closest-hit shader written like a raster stage, but a closest hit reads what it hit from `StageInput`. Remove `{name}`, and read members such as `hit_barycentrics` from a `StageInput` parameter. See {docs}.",
					docs = crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH)
				)));
			}
			parser::TypeName::Record {
				role: parser::RecordRole::Interface,
				fields,
			} => {
				if context.interface_parameter.is_some() {
					return Err(entry_error("main can declare only one interface parameter"));
				}
				validate_unique_type_fields(fields, "interface")?;
				if let Some(field) = fields.iter().find(|field| is_builtin_interface_field(field.name)) {
					return Err(entry_error(&format!(
						"{} is a vertex interface output and cannot be an interface parameter",
						field.name
					)));
				}
				for field in fields {
					declarations.push(Node {
						node: Nodes::Input {
							name: interface_symbol(field.name),
							format: resolve_entry_field_type(types, &field.type_name)?,
							location: interface_location(fields, field.name, false)?,
							per_primitive: field.per_primitive,
						},
					});
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
	types: &[NodeReference],
	declarations: &mut Vec<Node>,
) -> Result<Option<(parser::RecordRole, &'tree [parser::TypeField<'source>])>, LexError> {
	let parser::TypeName::Record { role, fields } = return_type else {
		return match return_type {
			parser::TypeName::Named("void") => Ok(None),
			_ => Err(entry_error("Structural main must return interface, output, or void")),
		};
	};

	validate_unique_type_fields(fields, "entry-point return")?;
	if let Some(field) = fields.iter().find(|field| field.per_primitive) {
		return Err(LexError::invalid(format!(
			"main returns `{name}: primitive<{type_name}>`. The most likely cause is a `primitive<T>` field written in a stage's outputs, but `primitive<T>` marks a fragment input, which a mesh stage writes with an `output` array. Declare `{name}` as `{type_name}` here. See {docs}.",
			name = field.name,
			type_name = field.type_name,
			docs = crate::online_docs_url(RASTER_ENTRY_DOCS_PATH)
		)));
	}
	if *role == parser::RecordRole::Interface
		&& let Some(field) = fields
			.iter()
			.find(|field| field.name == crate::RENDER_TARGET_ARRAY_INDEX_FIELD)
		&& !matches!(field.type_name, parser::TypeName::Named("u32"))
	{
		return Err(LexError::invalid(format!(
			"main returns `{name}: {type_name}`. The most likely cause is a layer index declared with another type, but `{name}` picks the render target array layer, which every backend reads as a `u32`. Declare `{name}` as `u32`. See {docs}.",
			name = field.name,
			type_name = field.type_name,
			docs = crate::online_docs_url(RASTER_ENTRY_DOCS_PATH)
		)));
	}
	for (attachment, field) in fields.iter().enumerate() {
		let location = match role {
			// Builtin fields write native outputs, which take no varying location; the VM routes them by name.
			parser::RecordRole::Interface if is_builtin_interface_field(field.name) => 0,
			parser::RecordRole::Interface => interface_location(fields, field.name, true)?,
			parser::RecordRole::Output => {
				u8::try_from(attachment).map_err(|_| entry_error("An output record cannot contain more than 256 fields"))?
			}
		};
		let symbol = output_symbol(*role, field.name);
		declarations.push(Node::output(
			&symbol,
			resolve_entry_field_type(types, &field.type_name)?,
			location,
		));
	}
	Ok(Some((*role, fields.as_slice())))
}

fn resolve_entry_field_type(types: &[NodeReference], type_name: &parser::TypeName<'_>) -> Result<NodeReference, LexError> {
	if !matches!(type_name, parser::TypeName::Named(_)) {
		return Err(entry_error(
			"Structural fields must use named types; use dedicated declarations for arrays",
		));
	}
	super::resolution::resolve_type_name(types, type_name)
}

/// Assigns one interface location by name so declaration order cannot break stage linkage.
///
/// A vertex return passes `exclude_builtins`, so its builtin fields don't shift the locations of the varyings the next
/// stage declares without them.
fn interface_location(fields: &[parser::TypeField<'_>], name: &str, exclude_builtins: bool) -> Result<u8, LexError> {
	let location = fields
		.iter()
		.filter(|field| (!exclude_builtins || !is_builtin_interface_field(field.name)) && field.name < name)
		.count();
	u8::try_from(location).map_err(|_| entry_error("An interface cannot contain more than 256 fields"))
}

/// Reports whether a vertex interface field writes a native raster output instead of a varying.
fn is_builtin_interface_field(name: &str) -> bool {
	matches!(name, "position" | crate::RENDER_TARGET_ARRAY_INDEX_FIELD)
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
	if let Some(replacement) = contextual_access(node, context)? {
		*node = replacement;
		return Ok(());
	}

	match node.node_mut() {
		parser::Nodes::Conditional {
			condition,
			statements,
			else_branch,
		} => {
			rewrite_node(condition, context)?;
			*statements = rewrite_statements(std::mem::take(statements), context, false)?;
			match else_branch {
				Some(parser::ElseBranch::Block(statements)) => {
					*statements = rewrite_statements(std::mem::take(statements), context, false)?;
				}
				Some(parser::ElseBranch::If(conditional)) => rewrite_node(conditional, context)?,
				None => {}
			}
			Ok(())
		}
		parser::Nodes::Match { scrutinee, arms } => {
			rewrite_node(scrutinee, context)?;
			for arm in arms {
				arm.statements = rewrite_statements(std::mem::take(&mut arm.statements), context, false)?;
			}
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
		parser::Nodes::Const { value, .. } => rewrite_node(value, context),
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
		parser::Expressions::Unary { operand, .. } => rewrite_node(operand, context),
		parser::Expressions::Call { parameters, .. } => {
			for parameter in parameters {
				rewrite_node(parameter, context)?;
			}
			Ok(())
		}
		parser::Expressions::RecordLiteral { .. } => Err(entry_error(
			"Record literals are contextual values and can appear only directly after return",
		)),
		// A block's final value can read contextual parameters too, such as `if (c) { input.uv } else { … }`.
		parser::Expressions::Macro { body, .. }
		| parser::Expressions::Return { value: Some(body) }
		| parser::Expressions::Yield { value: body } => rewrite_node(body, context),
		parser::Expressions::Member { .. }
		| parser::Expressions::Literal { .. }
		| parser::Expressions::VariableDeclaration { .. }
		| parser::Expressions::Return { value: None }
		| parser::Expressions::Continue
		| parser::Expressions::Break
		| parser::Expressions::Discard => Ok(()),
	}
}

/// Resolves a direct `parameter.field` access before its component nodes enter normal name lookup.
fn contextual_access<'a>(
	node: &parser::Node<'a>,
	context: &EntryContext<'_, 'a>,
) -> Result<Option<parser::Node<'a>>, LexError> {
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
		// Compute fields reuse the intrinsic nodes that already carry backend and VM semantics.
		return match field.as_ref() {
			// Raster fields lower to their namespaced builtin inputs, so the field names stay free for the shader's own
			// variables.
			"vertex_index" => Ok(Some(parser::Node::member_expression(crate::VERTEX_INDEX_BUILTIN))),
			"instance_index" => Ok(Some(parser::Node::member_expression(crate::INSTANCE_INDEX_BUILTIN))),
			"primitive_index" => Ok(Some(parser::Node::member_expression(crate::PRIMITIVE_INDEX_BUILTIN))),
			"thread_id" => Ok(Some(parser::Node::call("thread_id", Vec::new()))),
			"thread_idx" => Ok(Some(parser::Node::call("thread_idx", Vec::new()))),
			"threadgroup_position" => Ok(Some(parser::Node::call("threadgroup_position", Vec::new()))),
			"subgroup_lane_index" => Ok(Some(parser::Node::call("subgroup_lane_index", Vec::new()))),
			// The tile field: the size of the tile stage's outputs.
			"extent" => Ok(Some(parser::Node::call("output_extent", Vec::new()))),
			field => match RAY_TRACING_STAGE_INPUTS.iter().find(|input| **input == field) {
				Some(input) => Ok(Some(parser::Node::call(input, Vec::new()))),
				None => Err(entry_error(&format!("StageInput does not define `{field}`"))),
			},
		};
	}
	let Some((interface_parameter, fields)) = &context.interface_parameter else {
		return Ok(None);
	};
	if *interface_parameter != parameter.as_ref() {
		return Ok(None);
	}
	if fields.iter().any(|declared| declared.name == field.as_ref()) {
		Ok(Some(parser::Node::member_expression(interface_symbol(field))))
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
		(parser::RecordRole::Interface, crate::RENDER_TARGET_ARRAY_INDEX_FIELD) => {
			crate::STRUCTURAL_RENDER_TARGET_ARRAY_INDEX_OUTPUT.to_string()
		}
		(parser::RecordRole::Interface, _) => interface_symbol(field_name),
		(parser::RecordRole::Output, _) => format!("{OUTPUT_SYMBOL_PREFIX}{field_name}"),
	}
}

fn entry_error(message: &str) -> LexError {
	LexError::invalid(format!(
		"Invalid structural entry point: {message}. The most likely cause is that main's declared record shape and its parameter or return value do not match."
	))
}

/// Reports whether `parameter` takes a layer, which `types` names and the program declares before `main`.
fn is_layer_parameter(parameter: &parser::Node<'_>, types: &[NodeReference]) -> bool {
	let parser::Nodes::Parameter {
		r#type: parser::TypeName::Named(r#type),
		..
	} = parameter.node()
	else {
		return false;
	};
	super::resolution::get_reference(types, r#type)
		.is_some_and(|declaration| matches!(declaration.borrow().node(), Nodes::Layer { .. }))
}
