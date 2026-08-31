use super::*;

pub(crate) fn parse_const<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("const")?;

	let r#type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find a type for const {}.", name),
		},
		_ => e,
	})?;
	let (r#type, mut iterator) = parse_type_name(iterator, r#type)?;

	iterator.next_str("=").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find = after type for const {}.", name),
		},
		_ => e,
	})?;

	let parsers = vec![parse_function_call, parse_literal, parse_variable];
	let (expressions, new_iterator) = execute_expression_parsers(&parsers, iterator, Vec::new())?;
	iterator = new_iterator;

	iterator.next_str(";").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find ; after const {} value.", name),
		},
		_ => e,
	})?;

	let value = expression_atoms_to_node(&expressions);

	Ok((Node::constant_with_type(name, r#type, value), iterator))
}

/// Parses a named resource descriptor and preserves its source type name for semantic resolution.
// Descriptor grammar validation is one ordered parse transaction because every key must be unique.
#[allow(clippy::too_many_lines)]
pub(crate) fn parse_descriptor<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("descriptor")?;

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after descriptor in resource {}. The most likely cause is that the descriptor properties are missing.",
			name
		))
	})?;
	iterator.next_str("{").map_err(|_| {
		syntax_error(format!(
			"Expected {{ after < in descriptor {}. The most likely cause is that positional descriptor syntax was used.",
			name
		))
	})?;

	let mut descriptor_type = None;
	let mut slot = None;
	let mut access = None;
	let mut memory_class = None;
	let mut count = None;

	loop {
		if iterator.clone().next().copied() == Some("}") {
			iterator.next();
			break;
		}

		let key = iterator.next_identifier().map_err(|_| {
			syntax_error(format!(
				"Expected a property name in descriptor {}. The most likely cause is that two properties are not separated by a comma.",
				name
			))
		})?;
		iterator.next_str(":").map_err(|_| {
			syntax_error(format!(
				"Expected : after property `{}` in descriptor {}. The most likely cause is that the property value is malformed.",
				key, name
			))
		})?;

		match key {
			"type" => {
				if descriptor_type.is_some() {
					return Err(syntax_error(format!(
						"Duplicate `type` property in descriptor {}. The most likely cause is that the property was declared twice.",
						name
					)));
				}

				let resource_type = iterator.next_identifier().map_err(|_| {
					syntax_error(format!(
						"Expected a resource type in descriptor {}. The most likely cause is that the `type` property is empty.",
						name
					))
				})?;
				let runtime_array = if iterator.clone().next().copied() == Some("[") {
					iterator.next();
					iterator.next_str("]").map_err(|_| {
						syntax_error(format!(
							"Expected ] after the runtime array marker in descriptor {}. The most likely cause is that the resource used a fixed count inside `[]`.",
							name
						))
					})?;
					true
				} else {
					false
				};
				let format = if iterator.clone().next().copied() == Some("<") {
					iterator.next();
					let format = iterator.next_identifier().map_err(|_| {
						syntax_error(format!(
							"Expected a storage image format in descriptor {}. The most likely cause is that the StorageImage format argument is missing.",
							name
						))
					})?;
					iterator.next_str(">").map_err(|_| {
						syntax_error(format!(
							"Expected > after storage image format in descriptor {}. The most likely cause is that the resource type arguments are malformed.",
							name
						))
					})?;
					if resource_type != "StorageImage" {
						return Err(syntax_error(format!(
							"Resource type {} cannot declare format `{}` in descriptor {}. The most likely cause is that a storage image format was attached to a non-StorageImage resource.",
							resource_type, format, name
						)));
					}
					Some(format)
				} else {
					None
				};
				descriptor_type = Some((resource_type, runtime_array, format));
			}
			"binding" => {
				if slot.is_some() {
					return Err(syntax_error(format!(
						"Duplicate `binding` property in descriptor {}. The most likely cause is that the property was declared twice.",
						name
					)));
				}
				slot = Some(
					iterator
						.next()
						.ok_or_else(|| {
							syntax_error(format!(
								"Expected a binding in descriptor {}. The most likely cause is that the `binding` property is empty.",
								name
							))
						})?
						.parse::<u32>()
						.map_err(|_| {
							syntax_error(format!(
								"Invalid binding in descriptor {}. The most likely cause is that the binding is not a u32 literal.",
								name
							))
						})?,
				);
			}
			"access" => {
				if access.is_some() {
					return Err(syntax_error(format!(
						"Duplicate `access` property in descriptor {}. The most likely cause is that the property was declared twice.",
						name
					)));
				}
				let value = iterator.next().ok_or_else(|| {
					syntax_error(format!(
						"Expected an access mode in descriptor {}. The most likely cause is that the `access` property is empty.",
						name
					))
				})?;
				access = Some(match *value {
					"read" => (true, false),
					"write" => (false, true),
					"read_write" => (true, true),
					_ => {
						return Err(syntax_error(format!(
							"Invalid access mode `{}` in descriptor {}. The most likely cause is that the access is not read, write, or read_write.",
							value, name
						)));
					}
				});
			}
			"memory" => {
				if memory_class.is_some() {
					return Err(syntax_error(format!(
						"Duplicate `memory` property in descriptor {}. The most likely cause is that the property was declared twice.",
						name
					)));
				}
				let value = iterator.next().ok_or_else(|| {
					syntax_error(format!(
						"Expected a memory class in descriptor {}. The most likely cause is that the `memory` property is empty.",
						name
					))
				})?;
				if !matches!(*value, "constant" | "device") {
					return Err(syntax_error(format!(
						"Invalid memory class `{}` in descriptor {}. The most likely cause is that the memory is not constant or device.",
						value, name
					)));
				}
				memory_class = Some(*value);
			}
			"count" => {
				if count.is_some() {
					return Err(syntax_error(format!(
						"Duplicate `count` property in descriptor {}. The most likely cause is that the property was declared twice.",
						name
					)));
				}
				let value = iterator
					.next()
					.ok_or_else(|| {
						syntax_error(format!(
							"Expected a resource count in descriptor {}. The most likely cause is that the `count` property is empty.",
							name
						))
					})?
					.parse::<u32>()
					.map_err(|_| {
						syntax_error(format!(
							"Invalid resource count in descriptor {}. The most likely cause is that the count is not a u32 literal.",
							name
						))
					})?;
				count = Some(NonZeroU32::new(value).ok_or_else(|| {
					syntax_error(format!(
						"Invalid resource count in descriptor {}. The most likely cause is that the resource array was declared with zero elements.",
						name
					))
				})?);
			}
			_ => {
				return Err(syntax_error(format!(
					"Unknown property `{}` in descriptor {}. The most likely cause is that the property name is misspelled.",
					key, name
				)));
			}
		}

		match iterator.next().copied() {
			Some(",") => {}
			Some("}") => break,
			_ => {
				return Err(syntax_error(format!(
					"Expected , or }} after property `{}` in descriptor {}. The most likely cause is that the next property is not separated by a comma.",
					key, name
				)));
			}
		}
	}

	let (resource_type, runtime_array, format) = descriptor_type.ok_or_else(|| {
		syntax_error(format!(
			"Descriptor {} is missing `type`. The most likely cause is that the required property was omitted.",
			name
		))
	})?;
	let slot = slot.ok_or_else(|| {
		syntax_error(format!(
			"Descriptor {} is missing `binding`. The most likely cause is that the required property was omitted.",
			name
		))
	})?;
	let (read, write) = access.ok_or_else(|| {
		syntax_error(format!(
			"Descriptor {} is missing `access`. The most likely cause is that the required property was omitted.",
			name
		))
	})?;
	if runtime_array && count.is_some() {
		return Err(syntax_error(format!(
			"Runtime buffer descriptor {name} cannot declare a resource count. The most likely cause is that a runtime element array was combined with descriptor-array syntax."
		)));
	}

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after descriptor {} properties. The most likely cause is that the descriptor declaration is incomplete.",
			name
		))
	})?;
	iterator.next_str(";").map_err(|_| {
		syntax_error(format!(
			"Expected ; after descriptor {}. The most likely cause is that the declaration terminator is missing.",
			name
		))
	})?;

	Ok((
		Node {
			node: Nodes::Descriptor {
				name,
				resource_type,
				runtime_array,
				format,
				slot,
				read,
				write,
				memory_class,
				count,
			},
		},
		iterator,
	))
}

