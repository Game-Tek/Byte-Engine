//! BESL analysis and lowering into executable VM instructions.

use std::collections::{HashMap, HashSet};

use super::*;

/// Compiles one lexed program while keeping compiler implementation details behind this seam.
#[allow(clippy::mutable_key_type)]
pub(super) fn compile(program: NodeReference, specializations: &SpecializationValues) -> Result<ExecutableProgram, VmError> {
	let main = resolve_main_function(&program)?;
	let main_signature = extract_function_signature(&main)?;
	if !main_signature.params.is_empty() {
		return Err(VmError::UnsupportedMainSignature {
			message: "Main functions with parameters are not supported".to_string(),
		});
	}
	if main_signature.return_type.is_some() {
		return Err(VmError::UnsupportedMainSignature {
			message: format!(
				"Main functions must return void, but found `{}`",
				main_signature.return_type.as_ref().map(ValueType::name).unwrap_or("void")
			),
		});
	}

	let function_nodes = collect_functions(&main);
	for function in &function_nodes {
		reject_raw_code_nodes(function)?;
	}
	// NodeReference hashing is pointer-identity based, so function lookup remains stable for RefCell-backed nodes.
	let mut function_ids = HashMap::new();
	for (index, function) in function_nodes.iter().enumerate() {
		function_ids.insert(function.clone(), index);
	}

	let mut descriptor_layouts = HashMap::new();
	let mut functions = Vec::with_capacity(function_nodes.len());
	for function in &function_nodes {
		functions.push(Compiler::compile_function(
			function,
			&function_ids,
			&mut descriptor_layouts,
			specializations,
		)?);
	}

	Ok(ExecutableProgram {
		descriptor_layouts,
		functions,
		main_function: function_ids[&main],
	})
}

/// Rejects malformed linked calls before any argument is indexed or lowered.
fn require_argument_count(arguments: &[NodeReference], expected: usize) -> Result<(), VmError> {
	let found = arguments.len();
	if found != expected {
		return Err(VmError::CallArgumentMismatch { expected, found });
	}
	Ok(())
}

/// The `Compiler` struct lowers one BESL function into bounded register-machine instructions.
struct Compiler<'a> {
	function_ids: &'a HashMap<NodeReference, usize>,
	specializations: &'a SpecializationValues,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: HashMap<NodeReference, usize>,
	register_count: usize,
	return_type: Option<ValueType>,
	parameter_count: usize,
	loop_continue_targets: Vec<usize>,
	loop_continue_patches: Vec<Vec<usize>>,
}

impl<'a> Compiler<'a> {
	#[allow(clippy::mutable_key_type)]
	fn compile_function(
		function: &NodeReference,
		function_ids: &'a HashMap<NodeReference, usize>,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
		specializations: &'a SpecializationValues,
	) -> Result<ExecutableFunction, VmError> {
		let signature = extract_function_signature(function)?;
		let mut compiler = Self {
			function_ids,
			specializations,
			instructions: Vec::new(),
			local_types: Vec::new(),
			locals_by_reference: HashMap::new(),
			register_count: 0,
			return_type: signature.return_type.clone(),
			parameter_count: signature.params.len(),
			loop_continue_targets: Vec::new(),
			loop_continue_patches: Vec::new(),
		};

		for (index, param) in signature.params.iter().enumerate() {
			compiler.local_types.push(param.value_type.clone());
			compiler.locals_by_reference.insert(param.node.clone(), index);
		}

		for statement in &signature.statements {
			compiler.compile_statement(statement, descriptor_layouts)?;
		}

		if compiler.return_type.is_none() && !matches!(compiler.instructions.last(), Some(Instruction::Return { .. })) {
			compiler.instructions.push(Instruction::Return { register: None });
		}

		Ok(ExecutableFunction {
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
			parameter_count: compiler.parameter_count,
			return_type: compiler.return_type,
		})
	}

