use std::cell::RefCell;

use super::resolution::*;
use super::*;
use crate::parser;

const ATOMIC_INTRINSICS_DOCUMENTATION: &str =
	"https://byte-engine.0x44491229.dev/docs/reference/besl/intrinsics#buffer-and-workgroup-atomics";
const ARRAY_DOCUMENTATION: &str = "https://byte-engine.0x44491229.dev/docs/reference/besl/language#pass-and-return-arrays";

#[derive(Clone, Copy)]
enum AtomicAccessRequirement {
	Write,
	ReadWrite,
}

#[derive(Clone, Copy)]
enum AtomicTargetRoot {
	Binding { read: bool, write: bool },
	Workgroup,
}

/// Returns the access policy shared by every portable buffer and workgroup atomic intrinsic.
fn atomic_access_requirement(name: &str) -> Option<AtomicAccessRequirement> {
	match name {
		"atomic_store" => Some(AtomicAccessRequirement::Write),
		"atomic_load"
		| "atomic_exchange"
		| "atomic_compare_exchange"
		| "atomic_add"
		| "atomic_sub"
		| "atomic_min"
		| "atomic_max"
		| "atomic_and"
		| "atomic_or"
		| "atomic_xor" => Some(AtomicAccessRequirement::ReadWrite),
		_ => None,
	}
}

/// Peels indexing and member access without accepting a copied atomic value as addressable storage.
fn atomic_target_root(target: &NodeReference) -> Option<AtomicTargetRoot> {
	let next = {
		let target = target.borrow();
		match target.node() {
			Nodes::Binding { read, write, .. } => {
				return Some(AtomicTargetRoot::Binding {
					read: *read,
					write: *write,
				});
			}
			Nodes::Workgroup { .. } => return Some(AtomicTargetRoot::Workgroup),
			Nodes::Expression(Expressions::Member { source, .. }) => source.clone(),
			Nodes::Expression(Expressions::Accessor { left, .. }) => left.clone(),
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => elements[0].clone(),
			_ => return None,
		}
	};

	atomic_target_root(&next)
}

/// Rejects atomic calls that cannot preserve portable address-space and access semantics.
fn validate_atomic_target(name: &str, target: &NodeReference, requirement: AtomicAccessRequirement) -> Result<(), LexError> {
	let Some(root) = atomic_target_root(target) else {
		return Err(LexError::invalid(format!(
			"Atomic target must come directly from a buffer or workgroup. The most likely cause is that `{name}` received a local value, function parameter, or function result. See {ATOMIC_INTRINSICS_DOCUMENTATION}."
		)));
	};

	let AtomicTargetRoot::Binding { read, write } = root else {
		return Ok(());
	};
	let valid_access = match requirement {
		AtomicAccessRequirement::Write => write,
		AtomicAccessRequirement::ReadWrite => read && write,
	};
	if valid_access {
		return Ok(());
	}

	let message = match requirement {
		AtomicAccessRequirement::Write => format!(
			"Atomic store requires a writable buffer. The most likely cause is that the descriptor used by `{name}` does not include write access. See {ATOMIC_INTRINSICS_DOCUMENTATION}."
		),
		AtomicAccessRequirement::ReadWrite => format!(
			"Atomic operation requires a read-write buffer. The most likely cause is that the descriptor used by `{name}` does not use `read_write` access. See {ATOMIC_INTRINSICS_DOCUMENTATION}."
		),
	};
	Err(LexError::invalid(message))
}

/// Reports whether `name` is the wrapper member of the lowered fixed-array binding that `left` references.
fn is_fixed_array_alias(left: &NodeReference, name: &str) -> bool {
	let left = left.borrow();
	let Nodes::Expression(Expressions::Member { source, .. }) = left.node() else {
		return false;
	};
	matches!(
		source.borrow().node(),
		Nodes::Binding {
			r#type: BindingTypes::BufferArray { fixed: Some(fixed), .. },
			..
		} if fixed.alias == name
	)
}