/// Parses stage-interface storage declared directly in BESL source.
// Stage-interface grammar validation is one ordered parse transaction over a shared iterator.
#[allow(clippy::too_many_lines)]
pub(crate) fn parse_shader_interface_declaration<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let declaration = iterator.next().copied().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
	if !matches!(declaration, "input" | "output" | "task_payload" | "workgroup") {
		return Err(ParsingFailReasons::NotMine);
	}

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after {declaration} in {name}. The most likely cause is that the declaration arguments are missing."
		))
	})?;
	let format = iterator.next_identifier().map_err(|_| {
		syntax_error(format!(
			"Expected a type in {declaration} {name}. The most likely cause is that the first declaration argument is missing."
		))
	})?;

	let node = match declaration {
		"input" | "output" => {
			iterator.next_str(",").map_err(|_| {
				syntax_error(format!(
					"Expected , after the type in {declaration} {name}. The most likely cause is that the location is missing."
				))
			})?;
			let location = iterator
				.next()
				.ok_or_else(|| {
					syntax_error(format!(
						"Expected a location in {declaration} {name}. The most likely cause is that the second declaration argument is missing."
					))
				})?
				.parse::<u8>()
				.map_err(|_| {
					syntax_error(format!(
						"Invalid location in {declaration} {name}. The most likely cause is that the location is not a u8 literal."
					))
				})?;

			if declaration == "input" {
				Node::input(name, format, location)
			} else if iterator.clone().next().copied() == Some(",") {
				iterator.next();
				let count = iterator
					.next()
					.ok_or_else(|| {
						syntax_error(format!(
							"Expected an element count in output {name}. The most likely cause is that the third declaration argument is missing."
						))
					})?
					.parse::<u32>()
					.map_err(|_| {
						syntax_error(format!(
							"Invalid element count in output {name}. The most likely cause is that the count is not a u32 literal."
						))
					})?;
				if count == 0 {
					return Err(syntax_error(format!(
						"Invalid element count in output {name}. The most likely cause is that an output array was declared with zero elements."
					)));
				}
				Node::output_array(name, format, location, count)
			} else {
				Node::output(name, format, location)
			}
		}
		"task_payload" => {
			iterator.next_str(",").map_err(|_| {
				syntax_error(format!(
					"Expected , after the type in task_payload {name}. The most likely cause is that the element count is missing."
				))
			})?;
			let count = iterator
				.next()
				.ok_or_else(|| {
					syntax_error(format!(
						"Expected an element count in task_payload {name}. The most likely cause is that the second declaration argument is missing."
					))
				})?
				.parse::<u32>()
				.map_err(|_| {
					syntax_error(format!(
						"Invalid element count in task_payload {name}. The most likely cause is that the count is not a u32 literal."
					))
				})?;
			if count == 0 {
				return Err(syntax_error(format!(
					"Invalid element count in task_payload {name}. The most likely cause is that a task-payload array was declared with zero elements."
				)));
			}
			Node::task_payload(name, format, count)
		}
		"workgroup" => {
			let count = if iterator.clone().next().copied() == Some(",") {
				iterator.next();
				let count = iterator
					.next()
					.ok_or_else(|| {
						syntax_error(format!(
							"Expected an element count in workgroup {name}. The most likely cause is that the second declaration argument is missing."
						))
					})?
					.parse::<u32>()
					.map_err(|_| {
						syntax_error(format!(
							"Invalid element count in workgroup {name}. The most likely cause is that the count is not a u32 literal."
						))
					})?;
				Some(NonZeroUsize::new(count as usize).ok_or_else(|| {
					syntax_error(format!(
						"Invalid element count in workgroup {name}. The most likely cause is that a workgroup array was declared with zero elements."
					))
				})?)
			} else {
				None
			};
			Node::workgroup(name, format, count)
		}
		_ => unreachable!("Shader interface declaration was validated above."),
	};

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after {declaration} {name} arguments. The most likely cause is that the declaration is incomplete."
		))
	})?;
	iterator.next_str(";").map_err(|_| {
		syntax_error(format!(
			"Expected ; after {declaration} {name}. The most likely cause is that the declaration terminator is missing."
		))
	})?;

	Ok((node, iterator))
}