	/// Compiles one BESL statement into bytecode while tracking locals and descriptors.
	fn compile_statement(
		&mut self,
		statement: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let borrowed = statement.borrow();
		let result = match borrowed.node() {
			Nodes::Conditional { condition, statements } => {
				let condition = condition.clone();
				let statements = statements.clone();
				drop(borrowed);
				self.compile_conditional(&condition, &statements, descriptor_layouts)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let initializer = initializer.clone();
				let condition = condition.clone();
				let update = update.clone();
				let statements = statements.clone();
				drop(borrowed);
				self.compile_for_loop(&initializer, &condition, &update, &statements, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				left,
				right,
			}) => {
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);
				self.compile_assignment(statement, left, right, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Return { value }) => {
				let value = value.clone();
				drop(borrowed);
				self.compile_return_statement(value.as_ref(), descriptor_layouts)
			}
			Nodes::Expression(Expressions::Continue) => {
				drop(borrowed);
				if self.loop_continue_targets.is_empty() {
					return Err(VmError::UnsupportedStatement {
						message: "`continue` must be used inside a loop".to_string(),
					});
				}
				let jump_index = self.instructions.len();
				let target = self
					.loop_continue_targets
					.last()
					.copied()
					.expect("Expected loop continue target");
				self.instructions.push(Instruction::Jump { target });
				self.loop_continue_patches
					.last_mut()
					.expect("Expected continue patch stack")
					.push(jump_index);
				Ok(())
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_call_statement(&function, &parameters, descriptor_layouts)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = intrinsic.clone();
				let arguments = arguments.clone();
				drop(borrowed);
				self.compile_intrinsic_call_statement(&intrinsic, &arguments, descriptor_layouts)
			}
			Nodes::Raw { .. } => Ok(()),
			Nodes::Expression(Expressions::Member { .. }) | Nodes::Expression(Expressions::Accessor { .. }) => Ok(()),
			Nodes::Expression(other) => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement node: {}", describe_node(node)),
			}),
		};

		result
	}

	fn compile_conditional(
		&mut self,
		condition: &NodeReference,
		statements: &[NodeReference],
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let condition_register = self.compile_value_expression(condition, &ValueType::Bool, descriptor_layouts)?;
		let jump_if_zero_index = self.instructions.len();
		self.instructions.push(Instruction::JumpIfZero {
			register: condition_register,
			target: usize::MAX,
		});

		for statement in statements {
			self.compile_statement(statement, descriptor_layouts)?;
		}

		let conditional_end = self.instructions.len();
		match &mut self.instructions[jump_if_zero_index] {
			Instruction::JumpIfZero { target, .. } => *target = conditional_end,
			_ => unreachable!("Expected JumpIfZero placeholder"),
		}

		Ok(())
	}

	fn compile_for_loop(
		&mut self,
		initializer: &NodeReference,
		condition: &NodeReference,
		update: &NodeReference,
		statements: &[NodeReference],
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		self.compile_statement(initializer, descriptor_layouts)?;

		let condition_start = self.instructions.len();
		let condition_register = self.compile_value_expression(condition, &ValueType::Bool, descriptor_layouts)?;
		let jump_if_zero_index = self.instructions.len();
		self.instructions.push(Instruction::JumpIfZero {
			register: condition_register,
			target: usize::MAX,
		});
		let loop_end_placeholder_index = jump_if_zero_index;

		let continue_target = usize::MAX;
		self.loop_continue_targets.push(continue_target);
		self.loop_continue_patches.push(Vec::new());
		for statement in statements {
			self.compile_statement(statement, descriptor_layouts)?;
		}
		self.loop_continue_targets.pop();

		let update_start = self.instructions.len();
		self.compile_statement(update, descriptor_layouts)?;
		for jump_index in self.loop_continue_patches.pop().expect("Expected continue patch list") {
			match &mut self.instructions[jump_index] {
				Instruction::Jump { target } => *target = update_start,
				_ => unreachable!("Expected continue jump placeholder"),
			}
		}
		self.instructions.push(Instruction::Jump { target: condition_start });

		let loop_end = self.instructions.len();
		match &mut self.instructions[loop_end_placeholder_index] {
			Instruction::JumpIfZero { target, .. } => *target = loop_end,
			_ => unreachable!("Expected JumpIfZero placeholder"),
		}

		Ok(())
	}

	fn compile_assignment(
		&mut self,
		statement: &NodeReference,
		left: NodeReference,
		right: NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let left_expression = left.borrow();

		match left_expression.node() {
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
				let name = name.clone();
				let value_type = resolve_value_type(r#type)?;
				drop(left_expression);

				let local = self.define_local(statement.clone(), left, &name, value_type.clone());
				let register = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
				self.instructions.push(Instruction::StoreLocal { local, register });
				Ok(())
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(left_expression);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					let value_type = self
						.local_types
						.get(local)
						.cloned()
						.ok_or(VmError::UninitializedLocal { local })?;
					let register = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreLocal { local, register });
					// Later references resolve to the most recent assignment, so every assignment must remain an alias for the local slot.
					self.locals_by_reference.insert(statement.clone(), local);
					self.locals_by_reference.insert(left, local);
					Ok(())
				} else {
					let target = self.resolve_output_access(&left, descriptor_layouts)?;
					let target = self.lower_buffer_access(target, descriptor_layouts)?;
					let register = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
					self.emit_buffer_store(target, register);
					Ok(())
				}
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(left_expression);

				let target = if accessor_references_output(&left) {
					self.resolve_output_array_access(&left, descriptor_layouts)?
				} else {
					self.resolve_memory_access(&left, RequiredAccess::Write, descriptor_layouts)?
				};
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let register = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
				self.emit_buffer_store(target, register);
				Ok(())
			}
			node => Err(VmError::UnsupportedAssignmentTarget {
				message: format!("Unsupported assignment target: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles a scalar BESL expression into one register-producing VM instruction sequence.
	fn compile_value_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0].clone();
				drop(borrowed);
				self.compile_value_expression(&inner, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_function_call_expression(&function, &parameters, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = intrinsic.clone();
				let arguments = arguments.clone();
				drop(borrowed);
				self.compile_intrinsic_call_expression(&intrinsic, &arguments, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				let comparison = comparison_operator(operator);
				let arithmetic = if comparison.is_none() {
					Some(arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported value operator: {:?}", operator),
					})?)
				} else {
					None
				};
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);

				let operand_hint = if comparison.is_some() {
					&ValueType::U32
				} else if matches!(
					arithmetic,
					Some(ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr)
				) {
					&ValueType::Bool
				} else {
					expected_type
				};
				let mut left_type = self.infer_expression_type(&left, operand_hint, descriptor_layouts)?;
				let mut right_type = self.infer_expression_type(&right, operand_hint, descriptor_layouts)?;
				let result_type = if comparison.is_some() {
					(left_type, right_type) = resolve_comparison_operand_types(&left, &right, left_type, right_type)?;
					ValueType::Bool
				} else {
					binary_result_type(arithmetic.expect("Expected arithmetic operator"), &left_type, &right_type)?
				};
				if &result_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: result_type.name().to_string(),
					});
				}

				let left = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
				let right = self.compile_value_expression(&right, &right_type, descriptor_layouts)?;
				let register = self.allocate_register();
				if let Some(operator) = comparison {
					self.instructions.push(Instruction::Compare {
						register,
						operator,
						left,
						right,
					});
				} else {
					self.instructions.push(Instruction::Arithmetic {
						register,
						operator: arithmetic.expect("Expected arithmetic operator"),
						left,
						right,
					});
				}
				Ok(register)
			}
			Nodes::Expression(Expressions::Literal { value }) => {
				let value = value.clone();
				drop(borrowed);

				let register = self.allocate_register();
				let value = parse_literal(&value, expected_type)?;
				self.instructions.push(Instruction::LoadLiteral { register, value });
				Ok(register)
			}
			Nodes::Expression(Expressions::Member { source, name }) => {
				let source = source.clone();
				let member_name = name.clone();
				drop(borrowed);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					let actual_type = self.local_types.get(local).ok_or(VmError::UninitializedLocal { local })?;
					if actual_type != expected_type {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: actual_type.name().to_string(),
						});
					}

					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLocal { register, local });
					Ok(register)
				} else if matches!(source.borrow().node(), Nodes::Input { .. }) {
					let target = self.resolve_input_access(expression, descriptor_layouts)?;
					if &target.value_type != expected_type {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: target.value_type.name().to_string(),
						});
					}

					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadBuffer {
						register,
						slot: target.slot,
						offset: target.offset,
						value_type: target.value_type,
					});
					Ok(register)
				} else if is_resource_type(expected_type) && matches!(source.borrow().node(), Nodes::Binding { .. }) {
					let (slot, layout) = {
						let source_ref = source.borrow();
						let Nodes::Binding { slot, r#type, .. } = source_ref.node() else {
							unreachable!("Resource sources are checked before compiling the handle")
						};
						let layout = match r#type {
							BindingTypes::CombinedImageSampler { .. } => DescriptorLayout::Texture,
							BindingTypes::Image { .. } => DescriptorLayout::Image,
							BindingTypes::Buffer { .. } => {
								return Err(VmError::TypeMismatch {
									expected: expected_type.name().to_string(),
									found: "buffer".to_string(),
								});
							}
						};
						(ResourceSlot::new(*slot), layout)
					};
					match descriptor_layouts.get(&slot) {
						Some(existing) if existing != &layout => {
							return Err(VmError::UnsupportedDescriptor {
								slot,
								message: "Descriptor slot was reused with a different resource type".to_string(),
							});
						}
						Some(_) => {}
						None => {
							descriptor_layouts.insert(slot, layout);
						}
					}
					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLiteral {
						register,
						value: Value::Resource {
							slot,
							value_type: expected_type.clone(),
						},
					});
					Ok(register)
				} else {
					let source_value = {
						let source_ref = source.borrow();
						match source_ref.node() {
							Nodes::Specialization { name, r#type } => {
								let declared_type = resolve_value_type(r#type)?;
								let value = self
									.specializations
									.get(name)
									.ok_or_else(|| VmError::MissingSpecialization { name: name.clone() })?;
								if !value.matches_type(&declared_type) {
									return Err(VmError::TypeMismatch {
										expected: declared_type.name().to_string(),
										found: value.value_type().name().to_string(),
									});
								}
								Some(Ok(value.clone()))
							}
							Nodes::Const { value, .. } | Nodes::Literal { value, .. } => {
								let value = value.clone();
								drop(source_ref);
								return self.compile_value_expression(&value, expected_type, descriptor_layouts);
							}
							_ => None,
						}
					};
					let value = source_value.ok_or_else(|| VmError::UnsupportedExpression {
						message: format!(
							"Unsupported source for member `{member_name}`. The member resolves to a {} node that the VM cannot load.",
							describe_node(source.borrow().node())
						),
					})??;
					if !value.matches_type(expected_type) {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					}
					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLiteral { register, value });
					Ok(register)
				}
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(borrowed);
				self.compile_accessor_expression(expression, expected_type, descriptor_layouts)
			}
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles either a buffer access chain or a projection from a temporary aggregate value.
	fn compile_accessor_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		if accessor_references_buffer(expression) {
			let target = self.resolve_memory_access(expression, RequiredAccess::Read, descriptor_layouts)?;
			if &target.value_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: target.value_type.name().to_string(),
				});
			}
			return self.compile_resolved_buffer_load(target, descriptor_layouts);
		}
		let (left, right) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedExpression {
					message: "Expected an aggregate accessor".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
		if let Ok(member_name) = extract_member_name(&right) {
			let (index, result_type) = aggregate_member(&left_type, &member_name)?;
			if &result_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: result_type.name().to_string(),
				});
			}
			let source = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
			let register = self.allocate_register();
			self.instructions.push(Instruction::Extract {
				register,
				source,
				index,
				value_type: result_type,
			});
			return Ok(register);
		}

		let (result_type, count) = array_element_type(&left_type)?;
		if &result_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: result_type.name().to_string(),
			});
		}
		let source = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
		let index = self.compile_value_expression(&right, &ValueType::U32, descriptor_layouts)?;
		let register = self.allocate_register();
		self.instructions.push(Instruction::ExtractDynamic {
			register,
			source,
			index,
			count,
			value_type: result_type,
		});
		Ok(register)
	}

	fn compile_resolved_buffer_load(
		&mut self,
		target: ResolvedBufferAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let target = self.lower_buffer_access(target, descriptor_layouts)?;
		let register = self.allocate_register();
		if let Some(index) = target.index {
			self.instructions.push(Instruction::LoadBufferIndexed {
				register,
				slot: target.slot,
				offset: target.offset,
				stride: target.stride,
				count: target.count,
				index,
				value_type: target.value_type,
			});
		} else {
			self.instructions.push(Instruction::LoadBuffer {
				register,
				slot: target.slot,
				offset: target.offset,
				value_type: target.value_type,
			});
		}
		Ok(register)
	}

	/// Lowers a validated buffer access after type analysis so its dynamic index is emitted exactly once.
	fn lower_buffer_access(
		&mut self,
		target: ResolvedBufferAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<LoweredBufferAccess, VmError> {
		let index = match target.index_expression {
			Some(index_expression) => {
				Some(self.compile_value_expression(&index_expression, &ValueType::U32, descriptor_layouts)?)
			}
			None => None,
		};

		Ok(LoweredBufferAccess {
			slot: target.slot,
			offset: target.offset,
			stride: target.stride,
			count: target.count,
			index,
			value_type: target.value_type,
		})
	}

	/// Emits the indexed or direct store selected by a lowered buffer access.
	fn emit_buffer_store(&mut self, target: LoweredBufferAccess, register: usize) {
		if let Some(index) = target.index {
			self.instructions.push(Instruction::StoreBufferIndexed {
				slot: target.slot,
				offset: target.offset,
				stride: target.stride,
				count: target.count,
				index,
				value_type: target.value_type,
				register,
			});
		} else {
			self.instructions.push(Instruction::StoreBuffer {
				slot: target.slot,
				offset: target.offset,
				value_type: target.value_type,
				register,
			});
		}
	}

	/// Lowers value-producing texture, image, atomic, numeric, and invocation intrinsics into typed instructions.
	fn compile_intrinsic_call_expression(
		&mut self,
		intrinsic: &NodeReference,
		arguments: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let intrinsic_ref = intrinsic.borrow();
		let (name, return_type) = match intrinsic_ref.node() {
			Nodes::Intrinsic { name, r#return, .. } => (name.clone(), resolve_value_type(r#return)?),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an intrinsic, but found {}", describe_node(node)),
				});
			}
		};
		drop(intrinsic_ref);

		if name != "normalize" && name != "reflect" && &return_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: return_type.name().to_string(),
			});
		}

		match name.as_str() {
			"sample" => {
				require_argument_count(arguments, 2)?;

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SampleTexture { register, slot, uv });
				Ok(register)
			}
			"texture_lod" => {
				require_argument_count(arguments, 2)?;
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord_type = self.infer_expression_type(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &coord_type, descriptor_layouts)?;
				let register = self.allocate_register();
				match coord_type {
					ValueType::Vec2F => self.instructions.push(Instruction::SampleTexture {
						register,
						slot,
						uv: coord,
					}),
					ValueType::Vec3F => self.instructions.push(Instruction::SampleTexture3D {
						register,
						slot,
						uvw: coord,
					}),
					other => {
						return Err(VmError::TypeMismatch {
							expected: "vec2f or vec3f".to_string(),
							found: other.name().to_string(),
						});
					}
				}
				Ok(register)
			}
			"fetch" => {
				require_argument_count(arguments, 2)?;

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FetchTexture { register, slot, coord });
				Ok(register)
			}
			"fetch_u32" => {
				require_argument_count(arguments, 2)?;
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FetchTextureU32 { register, slot, coord });
				Ok(register)
			}
			"image_load" | "image_load_u32" => {
				require_argument_count(arguments, 2)?;
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				if name == "image_load" {
					self.instructions.push(Instruction::LoadImage { register, slot, coord });
				} else {
					self.instructions.push(Instruction::LoadImageU32 { register, slot, coord });
				}
				Ok(register)
			}
			"image_atomic_or" => {
				require_argument_count(arguments, 3)?;
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::ReadWrite, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let value = self.compile_value_expression(&arguments[2], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ImageAtomicOr {
					register,
					slot,
					coord,
					value,
				});
				Ok(register)
			}
			"atomic_load" => {
				require_argument_count(arguments, 1)?;
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				self.compile_resolved_buffer_load(target, descriptor_layouts)
			}
			"atomic_add" => {
				require_argument_count(arguments, 2)?;
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::ReadWrite, descriptor_layouts)?;
				if target.value_type != ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: ValueType::U32.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let value = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::AtomicAddBuffer {
					register,
					slot: target.slot,
					offset: target.offset,
					stride: target.stride,
					count: target.count,
					index: target.index,
					value,
				});
				Ok(register)
			}
			"texture_size" => {
				require_argument_count(arguments, 1)?;

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::TextureSize { register, slot });
				Ok(register)
			}
			"image_size" => {
				require_argument_count(arguments, 1)?;

				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Any, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ImageSize { register, slot });
				Ok(register)
			}
			"dot" => {
				require_argument_count(arguments, 2)?;

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
					.into_iter()
					.find(|candidate| {
						self.infer_expression_type(&arguments[0], candidate, descriptor_layouts).ok() == Some(candidate.clone())
							&& self.infer_expression_type(&arguments[1], candidate, descriptor_layouts).ok()
								== Some(candidate.clone())
					})
					.ok_or_else(|| VmError::UnsupportedExpression {
						message: "`dot` expects two float vectors of matching size".to_string(),
					})?;

				let left = self.compile_value_expression(&arguments[0], &supported_type, descriptor_layouts)?;
				let right = self.compile_value_expression(&arguments[1], &supported_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::DotProduct { register, left, right });
				Ok(register)
			}
			"cross" => {
				require_argument_count(arguments, 2)?;

				let left = self.compile_value_expression(&arguments[0], &ValueType::Vec3F, descriptor_layouts)?;
				let right = self.compile_value_expression(&arguments[1], &ValueType::Vec3F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::CrossProduct { register, left, right });
				Ok(register)
			}
			"length" => {
				require_argument_count(arguments, 1)?;

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
					.into_iter()
					.find(|candidate| {
						self.infer_expression_type(&arguments[0], candidate, descriptor_layouts).ok() == Some(candidate.clone())
					})
					.ok_or_else(|| VmError::UnsupportedExpression {
						message: "`length` expects one float vector argument".to_string(),
					})?;

				let value = self.compile_value_expression(&arguments[0], &supported_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::Length { register, value });
				Ok(register)
			}
			"normalize" => {
				require_argument_count(arguments, 1)?;

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
					.into_iter()
					.find(|candidate| {
						self.infer_expression_type(&arguments[0], candidate, descriptor_layouts).ok() == Some(candidate.clone())
					})
					.ok_or_else(|| VmError::UnsupportedExpression {
						message: "`normalize` expects one float vector argument".to_string(),
					})?;
				if &supported_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: supported_type.name().to_string(),
					});
				}

				let value = self.compile_value_expression(&arguments[0], &supported_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::Normalize { register, value });
				Ok(register)
			}
			"reflect" => {
				require_argument_count(arguments, 2)?;

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
					.into_iter()
					.find(|candidate| {
						self.infer_expression_type(&arguments[0], candidate, descriptor_layouts).ok() == Some(candidate.clone())
							&& self.infer_expression_type(&arguments[1], candidate, descriptor_layouts).ok()
								== Some(candidate.clone())
					})
					.ok_or_else(|| VmError::UnsupportedExpression {
						message: "`reflect` expects two float vectors of matching size".to_string(),
					})?;
				if &supported_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: supported_type.name().to_string(),
					});
				}

				let incident = self.compile_value_expression(&arguments[0], &supported_type, descriptor_layouts)?;
				let normal = self.compile_value_expression(&arguments[1], &supported_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::Reflect {
					register,
					incident,
					normal,
				});
				Ok(register)
			}
			"abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "round" | "fract" | "radians" | "inversesqrt" | "log2"
			| "fwidth" => {
				require_argument_count(arguments, 1)?;

				let value = self.compile_value_expression(&arguments[0], &return_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register,
					operator: match name.as_str() {
						"abs" => ScalarUnaryOperator::Abs,
						"sqrt" => ScalarUnaryOperator::Sqrt,
						"exp" => ScalarUnaryOperator::Exp,
						"sin" => ScalarUnaryOperator::Sin,
						"cos" => ScalarUnaryOperator::Cos,
						"tan" => ScalarUnaryOperator::Tan,
						"round" => ScalarUnaryOperator::Round,
						"fract" => ScalarUnaryOperator::Fract,
						"radians" => ScalarUnaryOperator::Radians,
						"inversesqrt" => ScalarUnaryOperator::InverseSqrt,
						"log2" => ScalarUnaryOperator::Log2,
						"fwidth" => ScalarUnaryOperator::Fwidth,
						_ => unreachable!("Expected scalar unary intrinsic"),
					},
					value,
				});
				Ok(register)
			}
			"f32" => {
				require_argument_count(arguments, 1)?;
				if expected_type != &ValueType::F32 {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: ValueType::F32.name().to_string(),
					});
				}

				let value = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register,
					operator: ScalarUnaryOperator::FromU32ToF32,
					value,
				});
				Ok(register)
			}
			"u32" => {
				require_argument_count(arguments, 1)?;
				if expected_type != &ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: ValueType::U32.name().to_string(),
					});
				}

				let source_type = self.infer_expression_type(&arguments[0], &ValueType::F32, descriptor_layouts)?;
				if source_type == ValueType::U32 {
					return self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts);
				}
				let operator = match source_type {
					ValueType::U8 => ScalarUnaryOperator::FromU8ToU32,
					ValueType::U16 => ScalarUnaryOperator::FromU16ToU32,
					ValueType::F32 => ScalarUnaryOperator::FromF32ToU32,
					ref other => {
						return Err(VmError::TypeMismatch {
							expected: "u8, u16, or f32".to_string(),
							found: other.name().to_string(),
						});
					}
				};
				let value = self.compile_value_expression(&arguments[0], &source_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register,
					operator,
					value,
				});
				Ok(register)
			}
			"min" | "max" | "pow" | "step" => {
				require_argument_count(arguments, 2)?;
				let argument_type = if name == "step" { ValueType::F32 } else { return_type.clone() };
				let left = self.compile_value_expression(&arguments[0], &argument_type, descriptor_layouts)?;
				let right = self.compile_value_expression(&arguments[1], &argument_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::BinaryScalar {
					register,
					operator: match name.as_str() {
						"min" => ScalarBinaryOperator::Min,
						"max" => ScalarBinaryOperator::Max,
						"pow" => ScalarBinaryOperator::Pow,
						"step" => ScalarBinaryOperator::Step,
						_ => unreachable!("Expected binary intrinsic"),
					},
					left,
					right,
				});
				Ok(register)
			}
			"smoothstep" | "mix" | "clamp" => {
				require_argument_count(arguments, 3)?;

				let argument_type = if name == "clamp" {
					return_type.clone()
				} else {
					ValueType::F32
				};
				let first = self.compile_value_expression(&arguments[0], &argument_type, descriptor_layouts)?;
				let second = self.compile_value_expression(&arguments[1], &argument_type, descriptor_layouts)?;
				let third = self.compile_value_expression(&arguments[2], &argument_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::TernaryScalar {
					register,
					operator: match name.as_str() {
						"smoothstep" => ScalarTernaryOperator::Smoothstep,
						"mix" => ScalarTernaryOperator::Mix,
						"clamp" => ScalarTernaryOperator::Clamp,
						_ => unreachable!("Expected scalar ternary intrinsic"),
					},
					first,
					second,
					third,
				});
				Ok(register)
			}
			"thread_idx" => {
				require_argument_count(arguments, 0)?;

				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadIdx { register });
				Ok(register)
			}
			"thread_id" => {
				require_argument_count(arguments, 0)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadId { register });
				Ok(register)
			}
			"threadgroup_position" => {
				require_argument_count(arguments, 0)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadgroupPosition { register });
				Ok(register)
			}
			_ => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported intrinsic `{}`", name),
			}),
		}
	}

	fn compile_function_call_expression(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let function_ref = function.borrow();
		match function_ref.node() {
			Nodes::Struct { fields, .. } => {
				let constructor_type = resolve_value_type(function)?;
				let fields = fields.clone();
				drop(function_ref);
				self.compile_constructor_expression(
					function,
					parameters,
					expected_type,
					constructor_type,
					&fields,
					descriptor_layouts,
				)
			}
			Nodes::Function { .. } => {
				let signature = extract_function_signature(function)?;
				drop(function_ref);
				let return_type = signature.return_type.ok_or_else(|| VmError::UnsupportedExpression {
					message: "Void functions cannot be used as value expressions".to_string(),
				})?;
				if &return_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: return_type.name().to_string(),
					});
				}
				require_argument_count(parameters, signature.params.len())?;

				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				let register = self.allocate_register();
				self.instructions.push(Instruction::Call {
					register: Some(register),
					function: *self
						.function_ids
						.get(function)
						.ok_or_else(|| VmError::UnsupportedExpression {
							message: "Unknown function reference".to_string(),
						})?,
					arguments,
				});
				Ok(register)
			}
			node => Err(VmError::UnsupportedExpression {
				message: format!("Expected a callable value, but found {}", describe_node(node)),
			}),
		}
	}

	fn compile_constructor_expression(
		&mut self,
		_function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		constructor_type: ValueType,
		fields: &[NodeReference],
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		if &constructor_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: constructor_type.name().to_string(),
			});
		}

		let mut components = Vec::with_capacity(parameters.len());
		if matches!(
			constructor_type,
			ValueType::Struct { .. } | ValueType::Mat4F | ValueType::Mat4x3F
		) {
			if fields.len() != parameters.len() {
				return Err(VmError::UnsupportedExpression {
					message: format!(
						"Constructor for `{}` expected {} parameters, but found {}",
						expected_type.name(),
						fields.len(),
						parameters.len()
					),
				});
			}
			for (field, parameter) in fields.iter().zip(parameters) {
				let field_type = match field.borrow().node() {
					Nodes::Member { r#type, .. } => resolve_value_type(r#type)?,
					node => {
						return Err(VmError::UnsupportedExpression {
							message: format!("Expected a constructor field, but found {}", describe_node(node)),
						});
					}
				};
				components.push(self.compile_value_expression(parameter, &field_type, descriptor_layouts)?);
			}
		} else {
			let scalar_type = vector_scalar_type(&constructor_type).ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("`{}` is not a flattenable vector constructor", constructor_type.name()),
			})?;
			let packed_u16 = constructor_type == ValueType::Vec2U16 || constructor_type == ValueType::Vec4U16;
			for parameter in parameters {
				// Packed u16 constructors accept ordinary u32 coordinate arithmetic and
				// apply the shader backend's narrowing conversion per component.
				let parameter_hint = if packed_u16 { ValueType::U32 } else { scalar_type.clone() };
				let parameter_type = self.infer_expression_type(parameter, &parameter_hint, descriptor_layouts)?;
				let parameter_scalar = vector_scalar_type(&parameter_type).unwrap_or_else(|| parameter_type.clone());
				let compatible = parameter_scalar == scalar_type || packed_u16 && parameter_scalar == ValueType::U32;
				if !compatible {
					return Err(VmError::TypeMismatch {
						expected: scalar_type.name().to_string(),
						found: parameter_type.name().to_string(),
					});
				}
				components.push(self.compile_value_expression(parameter, &parameter_type, descriptor_layouts)?);
			}
		}

		let register = self.allocate_register();
		self.instructions.push(Instruction::Construct {
			register,
			value_type: constructor_type,
			components,
		});
		Ok(register)
	}

	fn infer_expression_type(
		&self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ValueType, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0].clone();
				drop(borrowed);
				self.infer_expression_type(&inner, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Literal { value }) => {
				if matches!(value.as_str(), "true" | "false") {
					return Ok(ValueType::Bool);
				}
				// Decimal and scientific notation remain floating-point when comparisons
				// do not otherwise provide an operand type.
				if value.contains(['.', 'e', 'E']) || supports_scalar_broadcast(expected_type) {
					Ok(ValueType::F32)
				} else {
					Ok(expected_type.clone())
				}
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(borrowed);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					self.local_types
						.get(local)
						.cloned()
						.ok_or(VmError::UninitializedLocal { local })
				} else if matches!(source.borrow().node(), Nodes::Input { .. }) {
					Ok(self.resolve_input_access(expression, descriptor_layouts)?.value_type)
				} else {
					resolve_referenced_value_type(&source)
				}
			}
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);
				if accessor_references_buffer(expression) {
					Ok(self
						.resolve_memory_access(expression, RequiredAccess::Read, descriptor_layouts)?
						.value_type)
				} else {
					let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
					if let Ok(member_name) = extract_member_name(&right) {
						Ok(aggregate_member(&left_type, &member_name)?.1)
					} else {
						Ok(array_element_type(&left_type)?.0)
					}
				}
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				let intrinsic = intrinsic.clone();
				drop(borrowed);
				resolve_callable_return_type(&intrinsic)
			}
			Nodes::Expression(Expressions::FunctionCall { function, .. }) => resolve_callable_return_type(function),
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				if comparison_operator(operator).is_some() {
					Ok(ValueType::Bool)
				} else {
					let operator = arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported value operator: {:?}", operator),
					})?;
					if matches!(operator, ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr) {
						return Ok(ValueType::Bool);
					}
					let left = left.clone();
					let right = right.clone();
					drop(borrowed);
					let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
					let right_type = self.infer_expression_type(&right, expected_type, descriptor_layouts)?;
					binary_result_type(operator, &left_type, &right_type)
				}
			}
			Nodes::Expression(Expressions::Continue) => Err(VmError::UnsupportedExpression {
				message: "`continue` is only valid as a statement".to_string(),
			}),
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	fn define_local(
		&mut self,
		statement: NodeReference,
		declaration: NodeReference,
		_name: &str,
		value_type: ValueType,
	) -> usize {
		let local = self.local_types.len();
		self.local_types.push(value_type);
		self.locals_by_reference.insert(statement, local);
		self.locals_by_reference.insert(declaration, local);
		local
	}

	fn allocate_register(&mut self) -> usize {
		let register = self.register_count;
		self.register_count += 1;
		register
	}

	/// Resolves a BESL accessor into the descriptor slot and packed byte offset that the VM should access.
	fn resolve_memory_access(
		&self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (binding, selectors) = extract_access_chain(expression)?;
		let Some(AccessSelector::Member(member_name)) = selectors.first() else {
			return Err(VmError::UnsupportedExpression {
				message: "Buffer access must select a named member first".to_string(),
			});
		};

		let binding_ref = binding.borrow();
		let (slot, layout) = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				let layout = match r#type {
					BindingTypes::Buffer { members } => compile_buffer_layout(members)?,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only buffer descriptors are supported".to_string(),
						});
					}
				};

				(slot, layout)
			}
			Nodes::PushConstant { members } => {
				if access.requires_write() {
					return Err(VmError::UnsupportedAssignmentTarget {
						message: "Push constant members are read-only".to_string(),
					});
				}

				(PUSH_CONSTANT_SLOT, compile_buffer_layout(members)?)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		let descriptor_layout = if slot == PUSH_CONSTANT_SLOT {
			DescriptorLayout::PushConstant(layout.clone())
		} else {
			DescriptorLayout::Buffer(layout.clone())
		};

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &descriptor_layout => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, descriptor_layout);
			}
		}

		let member = layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;
		let mut offset = member.offset();
		let mut current_stride = member.value_type().size();
		let mut current_count = member.count();
		let mut value_type = member.value_type().clone();
		let mut index = None;
		let mut indexed_stride = current_stride;
		let mut indexed_count = current_count;
		for selector in selectors.iter().skip(1) {
			match selector {
				AccessSelector::Index(index_expression) => {
					if index.is_some() {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` cannot use more than one dynamic index", member_name),
						});
					}
					indexed_stride = current_stride;
					indexed_count = current_count;
					index = Some(index_expression.clone());
					current_count = 1;
				}
				AccessSelector::Member(field_name) => {
					if current_count > 1 {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` is an array and requires an element index", member_name),
						});
					}
					let (field_offset, field_type, field_count) = aggregate_member_layout(&value_type, field_name)?;
					offset += field_offset;
					value_type = field_type;
					current_stride = value_type.size();
					current_count = field_count;
				}
			}
		}
		if current_count > 1 {
			return Err(VmError::UnsupportedExpression {
				message: format!("Buffer member `{}` is an array and requires an element index", member_name),
			});
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset,
			stride: indexed_stride,
			count: indexed_count,
			index_expression: index,
			value_type,
		})
	}

	fn resolve_texture_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResourceSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
				if !matches!(
					value_type,
					ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D
				) {
					return Err(VmError::TypeMismatch {
						expected: "texture resource".to_string(),
						found: value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				match r#type {
					BindingTypes::CombinedImageSampler { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only texture descriptors can be sampled or fetched".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Texture => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Texture);
				Ok(slot)
			}
		}
	}

	fn resolve_image_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResourceSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
				if value_type != ValueType::Texture2D {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Texture2D.name().to_string(),
						found: value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				match r#type {
					BindingTypes::Image { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only image descriptors can be written through `write`".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Image => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Image);
				Ok(slot)
			}
		}
	}

	fn resolve_output_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, output_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Output {
				name,
				format,
				location,
				count,
			} => {
				if name != &output_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct output assignment is supported for `{}`", output_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				(
					if output_name == "position" {
						builtin_position_slot()
					} else {
						output_slot(*location)
					},
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: output_name.clone(),
							offset: 0,
							value_type: value_type.clone(),
							count,
						}],
						size: value_type.size() * count,
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.members()[0].value_type().size(),
			count: layout.members()[0].count(),
			index_expression: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}

	/// Resolves one dynamically indexed mesh output-array write.
	fn resolve_output_array_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (left, index_expression) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedAssignmentTarget {
					message: "Expected an indexed output array".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let mut target = self.resolve_output_access(&left, descriptor_layouts)?;
		target.index_expression = Some(index_expression);
		Ok(target)
	}

	fn resolve_input_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, input_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Input { name, format, location } => {
				if name != &input_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct input reads are supported for `{}`", input_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				(
					input_slot(*location),
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: input_name.clone(),
							offset: 0,
							value_type: value_type.clone(),
							count: 1,
						}],
						size: value_type.size(),
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.size(),
			count: 1,
			index_expression: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}

	fn compile_call_statement(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let function_ref = function.borrow();
		match function_ref.node() {
			Nodes::Function { .. } => {
				let signature = extract_function_signature(function)?;
				drop(function_ref);
				require_argument_count(parameters, signature.params.len())?;
				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				self.instructions.push(Instruction::Call {
					register: None,
					function: *self
						.function_ids
						.get(function)
						.ok_or_else(|| VmError::UnsupportedExpression {
							message: "Unknown function reference".to_string(),
						})?,
					arguments,
				});
				Ok(())
			}
			node => Err(VmError::UnsupportedStatement {
				message: format!("Expected a function call statement, but found {}", describe_node(node)),
			}),
		}
	}

	fn compile_return_statement(
		&mut self,
		value: Option<&NodeReference>,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		match (self.return_type.clone(), value) {
			(None, None) => {
				self.instructions.push(Instruction::Return { register: None });
				Ok(())
			}
			(None, Some(_)) => Err(VmError::UnsupportedStatement {
				message: "Void functions cannot return a value".to_string(),
			}),
			(Some(return_type), Some(value)) => {
				let register = self.compile_value_expression(value, &return_type, descriptor_layouts)?;
				self.instructions.push(Instruction::Return {
					register: Some(register),
				});
				Ok(())
			}
			(Some(return_type), None) => Err(VmError::UnsupportedStatement {
				message: format!("Function with return type `{}` must return a value", return_type.name()),
			}),
		}
	}

	fn compile_intrinsic_call_statement(
		&mut self,
		intrinsic: &NodeReference,
		arguments: &[NodeReference],
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let intrinsic_ref = intrinsic.borrow();
		let name = match intrinsic_ref.node() {
			Nodes::Intrinsic { name, .. } => name.clone(),
			node => {
				return Err(VmError::UnsupportedStatement {
					message: format!("Expected an intrinsic, but found {}", describe_node(node)),
				});
			}
		};
		drop(intrinsic_ref);

		match name.as_str() {
			"set_mesh_output_counts" => {
				require_argument_count(arguments, 2)?;
				let vertex_count = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let primitive_count = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshOutputCounts {
					vertex_count,
					primitive_count,
				});
				Ok(())
			}
			"set_mesh_vertex_position" => {
				require_argument_count(arguments, 2)?;
				let index = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let position = self.compile_value_expression(&arguments[1], &ValueType::Vec4F, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshVertexPosition { index, position });
				Ok(())
			}
			"set_mesh_triangle" => {
				require_argument_count(arguments, 2)?;
				let index = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let triangle = self.compile_value_expression(&arguments[1], &ValueType::Vec3U, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshTriangle { index, triangle });
				Ok(())
			}
			"write" => {
				require_argument_count(arguments, 3)?;

				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let value = self.compile_value_expression(&arguments[2], &ValueType::Vec4F, descriptor_layouts)?;
				self.instructions.push(Instruction::WriteImage { slot, coord, value });
				Ok(())
			}
			"guard_image_bounds" => {
				require_argument_count(arguments, 2)?;
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Any, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				self.instructions.push(Instruction::GuardImageBounds { slot, coord });
				Ok(())
			}
			"atomic_store" => {
				require_argument_count(arguments, 2)?;
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				if target.value_type != ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: ValueType::U32.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let register = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				self.emit_buffer_store(target, register);
				Ok(())
			}
			"atomic_add" | "image_atomic_or" => {
				self.compile_intrinsic_call_expression(intrinsic, arguments, &ValueType::U32, descriptor_layouts)?;
				Ok(())
			}
			_ => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported intrinsic statement `{}`", name),
			}),
		}
	}
}

