use std::cell::RefCell;

use super::resolution::*;
use super::*;
use crate::parser;

// This exhaustive parser-to-lexer boundary keeps each source node variant's lowering beside the others.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub(super) fn lex_parsed_node(
	chain: Vec<NodeReference>,
	parser_node: &parser::Node,
	next_intrinsic_expansion_id: &mut usize,
) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Null => Node::new(Nodes::Null).into(),
		parser::Nodes::Scope { name, children } => {
			assert_ne!(*name, "root"); // The root scope node cannot be an inner part of the program.

			let this: NodeReference = Node::scope(name.to_string()).into();
			for child in children {
				let child = lex_child_with_parent(&chain, &this, child, next_intrinsic_expansion_id)?;
				this.borrow_mut().add_child(child);
			}

			this
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = get_reference(&chain, name) {
				// If the type already exists, return it.
				return Ok(n);
			}

			let this: NodeReference = Node::r#struct(name, Vec::new()).into();
			for field in fields {
				let field = lex_child_with_parent(&chain, &this, field, next_intrinsic_expansion_id)?;
				this.borrow_mut().add_child(field);
			}

			this
		}
		parser::Nodes::Specialization { name, r#type } => {
			let t = resolve_type(&chain, r#type)?;

			let this = Node::new(Nodes::Specialization {
				name: name.to_string(),
				r#type: t,
			});

			this.into()
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(['<', '>']);

				let outer_type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No outer name".to_string()),
				})?;

				let outer_type = resolve_type(&chain, outer_type_name)?;

				let inner_type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No inner name".to_string()),
				})?;

				let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
					Node::internal_new(Node {
						node: Nodes::Struct {
							name: format!("{}*", stripped),
							template: Some(outer_type.clone()),
							fields: Vec::new(),
							types: Vec::new(),
						},
					})
				} else {
					resolve_type(&chain, inner_type_name)?
				};

				if let Some(n) = get_reference(&chain, r#type) {
					// If the specialized generic type already exists, return it.
					return Ok(n);
				}

				let children = Vec::new();

				let this = Node {
					node: Nodes::Struct {
						name: r#type.to_string(),
						template: Some(outer_type),
						fields: children,
						types: vec![inner_type],
					},
				};

				let this: NodeReference = this.into();

				return Ok(this);
			} else if r#type.contains('[') {
				let mut s = r#type.split(['[', ']']);

				let type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No type name".to_string()),
				})?;

				let member_type = resolve_type(&chain, type_name)?;

				let count = s
					.next()
					.ok_or(LexError::Undefined {
						message: Some("No count".to_string()),
					})?
					.parse()
					.map_err(|_| LexError::Undefined {
						message: Some("Invalid count".to_string()),
					})?;

				return Ok(Node::array(name, member_type, count));
			} else {
				resolve_type(&chain, r#type)?
			};

			let this: NodeReference = Node::member(name, t).into();

			this
		}
		parser::Nodes::Parameter { name, r#type } => {
			let t = resolve_type_name(&chain, r#type)?;

			let this = Node::new(Nodes::Parameter {
				name: name.to_string(),
				r#type: t,
			});

			this.into()
		}
		parser::Nodes::Input { name, format, location } => {
			let t = resolve_type(&chain, format)?;

			let this = Node::new(Nodes::Input {
				name: name.to_string(),
				format: t,
				location: *location,
			});

			this.into()
		}
		parser::Nodes::Output {
			name,
			format,
			location,
			count,
		} => {
			let t = resolve_type(&chain, format)?;

			let this = Node::new(Nodes::Output {
				name: name.to_string(),
				format: t,
				location: *location,
				count: *count,
			});

			this.into()
		}
		parser::Nodes::TaskPayload { name, format, count } => {
			let format = resolve_type(&chain, format)?;
			Node::new(Nodes::TaskPayload {
				name: name.to_string(),
				format,
				count: *count,
			})
			.into()
		}
		parser::Nodes::Workgroup { name, format, count } => {
			let format = resolve_type(&chain, format)?;
			Node::new(Nodes::Workgroup {
				name: name.to_string(),
				format,
				count: *count,
			})
			.into()
		}
		parser::Nodes::Function {
			name,
			return_type,
			statements,
			params,
			..
		} => {
			let t = resolve_type_name(&chain, return_type)?;

			let this: NodeReference = Node::function(name, Vec::new(), t, Vec::new()).into();

			for param in params {
				let param = lex_child_with_parent(&chain, &this, param, next_intrinsic_expansion_id)?;
				match this.borrow_mut().node_mut() {
					Nodes::Function { params, .. } => {
						params.push(param);
					}
					_ => {
						panic!("Expected function");
					}
				}
			}

			let mut scoped_chain = extend_chain(&chain, &this);

			for statement in statements {
				let statement = lex_parsed_node(scoped_chain.clone(), statement, next_intrinsic_expansion_id)?;
				this.borrow_mut().add_child(statement);
				scoped_chain.push(
					this.borrow()
						.get_children()
						.and_then(|children| children.last().cloned())
						.unwrap(),
				);
			}

			this
		}
		parser::Nodes::Conditional { condition, statements } => {
			let condition = lex_parsed_node(chain.clone(), condition, next_intrinsic_expansion_id)?;
			let mut lexed_statements = Vec::with_capacity(statements.len());
			let mut scoped_chain = chain.clone();

			for statement in statements {
				let statement = lex_parsed_node(scoped_chain.clone(), statement, next_intrinsic_expansion_id)?;
				scoped_chain.push(statement.clone());
				lexed_statements.push(statement);
			}

			Node::conditional(condition, lexed_statements).into()
		}
		parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			let initializer = lex_parsed_node(chain.clone(), initializer, next_intrinsic_expansion_id)?;
			let mut scoped_chain = chain.clone();
			scoped_chain.push(initializer.clone());
			let condition = lex_parsed_node(scoped_chain.clone(), condition, next_intrinsic_expansion_id)?;
			let update = lex_parsed_node(scoped_chain.clone(), update, next_intrinsic_expansion_id)?;
			let mut lexed_statements = Vec::with_capacity(statements.len());

			for statement in statements {
				let statement = lex_parsed_node(scoped_chain.clone(), statement, next_intrinsic_expansion_id)?;
				scoped_chain.push(statement.clone());
				lexed_statements.push(statement);
			}

			Node::for_loop(initializer, condition, update, lexed_statements).into()
		}
		parser::Nodes::PushConstant { members } => {
			let this: NodeReference = Node::push_constant(vec![]).into();

			for member in members
				.iter()
				.filter(|member| matches!(member.node, parser::Nodes::Member { .. }))
			{
				let c = lex_child_with_parent(&chain, &this, member, next_intrinsic_expansion_id)?;
				this.borrow_mut().add_child(c);
			}

			this
		}
		parser::Nodes::Binding {
			name,
			r#type,
			slot,
			read,
			write,
			memory_class,
			count,
		} => {
			let r#type = match &r#type.node {
				parser::Nodes::Type { members, .. } => BindingTypes::Buffer {
					members: members
						.iter()
						.map(|m| lex_parsed_node(chain.clone(), m, next_intrinsic_expansion_id))
						.collect::<Result<Vec<NodeReference>, LexError>>()?,
				},
				parser::Nodes::Image { format } => BindingTypes::Image {
					format: format.to_string(),
				},
				parser::Nodes::CombinedImageSampler { format } => BindingTypes::CombinedImageSampler {
					format: format.to_string(),
				},
				_ => {
					return Err(LexError::Undefined {
						message: Some("Invalid binding type".to_string()),
					});
				}
			};

			let memory_class = match memory_class {
				Some(BufferMemoryClass::Constant) => BufferMemoryClass::Constant,
				Some(BufferMemoryClass::Device) => BufferMemoryClass::Device,
				None => BufferMemoryClass::Device,
			};

			let this = if let Some(count) = count {
				Node::binding_array_in_memory(name, r#type, *slot, *read, *write, memory_class, count.get())
			} else {
				Node::binding_in_memory(name, r#type, *slot, *read, *write, memory_class)
			};

			this.into()
		}
		parser::Nodes::Descriptor {
			name,
			resource_type,
			format,
			slot,
			read,
			write,
			memory_class,
			count,
		} => {
			let r#type = resolve_descriptor_type(&chain, resource_type, *format)?;
			let memory_class = match &r#type {
				BindingTypes::Buffer { .. } => match *memory_class {
					Some("constant") => BufferMemoryClass::Constant,
					Some("device") => BufferMemoryClass::Device,
					Some(class) => {
						return Err(LexError::Undefined {
							message: Some(format!(
								"Invalid buffer memory class `{class}` for descriptor {name}. The most likely cause is that the descriptor does not use constant or device memory."
							)),
						});
					}
					None => BufferMemoryClass::Device,
				},
				_ if memory_class.is_some() => {
					return Err(LexError::Undefined {
						message: Some(format!(
							"Descriptor {name} declares a buffer memory class for a non-buffer resource. The most likely cause is that constant or device was attached to an image or texture descriptor."
						)),
					});
				}
				_ => BufferMemoryClass::Constant,
			};

			if *write && matches!(&r#type, BindingTypes::Buffer { .. }) && memory_class == BufferMemoryClass::Constant {
				return Err(LexError::Undefined {
					message: Some(format!(
						"Writable buffer descriptor {name} uses constant memory. The most likely cause is that a writable buffer needs the device memory class."
					)),
				});
			}

			Node::binding_with_count(name, r#type, *slot, *read, *write, memory_class, *count).into()
		}
		parser::Nodes::Type { name, members } => {
			let mut this = Node::r#struct(name, Vec::new());

			for member in members {
				let c = lex_parsed_node(chain.clone(), member, next_intrinsic_expansion_id)?;
				this.add_child(c);
			}

			this.into()
		}
		parser::Nodes::Image { format } => {
			let this = Node::binding(
				"image",
				BindingTypes::Image {
					format: format.to_string(),
				},
				0,
				false,
				false,
			);

			this.into()
		}
		parser::Nodes::CombinedImageSampler { format } => {
			let this = Node::binding(
				"combined_image_sampler",
				BindingTypes::CombinedImageSampler {
					format: format.to_string(),
				},
				0,
				false,
				false,
			);

			this.into()
		}
		parser::Nodes::RawCode {
			glsl,
			hlsl,
			msl,
			input,
			output,
			..
		} => lex_raw_code(&chain, glsl.as_deref(), hlsl.as_deref(), msl.as_deref(), input, output)?.into(),
		parser::Nodes::Literal { name, body } => Node::new(Nodes::Literal {
			name: name.to_string(),
			value: lex_parsed_node(chain, body, next_intrinsic_expansion_id)?,
		})
		.into(),
		parser::Nodes::Expression(expression) => {
			let this = match expression {
				parser::Expressions::Return { value } => Node::expression(Expressions::Return {
					value: match value {
						Some(value) => Some(lex_parsed_node(chain.clone(), value, next_intrinsic_expansion_id)?),
						None => None,
					},
				}),
				parser::Expressions::Continue => Node::expression(Expressions::Continue),
				parser::Expressions::Discard => Node::expression(Expressions::Discard),
				parser::Expressions::Accessor { left, right } => {
					let left = lex_parsed_node(chain.clone(), left, next_intrinsic_expansion_id)?;

					let right = {
						let left = left.clone();

						let mut chain = chain.clone();
						chain.push(left); // Add left to chain to be able to access its members

						lex_parsed_node(chain.clone(), right, next_intrinsic_expansion_id)?
					};

					Node::expression(Expressions::Accessor { left, right })
				}
				parser::Expressions::Member { name } => Node::expression(Expressions::Member {
					source: resolve_member(&chain, name)?,
					name: name.to_string(),
				}),
				parser::Expressions::Literal { value } => Node::expression(Expressions::Literal {
					value: value.to_string(),
				}),
				parser::Expressions::Expression(elements) => Node {
					node: Nodes::Expression(Expressions::Expression {
						elements: elements
							.iter()
							.map(|e| lex_parsed_node(chain.clone(), e, next_intrinsic_expansion_id))
							.collect::<Result<Vec<NodeReference>, LexError>>()?,
					}),
				},
				parser::Expressions::Call { name, parameters } => {
					let parameters = parameters
						.iter()
						.map(|e| lex_parsed_node(chain.clone(), e, next_intrinsic_expansion_id))
						.collect::<Result<Vec<NodeReference>, LexError>>()?;
					let function = resolve_call_target(&chain, name, &parameters)?;
					let r = function.clone(); // Clone to be able to borrow it in and return it

					{
						// Validate function call
						let b = RefCell::borrow(&function.0);
						match b.node() {
							Nodes::Function { params, .. } | Nodes::Struct { fields: params, .. } => {
								if params.len() != parameters.len() {
									return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
								}
								Node::expression(Expressions::FunctionCall { function: r, parameters })
							}
							Nodes::Intrinsic { elements, .. } => Node::expression(Expressions::IntrinsicCall {
								intrinsic: r,
								arguments: parameters.clone(),
								elements: {
									let expansion_id = *next_intrinsic_expansion_id;
									*next_intrinsic_expansion_id = next_intrinsic_expansion_id.checked_add(1).expect(
										"Intrinsic expansion count overflowed. The most likely cause is an invalid shader with too many intrinsic calls.",
									);
									build_intrinsic(elements, &parameters, expansion_id)?
								},
							}),
							_ => {
								return Err(LexError::Undefined {
									message: Some("Encountered parsing error while evaluating function call. Expected Function | Struct | Intrinsic, but found other.".to_string()),
								});
							}
						}
					}
				}
				parser::Expressions::Operator { name, left, right } => Node::expression(Expressions::Operator {
					operator: match *name {
						"+" => Operators::Plus,
						"-" => Operators::Minus,
						"*" => Operators::Multiply,
						"/" => Operators::Divide,
						"%" => Operators::Modulo,
						"<<" => Operators::ShiftLeft,
						">>" => Operators::ShiftRight,
						"&" => Operators::BitwiseAnd,
						"|" => Operators::BitwiseOr,
						"=" => Operators::Assignment,
						"==" => Operators::Equality,
						"<" => Operators::LessThan,
						"!=" => Operators::Inequality,
						">" => Operators::GreaterThan,
						"<=" => Operators::LessThanOrEqual,
						">=" => Operators::GreaterThanOrEqual,
						"&&" => Operators::LogicalAnd,
						"||" => Operators::LogicalOr,
						_ => {
							panic!("Invalid operator")
						}
					},
					left: lex_parsed_node(chain.clone(), left, next_intrinsic_expansion_id)?,
					right: lex_parsed_node(chain.clone(), right, next_intrinsic_expansion_id)?,
				}),
				parser::Expressions::VariableDeclaration { name, r#type } => {
					Node::expression(Expressions::VariableDeclaration {
						name: name.to_string(),
						r#type: resolve_type_name(&chain, r#type)?,
					})
				}
				parser::Expressions::RawCode {
					glsl,
					hlsl,
					msl,
					input,
					output,
				} => lex_raw_code(&chain, *glsl, *hlsl, *msl, input, output)?,
				parser::Expressions::Macro { name, body } => {
					Node::r#macro(name, lex_parsed_node(chain, body, next_intrinsic_expansion_id)?)
				}
			};

			this.into()
		}
		parser::Nodes::Intrinsic {
			name,
			elements,
			r#return,
			..
		} => {
			let this: NodeReference = Node::intrinsic(name, Vec::new(), resolve_type(&chain, r#return)?).into();

			for element in elements {
				let element = lex_child_with_parent(&chain, &this, element, next_intrinsic_expansion_id)?;
				this.borrow_mut().add_child(element);
			}

			this
		}
		parser::Nodes::Const { name, r#type, value } => {
			let t = resolve_type_name(&chain, r#type)?;

			let v = lex_parsed_node(chain.clone(), value, next_intrinsic_expansion_id)?;

			Node::constant(name, t, v).into()
		}
	};

	Ok(node)
}
