use super::*;

pub(crate) fn parse_const<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("const")?;

	let r#type = iterator
		.next_identifier()
		.map_err(|error| error.claimed(|| format!("Expected to find a type for const {name}.")))?;
	let (r#type, mut iterator) = parse_type_name(iterator, r#type)?;

	iterator
		.next_str("=")
		.map_err(|error| error.claimed(|| format!("Expected to find = after type for const {name}.")))?;

	let (value, mut iterator) = parse_expression_node(&[parse_function_call, parse_literal, parse_variable], iterator)?;

	iterator
		.next_str(";")
		.map_err(|error| error.claimed(|| format!("Expected to find ; after const {name} value.")))?;

	Ok((Node::constant(name, r#type, value), iterator))
}

/// Parses a named resource descriptor and preserves its source type name for semantic resolution.
// Descriptor grammar validation is one ordered parse transaction because every key must be unique.
#[allow(clippy::too_many_lines)]
pub(crate) fn parse_descriptor<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("descriptor")?;

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	// Reads the u32 value of property `key`, which the messages call `what`.
	let next_u32 = |iterator: &mut std::slice::Iter<'i, &'a str>, what: &str, key: &str| -> Result<u32, ParsingFailReasons> {
		let value = iterator.next().ok_or_else(|| {
			syntax_error(format!(
				"Expected a {what} in descriptor {name}. The most likely cause is that the `{key}` property is empty."
			))
		})?;
		value.parse().map_err(|_| {
			syntax_error(format!(
				"Invalid {what} in descriptor {name}. The most likely cause is that the {key} is not a u32 literal."
			))
		})
	};
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after descriptor in resource {name}. The most likely cause is that the descriptor properties are missing."
		))
	})?;
	iterator.next_str("{").map_err(|_| {
		syntax_error(format!(
			"Expected {{ after < in descriptor {name}. The most likely cause is that positional descriptor syntax was used."
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
				"Expected a property name in descriptor {name}. The most likely cause is that two properties are not separated by a comma."
			))
		})?;
		iterator.next_str(":").map_err(|_| {
			syntax_error(format!(
				"Expected : after property `{key}` in descriptor {name}. The most likely cause is that the property value is malformed."
			))
		})?;

		let repeated = match key {
			"type" => descriptor_type.is_some(),
			"binding" => slot.is_some(),
			"access" => access.is_some(),
			"memory" => memory_class.is_some(),
			"count" => count.is_some(),
			_ => false,
		};
		if repeated {
			return Err(syntax_error(format!(
				"Duplicate `{key}` property in descriptor {name}. The most likely cause is that the property was declared twice."
			)));
		}

		match key {
			"type" => {
				let resource_type = iterator.next_identifier().map_err(|_| {
					syntax_error(format!(
						"Expected a resource type in descriptor {name}. The most likely cause is that the `type` property is empty."
					))
				})?;
				let runtime_array = if iterator.clone().next().copied() == Some("[") {
					iterator.next();
					iterator.next_str("]").map_err(|_| {
						syntax_error(format!(
							"Expected ] after the runtime array marker in descriptor {name}. The most likely cause is that the resource used a fixed count inside `[]`."
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
							"Expected a storage image format in descriptor {name}. The most likely cause is that the StorageImage format argument is missing."
						))
					})?;
					iterator.next_str(">").map_err(|_| {
						syntax_error(format!(
							"Expected > after storage image format in descriptor {name}. The most likely cause is that the resource type arguments are malformed."
						))
					})?;
					if resource_type != "StorageImage" {
						return Err(syntax_error(format!(
							"Resource type {resource_type} cannot declare format `{format}` in descriptor {name}. The most likely cause is that a storage image format was attached to a non-StorageImage resource."
						)));
					}
					Some(format)
				} else {
					None
				};
				descriptor_type = Some((resource_type, runtime_array, format));
			}
			"binding" => slot = Some(next_u32(&mut iterator, "binding", "binding")?),
			"access" => {
				let value = iterator.next().ok_or_else(|| {
					syntax_error(format!(
						"Expected an access mode in descriptor {name}. The most likely cause is that the `access` property is empty."
					))
				})?;
				access = Some(match *value {
					"read" => (true, false),
					"write" => (false, true),
					"read_write" => (true, true),
					_ => {
						return Err(syntax_error(format!(
							"Invalid access mode `{value}` in descriptor {name}. The most likely cause is that the access is not read, write, or read_write."
						)));
					}
				});
			}
			"memory" => {
				let value = iterator.next().ok_or_else(|| {
					syntax_error(format!(
						"Expected a memory class in descriptor {name}. The most likely cause is that the `memory` property is empty."
					))
				})?;
				memory_class = Some(match *value {
					"constant" => BufferMemoryClass::Constant,
					"device" => BufferMemoryClass::Device,
					_ => {
						return Err(syntax_error(format!(
							"Invalid memory class `{value}` in descriptor {name}. The most likely cause is that the memory is not constant or device."
						)));
					}
				});
			}
			"count" => {
				let value = next_u32(&mut iterator, "resource count", "count")?;
				count = Some(NonZeroU32::new(value).ok_or_else(|| {
					syntax_error(format!(
						"Invalid resource count in descriptor {name}. The most likely cause is that the resource array was declared with zero elements."
					))
				})?);
			}
			_ => {
				return Err(syntax_error(format!(
					"Unknown property `{key}` in descriptor {name}. The most likely cause is that the property name is misspelled."
				)));
			}
		}

		match iterator.next().copied() {
			Some(",") => {}
			Some("}") => break,
			_ => {
				return Err(syntax_error(format!(
					"Expected , or }} after property `{key}` in descriptor {name}. The most likely cause is that the next property is not separated by a comma."
				)));
			}
		}
	}

	let missing = |key: &str| {
		syntax_error(format!(
			"Descriptor {name} is missing `{key}`. The most likely cause is that the required property was omitted."
		))
	};
	let (resource_type, runtime_array, format) = descriptor_type.ok_or_else(|| missing("type"))?;
	let slot = slot.ok_or_else(|| missing("binding"))?;
	let (read, write) = access.ok_or_else(|| missing("access"))?;
	if runtime_array && count.is_some() {
		return Err(syntax_error(format!(
			"Runtime buffer descriptor {name} cannot declare a resource count. The most likely cause is that a runtime element array was combined with descriptor-array syntax."
		)));
	}

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after descriptor {name} properties. The most likely cause is that the descriptor declaration is incomplete."
		))
	})?;
	iterator.next_str(";").map_err(|_| {
		syntax_error(format!(
			"Expected ; after descriptor {name}. The most likely cause is that the declaration terminator is missing."
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
	if !matches!(
		declaration,
		"input" | "output" | "vertex_output" | "task_payload" | "workgroup"
	) {
		return Err(ParsingFailReasons::NotMine);
	}

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	// Reads the nonzero element count of a `kind` array from its `ordinal` declaration argument. The messages call the
	// array `array`.
	let parse_count = |iterator: &mut std::slice::Iter<'i, &'a str>,
	                   kind: &str,
	                   ordinal: &str,
	                   array: &str|
	 -> Result<NonZeroUsize, ParsingFailReasons> {
		let count = iterator.next().ok_or_else(|| {
			syntax_error(format!(
				"Expected an element count in {kind} {name}. The most likely cause is that the {ordinal} declaration argument is missing."
			))
		})?;
		let count = count.parse::<u32>().map_err(|_| {
			syntax_error(format!(
				"Invalid element count in {kind} {name}. The most likely cause is that the count is not a u32 literal."
			))
		})?;
		NonZeroUsize::new(count as usize).ok_or_else(|| {
			syntax_error(format!(
				"Invalid element count in {kind} {name}. The most likely cause is that {array} was declared with zero elements."
			))
		})
	};
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
		"input" | "output" | "vertex_output" => {
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
			} else if declaration == "vertex_output" || iterator.clone().next().copied() == Some(",") {
				// Vertex outputs only exist as mesh output arrays, so their element count is required.
				iterator.next_str(",").map_err(|_| {
					syntax_error(format!(
						"Expected , after the location in {declaration} {name}. The most likely cause is that the element count is missing."
					))
				})?;
				let count = parse_count(&mut iterator, "output", "third", "an output array")?;
				Node::output_array(name, format, location, Some(count), declaration == "vertex_output")
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
			let count = parse_count(&mut iterator, "task_payload", "second", "a task-payload array")?;
			Node::task_payload(name, format, count)
		}
		"workgroup" => {
			let count = if iterator.clone().next().copied() == Some(",") {
				iterator.next();
				Some(parse_count(&mut iterator, "workgroup", "second", "a workgroup array")?)
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

		iterator.next_str(":").map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected : after push-constant member {token}."),
		})?;
		let member_type = iterator.next_identifier().map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected a type after push-constant member {token}."),
		})?;
		members.push(Node::member(token, member_type));
	}

	Ok((Node::push_constant(members), iterator))
}