/// The `ResolvedBufferAccess` struct carries a validated packed-memory target into instruction lowering.
struct ResolvedBufferAccess {
	slot: ResourceSlot,
	offset: usize,
	stride: usize,
	count: usize,
	index_expression: Option<NodeReference>,
	value_type: ValueType,
}

/// The `LoweredBufferAccess` struct carries the single compiled index register used by a memory instruction.
struct LoweredBufferAccess {
	slot: ResourceSlot,
	offset: usize,
	stride: usize,
	count: usize,
	index: Option<usize>,
	value_type: ValueType,
}

/// The `FunctionParameter` struct links one lexical parameter identity to its portable VM value type.
struct FunctionParameter {
	node: NodeReference,
	value_type: ValueType,
}

/// The `FunctionSignature` struct supplies parameter, return, and body information while lowering function calls.
struct FunctionSignature {
	params: Vec<FunctionParameter>,
	return_type: Option<ValueType>,
	statements: Vec<NodeReference>,
}

#[derive(Clone, Copy)]
enum RequiredAccess {
	Read,
	Write,
	ReadWrite,
	Any,
}

impl RequiredAccess {
	const fn requires_read(self) -> bool {
		matches!(self, Self::Read | Self::ReadWrite)
	}

	const fn requires_write(self) -> bool {
		matches!(self, Self::Write | Self::ReadWrite)
	}
}