/// Selects the memory class of a binding from its declared `constant` or `device` keyword, the same way for bindings
/// declared in source and bindings built by engine code. Buffers default to device memory. Other resources have no
/// buffer memory, so they reject a declared class.
fn binding_memory_class(
	name: &str,
	r#type: &BindingTypes,
	declared: Option<&str>,
	write: bool,
) -> Result<BufferMemoryClass, LexError> {
	let is_buffer = matches!(r#type, BindingTypes::Buffer { .. } | BindingTypes::BufferArray { .. });
	let memory_class = match (is_buffer, declared) {
		(true, Some("constant")) => BufferMemoryClass::Constant,
		(true, Some("device") | None) => BufferMemoryClass::Device,
		(true, Some(class)) => {
			return Err(LexError::invalid(format!(
				"Invalid buffer memory class `{class}` for descriptor {name}. The most likely cause is that the descriptor does not use constant or device memory."
			)));
		}
		(false, None) => BufferMemoryClass::Constant,
		(false, Some(_)) => {
			return Err(LexError::invalid(format!(
				"Descriptor {name} declares a buffer memory class for a non-buffer resource. The most likely cause is that constant or device was attached to an image or texture descriptor."
			)));
		}
	};
	if write && is_buffer && memory_class == BufferMemoryClass::Constant {
		return Err(LexError::invalid(format!(
			"Writable buffer descriptor {name} uses constant memory. The most likely cause is that a writable buffer needs the device memory class."
		)));
	}
	Ok(memory_class)
}

/// Rejects `break` and `continue` outside a loop, as Rust does. It checks the parsed tree, because lexing drops
/// match arms that can never run, and those arms must still be valid code.
fn validate_loop_control(statements: &[parser::Node], in_loop: bool) -> Result<(), LexError> {
	for statement in statements {
		match statement.node() {
			parser::Nodes::Expression(parser::Expressions::Break | parser::Expressions::Continue) if !in_loop => {
				return Err(LexError::invalid(
					"`break` or `continue` outside a loop. The most likely cause is a `break` or `continue` in a function body, branch, or match arm without an enclosing `for` loop.",
				));
			}
			parser::Nodes::Conditional {
				statements, else_branch, ..
			} => {
				validate_loop_control(statements, in_loop)?;
				match else_branch {
					Some(parser::ElseBranch::Block(statements)) => validate_loop_control(statements, in_loop)?,
					Some(parser::ElseBranch::If(conditional)) => {
						validate_loop_control(std::slice::from_ref(conditional), in_loop)?
					}
					None => {}
				}
			}
			parser::Nodes::Match { arms, .. } => {
				for arm in arms {
					validate_loop_control(&arm.statements, in_loop)?;
				}
			}
			parser::Nodes::ForLoop { statements, .. } => validate_loop_control(statements, true)?,
			_ => {}
		}
	}

	Ok(())
}

/// The `Lexer` struct carries the state that linking one program threads through every parsed node: the lexical
/// scope chain that name lookups search, and the counter that keeps inlined intrinsic locals unique.
///
/// Create it with [`Lexer::new`] over the program root, then call [`Lexer::lex`] for each top-level declaration.
pub(super) struct Lexer {
	/// The enclosing declarations and earlier statements visible to the node being lexed, innermost last.
	scopes: Vec<NodeReference>,
	next_intrinsic_expansion_id: usize,
}

impl Lexer {
	/// Starts linking declarations whose names resolve against `root`.
	pub(super) fn new(root: NodeReference) -> Self {
		Self {
			scopes: vec![root],
			next_intrinsic_expansion_id: 0,
		}
	}

	/// Runs `f` with `parent` as the innermost scope, then restores the scope chain, even when `f` fails.
	fn in_scope<R>(&mut self, parent: &NodeReference, f: impl FnOnce(&mut Self) -> R) -> R {
		let length = self.scopes.len();
		self.scopes.push(parent.clone());
		let result = f(self);
		self.scopes.truncate(length);
		result
	}

	/// Lexes the statements of a block in order. Each statement can see the current scope and the statements before
	/// it, and none of them stays visible after the block.
	fn lex_block(&mut self, statements: &[parser::Node]) -> Result<Vec<NodeReference>, LexError> {
		let length = self.scopes.len();
		let result = statements
			.iter()
			.map(|statement| {
				let statement = self.lex(statement)?;
				self.scopes.push(statement.clone());
				Ok(statement)
			})
			.collect();
		self.scopes.truncate(length);
		result
	}

	/// Lexes the children of `this` in its scope and appends each one to it as it is linked.
	fn lex_children(&mut self, this: &NodeReference, children: &[parser::Node]) -> Result<(), LexError> {
		self.in_scope(this, |lexer| {
			for child in children {
				let child = lexer.lex(child)?;
				this.borrow_mut().add_child(child);
			}
			Ok(())
		})
	}

	/// Links one parsed node and its subtree against the current scope chain.
	// This exhaustive parser-to-lexer boundary keeps each source node variant's lowering beside the others.
	#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
	pub(super) fn lex(&mut self, parser_node: &parser::Node) -> Result<NodeReference, LexError> {
		let node = match &parser_node.node {
			parser::Nodes::Scope { name, children } => {
				assert_ne!(*name, "root"); // The root scope node cannot be an inner part of the program.

				let this: NodeReference = Node::scope(name.to_string()).into();
				self.lex_children(&this, children)?;
				this
			}
			parser::Nodes::Struct { name, fields } => {
				if let Some(n) = get_reference(&self.scopes, name) {
					// If the type already exists, return it.
					return Ok(n);
				}

				let this: NodeReference = Node::r#struct(name, Vec::new()).into();
				self.lex_children(&this, fields)?;
				this
			}
			parser::Nodes::Specialization { name, r#type } => {
				let t = resolve_type(&self.scopes, r#type)?;

				let this = Node::new(Nodes::Specialization {
					name: name.to_string(),
					r#type: t,
				});

				this.into()
			}
			parser::Nodes::Member { name, r#type } => {
				let t = if r#type.contains('<') {
					let mut s = r#type.split(['<', '>']);

					let outer_type_name = s.next().ok_or(LexError::invalid("No outer name"))?;

					let outer_type = resolve_type(&self.scopes, outer_type_name)?;

					let inner_type_name = s.next().ok_or(LexError::invalid("No inner name"))?;

					let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
						NodeReference::from(Node {
							node: Nodes::Struct {
								name: format!("{}*", stripped),
								template: Some(outer_type.clone()),
								fields: Vec::new(),
								types: Vec::new(),
							},
						})
					} else {
						resolve_type(&self.scopes, inner_type_name)?
					};

					if let Some(n) = get_reference(&self.scopes, r#type) {
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

					let type_name = s.next().ok_or(LexError::invalid("No type name"))?;

					let member_type = resolve_type(&self.scopes, type_name)?;

					let count = s
						.next()
						.ok_or(LexError::invalid("No count"))?
						.parse()
						.map_err(|_| LexError::invalid("Invalid count"))?;

					return Ok(Node::array(name, member_type, count));
				} else {
					resolve_type(&self.scopes, r#type)?
				};

				let this: NodeReference = Node::member(name, t).into();

				this
			}
			parser::Nodes::Parameter { name, r#type } => {
				let t = resolve_type_name(&self.scopes, r#type)?;

				let this = Node::new(Nodes::Parameter {
					name: name.to_string(),
					r#type: t,
				});

				this.into()
			}
			parser::Nodes::Input { name, format, location } => {
				let t = resolve_type(&self.scopes, format)?;

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
				per_vertex,
			} => {
				let t = resolve_type(&self.scopes, format)?;

				let this = Node::new(Nodes::Output {
					name: name.to_string(),
					format: t,
					location: *location,
					count: *count,
					per_vertex: *per_vertex,
				});

				this.into()
			}
			parser::Nodes::TaskPayload { name, format, count } => {
				let format = resolve_type(&self.scopes, format)?;
				Node::new(Nodes::TaskPayload {
					name: name.to_string(),
					format,
					count: *count,
				})
				.into()
			}
			parser::Nodes::Workgroup { name, format, count } => {
				let format = resolve_type(&self.scopes, format)?;
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
				validate_loop_control(statements, false)?;
				validate_return_type(name, return_type)?;
				let t = resolve_type_name(&self.scopes, return_type)?;

				let this: NodeReference = Node::function(name, Vec::new(), t, Vec::new()).into();

				self.in_scope(&this, |lexer| {
					for param in params {
						let param = lexer.lex(param)?;
						let mut function = this.borrow_mut();
						let Nodes::Function { params, .. } = function.node_mut() else {
							unreachable!("The node was built as a function above");
						};
						params.push(param);
					}

					// Each statement is added to the function as it is linked and also stays in scope for later ones.
					for statement in statements {
						let statement = lexer.lex(statement)?;
						this.borrow_mut().add_child(statement.clone());
						lexer.scopes.push(statement);
					}
					Ok::<_, LexError>(())
				})?;

				this
			}
			parser::Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => {
				let condition = self.lex(condition)?;
				// Each branch gets its own scope, so declarations in one branch are not visible in the other.
				let statements = self.lex_block(statements)?;
				let else_branch = match else_branch {
					Some(parser::ElseBranch::Block(statements)) => Some(ElseBranch::Block(self.lex_block(statements)?)),
					Some(parser::ElseBranch::If(conditional)) => Some(ElseBranch::If(self.lex(conditional)?)),
					None => None,
				};

				Node::conditional(condition, statements, else_branch).into()
			}
			parser::Nodes::Match { scrutinee, arms } => {
				let scrutinee = self.lex(scrutinee)?;
				let r#type = infer_expression_type(&scrutinee);
				let domain = matching::MatchDomain::of(r#type.as_ref())?;

				// Every arm is lexed, even an unreachable one, so its errors surface as they do in Rust.
				// Each arm gets its own scope, so declarations in one arm are not visible in the others.
				let arms = arms
					.iter()
					.map(|arm| {
						let values = arm.patterns.iter().map(|pattern| domain.pattern_value(pattern));
						let statements = self.lex_block(&arm.statements)?;
						Ok((values.collect::<Result<_, _>>()?, statements))
					})
					.collect::<Result<_, LexError>>()?;

				let (arms, default) = matching::normalize_arms(domain, arms)?;
				let r#type = r#type.expect("A match domain always comes from a known type");
				Node::r#match(scrutinee, r#type, arms, default).into()
			}
			parser::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let initializer = self.lex(initializer)?;
				// The loop variable is visible to the condition, the update, and the body, but not after the loop.
				let (condition, update, statements) = self.in_scope(&initializer, |lexer| {
					Ok::<_, LexError>((lexer.lex(condition)?, lexer.lex(update)?, lexer.lex_block(statements)?))
				})?;

				Node::for_loop(initializer, condition, update, statements).into()
			}
			parser::Nodes::PushConstant { members } => {
				let this: NodeReference = Node::push_constant(vec![]).into();

				self.in_scope(&this, |lexer| {
					for member in members
						.iter()
						.filter(|member| matches!(member.node, parser::Nodes::Member { .. }))
					{
						let member = lexer.lex(member)?;
						this.borrow_mut().add_child(member);
					}
					Ok::<_, LexError>(())
				})?;

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
				let r#type = match r#type {
					parser::BindingResource::Buffer { members } => BindingTypes::Buffer {
						members: members
							.iter()
							.map(|member| self.lex(member))
							.collect::<Result<Vec<NodeReference>, LexError>>()?,
					},
					parser::BindingResource::Image { format } => BindingTypes::Image {
						format: format.to_string(),
					},
					parser::BindingResource::CombinedImageSampler { format } => BindingTypes::CombinedImageSampler {
						format: format.to_string(),
					},
				};
				let declared = memory_class.map(|memory_class| match memory_class {
					BufferMemoryClass::Constant => "constant",
					BufferMemoryClass::Device => "device",
				});
				let memory_class = binding_memory_class(name, &r#type, declared, *write)?;

				match count {
					Some(count) => Node::binding_array_in_memory(name, r#type, *slot, *read, *write, memory_class, count.get()),
					None => Node::binding_in_memory(name, r#type, *slot, *read, *write, memory_class),
				}
				.into()
			}
			parser::Nodes::Descriptor {
				name,
				resource_type,
				format,
				runtime_array,
				slot,
				read,
				write,
				memory_class,
				count,
			} => {
				let r#type = resolve_descriptor_type(&self.scopes, resource_type, *format, *runtime_array)?;
				let memory_class = binding_memory_class(name, &r#type, *memory_class, *write)?;

				Node::binding_with_count(name, r#type, *slot, *read, *write, memory_class, *count).into()
			}
			parser::Nodes::RawCode {
				glsl,
				hlsl,
				msl,
				input,
				output,
				..
			} => lex_raw_code(&self.scopes, glsl.as_deref(), hlsl.as_deref(), msl.as_deref(), input, output)?.into(),
			parser::Nodes::Expression(expression) => {
				let this = match expression {
					parser::Expressions::Return { value } => Node::expression(Expressions::Return {
						value: match value {
							Some(value) => Some(self.lex(value)?),
							None => None,
						},
					}),
					parser::Expressions::Continue => Node::expression(Expressions::Continue),
					parser::Expressions::Break => Node::expression(Expressions::Break),
					parser::Expressions::Discard => Node::expression(Expressions::Discard),
					parser::Expressions::Accessor { left, right } => {
						let left = self.lex(left)?;
						// `binding.alias` on a lowered fixed array names the binding's own elements, so drop the hop.
						if let parser::Nodes::Expression(parser::Expressions::Member { name }) = &right.node
							&& is_fixed_array_alias(&left, name)
						{
							return Ok(left);
						}

						// A name after `.` lives in the member namespace of the left side's type, so a local,
						// binding, or field with the same name elsewhere in scope never shadows it.
						// An index after `[` is an ordinary expression in the enclosing scope.
						let right = match &right.node {
							parser::Nodes::Expression(parser::Expressions::Member { name }) => {
								Node::expression(Expressions::Member {
									source: resolve_accessed_member(&left, name)?,
									name: name.to_string(),
								})
								.into()
							}
							_ => self.lex(right)?,
						};
						if super::resolution::is_array_texture_reference(&left)
							&& !super::resolution::infer_expression_type(&right)
								.is_some_and(|r#type| r#type.borrow().get_name() == Some("u32"))
						{
							return Err(LexError::invalid(
								"Texture2DArray layer index must be u32. The most likely cause is that the indexed expression has another numeric type.",
							));
						}

						Node::expression(Expressions::Accessor { left, right })
					}
					parser::Expressions::Member { name } => {
						let source = resolve_member(&self.scopes, name)?;
						// Functions are not values. A function body that names its own function would also hold a strong
						// reference to its ancestor, which forms an `Rc` cycle that is never freed.
						if matches!(source.borrow().node(), Nodes::Function { .. }) {
							return Err(LexError::invalid(format!(
								"Function `{name}` can't be used as a value. The most likely cause is a missing `()` after the function name."
							)));
						}
						Node::expression(Expressions::Member {
							source,
							name: name.to_string(),
						})
					}
					parser::Expressions::Literal { value } => Node::expression(Expressions::Literal {
						value: value.to_string(),
					}),
					parser::Expressions::RecordLiteral { .. } => {
						return Err(LexError::invalid(
							"Record literals are valid only as structural main return values. The most likely cause is that a record value escaped entry-point normalization.",
						));
					}
					parser::Expressions::Expression(elements) => Node {
						node: Nodes::Expression(Expressions::Expression {
							elements: elements
								.iter()
								.map(|element| self.lex(element))
								.collect::<Result<Vec<NodeReference>, LexError>>()?,
						}),
					},
					parser::Expressions::Call { name, parameters } => {
						let parameters = parameters
							.iter()
							.map(|parameter| self.lex(parameter))
							.collect::<Result<Vec<NodeReference>, LexError>>()?;
						let function = resolve_call_target(&self.scopes, name, &parameters)?;
						let r = function.clone(); // Clone to be able to borrow it in and return it

						{
							// Validate function call
							let b = RefCell::borrow(&function.0);
							match b.node() {
								Nodes::Function { params, .. } | Nodes::Struct { fields: params, .. } => {
									if params.len() != parameters.len() {
										return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
									}
									Node::expression(Expressions::FunctionCall {
										function: r.into(),
										parameters,
									})
								}
								Nodes::Intrinsic { name, elements, .. } => {
									if let Some(requirement) = atomic_access_requirement(name)
										&& let Some(target) = parameters.first()
									{
										validate_atomic_target(name, target, requirement)?;
									}
									Node::expression(Expressions::IntrinsicCall {
										intrinsic: r,
										arguments: parameters.clone(),
										elements: {
											let expansion_id = self.next_intrinsic_expansion_id;
											self.next_intrinsic_expansion_id = expansion_id.checked_add(1).expect(
												"Intrinsic expansion count overflowed. The most likely cause is an invalid shader with too many intrinsic calls.",
											);
											build_intrinsic(elements, &parameters, expansion_id)?
										},
									})
								}
								_ => {
									return Err(LexError::invalid(
										"Encountered parsing error while evaluating function call. Expected Function | Struct | Intrinsic, but found other.",
									));
								}
							}
						}
					}
					parser::Expressions::Operator { operator, left, right } => Node::expression(Expressions::Operator {
						operator: *operator,
						left: self.lex(left)?,
						right: self.lex(right)?,
					}),
					parser::Expressions::VariableDeclaration { name, r#type } => {
						Node::expression(Expressions::VariableDeclaration {
							name: name.to_string(),
							r#type: resolve_type_name(&self.scopes, r#type)?,
						})
					}
					parser::Expressions::RawCode {
						glsl,
						hlsl,
						msl,
						input,
						output,
					} => lex_raw_code(&self.scopes, *glsl, *hlsl, *msl, input, output)?,
					parser::Expressions::Macro { name, body } => Node::r#macro(name, self.lex(body)?),
				};

				this.into()
			}
			parser::Nodes::Intrinsic {
				name,
				elements,
				r#return,
				..
			} => {
				let this: NodeReference = Node::intrinsic(name, Vec::new(), resolve_type(&self.scopes, r#return)?).into();
				self.lex_children(&this, elements)?;
				this
			}
			parser::Nodes::Const { name, r#type, value } => {
				let t = resolve_type_name(&self.scopes, r#type)?;

				let v = self.lex(value)?;

				Node::constant(name, t, v).into()
			}
		};

		Ok(node)
	}
}

/// Rejects a function that returns an array of more than four elements or of non-scalar elements.
///
/// HLSL can't return arrays, so backends return only short `f32`, `u16`, and `u32` arrays, which they carry as vectors.
fn validate_return_type(name: &str, return_type: &parser::TypeName) -> Result<(), LexError> {
	let parser::TypeName::Array { element, count } = return_type else {
		return Ok(());
	};
	let compact = (2..=4).contains(count) && matches!(**element, parser::TypeName::Named("f32" | "u16" | "u32"));
	if compact {
		return Ok(());
	}
	Err(LexError::invalid(format!(
		"Function `{name}` can't return an array of {count} elements. The most likely cause is a function that returns a larger array; only arrays of two to four `f32`, `u16`, or `u32` values can be returned, so return a vector or struct with the values the caller needs instead. See {ARRAY_DOCUMENTATION}."
	)))
}
