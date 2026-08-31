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

	fn atoms_to_node<'a>(atoms: &[Atoms<'a>]) -> Node<'a> {
		let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

		if let Some((i, e)) = max_precedence_item {
			match e {
				Atoms::Operator { name } => {
					let left = atoms_to_node(&atoms[..i]);
					let right = atoms_to_node(&atoms[i + 1..]);
					Node {
						node: Nodes::Expression(Expressions::Operator {
							name,
							left: Box::new(left),
							right: Box::new(right),
						}),
					}
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| atoms_to_node(v)).collect::<Vec<_>>();
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
				Atoms::Member { name } => Node {
					node: Nodes::Expression(Expressions::Member { name: (*name).into() }),
				},
				_ => panic!("Unexpected atom in const expression"),
			}
		} else {
			panic!("No atoms in const expression");
		}
	}

	let value = atoms_to_node(&expressions);

	Ok((Node::constant_with_type(name, r#type, value), iterator))
}

/// Parses a flat resource descriptor and preserves its source type name for semantic resolution.
// Descriptor grammar validation is one ordered parse transaction; splitting it would duplicate iterator-state handling.
#[allow(clippy::too_many_lines)]
pub(crate) fn parse_descriptor<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("descriptor")?;

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after descriptor in resource {}. The most likely cause is that the descriptor arguments are missing.",
			name
		))
	})?;
	let resource_type = iterator.next_identifier().map_err(|_| {
		syntax_error(format!(
			"Expected a resource type in descriptor {}. The most likely cause is that the first descriptor argument is missing.",
			name
		))
	})?;
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
	iterator.next_str(",").map_err(|_| {
		syntax_error(format!(
			"Expected , after resource type in descriptor {}. The most likely cause is that the descriptor arguments are malformed.",
			name
		))
	})?;

	let slot = iterator
		.next()
		.ok_or_else(|| {
			syntax_error(format!(
				"Expected a slot in descriptor {}. The most likely cause is that the second descriptor argument is missing.",
				name
			))
		})?
		.parse::<u32>()
		.map_err(|_| {
			syntax_error(format!(
				"Invalid slot in descriptor {}. The most likely cause is that the slot is not a u32 literal.",
				name
			))
		})?;
	iterator.next_str(",").map_err(|_| {
		syntax_error(format!(
			"Expected , after slot in descriptor {}. The most likely cause is that the descriptor arguments are malformed.",
			name
		))
	})?;

	let access = iterator.next().ok_or_else(|| {
		syntax_error(format!(
			"Expected an access mode in descriptor {}. The most likely cause is that the third descriptor argument is missing.",
			name
		))
	})?;
	let (read, write) = match *access {
		"read" => (true, false),
		"write" => (false, true),
		"read_write" => (true, true),
		_ => {
			return Err(syntax_error(format!(
				"Invalid access mode `{}` in descriptor {}. The most likely cause is that the access is not read, write, or read_write.",
				access, name
			)));
		}
	};

	let (memory_class, count) = if iterator.clone().next().copied() == Some(",") {
		iterator.next();
		let memory_class_or_count = iterator
			.next()
			.ok_or_else(|| {
				syntax_error(format!(
					"Expected a buffer memory class or resource count in descriptor {}. The most likely cause is that the fourth descriptor argument is missing.",
					name
				))
			})?;

		if matches!(*memory_class_or_count, "constant" | "device") {
			let count = if iterator.clone().next().copied() == Some(",") {
				iterator.next();
				let count = iterator
					.next()
					.ok_or_else(|| {
						syntax_error(format!(
							"Expected a resource count in descriptor {}. The most likely cause is that the fifth descriptor argument is missing.",
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
				Some(NonZeroU32::new(count).ok_or_else(|| {
					syntax_error(format!(
						"Invalid resource count in descriptor {}. The most likely cause is that the resource array was declared with zero elements.",
						name
					))
				})?)
			} else {
				None
			};
			(Some(*memory_class_or_count), count)
		} else {
			let count = memory_class_or_count.parse::<u32>().map_err(|_| {
				syntax_error(format!(
					"Invalid buffer memory class or resource count `{}` in descriptor {}. The most likely cause is that the fourth descriptor argument is neither constant, device, nor a u32 count.",
					memory_class_or_count, name
				))
			})?;
			(
				None,
				Some(NonZeroU32::new(count).ok_or_else(|| {
					syntax_error(format!(
						"Invalid resource count in descriptor {}. The most likely cause is that the resource array was declared with zero elements.",
						name
					))
				})?),
			)
		}
	} else {
		(None, None)
	};

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after descriptor {} arguments. The most likely cause is that the descriptor declaration is incomplete.",
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

pub(crate) fn parse_struct<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("struct")?;
	iterator.next_str("{").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find {{ after struct {} declaration.", name),
		},
		_ => e,
	})?;

	let mut fields = vec![];

	while let Some(&v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		iterator.next_str(":").map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find : after name for member {} in struct {}", v, name),
			},
			_ => e,
		})?;

		let type_name = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find a type name after : for member {} in struct {}", v, name),
			},
			_ => e,
		})?;

		// See if is array type
		let type_name = if iterator.clone().peekable().peek().map(|v| v.as_ref()) == Some("[") {
			iterator.next();
			let count = iterator
				.next()
				.and_then(|v| v.parse::<u32>().ok())
				.ok_or(ParsingFailReasons::BadSyntax {
					message: format!("Expected to find a number after [ for member {} in struct {}", v, name),
				})?;
			iterator.next().unwrap();
			format!("{}[{}]", type_name, count)
		} else {
			type_name.to_string()
		};

		fields.push(make_member(v, &type_name));
	}

	let node = Node::r#struct(name, fields);

	Ok((node, iterator))
}
pub(crate) fn parse_type_name<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	base_type: &'a str,
) -> Result<(TypeName<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	let mut type_name = TypeName::Named(base_type);

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