/// Validates one binding's declared access at the shared descriptor-resolution seam.
fn require_descriptor_access(
	slot: ResourceSlot,
	readable: bool,
	writable: bool,
	required: RequiredAccess,
) -> Result<(), VmError> {
	if required.requires_read() && !readable {
		return Err(VmError::DescriptorAccessDenied { slot, access: "read" });
	}
	if required.requires_write() && !writable {
		return Err(VmError::DescriptorAccessDenied { slot, access: "write" });
	}
	Ok(())
}

/// Resolves untyped comparison literals from their typed peer and rejects incompatible operands before lowering.
fn resolve_comparison_operand_types(
	left: &NodeReference,
	right: &NodeReference,
	mut left_type: ValueType,
	mut right_type: ValueType,
) -> Result<(ValueType, ValueType), VmError> {
	let left_is_literal = is_literal_expression(left);
	let right_is_literal = is_literal_expression(right);
	if left_is_literal && !right_is_literal {
		left_type = right_type.clone();
	} else if right_is_literal && !left_is_literal {
		right_type = left_type.clone();
	}

	if left_type != right_type {
		return Err(VmError::TypeMismatch {
			expected: left_type.name().to_string(),
			found: right_type.name().to_string(),
		});
	}

	Ok((left_type, right_type))
}