pub(crate) fn parse_member<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let mut r#type = iterator
		.next_identifier()
		.map_err(|error| error.claimed(|| format!("Expected to find type while parsing member {name}.")))?
		.to_string();

	if iterator.clone().next().copied() == Some("<") {
		if r#type == "descriptor" {
			return Err(ParsingFailReasons::BadSyntax {
				message: format!(
					"Invalid descriptor declaration for {name}. The most likely cause is that required slot or access arguments are missing."
				),
			});
		}
		iterator.next();
		r#type.push('<');
		let next = iterator.next().ok_or_else(|| ParsingFailReasons::BadSyntax {
			message: format!("Expected to find type while parsing generic argument for member {name}"),
		})?;
		r#type.push_str(next);
		iterator.next();
		r#type.push('>');
	}

	iterator.next().ok_or_else(|| ParsingFailReasons::BadSyntax {
		message: "Expected semicolon".to_string(),
	})?; // Skip semicolon

	Ok((Node::member(name, r#type), iterator))
}

pub(crate) fn parse_macro<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("#")?;
	iterator.next_str("[")?;
	iterator
		.next_identifier()
		.map_err(|error| error.claimed(|| "Expected to find macro name after #[.".to_string()))?;
	iterator
		.next_str("]")
		.map_err(|error| error.claimed(|| "Expected to find ] after macro name.".to_string()))?;

	Ok((Node::scope("MACRO", Vec::new()), iterator))
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
	loop {
		let token = *iterator.next().ok_or_else(&invalid)?;
		if token == "}" {
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
		fields.push(Node::member(token, type_name));
		needs_comma = true;
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

	while iterator.clone().next().copied() == Some("[") {
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