/// Parses the single push-constant block exposed to shader source as `push_constant`.
pub(crate) fn parse_push_constant<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("push_constant")?;
	iterator.next_str(":")?;
	iterator.next_str("push_constant")?;
	iterator.next_str("{").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected { after push_constant declaration.".to_string(),
	})?;

	let mut members = Vec::new();
	loop {
		let Some(token) = iterator.next().copied() else {
			return Err(ParsingFailReasons::BadSyntax {
				message: "Push-constant declaration is missing a closing }.".to_string(),
			});
		};
		if token == "}" {
			break;
		}
		if token == "," {
			continue;
		}

		let member_name = token;
		iterator.next_str(":").map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected : after push-constant member {member_name}."),
		})?;
		let member_type = iterator.next_identifier().map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected a type after push-constant member {member_name}."),
		})?;
		members.push(make_member(member_name, member_type));
	}

	Ok((Node::push_constant(members), iterator))
}

pub(crate) fn parse_member<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let mut r#type = iterator
		.next_identifier()
		.map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find type while parsing member {}.", name),
			},
			_ => e,
		})?
		.to_string();

	if let Some(&&n) = iterator.clone().peekable().peek()
		&& n == "<"
	{
		if r#type == "descriptor" {
			return Err(ParsingFailReasons::BadSyntax {
				message: format!(
					"Invalid descriptor declaration for {name}. The most likely cause is that required slot or access arguments are missing."
				),
			});
		}
		iterator.next();
		r#type.push('<');
		let next = iterator.next().ok_or(ParsingFailReasons::BadSyntax {
			message: format!("Expected to find type while parsing generic argument for member {}", name),
		})?;
		r#type.push_str(next.as_ref());
		iterator.next();
		r#type.push('>');
	}

	let node = Node::member(name, &r#type);

	iterator.next().ok_or(ParsingFailReasons::BadSyntax {
		message: "Expected semicolon".to_string(),
	})?; // Skip semicolon

	Ok(((node), iterator))
}