fn is_literal_expression(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Literal { .. }) => true,
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => is_literal_expression(&elements[0]),
		_ => false,
	}
}

fn resolve_main_function(program: &NodeReference) -> Result<NodeReference, VmError> {
	let function = {
		let node = program.borrow();
		match node.node() {
			Nodes::Function { name, .. } if name == "main" => Some(program.clone()),
			_ => None,
		}
	};

	if let Some(function) = function {
		return Ok(function);
	}

	program.get_main().ok_or(VmError::MissingMainFunction)
}

fn collect_functions(main: &NodeReference) -> Vec<NodeReference> {
	let mut functions = Vec::new();
	let mut seen = HashSet::new();
	collect_reachable_function(main, &mut seen, &mut functions);
	functions
}

/// Adds one function and every function referenced by its executable expressions.
fn collect_reachable_function(function: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
	if !seen.insert(function.identity()) {
		return;
	}
	functions.push(function.clone());
	let statements = match function.borrow().node() {
		Nodes::Function { statements, .. } => statements.clone(),
		_ => return,
	};
	for statement in statements {
		collect_function_references(&statement, seen, functions);
	}
}

fn collect_function_references(node: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
	let (called_function, children) = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				(Some(function.clone()), parameters.clone())
			}
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => (None, arguments.clone()),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => (None, vec![left.clone(), right.clone()]),
			Nodes::Expression(Expressions::Expression { elements }) => (None, elements.clone()),
			Nodes::Expression(Expressions::Return { value }) => (None, value.iter().cloned().collect()),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => (None, vec![value.clone()]),
			_ => (None, Vec::new()),
		}
	};
	if let Some(function) = called_function {
		if matches!(function.borrow().node(), Nodes::Function { .. }) {
			collect_reachable_function(&function, seen, functions);
		}
	}
	for child in children {
		collect_function_references(&child, seen, functions);
	}
}

