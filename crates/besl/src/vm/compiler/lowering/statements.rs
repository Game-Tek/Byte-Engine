use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn compile_statement(
		&mut self,
		statement: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let borrowed = statement.borrow();

		match borrowed.node() {
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => {
				let condition = condition.clone();
				let statements = statements.clone();
				let else_branch = else_branch.clone();
				drop(borrowed);
				self.compile_conditional(&condition, &statements, else_branch.as_ref(), descriptor_layouts)
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
			Nodes::Expression(Expressions::Break) => {
				drop(borrowed);
				let Some(patches) = self.loop_break_patches.last_mut() else {
					return Err(VmError::UnsupportedStatement {
						message: "`break` must be used inside a loop".to_string(),
					});
				};
				// The loop's end is unknown until its body is compiled, so the jump is patched afterwards.
				patches.push(self.instructions.len());
				self.instructions.push(Instruction::Jump { target: usize::MAX });
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
		}
	}

	/// Points the placeholder `Jump` or `JumpIfZero` at `index` to `target`, once the target is known.
	fn patch_jump(&mut self, index: usize, target: usize) {
		match &mut self.instructions[index] {
			Instruction::Jump { target: placeholder } | Instruction::JumpIfZero { target: placeholder, .. } => {
				*placeholder = target;
			}
			_ => unreachable!("Expected a jump placeholder"),
		}
	}

	pub(super) fn compile_conditional(
		&mut self,
		condition: &NodeReference,
		statements: &[NodeReference],
		else_branch: Option<&crate::ElseBranch>,
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

		let Some(else_branch) = else_branch else {
			self.patch_jump(jump_if_zero_index, self.instructions.len());
			return Ok(());
		};

		// The then branch ends by jumping over the else branch.
		let skip_else_index = self.instructions.len();
		self.instructions.push(Instruction::Jump { target: usize::MAX });
		self.patch_jump(jump_if_zero_index, self.instructions.len());

		// An `else if` link compiles as one nested conditional statement.
		for statement in else_branch.statements() {
			self.compile_statement(statement, descriptor_layouts)?;
		}
		self.patch_jump(skip_else_index, self.instructions.len());

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
		self.loop_break_patches.push(Vec::new());
		for statement in statements {
			self.compile_statement(statement, descriptor_layouts)?;
		}
		self.loop_continue_targets.pop();

		let update_start = self.instructions.len();
		self.compile_statement(update, descriptor_layouts)?;
		for jump_index in self.loop_continue_patches.pop().expect("Expected continue patch list") {
			self.patch_jump(jump_index, update_start);
		}
		self.instructions.push(Instruction::Jump { target: condition_start });

		let loop_end = self.instructions.len();
		self.patch_jump(loop_end_placeholder_index, loop_end);
		for jump_index in self.loop_break_patches.pop().expect("Expected break patch list") {
			self.patch_jump(jump_index, loop_end);
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
				if let Some(value_type) = self.local_path_type(&left) {
					let value = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
					return self.compile_local_store(&left, value, descriptor_layouts);
				}
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

	/// Returns the type of `expression` when it is a local or a chain of named members inside one, such as
	/// `probe.position.z`, and `None` for anything else.
	fn local_path_type(&self, expression: &NodeReference) -> Option<ValueType> {
		match expression.borrow().node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				self.local_path_type(&elements[0])
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let local = self.locals_by_reference.get(source)?;
				self.local_types.get(*local).cloned()
			}
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let member_name = extract_member_name(right).ok()?;
				aggregate_member(&self.local_path_type(left)?, &member_name)
					.ok()
					.map(|(_, member_type)| member_type)
			}
			_ => None,
		}
	}

	/// Stores the `value` register into `target`, a path that [`Self::local_path_type`] accepts.
	///
	/// Registers hold whole values, so a member store inserts `value` into the enclosing value and stores that in turn,
	/// until it reaches the local.
	fn compile_local_store(
		&mut self,
		target: &NodeReference,
		value: usize,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let borrowed = target.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0].clone();
				drop(borrowed);
				self.compile_local_store(&inner, value, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let local = self.locals_by_reference[source];
				self.instructions.push(Instruction::StoreLocal { local, register: value });
				Ok(())
			}
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let (left, member_name) = (left.clone(), extract_member_name(right)?);
				drop(borrowed);
				let parent_type = self.local_path_type(&left).expect("Local store targets are local paths");
				let (index, _) = aggregate_member(&parent_type, &member_name)?;
				let source = self.compile_value_expression(&left, &parent_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::Insert {
					register,
					source,
					index,
					value,
				});
				self.compile_local_store(&left, register, descriptor_layouts)
			}
			_ => unreachable!("Local store targets are local paths"),
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
					if !matches!(target.value_type, ValueType::U32 | ValueType::I32) {
						return Err(VmError::TypeMismatch {
							expected: "u32 or i32".to_string(),
							found: target.value_type.name().to_string(),
						});
					}
					let index = target
						.index_expression
						.as_ref()
						.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
						.transpose()?;
					let value = self.compile_value_expression(&arguments[1], &target.value_type, descriptor_layouts)?;
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
				if !matches!(target.value_type, ValueType::U32 | ValueType::I32) {
					return Err(VmError::TypeMismatch {
						expected: "u32 or i32".to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let value_type = target.value_type.clone();
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let register = self.compile_value_expression(&arguments[1], &value_type, descriptor_layouts)?;
				self.emit_buffer_store(target, register);
				Ok(())
			}
			"atomic_exchange"
			| "atomic_add"
			| "atomic_sub"
			| "atomic_min"
			| "atomic_max"
			| "atomic_and"
			| "atomic_or"
			| "atomic_xor"
			| "atomic_compare_exchange" => {
				let return_type = match intrinsic.borrow().node() {
					Nodes::Intrinsic { r#return, .. } => resolve_value_type(r#return)?,
					_ => unreachable!("Statement intrinsic was already validated"),
				};
				self.compile_intrinsic_call_expression(intrinsic, arguments, &return_type, descriptor_layouts)?;
				Ok(())
			}
			"image_atomic_or" => {
				self.compile_intrinsic_call_expression(intrinsic, arguments, &ValueType::U32, descriptor_layouts)?;
				Ok(())
			}
			_ => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported intrinsic statement `{}`", name),
			}),
		}
	}
}
