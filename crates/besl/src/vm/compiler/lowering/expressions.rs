use super::*;

impl<'a> Compiler<'a> {
	/// Emits a workgroup-storage load after checking the slot holds the type the caller expects.
	///
	/// Both accessor lowering and the `atomic_load` intrinsic read workgroup storage this way, so the
	/// type check and the [`WorkgroupInstruction::LoadWorkgroup`] emission live here once.
	pub(super) fn compile_workgroup_load(
		&mut self,
		target: ResolvedWorkgroupAccess,
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		if &target.value_type != expected_type {
			return Err(VmError::mismatch(expected_type.name(), target.value_type.name()));
		}
		let index = self.compile_index(target.index_expression.as_ref())?;
		Ok(self.emit_to(|register| WorkgroupInstruction::LoadWorkgroup {
			register,
			name: target.name,
			index,
			count: target.count,
		}))
	}

	/// Compiles a scalar BESL expression into one register-producing VM instruction sequence.
	// Expression lowering is an exhaustive AST dispatcher; branch-local validation remains next to emitted instructions.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	pub(super) fn compile_value_expression(
		&mut self,
		expression: &NodeId,
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		let borrowed = &self.program[expression];
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0];
				self.compile_value_expression(&inner, expected_type)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = *function;
				let parameters = parameters.clone();
				self.compile_function_call_expression(&function, &parameters, expected_type)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = *intrinsic;
				let arguments = arguments.clone();
				self.compile_intrinsic_call_expression(&intrinsic, &arguments, expected_type)
			}
			Nodes::Expression(Expressions::Operator {
				operator: operator @ (Operators::LogicalAnd | Operators::LogicalOr),
				left,
				right,
			}) => {
				let (operator, left, right) = (*operator, *left, *right);
				if expected_type != &ValueType::Bool {
					return Err(VmError::mismatch(expected_type.name(), ValueType::Bool.name()));
				}
				self.compile_short_circuit(operator, &left, &right)
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
				let left = *left;
				let right = *right;

				let operand_hint = if comparison.is_some() {
					&ValueType::U32
				} else {
					expected_type
				};
				let mut left_type = self.infer_expression_type(&left, operand_hint)?;
				let mut right_type = self.infer_expression_type(&right, operand_hint)?;
				let result_type = if comparison.is_some() {
					(left_type, right_type) =
						resolve_comparison_operand_types(self.program, &left, &right, left_type, right_type)?;
					ValueType::Bool
				} else {
					binary_result_type(arithmetic.expect("Expected arithmetic operator"), &left_type, &right_type)?
				};
				if &result_type != expected_type {
					return Err(VmError::mismatch(expected_type.name(), result_type.name()));
				}

				let left = self.compile_value_expression(&left, &left_type)?;
				let right = self.compile_value_expression(&right, &right_type)?;
				let register = self.allocate_register();
				if let Some(operator) = comparison {
					self.emit(NumericInstruction::Compare {
						register,
						operator,
						left,
						right,
					});
				} else {
					self.emit(NumericInstruction::Arithmetic {
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

				let register = self.allocate_register();
				let value = parse_literal(&value, expected_type)?;
				self.emit(ValueInstruction::LoadLiteral { register, value });
				Ok(register)
			}
			Nodes::Expression(Expressions::Member { source, name }) => {
				let source = *source;
				let member_name = self.program.text(*name);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					let actual_type = self.local_types.get(local).ok_or(VmError::UninitializedLocal { local })?;
					if actual_type != expected_type {
						return Err(VmError::mismatch(expected_type.name(), actual_type.name()));
					}

					Ok(self.emit_to(|register| LocalInstruction::LoadLocal { register, local }))
				} else if matches!(self.program[source].node(), Nodes::Input { .. }) {
					let target = self.resolve_input_access(expression)?;
					if &target.value_type != expected_type {
						return Err(VmError::mismatch(expected_type.name(), target.value_type.name()));
					}

					Ok(self.emit_to(|register| BufferInstruction::Load {
						register,
						access: BufferAccess {
							slot: target.slot,
							offset: target.offset,
							stride: 0,
							count: None,
							index: None,
							value_type: target.value_type,
						},
					}))
				} else if is_resource_type(expected_type) && matches!(self.program[source].node(), Nodes::Binding { .. }) {
					let (slot, layout) = {
						let source_ref = &self.program[source];
						let Nodes::Binding {
							name, r#type, placement, ..
						} = source_ref.node()
						else {
							unreachable!("Resource sources are checked before compiling the handle")
						};
						let layout = match r#type {
							BindingTypes::CombinedImageSampler { .. } => DescriptorLayout::Texture,
							BindingTypes::Image { .. } => DescriptorLayout::Image,
							BindingTypes::Buffer { .. } | BindingTypes::BufferArray { .. } => {
								return Err(VmError::mismatch(expected_type.name(), "buffer"));
							}
							BindingTypes::AccelerationStructure => {
								return Err(VmError::mismatch(expected_type.name(), "acceleration structure"));
							}
						};
						(placed_slot(self.program.text(*name), *placement)?, layout)
					};
					self.claim_descriptor_layout(slot, layout, "resource type")?;
					Ok(self.emit_to(|register| ValueInstruction::LoadLiteral {
						register,
						value: Value::Resource {
							slot,
							value_type: expected_type.clone(),
						},
					}))
				} else {
					let source_value = {
						let source_ref = &self.program[source];
						match source_ref.node() {
							Nodes::Specialization { name, r#type, .. } => {
								let declared_type = resolve_value_type(self.program, r#type)?;
								let name = self.program.text(*name);
								let value = self
									.specializations
									.get(name)
									.ok_or_else(|| VmError::MissingSpecialization { name: name.to_string() })?;
								if !value.matches_type(&declared_type) {
									return Err(VmError::mismatch(declared_type.name(), value.value_type().name()));
								}
								Some(Ok(value.clone()))
							}
							Nodes::Const { value, .. } => {
								let value = *value;
								return self.compile_value_expression(&value, expected_type);
							}
							_ => None,
						}
					};
					let value = source_value.ok_or_else(|| VmError::UnsupportedExpression {
						message: format!(
							"Unsupported source `{}` for member `{member_name}`. The source resolves to a {} node that the VM cannot load.",
							self.program.name(source).unwrap_or("<unnamed>"),
							describe_node(self.program[source].node()),
						),
					})??;
					if !value.matches_type(expected_type) {
						return Err(VmError::mismatch(expected_type.name(), value.value_type().name()));
					}
					Ok(self.emit_to(|register| ValueInstruction::LoadLiteral { register, value }))
				}
			}
			Nodes::Expression(Expressions::Accessor { .. }) => self.compile_accessor_expression(expression, expected_type),
			Nodes::Expression(Expressions::Unary { operator, operand }) => {
				let operator = *operator;
				let operand = *operand;

				let operand_type = self.infer_expression_type(&operand, expected_type)?;
				let result_type = prefix_result_type(operator, &operand_type)?;
				if &result_type != expected_type {
					return Err(VmError::mismatch(expected_type.name(), result_type.name()));
				}
				let value = self.compile_value_expression(&operand, &operand_type)?;
				Ok(self.emit_to(|register| NumericInstruction::UnaryScalar {
					register,
					operator: prefix_operator(operator),
					value,
				}))
			}
			Nodes::Expression(Expressions::Ternary {
				condition,
				if_true,
				if_false,
			}) => {
				let (condition, if_true, if_false) = (*condition, *if_true, *if_false);
				self.compile_ternary(&condition, &if_true, &if_false, expected_type)
			}
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles a plain-value `if`, such as `if (index < count) { values[index] } else { 0.0 }`, so that only the
	/// selected branch runs, as it does on the GPU, which lets a branch read what the condition guards. Both branches
	/// store into one hidden local that the result loads.
	fn compile_ternary(
		&mut self,
		condition: &NodeId,
		if_true: &NodeId,
		if_false: &NodeId,
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		let condition = self.compile_value_expression(condition, &ValueType::Bool)?;
		let local = self.local_types.len();
		self.local_types.push(expected_type.clone());

		let skip_true_index = self.instructions.len();
		self.emit(ControlInstruction::JumpIfZero {
			register: condition,
			target: usize::MAX,
		});
		let value = self.compile_value_expression(if_true, expected_type)?;
		self.emit(LocalInstruction::StoreLocal { local, register: value });
		let skip_false_index = self.instructions.len();
		self.emit(ControlInstruction::Jump { target: usize::MAX });

		self.patch_jump(skip_true_index, self.instructions.len());
		let value = self.compile_value_expression(if_false, expected_type)?;
		self.emit(LocalInstruction::StoreLocal { local, register: value });
		self.patch_jump(skip_false_index, self.instructions.len());

		Ok(self.emit_to(|register| LocalInstruction::LoadLocal { register, local }))
	}

	/// Compiles `left && right` or `left || right` so that `right` runs only when `left` doesn't decide the result, as
	/// on the GPU, which lets `right` read what `left` guards, such as `index < count && values[index] > 0`. Both
	/// operands store into one hidden local that the result loads.
	fn compile_short_circuit(&mut self, operator: Operators, left: &NodeId, right: &NodeId) -> Result<usize, VmError> {
		let local = self.local_types.len();
		self.local_types.push(ValueType::Bool);

		let left = self.compile_value_expression(left, &ValueType::Bool)?;
		self.emit(LocalInstruction::StoreLocal { local, register: left });
		let false_index = self.instructions.len();
		self.emit(ControlInstruction::JumpIfZero {
			register: left,
			target: usize::MAX,
		});
		// A false left side decides `&&`, so it skips the right one. A true left side decides `||`, which falls through
		// to a jump over the right side, and a false one jumps to it.
		let skip_right_index = if operator == Operators::LogicalOr {
			let skip_right_index = self.instructions.len();
			self.emit(ControlInstruction::Jump { target: usize::MAX });
			self.patch_jump(false_index, self.instructions.len());
			skip_right_index
		} else {
			false_index
		};
		let right = self.compile_value_expression(right, &ValueType::Bool)?;
		self.emit(LocalInstruction::StoreLocal { local, register: right });
		self.patch_jump(skip_right_index, self.instructions.len());

		Ok(self.emit_to(|register| LocalInstruction::LoadLocal { register, local }))
	}

	/// Compiles a resource or buffer access, or a projection from a temporary aggregate value.
	pub(super) fn compile_accessor_expression(
		&mut self,
		expression: &NodeId,
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		if let Some((slot, count, index, value_type)) = resolve_texture_array_access(self.program, expression)? {
			if &value_type != expected_type {
				return Err(VmError::mismatch(expected_type.name(), value_type.name()));
			}
			// Every possible element occupies one flat host slot; only the selected resource is read at execution.
			for element in 0..count {
				self.claim_descriptor_layout(slot.offset(element as u32), DescriptorLayout::Texture, "resource type")?;
			}
			let index = self.compile_value_expression(&index, &ValueType::U32)?;
			return Ok(self.emit_to(|register| ValueInstruction::LoadResourceIndexed {
				register,
				slot,
				index,
				count,
				value_type,
			}));
		}
		if let Some(target) = resolve_workgroup_access(self.program, expression)? {
			return self.compile_workgroup_load(target, expected_type);
		}
		if let Some(target) = resolve_task_payload_access(self.program, expression)? {
			if &target.value_type != expected_type {
				return Err(VmError::mismatch(expected_type.name(), target.value_type.name()));
			}
			let index = self.compile_value_expression(&target.index_expression, &ValueType::U32)?;
			return Ok(self.emit_to(|register| WorkgroupInstruction::LoadTaskPayload {
				register,
				name: target.name,
				index,
				count: target.count,
				value_type: target.value_type,
			}));
		}
		if accessor_references_buffer(self.program, expression) {
			let target = self.resolve_memory_access(expression, RequiredAccess::Read)?;
			if &target.value_type != expected_type {
				return Err(VmError::mismatch(expected_type.name(), target.value_type.name()));
			}
			return self.compile_resolved_buffer_load(target);
		}
		let (left, right) = {
			let borrowed = &self.program[expression];
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedExpression {
					message: "Expected an aggregate accessor".to_string(),
				});
			};
			(*left, *right)
		};
		let left_type = self.infer_expression_type(&left, expected_type)?;
		if let Ok(member_name) = extract_member_name(self.program, &right) {
			let (index, result_type) = aggregate_member(&left_type, &member_name)?;
			if &result_type != expected_type {
				return Err(VmError::mismatch(expected_type.name(), result_type.name()));
			}
			let source = self.compile_value_expression(&left, &left_type)?;
			return Ok(self.emit_to(|register| ValueInstruction::Extract {
				register,
				source,
				selector: Selector::Member(index),
				value_type: result_type,
			}));
		}

		let (result_type, count) = array_element_type(&left_type)?;
		if &result_type != expected_type {
			return Err(VmError::mismatch(expected_type.name(), result_type.name()));
		}
		let source = self.compile_value_expression(&left, &left_type)?;
		let index = self.compile_value_expression(&right, &ValueType::U32)?;
		Ok(self.emit_to(|register| ValueInstruction::Extract {
			register,
			source,
			selector: Selector::Element { index, count },
			value_type: result_type,
		}))
	}

	pub(super) fn compile_resolved_buffer_load(&mut self, target: ResolvedBufferAccess) -> Result<usize, VmError> {
		let access = self.lower_buffer_access(target)?;
		Ok(self.emit_to(|register| BufferInstruction::Load { register, access }))
	}

	/// Lowers a validated buffer access after type analysis so its dynamic index is emitted exactly once.
	pub(super) fn lower_buffer_access(&mut self, target: ResolvedBufferAccess) -> Result<BufferAccess, VmError> {
		Ok(BufferAccess {
			index: self.compile_index(target.index_expression.as_ref())?,
			slot: target.slot,
			offset: target.offset,
			stride: target.stride,
			count: target.count,
			value_type: target.value_type,
		})
	}

	/// Emits the store of the value in `register` to `access`.
	pub(super) fn emit_buffer_store(&mut self, access: BufferAccess, register: usize) {
		self.emit(BufferInstruction::Store { access, register });
	}

	/// Lowers value-producing texture, image, atomic, numeric, and invocation intrinsics into typed instructions.
	pub(super) fn infer_expression_type(
		&mut self,
		expression: &NodeId,
		expected_type: &ValueType,
	) -> Result<ValueType, VmError> {
		let borrowed = &self.program[expression];
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0];
				self.infer_expression_type(&inner, expected_type)
			}
			Nodes::Expression(Expressions::Literal { value }) => {
				if matches!(value.as_str(), "true" | "false") {
					return Ok(ValueType::Bool);
				}
				// Decimal and scientific notation remain floating-point when comparisons
				// do not otherwise provide an operand type.
				if value.contains(['.', 'e', 'E']) || supports_scalar_broadcast(expected_type) {
					if expected_type == &ValueType::F16 || vector_scalar_type(expected_type) == Some(ValueType::F16) {
						Ok(ValueType::F16)
					} else {
						Ok(ValueType::F32)
					}
				} else {
					Ok(expected_type.clone())
				}
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = *source;

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					self.local_types
						.get(local)
						.cloned()
						.ok_or(VmError::UninitializedLocal { local })
				} else if matches!(self.program[source].node(), Nodes::Input { .. }) {
					Ok(self.resolve_input_access(expression)?.value_type)
				} else {
					resolve_referenced_value_type(self.program, &source)
				}
			}
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let left = *left;
				let right = *right;
				if let Some((_, _, _, value_type)) = resolve_texture_array_access(self.program, expression)? {
					Ok(value_type)
				} else if let Some(target) = resolve_workgroup_access(self.program, expression)? {
					Ok(target.value_type)
				} else if let Some(target) = resolve_task_payload_access(self.program, expression)? {
					Ok(target.value_type)
				} else if accessor_references_buffer(self.program, expression) {
					Ok(self.resolve_memory_access(expression, RequiredAccess::Read)?.value_type)
				} else {
					let left_type = self.infer_expression_type(&left, expected_type)?;
					if let Ok(member_name) = extract_member_name(self.program, &right) {
						Ok(aggregate_member(&left_type, &member_name)?.1)
					} else {
						Ok(array_element_type(&left_type)?.0)
					}
				}
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				let intrinsic = *intrinsic;
				resolve_callable_return_type(self.program, &intrinsic)
			}
			Nodes::Expression(Expressions::FunctionCall { function, .. }) => {
				resolve_callable_return_type(self.program, function)
			}
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				if comparison_operator(operator).is_some() || matches!(operator, Operators::LogicalAnd | Operators::LogicalOr) {
					Ok(ValueType::Bool)
				} else {
					let operator = arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported value operator: {:?}", operator),
					})?;
					let left = *left;
					let right = *right;
					let left_type = self.infer_expression_type(&left, expected_type)?;
					let right_type = self.infer_expression_type(&right, expected_type)?;
					binary_result_type(operator, &left_type, &right_type)
				}
			}
			Nodes::Expression(Expressions::Unary {
				operator: UnaryOperators::LogicalNot,
				..
			}) => Ok(ValueType::Bool),
			Nodes::Expression(Expressions::Unary { operand, .. }) => {
				let operand = *operand;
				self.infer_expression_type(&operand, expected_type)
			}
			Nodes::Expression(Expressions::Ternary { if_true, if_false, .. }) => {
				// A literal branch takes its type from the context, so the other branch decides when it can.
				let branch = if is_literal_expression(self.program, if_true) {
					*if_false
				} else {
					*if_true
				};
				self.infer_expression_type(&branch, expected_type)
			}
			Nodes::Expression(Expressions::Continue) => Err(VmError::UnsupportedExpression {
				message: "`continue` is only valid as a statement".to_string(),
			}),
			Nodes::Expression(Expressions::Break) => Err(VmError::UnsupportedExpression {
				message: "`break` is only valid as a statement".to_string(),
			}),
			Nodes::Expression(Expressions::Discard) => Err(VmError::UnsupportedExpression {
				message: "`discard` is only valid as a statement".to_string(),
			}),
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	pub(super) fn define_local(&mut self, statement: NodeId, declaration: NodeId, _name: &str, value_type: ValueType) -> usize {
		let local = self.local_types.len();
		self.local_types.push(value_type);
		self.locals_by_reference.insert(statement, local);
		self.locals_by_reference.insert(declaration, local);
		local
	}

	pub(super) fn allocate_register(&mut self) -> usize {
		let register = self.register_count;
		self.register_count += 1;
		register
	}
}