fn reject_raw_code_nodes(node: &NodeReference) -> Result<(), VmError> {
	let children = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Raw { glsl, hlsl, msl, .. } => {
				let has_code = [glsl.as_deref(), hlsl.as_deref(), msl.as_deref()]
					.into_iter()
					.flatten()
					.any(|code| !code.trim().is_empty());
				if has_code {
					return Err(VmError::UnsupportedRawCode);
				}
				Vec::new()
			}
			Nodes::Function { statements, .. } => statements.clone(),
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::Expression(Expressions::FunctionCall { parameters, .. }) => parameters.clone(),
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => arguments.clone(),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => vec![left.clone(), right.clone()],
			Nodes::Expression(Expressions::Expression { elements }) => elements.clone(),
			Nodes::Expression(Expressions::Return { value }) => value.iter().cloned().collect(),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => vec![value.clone()],
			_ => Vec::new(),
		}
	};

	for child in children {
		reject_raw_code_nodes(&child)?;
	}

	Ok(())
}

fn extract_function_signature(function: &NodeReference) -> Result<FunctionSignature, VmError> {
	let function_ref = function.borrow();
	let (params, return_type, statements) = match function_ref.node() {
		Nodes::Function {
			params,
			return_type,
			statements,
			..
		} => (params.clone(), return_type.clone(), statements.clone()),
		node => {
			return Err(VmError::UnsupportedExpression {
				message: format!("Expected a function, but found {}", describe_node(node)),
			});
		}
	};
	drop(function_ref);

	let mut compiled_params = Vec::with_capacity(params.len());
	for param in params {
		let param_ref = param.borrow();
		let value_type = match param_ref.node() {
			Nodes::Parameter { r#type, .. } => resolve_value_type(r#type)?,
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a parameter, but found {}", describe_node(node)),
				});
			}
		};
		drop(param_ref);
		compiled_params.push(FunctionParameter { node: param, value_type });
	}

	let return_type = resolve_function_return_type(&return_type)?;
	Ok(FunctionSignature {
		params: compiled_params,
		return_type,
		statements,
	})
}

