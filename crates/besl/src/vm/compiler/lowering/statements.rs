use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn compile_statement(
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
			Nodes::Expression(Expressions::Discard) => {
				drop(borrowed);
				self.instructions.push(Instruction::Discard);
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

	pub(super) fn compile_conditional(
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

	pub(super) fn compile_for_loop(
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

	pub(super) fn compile_assignment(
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
				if let Some(target) = resolve_workgroup_access(&left)? {
					let index = target
						.index_expression
						.as_ref()
						.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
						.transpose()?;
					let value = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreWorkgroup {
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						value,
					});
					return Ok(());
				}
				if let Some(target) = resolve_task_payload_access(&left)? {
					let index = self.compile_value_expression(&target.index_expression, &ValueType::U32, descriptor_layouts)?;
					let value = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreTaskPayload {
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						value,
					});
					return Ok(());
				}

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

	pub(super) fn compile_call_statement(
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

	pub(super) fn compile_return_statement(
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

	pub(super) fn compile_intrinsic_call_statement(
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
			"set_task_mesh_output_count" => {
				require_argument_count(arguments, 1)?;
				let count = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				self.instructions.push(Instruction::SetTaskMeshOutputCount { count });
				Ok(())
			}
			"workgroup_barrier" => {
				require_argument_count(arguments, 0)?;
				// Preserve the barrier as an instruction so workgroup execution can rendezvous every lane.
				self.instructions.push(Instruction::WorkgroupBarrier);
				Ok(())
			}
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
			"set_mesh_primitive_render_target_array_index" => {
				require_argument_count(arguments, 2)?;
				let index = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let array_index = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				self.instructions
					.push(Instruction::SetMeshPrimitiveRenderTargetArrayIndex { index, array_index });
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
				if let Some(target) = resolve_workgroup_access(&arguments[0])? {
					if target.value_type != ValueType::U32 {
						return Err(VmError::TypeMismatch {
							expected: ValueType::U32.name().to_string(),
							found: target.value_type.name().to_string(),
						});
					}
					let index = target
						.index_expression
						.as_ref()
						.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
						.transpose()?;
					let value = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreWorkgroup {
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						value,
					});
					return Ok(());
				}
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
			"atomic_add" | "atomic_compare_exchange" | "image_atomic_or" => {
				self.compile_intrinsic_call_expression(intrinsic, arguments, &ValueType::U32, descriptor_layouts)?;
				Ok(())
			}
			_ => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported intrinsic statement `{}`", name),
			}),
		}
	}
}