pub(crate) fn parse_macro<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let mut iter = iterator;

	iter.next_str("#")?;
	iter.next_str("[")?;
	iter.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: "Expected to find macro name after #[.".to_string(),
		},
		_ => e,
	})?;
	iter.next_str("]").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: "Expected to find ] after macro name.".to_string(),
		},
		_ => e,
	})?;

	Ok((make_scope("MACRO", vec![]), iter))
}

/// Parses one named struct and requires comma-delimited fields.
pub(crate) fn parse_struct<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("struct")?;
	let invalid = || ParsingFailReasons::BadSyntax {
		message: format!("Invalid struct {name}. The most likely cause is a missing `name: type` field, comma, or closing }}."),
	};
	iterator.next_str("{").map_err(|_| invalid())?;

	let mut fields = Vec::new();
	let mut needs_comma = false;
	let mut closed = false;
	while let Some(&token) = iterator.next() {
		if token == "}" {
			closed = true;
			break;
		}
		if needs_comma {
			if token != "," {
				return Err(invalid());
			}
			needs_comma = false;
			continue;
		}
		if token == "," {
			return Err(invalid());
		}

		iterator.next_str(":").map_err(|_| invalid())?;
		let type_name = iterator.next_identifier().map_err(|_| invalid())?;
		let type_name = if iterator.clone().next().copied() == Some("[") {
			iterator.next();
			let count = iterator
				.next()
				.and_then(|value| value.parse::<u32>().ok())
				.ok_or_else(&invalid)?;
			iterator.next_str("]").map_err(|_| invalid())?;
			format!("{type_name}[{count}]")
		} else {
			type_name.to_string()
		};
		fields.push(make_member(token, &type_name));
		needs_comma = true;
	}
	if !closed {
		return Err(invalid());
	}

	Ok((Node::r#struct(name, fields), iterator))
}

fn parse_record_type<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	role: RecordRole,
) -> Result<(Vec<TypeField<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	let invalid = || ParsingFailReasons::BadSyntax {
		message: format!(
			"Invalid anonymous {role} type. The most likely cause is a missing `name: type` field, comma, or closing }}."
		),
	};
	iterator.next_str("{").map_err(|_| invalid())?;
	let mut fields = Vec::new();
	loop {
		if iterator.clone().next().copied() == Some("}") {
			iterator.next();
			return Ok((fields, iterator));
		}
		let name = iterator.next_identifier().map_err(|_| invalid())?;
		iterator.next_str(":").map_err(|_| invalid())?;
		let base_type = iterator.next_identifier().map_err(|_| invalid())?;
		let (type_name, next) = parse_type_name(iterator, base_type)?;
		iterator = next;
		fields.push(TypeField { name, type_name });
		match iterator.clone().next().copied() {
			Some(",") => {
				iterator.next();
			}
			Some("}") => {}
			_ => return Err(invalid()),
		}
	}
}

/// Parses named, fixed-array, and anonymous record types without flattening their structure.
pub(crate) fn parse_type_name<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	base_type: &'a str,
) -> Result<(TypeName<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	let role = match base_type {
		"interface" => Some(RecordRole::Interface),
		"output" => Some(RecordRole::Output),
		_ => None,
	};
	let mut type_name = if let Some(role) = role.filter(|_| iterator.clone().next().copied() == Some("{")) {
		let (fields, next) = parse_record_type(iterator, role)?;
		iterator = next;
		TypeName::Record { role, fields }
	} else {
		TypeName::Named(base_type)
	};

	while iterator.clone().peekable().peek().map(|token| token.as_ref()) == Some("[") {
		iterator.next_str("[")?;
		let count = iterator
			.next_is(|token| token.chars().all(|c| c.is_ascii_digit()))?
			.parse::<u32>()
			.map_err(|_| ParsingFailReasons::BadSyntax {
				message: format!("Invalid array count for type {}", type_name),
			})?;
		iterator.next_str("]")?;

		type_name = TypeName::Array {
			element: Box::new(type_name),
			count,
		};
	}

	Ok((type_name, iterator))
}