fn resolve_function_return_type(return_type: &NodeReference) -> Result<Option<ValueType>, VmError> {
	if return_type.borrow().get_name() == Some("void") {
		Ok(None)
	} else {
		Ok(Some(resolve_value_type(return_type)?))
	}
}

fn resolve_callable_return_type(callable: &NodeReference) -> Result<ValueType, VmError> {
	let callable_ref = callable.borrow();
	match callable_ref.node() {
		Nodes::Struct { .. } => resolve_value_type(callable),
		Nodes::Intrinsic { r#return, .. } => {
			let return_type = r#return.clone();
			drop(callable_ref);
			resolve_value_type(&return_type)
		}
		Nodes::Function { return_type, .. } => {
			let return_type = return_type.clone();
			drop(callable_ref);
			resolve_function_return_type(&return_type)?.ok_or_else(|| VmError::UnsupportedExpression {
				message: "Void functions cannot be used as value expressions".to_string(),
			})
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a callable value, but found {}", describe_node(node)),
		}),
	}
}

fn resolve_value_type(node: &NodeReference) -> Result<ValueType, VmError> {
	let type_name = node
		.borrow()
		.get_name()
		.map(str::to_string)
		.unwrap_or_else(|| "unknown".to_string());

	match type_name.as_str() {
		"bool" => Ok(ValueType::Bool),
		"u8" => Ok(ValueType::U8),
		"u16" => Ok(ValueType::U16),
		"u32" => Ok(ValueType::U32),
		"i32" => Ok(ValueType::I32),
		"f32" => Ok(ValueType::F32),
		"atomicu32" => Ok(ValueType::U32),
		"vec2u16" => Ok(ValueType::Vec2U16),
		"vec4u16" => Ok(ValueType::Vec4U16),
		"vec2i" => Ok(ValueType::Vec2I),
		"vec2u" => Ok(ValueType::Vec2U),
		"vec3u" => Ok(ValueType::Vec3U),
		"vec4u" => Ok(ValueType::Vec4U),
		"vec2f" => Ok(ValueType::Vec2F),
		"vec3f" => Ok(ValueType::Vec3F),
		"vec4f" => Ok(ValueType::Vec4F),
		"mat4f" => Ok(ValueType::Mat4F),
		"mat4x3f" => Ok(ValueType::Mat4x3F),
		"Texture2D" => Ok(ValueType::Texture2D),
		"Texture3D" => Ok(ValueType::Texture3D),
		"ArrayTexture2D" => Ok(ValueType::ArrayTexture2D),
		_ => {
			let fields = match node.borrow().node() {
				Nodes::Struct { fields, .. } => fields.clone(),
				_ => return Err(VmError::UnsupportedType { type_name }),
			};
			let (fields, size) = compile_member_layouts(&fields, false)?;
			Ok(ValueType::Struct {
				name: type_name,
				fields,
				size,
			})
		}
	}
}

fn is_resource_type(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D
	)
}

fn compile_buffer_layout(members: &[NodeReference]) -> Result<BufferLayout, VmError> {
	let (compiled_members, offset) = compile_member_layouts(members, true)?;

	Ok(BufferLayout {
		members: compiled_members,
		size: offset,
	})
}

