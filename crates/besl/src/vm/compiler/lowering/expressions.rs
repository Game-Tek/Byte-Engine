use super::*;

impl<'a> Compiler<'a> {
	/// Compiles a scalar BESL expression into one register-producing VM instruction sequence.
	// Expression lowering is an exhaustive AST dispatcher; branch-local validation remains next to emitted instructions.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	pub(super) fn compile_value_expression(
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
							"Unsupported source `{}` for member `{member_name}`. The source resolves to a {} node that the VM cannot load.",
							source.borrow().get_name().unwrap_or("<unnamed>"),
							describe_node(source.borrow().node()),
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
	pub(super) fn compile_accessor_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		if let Some(target) = resolve_workgroup_access(expression)? {
			if &target.value_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: target.value_type.name().to_string(),
				});
			}
			let index = target
				.index_expression
				.as_ref()
				.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
				.transpose()?;
			let register = self.allocate_register();
			self.instructions.push(Instruction::LoadWorkgroup {
				register,
				name: target.name,
				index,
				count: target.count,
				value_type: target.value_type,
			});
			return Ok(register);
		}
		if let Some(target) = resolve_task_payload_access(expression)? {
			if &target.value_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: target.value_type.name().to_string(),
				});
			}
			let index = self.compile_value_expression(&target.index_expression, &ValueType::U32, descriptor_layouts)?;
			let register = self.allocate_register();
			self.instructions.push(Instruction::LoadTaskPayload {
				register,
				name: target.name,
				index,
				count: target.count,
				value_type: target.value_type,
			});
			return Ok(register);
		}
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

	pub(super) fn compile_resolved_buffer_load(
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
	pub(super) fn lower_buffer_access(
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
	pub(super) fn emit_buffer_store(&mut self, target: LoweredBufferAccess, register: usize) {
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
	pub(super) fn infer_expression_type(
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
				if let Some(target) = resolve_workgroup_access(expression)? {
					Ok(target.value_type)
				} else if let Some(target) = resolve_task_payload_access(expression)? {
					Ok(target.value_type)
				} else if accessor_references_buffer(expression) {
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

	pub(super) fn define_local(
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

	pub(super) fn allocate_register(&mut self) -> usize {
		let register = self.register_count;
		self.register_count += 1;
		register
	}
}
