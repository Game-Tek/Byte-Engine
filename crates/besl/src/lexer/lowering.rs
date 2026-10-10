use super::resolution::*;
use super::*;
use crate::parser;

mod yielding;

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

	match (root, requirement) {
		(AtomicTargetRoot::Binding { write: false, .. }, AtomicAccessRequirement::Write) => Err(LexError::invalid(format!(
			"Atomic store requires a writable buffer. The most likely cause is that the descriptor used by `{name}` does not include write access. See {ATOMIC_INTRINSICS_DOCUMENTATION}."
		))),
		(AtomicTargetRoot::Binding { read, write }, AtomicAccessRequirement::ReadWrite) if !(read && write) => {
			Err(LexError::invalid(format!(
				"Atomic operation requires a read-write buffer. The most likely cause is that the descriptor used by `{name}` does not use `read_write` access. See {ATOMIC_INTRINSICS_DOCUMENTATION}."
			)))
		}
		_ => Ok(()),
	}
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

/// Selects the memory class of a binding from its declared `constant` or `device` class, the same way for bindings
/// declared in source and bindings built by engine code. Buffers default to device memory. Other resources have no
/// buffer memory, so they reject a declared class.
fn binding_memory_class(
	name: &str,
	r#type: &BindingTypes,
	declared: Option<BufferMemoryClass>,
	write: bool,
) -> Result<BufferMemoryClass, LexError> {
	let is_buffer = matches!(r#type, BindingTypes::Buffer { .. } | BindingTypes::BufferArray { .. });
	let memory_class = match (is_buffer, declared) {
		(true, declared) => declared.unwrap_or(BufferMemoryClass::Device),
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

/// The `Lexer` struct carries the state that linking one program threads through every parsed node: the lexical
/// scope chain that name lookups search, the statements that `if` and `match` values hoist in front of the statement
/// being linked, and the counter that keeps generated locals unique.
///
/// Create it with [`Lexer::new`] over the program root, then call [`Lexer::lex`] for each top-level declaration.
pub(super) struct Lexer {
	/// The enclosing declarations and earlier statements visible to the node being lexed, innermost last.
	scopes: Vec<NodeReference>,
	/// Statements that must run before the statement being lexed, in order. `None` where nothing can run first: outside
	/// function bodies, and in a `for` condition or update, which run on every iteration. See [`yielding`].
	hoisted: Option<Vec<NodeReference>>,
	/// How many `for` loops enclose the node being lexed, so `break` and `continue` outside one are rejected.
	loop_depth: usize,
	/// Numbers inlined intrinsic locals and hoisted temporaries.
	next_generated_id: usize,
}

impl Lexer {
	/// Starts linking declarations whose names resolve against `root`.
	pub(super) fn new(root: NodeReference) -> Self {
		Self {
			scopes: vec![root],
			hoisted: None,
			loop_depth: 0,
			next_generated_id: 0,
		}
	}

	/// Returns a new number for a generated local name.
	fn generated_id(&mut self) -> usize {
		let id = self.next_generated_id;
		self.next_generated_id = id
			.checked_add(1)
			.expect("Generated local count overflowed. The most likely cause is an invalid shader with too many intrinsic calls or `if` and `match` values.");
		id
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
	/// it, and none of them stays visible after the block. Statements hoisted by `if` and `match` values come right
	/// before the statement that uses them.
	fn lex_block(&mut self, statements: &[parser::Node]) -> Result<Vec<NodeReference>, LexError> {
		let length = self.scopes.len();
		let mut block = Vec::with_capacity(statements.len());
		let result = statements.iter().try_for_each(|statement| {
			let statement = self.lex_statement(statement, &mut block)?;
			self.scopes.push(statement);
			Ok(())
		});
		self.scopes.truncate(length);
		result.map(|()| block)
	}

	/// Lexes one statement into `out`, after the statements its `if` and `match` values hoisted, and returns the
	/// statement so the caller can make its declarations visible to later statements.
	fn lex_statement(&mut self, statement: &parser::Node, out: &mut Vec<NodeReference>) -> Result<NodeReference, LexError> {
		let enclosing = self.hoisted.replace(Vec::new());
		let lexed = match &statement.node {
			parser::Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => self.lex_conditional_statement(condition, statements, else_branch.as_ref()),
			parser::Nodes::Match { scrutinee, arms } => self.lex_match_statement(scrutinee, arms),
			parser::Nodes::Expression(parser::Expressions::Yield { value }) => self.lex_final_statement(value),
			_ => self.lex(statement),
		};
		let hoisted = std::mem::replace(&mut self.hoisted, enclosing).unwrap_or_default();
		let lexed = lexed?;
		out.extend(hoisted);
		out.push(lexed.clone());
		Ok(lexed)
	}

	/// Lexes an `if` statement. Each branch gets its own scope, so declarations in one branch are not visible in the
	/// other.
	fn lex_conditional_statement(
		&mut self,
		condition: &parser::Node,
		statements: &[parser::Node],
		else_branch: Option<&parser::ElseBranch>,
	) -> Result<NodeReference, LexError> {
		let condition = self.lex(condition)?;
		let statements = self.lex_block(statements)?;
		let else_branch = match else_branch {
			Some(parser::ElseBranch::Block(statements)) => Some(ElseBranch::Block(self.lex_block(statements)?)),
			// The link's condition runs only when this one fails, so statements it hoists stay inside the `else`.
			Some(parser::ElseBranch::If(link)) => {
				let mut block = Vec::with_capacity(1);
				let link = self.lex_statement(link, &mut block)?;
				Some(if block.len() == 1 {
					ElseBranch::If(link)
				} else {
					ElseBranch::Block(block)
				})
			}
			None => None,
		};

		Ok(Node::conditional(condition, statements, else_branch).into())
	}

	/// Lexes a `match` statement. Each arm gets its own scope, so declarations in one arm are not visible in the others.
	fn lex_match_statement(&mut self, scrutinee: &parser::Node, arms: &[parser::MatchArm]) -> Result<NodeReference, LexError> {
		let (scrutinee, r#type, domain, arms) = self.lex_match_parts(scrutinee, arms, Self::lex_block)?;
		let (arms, default) = matching::normalize_arms(domain, arms)?;
		Ok(Node::r#match(scrutinee, r#type, arms, default).into())
	}

	/// Lexes a `match`'s scrutinee, then each arm's body with `lex_arm`, and returns them with the scrutinee's type and
	/// value domain. Every arm is lexed, even an unreachable one, so its errors surface as they do in Rust.
	#[allow(clippy::type_complexity)]
	fn lex_match_parts<T>(
		&mut self,
		scrutinee: &parser::Node,
		arms: &[parser::MatchArm],
		mut lex_arm: impl FnMut(&mut Self, &[parser::Node]) -> Result<T, LexError>,
	) -> Result<
		(
			NodeReference,
			NodeReference,
			matching::MatchDomain,
			Vec<(Vec<Option<i64>>, T)>,
		),
		LexError,
	> {
		let scrutinee = self.lex(scrutinee)?;
		let r#type = infer_expression_type(&scrutinee);
		let domain = matching::MatchDomain::of(r#type.as_ref())?;
		let arms = arms
			.iter()
			.map(|arm| {
				let values = arm.patterns.iter().map(|pattern| domain.pattern_value(pattern));
				let body = lex_arm(self, &arm.statements)?;
				Ok((values.collect::<Result<_, _>>()?, body))
			})
			.collect::<Result<_, LexError>>()?;
		let r#type = r#type.expect("A match domain always comes from a known type");
		Ok((scrutinee, r#type, domain, arms))
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

				// Lex the fields before the struct joins the scope chain. A field typed as its own struct would
				// otherwise hold a strong reference back to the struct, an `Rc` cycle that is never freed.
				let fields = fields
					.iter()
					.map(|field| {
						self.lex(field).map_err(|error| match error {
							LexError::ReferenceToUndefinedType { type_name } if type_name == *name => LexError::invalid(format!(
								"Struct `{name}` contains itself. The most likely cause is a field whose type is `{name}`, which would make the struct infinitely large."
							)),
							error => error,
						})
					})
					.collect::<Result<Vec<_>, _>>()?;
				Node::r#struct(name, fields).into()
			}
			parser::Nodes::Specialization { name, r#type, id } => {
				Node::specialization(name, resolve_type(&self.scopes, r#type)?, *id).into()
			}
			parser::Nodes::Member { name, r#type } => {
				if r#type.contains('<') {
					let mut s = r#type.split(['<', '>']);
					let outer_type_name = s.next().ok_or_else(|| LexError::invalid("No outer name"))?;
					let outer_type = resolve_type(&self.scopes, outer_type_name)?;
					let inner_type_name = s.next().ok_or_else(|| LexError::invalid("No inner name"))?;
					let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
						NodeReference::from(Node::new(Nodes::Struct {
							name: format!("{}*", stripped),
							template: Some(outer_type.clone()),
							fields: Vec::new(),
							types: Vec::new(),
						}))
					} else {
						resolve_type(&self.scopes, inner_type_name)?
					};

					if let Some(n) = get_reference(&self.scopes, r#type) {
						// If the specialized generic type already exists, return it.
						return Ok(n);
					}

					return Ok(Node::new(Nodes::Struct {
						name: r#type.to_string(),
						template: Some(outer_type),
						fields: Vec::new(),
						types: vec![inner_type],
					})
					.into());
				}
				if r#type.contains('[') {
					let mut s = r#type.split(['[', ']']);
					let type_name = s.next().ok_or_else(|| LexError::invalid("No type name"))?;
					let member_type = resolve_type(&self.scopes, type_name)?;
					let count = s
						.next()
						.ok_or_else(|| LexError::invalid("No count"))?
						.parse()
						.map_err(|_| LexError::invalid("Invalid count"))?;
					return Ok(Node::array(name, member_type, count));
				}

				Node::member(name, resolve_type(&self.scopes, r#type)?).into()
			}
			parser::Nodes::Parameter { name, r#type } => Node::new(Nodes::Parameter {
				name: name.to_string(),
				r#type: resolve_type_name(&self.scopes, r#type)?,
			})
			.into(),
			parser::Nodes::Input { name, format, location } => {
				Node::input(name, resolve_type(&self.scopes, format)?, *location).into()
			}
			parser::Nodes::Output {
				name,
				format,
				location,
				count,
				per_vertex,
			} => Node::output_array(name, resolve_type(&self.scopes, format)?, *location, *count, *per_vertex).into(),
			parser::Nodes::TaskPayload { name, format, count } => {
				Node::task_payload(name, resolve_type(&self.scopes, format)?, *count).into()
			}
			parser::Nodes::Workgroup { name, format, count } => {
				Node::workgroup(name, resolve_type(&self.scopes, format)?, *count).into()
			}
			parser::Nodes::Function {
				name,
				return_type,
				statements,
				params,
				..
			} => {
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

					let statements = lexer.lex_block(statements)?;
					this.borrow_mut().add_children(statements);
					Ok::<_, LexError>(())
				})?;

				this
			}
			// Statements reach `lex_statement`, so an `if` or `match` here is an operand that yields a value.
			parser::Nodes::Conditional { .. } | parser::Nodes::Match { .. } => {
				return self.lex_branch_value(parser_node, None, false);
			}
			parser::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let initializer = self.lex(initializer)?;
				// The loop variable is visible to the condition, the update, and the body, but not after the loop. The
				// condition and update run on every iteration, so nothing can be hoisted in front of them.
				let enclosing = self.hoisted.take();
				let header = self.in_scope(&initializer, |lexer| {
					Ok::<_, LexError>((lexer.lex(condition)?, lexer.lex(update)?))
				});
				self.hoisted = enclosing;
				let (condition, update) = header?;

				self.loop_depth += 1;
				let statements = self.in_scope(&initializer, |lexer| lexer.lex_block(statements));
				self.loop_depth -= 1;

				Node::for_loop(initializer, condition, update, statements?).into()
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
				let memory_class = binding_memory_class(name, &r#type, *memory_class, *write)?;
				Node::binding_in_memory(name, r#type, *slot, *read, *write, memory_class, *count).into()
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

				Node::binding_in_memory(name, r#type, *slot, *read, *write, memory_class, *count).into()
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
					parser::Expressions::Return { value } => {
						let expected = self.enclosing_return_type();
						Node::expression(Expressions::Return {
							value: value.as_deref().map(|value| self.lex_value(value, expected)).transpose()?,
						})
					}
					parser::Expressions::Continue | parser::Expressions::Break if self.loop_depth == 0 => {
						return Err(LexError::invalid(
							"`break` or `continue` outside a loop. The most likely cause is a `break` or `continue` in a function body, branch, or match arm without an enclosing `for` loop.",
						));
					}
					parser::Expressions::Continue => Node::expression(Expressions::Continue),
					parser::Expressions::Break => Node::expression(Expressions::Break),
					parser::Expressions::Discard => Node::expression(Expressions::Discard),
					parser::Expressions::Accessor { left, right } => {
						// Backends tell indexing from member access by the base's declared type, which a `?:` doesn't
						// have, so an `if` or `match` base becomes a local.
						let mut left = match yielding::branch_operand(left) {
							Some(branch) => self.lex_branch_value(branch, None, true)?,
							None => self.lex(left)?,
						};
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
							_ => {
								let mut at = self.hoisted_len();
								let right = self.lex(right)?;
								if self.hoisted_len() > at {
									left = self.capture_base(left, &mut at)?;
								}
								right
							}
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
					parser::Expressions::Expression(elements) => Node::expression(Expressions::Expression {
						elements: elements
							.iter()
							.map(|element| self.lex(element))
							.collect::<Result<Vec<NodeReference>, LexError>>()?,
					}),
					parser::Expressions::Call { name, parameters } => {
						let parameters = self.lex_arguments(name, parameters)?;
						let function = resolve_call_target(&self.scopes, name, &parameters)?;
						let callee = function.borrow();
						match callee.node() {
							Nodes::Function { params, .. } | Nodes::Struct { fields: params, .. } => {
								if params.len() != parameters.len() {
									return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
								}
								Node::expression(Expressions::FunctionCall {
									function: function.clone().into(),
									parameters,
								})
							}
							Nodes::Intrinsic { name, elements, .. } => {
								if let Some(requirement) = atomic_access_requirement(name)
									&& let Some(target) = parameters.first()
								{
									validate_atomic_target(name, target, requirement)?;
								}
								let expansion_id = self.generated_id();
								let elements = build_intrinsic(elements, &parameters, expansion_id)?;
								Node::expression(Expressions::IntrinsicCall {
									intrinsic: function.clone(),
									arguments: parameters,
									elements,
								})
							}
							_ => {
								return Err(LexError::invalid(
									"Encountered parsing error while evaluating function call. Expected Function | Struct | Intrinsic, but found other.",
								));
							}
						}
					}
					parser::Expressions::Operator { operator, left, right } => {
						let mut left = self.lex(left)?;
						let mut at = self.hoisted_len();
						let right = match operator {
							// The value knows its type from the target, as in a typed `let`.
							Operators::Assignment => self.lex_value(right, infer_expression_type(&left))?,
							Operators::LogicalAnd | Operators::LogicalOr => {
								// The right side runs only when the left one doesn't decide the result, so statements it
								// hoists go under a guard instead of in front of the statement.
								let enclosing = self.hoisted.as_ref().map(|_| Vec::new());
								let enclosing = std::mem::replace(&mut self.hoisted, enclosing);
								let right = self.lex(right);
								let guarded = std::mem::replace(&mut self.hoisted, enclosing);
								let right = right?;
								if let Some(guarded) = guarded.filter(|guarded| !guarded.is_empty()) {
									return self.short_circuit(*operator, left, guarded, right);
								}
								right
							}
							_ => self.lex(right)?,
						};
						// The left side ran before the statements the right side hoisted. An assignment's target keeps its
						// storage, so only its indices are captured.
						if self.hoisted_len() > at {
							if *operator == Operators::Assignment {
								self.capture_place(&left, &mut at)?;
							} else {
								left = self.capture(left, &mut at)?;
							}
						}
						Node::expression(Expressions::Operator {
							operator: *operator,
							left,
							right,
						})
					}
					parser::Expressions::Unary { operator, operand } => Node::expression(Expressions::Unary {
						operator: *operator,
						operand: self.lex(operand)?,
					}),
					parser::Expressions::Yield { .. } => {
						return Err(LexError::invalid(
							"A value without `;` can only end a block. The most likely cause is a parser change that placed a block value elsewhere.",
						));
					}
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
				Node::constant(name, resolve_type_name(&self.scopes, r#type)?, self.lex(value)?).into()
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