fn compile_member_layouts(
	members: &[NodeReference],
	allow_array_members: bool,
) -> Result<(Vec<BufferMemberLayout>, usize), VmError> {
	let mut offset = 0;
	let mut compiled_members = Vec::with_capacity(members.len());
	for member in members {
		let member = member.borrow();
		match member.node() {
			Nodes::Member { name, r#type, count } => {
				// Aggregate `Value` instances do not represent nested arrays, so only outer buffer layouts may retain counts.
				if count.is_some() && !allow_array_members {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Struct field `{}` cannot be an array", name),
					});
				}
				let value_type = resolve_value_type(r#type)?;
				if is_resource_type(&value_type) {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` cannot contain resource handles", name),
					});
				}
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				let member_size = value_type
					.size()
					.checked_mul(count)
					.ok_or_else(|| VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` exceeds addressable CPU memory", name),
					})?;
				compiled_members.push(BufferMemberLayout {
					name: name.clone(),
					offset,
					value_type: value_type.clone(),
					count,
				});
				offset = offset
					.checked_add(member_size)
					.ok_or_else(|| VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` exceeds addressable CPU memory", name),
					})?;
			}
			node => {
				return Err(VmError::UnsupportedBufferLayout {
					message: format!("Unsupported buffer member node: {}", describe_node(node)),
				});
			}
		}
	}
	Ok((compiled_members, offset))
}

enum AccessSelector {
	Member(String),
	Index(NodeReference),
}

fn extract_access_chain(expression: &NodeReference) -> Result<(NodeReference, Vec<AccessSelector>), VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			let inner = elements[0].clone();
			drop(borrowed);
			extract_access_chain(&inner)
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			let left = left.clone();
			let selector = match right.borrow().node() {
				Nodes::Expression(Expressions::Member { name, .. }) => AccessSelector::Member(name.clone()),
				_ => AccessSelector::Index(right.clone()),
			};
			drop(borrowed);
			let (binding, mut selectors) = extract_access_chain(&left)?;
			selectors.push(selector);
			Ok((binding, selectors))
		}
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);
			if matches!(source.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }) {
				Ok((source, Vec::new()))
			} else {
				Err(VmError::UnsupportedExpression {
					message: "Accessor is not rooted in a buffer binding".to_string(),
				})
			}
		}
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok((expression.clone(), Vec::new())),
		Nodes::Expression(expression) => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a buffer accessor, but found {}",
				match expression {
					Expressions::Return { .. } => "return",
					Expressions::Continue => "continue",
					Expressions::Member { .. } => "member",
					Expressions::Expression { .. } => "multi-element expression group",
					Expressions::Literal { .. } => "literal",
					Expressions::FunctionCall { .. } => "function call",
					Expressions::IntrinsicCall { .. } => "intrinsic call",
					Expressions::Operator { .. } => "operator",
					Expressions::VariableDeclaration { .. } => "variable declaration",
					Expressions::Accessor { .. } => "accessor",
					Expressions::Macro { .. } => "macro",
				}
			),
		}),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer accessor, but found {}", describe_node(node)),
		}),
	}
}

fn accessor_references_buffer(expression: &NodeReference) -> bool {
	extract_access_chain(expression)
		.ok()
		.is_some_and(|(binding, _)| matches!(binding.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }))
}

fn accessor_references_output(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Accessor { left, .. }) => output_member_references_interface(left),
		_ => false,
	}
}

fn output_member_references_interface(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			output_member_references_interface(&elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. }) => matches!(source.borrow().node(), Nodes::Output { .. }),
		_ => false,
	}
}

fn extract_binding_reference(expression: &NodeReference) -> Result<NodeReference, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(expression.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);

			let result = match source.borrow().node() {
				Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(source.clone()),
				Nodes::Expression(Expressions::Member { .. }) => extract_binding_reference(&source),
				_ => Err(VmError::UnsupportedExpression {
					message: format!(
						"Only direct binding or push constant member access is supported, but found {}",
						describe_node(source.borrow().node())
					),
				}),
			};

			result
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a binding or push constant reference, but found {}",
				describe_node(node)
			),
		}),
	}
}

fn extract_member_name(expression: &NodeReference) -> Result<String, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { name, .. }) => Ok(name.clone()),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer member name, but found {}", describe_node(node)),
		}),
	}
}

fn aggregate_member(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType), VmError> {
	match value_type {
		ValueType::Struct { fields, .. } => fields
			.iter()
			.enumerate()
			.find(|(_, field)| field.name() == member_name)
			.map(|(index, field)| (index, field.value_type().clone()))
			.ok_or_else(|| VmError::UnknownBufferMember {
				member: member_name.to_string(),
			}),
		ValueType::Vec2U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec2F => {
			vector_member(value_type, member_name, 2)
		}
		ValueType::Vec3U | ValueType::Vec3F => vector_member(value_type, member_name, 3),
		ValueType::Vec4U16 | ValueType::Vec4U | ValueType::Vec4F => vector_member(value_type, member_name, 4),
		ValueType::Mat4F => matrix_member(member_name, ValueType::Vec4F),
		ValueType::Mat4x3F => matrix_member(member_name, ValueType::Vec3F),
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` has no selectable members", value_type.name()),
		}),
	}
}

fn array_element_type(value_type: &ValueType) -> Result<(ValueType, usize), VmError> {
	let ValueType::Struct { fields, .. } = value_type else {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` cannot be indexed as an aggregate value", value_type.name()),
		});
	};
	let first = fields.first().ok_or_else(|| VmError::UnsupportedExpression {
		message: "Cannot index an empty aggregate value".to_string(),
	})?;
	if fields.iter().enumerate().any(|(index, field)| {
		field
			.name()
			.strip_prefix("value_")
			.and_then(|suffix| suffix.parse::<usize>().ok())
			!= Some(index)
			|| field.value_type() != first.value_type()
	}) {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` is a struct, not an indexable array value", value_type.name()),
		});
	}
	Ok((first.value_type().clone(), fields.len()))
}

fn aggregate_member_layout(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType, usize), VmError> {
	let (index, field_type) = aggregate_member(value_type, member_name)?;
	let offset = match value_type {
		ValueType::Struct { fields, .. } => fields[index].offset(),
		ValueType::Mat4F | ValueType::Mat4x3F => index * field_type.size(),
		_ => index * field_type.size(),
	};
	let count = match value_type {
		ValueType::Struct { fields, .. } => fields[index].count(),
		_ => 1,
	};
	Ok((offset, field_type, count))
}

fn vector_member(value_type: &ValueType, member_name: &str, component_count: usize) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name)
		.filter(|index| *index < component_count)
		.ok_or_else(|| VmError::UnsupportedExpression {
			message: format!("`{}` is not a component of `{}`", member_name, value_type.name()),
		})?;
	let scalar = vector_scalar_type(value_type).expect("Vector types have scalar components");
	Ok((index, scalar))
}

fn matrix_member(member_name: &str, column_type: ValueType) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name).ok_or_else(|| VmError::UnsupportedExpression {
		message: format!("`{}` is not a matrix column", member_name),
	})?;
	Ok((index, column_type))
}

fn component_index(name: &str) -> Option<usize> {
	match name {
		"x" | "r" => Some(0),
		"y" | "g" => Some(1),
		"z" | "b" => Some(2),
		"w" | "a" => Some(3),
		_ => None,
	}
}

fn resolve_referenced_value_type(source: &NodeReference) -> Result<ValueType, VmError> {
	match source.borrow().node() {
		Nodes::Member { r#type, .. }
		| Nodes::Parameter { r#type, .. }
		| Nodes::Specialization { r#type, .. }
		| Nodes::Const { r#type, .. } => resolve_value_type(r#type),
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => resolve_value_type(format),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => resolve_value_type(r#type),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Cannot resolve a value type from {}", describe_node(node)),
		}),
	}
}

fn describe_node(node: &Nodes) -> &'static str {
	match node {
		Nodes::Null => "null",
		Nodes::Scope { .. } => "scope",
		Nodes::Struct { .. } => "struct",
		Nodes::Member { .. } => "member",
		Nodes::Function { .. } => "function",
		Nodes::Conditional { .. } => "conditional",
		Nodes::ForLoop { .. } => "for loop",
		Nodes::Specialization { .. } => "specialization",
		Nodes::Expression(_) => "expression",
		Nodes::Raw { .. } => "raw",
		Nodes::Binding { .. } => "binding",
		Nodes::PushConstant { .. } => "push constant",
		Nodes::Intrinsic { .. } => "intrinsic",
		Nodes::Input { .. } => "input",
		Nodes::Output { .. } => "output",
		Nodes::Parameter { .. } => "parameter",
		Nodes::Literal { .. } => "literal",
		Nodes::Const { .. } => "const",
	}
}
