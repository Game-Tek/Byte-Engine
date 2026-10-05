//! Bounded execution of compiled VM instructions against bound resources.

use super::*;

#[derive(Clone, Copy)]
enum CollectiveBehavior {
	Ignore,
	Suspend,
	Reject,
}

enum FrameOutcome {
	WorkgroupBarrier(usize),
	SubgroupCollective(usize),
	Complete(Option<Value>),
}

/// Records how an instruction changes the active frame.
enum InstructionProgress {
	Advance,
	JumpTo(usize),
	Complete(Option<Value>),
	SuspendWorkgroupBarrier,
	SuspendSubgroupCollective,
}

/// The `WorkgroupLane` struct gives the scheduler one resumable frame and execution budget per invocation.
struct WorkgroupLane<'a> {
	frame: ExecutionFrame,
	state: ExecutionState<'a>,
	status: WorkgroupLaneStatus,
}

/// The `WorkgroupLaneStatus` enum records where a VM invocation stopped between scheduler rounds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkgroupLaneStatus {
	Running,
	WorkgroupBarrier(usize),
	SubgroupCollective(usize),
	Complete,
}

/// Groups configured VM lanes by their workgroup-local subgroup index.
fn subgroup_lane_groups(configs: &[ExecutionConfig], subgroup_size: u32) -> Vec<Vec<usize>> {
	let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
	for (lane_index, config) in configs.iter().enumerate() {
		let subgroup_index = config.thread_idx() / subgroup_size;
		if let Some((_, lanes)) = groups.iter_mut().find(|(index, _)| *index == subgroup_index) {
			lanes.push(lane_index);
		} else {
			groups.push((subgroup_index, vec![lane_index]));
		}
	}
	groups.into_iter().map(|(_, lanes)| lanes).collect()
}

/// Resolves fixed and runtime-sized buffer indices against the active binding.
fn read_bound_buffer_array_index(
	registers: &[Option<Value>],
	register: usize,
	declared_count: Option<usize>,
	buffer: &Buffer,
	stride: usize,
) -> Result<usize, VmError> {
	if stride == 0 {
		return Err(VmError::UnsupportedBufferLayout {
			message: "Runtime-sized buffer elements must occupy at least one byte".to_string(),
		});
	}
	let count = declared_count.unwrap_or_else(|| buffer.bytes().len() / stride);
	read_buffer_array_index(registers, register, count)
}

impl ExecutableProgram {
	/// Executes the compiled `main` function using the currently bound descriptor resources.
	pub fn run_main(&self, descriptors: &mut DescriptorBindings<'_>) -> Result<(), VmError> {
		self.run_main_with_config(descriptors, &ExecutionConfig::default())
	}

	/// Executes `main` with explicit execution limits and shader invocation coordinates.
	///
	/// Each call represents one invocation. Workgroup and subgroup collectives
	/// preserve local instruction order but do not schedule or wait for peers.
	pub fn run_main_with_config(
		&self,
		descriptors: &mut DescriptorBindings<'_>,
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		let mut state = ExecutionState::new(config);
		state.enter_call()?;
		let mut frame = self.create_frame(descriptors, self.main_function)?;
		let outcome = self.execute_frame(&mut frame, descriptors, &mut state, CollectiveBehavior::Ignore);
		descriptors.release_execution_frame(frame);
		state.leave_call();
		let FrameOutcome::Complete(return_value) = outcome? else {
			unreachable!(
				"Unexpected suspended main frame. The most likely cause is that ignored shader collectives returned a scheduler-visible outcome."
			)
		};
		if return_value.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions must not return a value".to_string(),
			});
		}
		Ok(())
	}

	/// Executes every invocation in one task or compute workgroup with synchronized collectives and shared bound state.
	///
	/// Configurations run in slice order between collectives. This ordering makes
	/// atomic compaction deterministic for VM assertions. The scheduler rejects
	/// collectives in nested helper functions because only the `main` frame participates
	/// in the rendezvous. Bind shared storage with
	/// [`DescriptorBindings::bind_workgroup_state`] before calling this method.
	pub fn run_workgroup(&self, descriptors: &mut DescriptorBindings<'_>, configs: &[ExecutionConfig]) -> Result<(), VmError> {
		if configs.is_empty() {
			return Ok(());
		}
		let subgroup_size = configs[0].subgroup_size();
		if subgroup_size == 0 || subgroup_size > 128 {
			return Err(VmError::InvalidSubgroupSize { size: subgroup_size });
		}
		if let Some(config) = configs.iter().find(|config| config.subgroup_size() != subgroup_size) {
			return Err(VmError::MismatchedSubgroupSize {
				expected: subgroup_size,
				found: config.subgroup_size(),
			});
		}
		let subgroups = subgroup_lane_groups(configs, subgroup_size);

		descriptors.begin_workgroup();
		let mut lanes: Vec<WorkgroupLane<'_>> = Vec::with_capacity(configs.len());
		for config in configs {
			let mut state = ExecutionState::new(config);
			if let Err(error) = state.enter_call() {
				for lane in lanes {
					descriptors.release_execution_frame(lane.frame);
				}
				return Err(error);
			}
			match self.create_frame(descriptors, self.main_function) {
				Ok(frame) => lanes.push(WorkgroupLane {
					frame,
					state,
					status: WorkgroupLaneStatus::Running,
				}),
				Err(error) => {
					for lane in lanes {
						descriptors.release_execution_frame(lane.frame);
					}
					return Err(error);
				}
			}
		}

		let result = (|| loop {
			for lane in &mut lanes {
				if lane.status != WorkgroupLaneStatus::Running {
					continue;
				}
				match self.execute_frame(&mut lane.frame, descriptors, &mut lane.state, CollectiveBehavior::Suspend)? {
					FrameOutcome::WorkgroupBarrier(instruction_index) => {
						lane.status = WorkgroupLaneStatus::WorkgroupBarrier(instruction_index);
					}
					FrameOutcome::SubgroupCollective(instruction_index) => {
						lane.status = WorkgroupLaneStatus::SubgroupCollective(instruction_index);
					}
					FrameOutcome::Complete(return_value) => {
						lane.state.leave_call();
						if return_value.is_some() {
							return Err(VmError::UnsupportedMainSignature {
								message: "Main functions must not return a value".to_string(),
							});
						}
						lane.status = WorkgroupLaneStatus::Complete;
					}
				}
			}

			if self.resume_ready_subgroups(&mut lanes, &subgroups, subgroup_size)? {
				continue;
			}
			if self.resume_workgroup_barrier(&mut lanes)? {
				continue;
			}
			if lanes.iter().all(|lane| lane.status == WorkgroupLaneStatus::Complete) {
				return Ok(());
			}

			return Err(VmError::UnsupportedStatement {
				message: "Workgroup execution stalled without a resolvable collective".to_string(),
			});
		})();
		for lane in lanes {
			descriptors.release_execution_frame(lane.frame);
		}
		result
	}

	/// Resumes every subgroup whose active lanes reached the same collective instruction.
	fn resume_ready_subgroups(
		&self,
		lanes: &mut [WorkgroupLane<'_>],
		subgroups: &[Vec<usize>],
		subgroup_size: u32,
	) -> Result<bool, VmError> {
		let mut resumed = false;
		for subgroup in subgroups {
			let Some((first_lane, expected_instruction)) =
				subgroup.iter().find_map(|lane_index| match lanes[*lane_index].status {
					WorkgroupLaneStatus::SubgroupCollective(instruction_index) => Some((*lane_index, instruction_index)),
					_ => None,
				})
			else {
				continue;
			};

			for lane_index in subgroup {
				match lanes[*lane_index].status {
					WorkgroupLaneStatus::SubgroupCollective(found_instruction) if found_instruction == expected_instruction => {
					}
					WorkgroupLaneStatus::SubgroupCollective(found_instruction) => {
						return Err(VmError::DivergentSubgroupCollective {
							lane: *lane_index,
							expected_instruction,
							found_instruction: Some(found_instruction),
						});
					}
					WorkgroupLaneStatus::Complete | WorkgroupLaneStatus::Running | WorkgroupLaneStatus::WorkgroupBarrier(_) => {
						return Err(VmError::DivergentSubgroupCollective {
							lane: *lane_index,
							expected_instruction,
							found_instruction: None,
						});
					}
				}
			}

			self.resume_subgroup_collective(lanes, subgroup, first_lane, expected_instruction, subgroup_size)?;
			resumed = true;
		}
		Ok(resumed)
	}

	/// Resolves one collective after every active lane in its subgroup reached it.
	fn resume_subgroup_collective(
		&self,
		lanes: &mut [WorkgroupLane<'_>],
		subgroup: &[usize],
		first_lane: usize,
		instruction_index: usize,
		subgroup_size: u32,
	) -> Result<(), VmError> {
		let function_index = lanes[first_lane].frame.function_index;
		let instruction = self
			.functions
			.get(function_index)
			.and_then(|function| function.instructions.get(instruction_index))
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown subgroup collective instruction {instruction_index}"),
			})?;

		for lane_index in subgroup {
			if lanes[*lane_index].frame.function_index != function_index
				|| lanes[*lane_index].frame.instruction_index != instruction_index
			{
				return Err(VmError::DivergentSubgroupCollective {
					lane: *lane_index,
					expected_instruction: instruction_index,
					found_instruction: Some(lanes[*lane_index].frame.instruction_index),
				});
			}
		}

		match instruction {
			Instruction::SubgroupBallot { register, predicate } => {
				let mut mask = [0; 4];
				for lane_index in subgroup {
					let predicate = expect_bool(register_ref(&lanes[*lane_index].frame.registers, *predicate)?)?;
					if predicate {
						let local_lane = lanes[*lane_index].state.config.thread_idx() % subgroup_size;
						mask[(local_lane / 32) as usize] |= 1 << (local_lane % 32);
					}
				}
				for lane_index in subgroup {
					lanes[*lane_index].frame.registers[*register] = Some(Value::Vec4U(mask));
				}
			}
			Instruction::SubgroupBroadcast {
				register,
				value,
				source_lane,
				value_type,
			} => {
				// Every lane must name the same source lane, so the first lane's choice is the reference.
				let expected_source_lane = expect_u32(register_ref(&lanes[first_lane].frame.registers, *source_lane)?)?;
				for lane_index in subgroup {
					let found = expect_u32(register_ref(&lanes[*lane_index].frame.registers, *source_lane)?)?;
					if found != expected_source_lane {
						return Err(VmError::DivergentSubgroupBroadcastLane {
							lane: *lane_index,
							expected: expected_source_lane,
							found,
						});
					}
				}
				// A source lane at or past the subgroup size matches no lane, so this also rejects out-of-range lanes.
				let source = subgroup
					.iter()
					.copied()
					.find(|lane_index| lanes[*lane_index].state.config.thread_idx() % subgroup_size == expected_source_lane)
					.ok_or(VmError::SubgroupBroadcastLaneOutOfRange {
						source_lane: expected_source_lane,
						subgroup_size,
					})?;
				let source_value = register_ref(&lanes[source].frame.registers, *value)?;
				if !source_value.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: source_value.value_type().name().to_string(),
					});
				}
				let source_value = source_value.clone();
				for lane_index in subgroup {
					lanes[*lane_index].frame.registers[*register] = Some(source_value.clone());
				}
			}
			_ => {
				return Err(VmError::UnsupportedStatement {
					message: "Expected a subgroup collective instruction".to_string(),
				});
			}
		}

		for lane_index in subgroup {
			lanes[*lane_index].frame.instruction_index += 1;
			lanes[*lane_index].status = WorkgroupLaneStatus::Running;
		}
		Ok(())
	}

	/// Resumes a workgroup only when every invocation reached the same barrier.
	fn resume_workgroup_barrier(&self, lanes: &mut [WorkgroupLane<'_>]) -> Result<bool, VmError> {
		let Some(expected_instruction) = lanes.iter().find_map(|lane| match lane.status {
			WorkgroupLaneStatus::WorkgroupBarrier(instruction_index) => Some(instruction_index),
			_ => None,
		}) else {
			return Ok(false);
		};

		for (lane_index, lane) in lanes.iter().enumerate() {
			match lane.status {
				WorkgroupLaneStatus::WorkgroupBarrier(found_instruction) if found_instruction == expected_instruction => {}
				WorkgroupLaneStatus::WorkgroupBarrier(found_instruction) => {
					return Err(VmError::DivergentWorkgroupBarrier {
						lane: lane_index,
						expected_instruction,
						found_instruction: Some(found_instruction),
					});
				}
				WorkgroupLaneStatus::Complete | WorkgroupLaneStatus::Running | WorkgroupLaneStatus::SubgroupCollective(_) => {
					return Err(VmError::DivergentWorkgroupBarrier {
						lane: lane_index,
						expected_instruction,
						found_instruction: None,
					});
				}
			}
		}

		for lane in lanes {
			lane.status = WorkgroupLaneStatus::Running;
		}
		Ok(true)
	}

	/// Executes one nested function while sharing invocation limits and selecting how its collectives participate.
	fn execute_function(
		&self,
		function_index: usize,
		argument_registers: &[usize],
		caller_registers: &[Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
		collective_behavior: CollectiveBehavior,
	) -> Result<Option<Value>, VmError> {
		state.enter_call()?;
		let result = (|| {
			let mut frame =
				self.create_frame_from_registers(descriptors, function_index, argument_registers, caller_registers)?;
			let outcome = self.execute_frame(&mut frame, descriptors, state, collective_behavior);
			descriptors.release_execution_frame(frame);
			match outcome? {
				FrameOutcome::Complete(value) => Ok(value),
				FrameOutcome::WorkgroupBarrier(_) | FrameOutcome::SubgroupCollective(_) => unreachable!(
					"Unexpected nested collective suspension. The most likely cause is that nested execution stopped rejecting shader collectives."
				),
			}
		})();
		state.leave_call();
		result
	}

	/// Reuses one retained frame for a parameterless entry function.
	fn create_frame(&self, descriptors: &mut DescriptorBindings<'_>, function_index: usize) -> Result<ExecutionFrame, VmError> {
		let function = self.function(function_index)?;
		let mut frame = descriptors.take_execution_frame();
		match frame.reset(function_index, function) {
			Ok(()) => Ok(frame),
			Err(error) => {
				descriptors.release_execution_frame(frame);
				Err(error)
			}
		}
	}

	/// Reuses one retained frame and populates its parameter locals from caller registers.
	fn create_frame_from_registers(
		&self,
		descriptors: &mut DescriptorBindings<'_>,
		function_index: usize,
		argument_registers: &[usize],
		caller_registers: &[Option<Value>],
	) -> Result<ExecutionFrame, VmError> {
		let function = self.function(function_index)?;
		let mut frame = descriptors.take_execution_frame();
		match frame.reset_from_registers(function_index, function, argument_registers, caller_registers) {
			Ok(()) => Ok(frame),
			Err(error) => {
				descriptors.release_execution_frame(frame);
				Err(error)
			}
		}
	}

	/// Resolves one compiled function before its frame borrows the function's storage requirements.
	fn function(&self, function_index: usize) -> Result<&ExecutableFunction, VmError> {
		self.functions
			.get(function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", function_index),
			})
	}

	/// Runs one function frame until it returns or reaches a scheduler-visible collective.
	fn execute_frame(
		&self,
		frame: &mut ExecutionFrame,
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
		collective_behavior: CollectiveBehavior,
	) -> Result<FrameOutcome, VmError> {
		let function = self
			.functions
			.get(frame.function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", frame.function_index),
			})?;

		while frame.instruction_index < function.instructions.len() {
			state.consume_instruction()?;
			let instruction = &function.instructions[frame.instruction_index];
			match self.execute_instruction(instruction, frame, descriptors, state, collective_behavior)? {
				InstructionProgress::Advance => frame.instruction_index += 1,
				InstructionProgress::JumpTo(target) => frame.instruction_index = target,
				InstructionProgress::Complete(value) => return Ok(FrameOutcome::Complete(value)),
				InstructionProgress::SuspendWorkgroupBarrier => {
					let barrier_instruction = frame.instruction_index;
					frame.instruction_index += 1;
					return Ok(FrameOutcome::WorkgroupBarrier(barrier_instruction));
				}
				InstructionProgress::SuspendSubgroupCollective => {
					return Ok(FrameOutcome::SubgroupCollective(frame.instruction_index));
				}
			}
		}

		match &function.return_type {
			Some(return_type) => Err(VmError::UnsupportedStatement {
				message: format!(
					"Function with return type `{}` ended without returning a value",
					return_type.name()
				),
			}),
			None => Ok(FrameOutcome::Complete(None)),
		}
	}

	/// Executes one instruction while leaving frame-progress transitions to [`Self::execute_frame`].
	fn execute_instruction(
		&self,
		instruction: &Instruction,
		frame: &mut ExecutionFrame,
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
		collective_behavior: CollectiveBehavior,
	) -> Result<InstructionProgress, VmError> {
		let registers = &mut frame.registers;
		match instruction {
			Instruction::Value(instruction) => {
				Self::execute_value_instruction(instruction, registers, &mut frame.constructor_values)?
			}
			Instruction::Numeric(instruction) => Self::execute_numeric_instruction(instruction, registers)?,
			Instruction::Local(instruction) => {
				Self::execute_local_instruction(instruction, registers, &mut frame.locals, state.config)?
			}
			Instruction::Workgroup(instruction) => {
				Self::execute_workgroup_instruction(instruction, registers, descriptors, state.config)?
			}
			Instruction::MeshOutput(instruction) => {
				Self::execute_mesh_output_instruction(instruction, registers, descriptors, state.config)?
			}
			Instruction::Buffer(instruction) => Self::execute_buffer_instruction(instruction, registers, descriptors)?,
			Instruction::Texture(instruction) => Self::execute_texture_instruction(instruction, registers, descriptors)?,
			Instruction::Image(instruction) => return Self::execute_image_instruction(instruction, registers, descriptors),
			Instruction::Control(instruction) => {
				return self.execute_control_instruction(instruction, registers, descriptors, state, collective_behavior);
			}
			Instruction::SubgroupBallot { register, predicate } => {
				return Self::execute_subgroup_ballot(*register, *predicate, registers, collective_behavior);
			}
			Instruction::SubgroupBroadcast { .. } => return Self::execute_subgroup_broadcast(collective_behavior),
			Instruction::WorkgroupBarrier => return Self::execute_workgroup_barrier(collective_behavior),
		}
		Ok(InstructionProgress::Advance)
	}

	/// Executes instructions that construct or extract register values.
	fn execute_value_instruction(
		instruction: &ValueInstruction,
		registers: &mut [Option<Value>],
		constructor_values: &mut Vec<Value>,
	) -> Result<(), VmError> {
		match instruction {
			ValueInstruction::LoadLiteral { register, value } => registers[*register] = Some(value.clone()),
			ValueInstruction::LoadResourceIndexed {
				register,
				slot,
				index,
				count,
				value_type,
			} => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				if index >= *count {
					return Err(VmError::DescriptorArrayIndexOutOfBounds {
						slot: *slot,
						index,
						count: *count,
					});
				}
				// Lowering validates the whole slot range, and the bounded index preserves that proof.
				registers[*register] = Some(Value::Resource {
					slot: ResourceSlot::new(slot.slot() + index as u32),
					value_type: value_type.clone(),
				});
			}
			ValueInstruction::Construct {
				register,
				value_type,
				components,
			} => {
				// Constructors are frequent in shader code. Retain this frame-local scratch vector instead of allocating per instruction.
				constructor_values.clear();
				for component in components {
					constructor_values.push(register_ref(registers, *component)?.clone());
				}
				registers[*register] = Some(construct_value(value_type, constructor_values)?);
			}
			ValueInstruction::Extract {
				register,
				source,
				index,
				value_type,
			} => {
				let source = register_ref(registers, *source)?;
				registers[*register] = Some(extract_value(source, *index, value_type)?);
			}
			ValueInstruction::Insert {
				register,
				source,
				index,
				value,
			} => {
				let mut aggregate = register_ref(registers, *source)?.clone();
				insert_value(&mut aggregate, *index, register_ref(registers, *value)?.clone())?;
				registers[*register] = Some(aggregate);
			}
			ValueInstruction::ExtractDynamic {
				register,
				source,
				index,
				count,
				value_type,
			} => {
				let source = register_ref(registers, *source)?;
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				if index >= *count {
					return Err(VmError::BufferArrayIndexOutOfBounds { index, count: *count });
				}
				registers[*register] = Some(extract_value(source, index, value_type)?);
			}
			ValueInstruction::InsertDynamic {
				register,
				source,
				index,
				count,
				value,
			} => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				if index >= *count {
					return Err(VmError::BufferArrayIndexOutOfBounds { index, count: *count });
				}
				let mut aggregate = register_ref(registers, *source)?.clone();
				insert_value(&mut aggregate, index, register_ref(registers, *value)?.clone())?;
				registers[*register] = Some(aggregate);
			}
		}
		Ok(())
	}

	/// Executes arithmetic and scalar operations against frame registers.
	fn execute_numeric_instruction(instruction: &NumericInstruction, registers: &mut [Option<Value>]) -> Result<(), VmError> {
		match instruction {
			NumericInstruction::Arithmetic {
				register,
				operator,
				left,
				right,
			} => {
				let left = register_ref(registers, *left)?;
				let right = register_ref(registers, *right)?;
				registers[*register] = Some(apply_arithmetic(*operator, left, right)?);
			}
			NumericInstruction::Compare {
				register,
				operator,
				left,
				right,
			} => {
				let left = register_ref(registers, *left)?;
				let right = register_ref(registers, *right)?;
				registers[*register] = Some(apply_comparison(*operator, left, right)?);
			}
			NumericInstruction::DotProduct { register, left, right } => {
				let left = register_ref(registers, *left)?;
				let right = register_ref(registers, *right)?;
				registers[*register] = Some(apply_dot_product(left, right)?);
			}
			NumericInstruction::CrossProduct { register, left, right } => {
				let left = register_ref(registers, *left)?;
				let right = register_ref(registers, *right)?;
				registers[*register] = Some(apply_cross_product(left, right)?);
			}
			NumericInstruction::Length { register, value } => {
				let value = register_ref(registers, *value)?;
				registers[*register] = Some(apply_length(value)?);
			}
			NumericInstruction::Normalize { register, value } => {
				let value = register_ref(registers, *value)?;
				registers[*register] = Some(apply_normalize(value)?);
			}
			NumericInstruction::Reflect {
				register,
				incident,
				normal,
			} => {
				let incident = register_ref(registers, *incident)?;
				let normal = register_ref(registers, *normal)?;
				registers[*register] = Some(apply_reflect(incident, normal)?);
			}
			NumericInstruction::UnaryScalar {
				register,
				operator,
				value,
			} => {
				let value = register_ref(registers, *value)?;
				registers[*register] = Some(apply_scalar_unary(*operator, value)?);
			}
			NumericInstruction::FloatPredicate {
				register,
				predicate,
				value,
			} => {
				let value = register_ref(registers, *value)?;
				registers[*register] = Some(apply_float_predicate(*predicate, value)?);
			}
			NumericInstruction::RoundToVec2I { register, value } => {
				let value = register_ref(registers, *value)?;
				let &Value::Vec2F(value) = value else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2F.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				};
				registers[*register] = Some(Value::Vec2I(value.map(|component| component.round() as i32)));
			}
			NumericInstruction::BinaryScalar {
				register,
				operator,
				left,
				right,
			} => {
				let left = register_ref(registers, *left)?;
				let right = register_ref(registers, *right)?;
				registers[*register] = Some(apply_scalar_binary(*operator, left, right)?);
			}
			NumericInstruction::TernaryScalar {
				register,
				operator,
				first,
				second,
				third,
			} => {
				let first = register_ref(registers, *first)?;
				let second = register_ref(registers, *second)?;
				let third = register_ref(registers, *third)?;
				registers[*register] = Some(apply_scalar_ternary(*operator, first, second, third)?);
			}
		}
		Ok(())
	}

	/// Executes local storage, invocation-coordinate, and non-suspending subgroup instructions.
	fn execute_local_instruction(
		instruction: &LocalInstruction,
		registers: &mut [Option<Value>],
		locals: &mut [Option<Value>],
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		match instruction {
			LocalInstruction::LoadLocal { register, local } => {
				let value = locals
					.get(*local)
					.and_then(Option::clone)
					.ok_or(VmError::UninitializedLocal { local: *local })?;
				registers[*register] = Some(value);
			}
			LocalInstruction::StoreLocal { local, register } => {
				locals[*local] = Some(register_ref(registers, *register)?.clone());
			}
			LocalInstruction::LoadBuiltin { register, builtin } => registers[*register] = Some(builtin.value(config)),
			LocalInstruction::SubgroupMask {
				register,
				operator,
				mask,
			} => {
				let mask = expect_vec4u(register_ref(registers, *mask)?)?;
				registers[*register] = Some(operator.apply(mask));
			}
			LocalInstruction::SubgroupBallotAndNot { register, mask, removed } => {
				let mask = expect_vec4u(register_ref(registers, *mask)?)?;
				let removed = expect_vec4u(register_ref(registers, *removed)?)?;
				registers[*register] = Some(Value::Vec4U(std::array::from_fn(|index| mask[index] & !removed[index])));
			}
		}
		Ok(())
	}

	/// Executes one subgroup ballot according to its scheduler behavior.
	fn execute_subgroup_ballot(
		register: usize,
		predicate: usize,
		registers: &mut [Option<Value>],
		collective_behavior: CollectiveBehavior,
	) -> Result<InstructionProgress, VmError> {
		match collective_behavior {
			CollectiveBehavior::Ignore => {
				let predicate = expect_bool(register_ref(registers, predicate)?)?;
				registers[register] = Some(Value::Vec4U([predicate as u32, 0, 0, 0]));
				Ok(InstructionProgress::Advance)
			}
			CollectiveBehavior::Suspend => Ok(InstructionProgress::SuspendSubgroupCollective),
			CollectiveBehavior::Reject => Err(VmError::UnsupportedStatement {
				message: "Subgroup collectives inside called functions cannot participate in workgroup rendezvous".to_string(),
			}),
		}
	}

	/// Rejects or suspends subgroup broadcasts when peer lanes are required.
	fn execute_subgroup_broadcast(collective_behavior: CollectiveBehavior) -> Result<InstructionProgress, VmError> {
		match collective_behavior {
			CollectiveBehavior::Ignore => Err(VmError::UnsupportedStatement {
				message: "Subgroup broadcasts require run_workgroup so the VM can supply peer lanes".to_string(),
			}),
			CollectiveBehavior::Suspend => Ok(InstructionProgress::SuspendSubgroupCollective),
			CollectiveBehavior::Reject => Err(VmError::UnsupportedStatement {
				message: "Subgroup collectives inside called functions cannot participate in workgroup rendezvous".to_string(),
			}),
		}
	}

	/// Resolves one workgroup barrier according to its scheduler behavior.
	fn execute_workgroup_barrier(collective_behavior: CollectiveBehavior) -> Result<InstructionProgress, VmError> {
		match collective_behavior {
			// A single invocation has no peers to await, so ordinary execution preserves program order.
			CollectiveBehavior::Ignore => Ok(InstructionProgress::Advance),
			CollectiveBehavior::Suspend => Ok(InstructionProgress::SuspendWorkgroupBarrier),
			CollectiveBehavior::Reject => Err(VmError::UnsupportedStatement {
				message: "Workgroup barriers inside called functions cannot participate in workgroup rendezvous".to_string(),
			}),
		}
	}

	/// Executes task-payload and workgroup-storage instructions.
	fn execute_workgroup_instruction(
		instruction: &WorkgroupInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		match instruction {
			WorkgroupInstruction::LoadTaskPayload {
				register,
				name,
				index,
				count,
				value_type,
			} => {
				let index = read_buffer_array_index(registers, *index, *count)?;
				let value = descriptors.task_payload_value(name, index)?;
				if !value.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				}
				registers[*register] = Some(value);
			}
			WorkgroupInstruction::StoreTaskPayload {
				name,
				index,
				count,
				value_type,
				value,
			} => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				let value = register_ref(registers, *value)?;
				if !value.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				}
				descriptors
					.task_outputs_mut()?
					.write_payload(name, index, *count, value.clone())?;
			}
			WorkgroupInstruction::LoadWorkgroup {
				register,
				name,
				index,
				count,
				value_type,
			} => {
				let index = index
					.map(|index| register_ref(registers, index).and_then(expect_u32))
					.transpose()?
					.unwrap_or(0) as usize;
				let value = descriptors.workgroup_state_mut()?.load(name, index, *count, value_type)?;
				registers[*register] = Some(value);
			}
			WorkgroupInstruction::StoreWorkgroup {
				name,
				index,
				count,
				value_type,
				value,
			} => {
				let index = index
					.map(|index| register_ref(registers, index).and_then(expect_u32))
					.transpose()?
					.unwrap_or(0) as usize;
				let value = register_ref(registers, *value)?;
				descriptors
					.workgroup_state_mut()?
					.store(name, index, *count, value_type, value.clone())?;
			}
			WorkgroupInstruction::AtomicWorkgroup {
				register,
				operation,
				name,
				index,
				count,
				value_type,
				value,
			} => {
				let index = index
					.map(|index| register_ref(registers, index).and_then(expect_u32))
					.transpose()?
					.unwrap_or(0) as usize;
				let value = register_ref(registers, *value)?;
				if !value.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				}
				let previous = descriptors
					.workgroup_state_mut()?
					.atomic_apply(name, index, *count, *operation, value)?;
				registers[*register] = Some(previous);
			}
			WorkgroupInstruction::AtomicCompareExchangeWorkgroup {
				register,
				name,
				index,
				count,
				value_type,
				expected,
				desired,
			} => {
				let index = index
					.map(|index| register_ref(registers, index).and_then(expect_u32))
					.transpose()?
					.unwrap_or(0) as usize;
				let expected = register_ref(registers, *expected)?;
				let desired = register_ref(registers, *desired)?;
				if !expected.matches_type(value_type) || !desired.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: format!("{} and {}", expected.value_type().name(), desired.value_type().name()),
					});
				}
				let previous = descriptors
					.workgroup_state_mut()?
					.atomic_compare_exchange(name, index, *count, expected, desired)?;
				registers[*register] = Some(previous);
			}
			WorkgroupInstruction::SetTaskMeshOutputCount { count } => {
				let count = expect_u32(register_ref(registers, *count)?)?;
				if count > config.max_task_mesh_output_count() {
					return Err(VmError::TaskMeshOutputCountLimitExceeded {
						requested: count,
						limit: config.max_task_mesh_output_count(),
					});
				}
				descriptors.task_outputs_mut()?.set_mesh_output_count(count);
			}
		}
		Ok(())
	}

	/// Executes mesh-output instructions against the bound output capture.
	fn execute_mesh_output_instruction(
		instruction: &MeshOutputInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		match instruction {
			MeshOutputInstruction::Counts {
				vertex_count,
				primitive_count,
			} => {
				let vertex_count = expect_u32(register_ref(registers, *vertex_count)?)?;
				let primitive_count = expect_u32(register_ref(registers, *primitive_count)?)?;
				descriptors.mesh_outputs_mut()?.set_counts(
					vertex_count,
					primitive_count,
					config.max_mesh_vertex_count(),
					config.max_mesh_primitive_count(),
					config.thread_idx() == 0,
				)?;
			}
			MeshOutputInstruction::VertexPosition { index, position } => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				let position = register_ref(registers, *position)?;
				let &Value::Vec4F(position) = position else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec4F.name().to_string(),
						found: position.value_type().name().to_string(),
					});
				};
				let outputs = descriptors.mesh_outputs_mut()?;
				let count = outputs.vertex_positions.len();
				let destination = outputs
					.vertex_positions
					.get_mut(index)
					.ok_or(VmError::MeshOutputIndexOutOfBounds {
						kind: "vertex",
						index,
						count,
					})?;
				*destination = position;
			}
			MeshOutputInstruction::Triangle { index, triangle } => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				let triangle = register_ref(registers, *triangle)?;
				let &Value::Vec3U(triangle) = triangle else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec3U.name().to_string(),
						found: triangle.value_type().name().to_string(),
					});
				};
				let outputs = descriptors.mesh_outputs_mut()?;
				let count = outputs.triangles.len();
				let destination = outputs.triangles.get_mut(index).ok_or(VmError::MeshOutputIndexOutOfBounds {
					kind: "primitive",
					index,
					count,
				})?;
				*destination = triangle;
			}
			MeshOutputInstruction::PrimitiveRenderTargetArrayIndex { index, array_index } => {
				let index = expect_u32(register_ref(registers, *index)?)? as usize;
				let array_index = expect_u32(register_ref(registers, *array_index)?)?;
				let outputs = descriptors.mesh_outputs_mut()?;
				let count = outputs.render_target_array_indices.len();
				let destination =
					outputs
						.render_target_array_indices
						.get_mut(index)
						.ok_or(VmError::MeshOutputIndexOutOfBounds {
							kind: "primitive",
							index,
							count,
						})?;
				*destination = array_index;
			}
		}
		Ok(())
	}

	/// Executes reads, writes, and atomics against bound buffers.
	fn execute_buffer_instruction(
		instruction: &BufferInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
	) -> Result<(), VmError> {
		match instruction {
			BufferInstruction::LoadBuffer {
				register,
				slot,
				offset,
				value_type,
			} => {
				let value = if *slot == PUSH_CONSTANT_SLOT {
					descriptors.push_constant_mut()?.read_value(*offset, value_type)?
				} else {
					descriptors.buffer_mut(*slot)?.read_value(*offset, value_type)?
				};
				registers[*register] = Some(value);
			}
			BufferInstruction::LoadBufferIndexed {
				register,
				slot,
				offset,
				stride,
				count,
				index,
				value_type,
			} => {
				let buffer = if *slot == PUSH_CONSTANT_SLOT {
					descriptors.push_constant_mut()?
				} else {
					descriptors.buffer_mut(*slot)?
				};
				let index = read_bound_buffer_array_index(registers, *index, *count, buffer, *stride)?;
				let value = buffer.read_value(*offset + *stride * index, value_type)?;
				registers[*register] = Some(value);
			}
			BufferInstruction::StoreBuffer {
				slot,
				offset,
				value_type,
				register,
			} => {
				let value = register_ref(registers, *register)?;
				descriptors.buffer_mut(*slot)?.write_value(*offset, value_type, value)?;
			}
			BufferInstruction::StoreBufferIndexed {
				slot,
				offset,
				stride,
				count,
				index,
				value_type,
				register,
			} => {
				let value = register_ref(registers, *register)?;
				let buffer = descriptors.buffer_mut(*slot)?;
				let index = read_bound_buffer_array_index(registers, *index, *count, buffer, *stride)?;
				buffer.write_value(*offset + *stride * index, value_type, value)?;
			}
			BufferInstruction::AtomicBuffer {
				register,
				operation,
				slot,
				offset,
				stride,
				count,
				index,
				value_type,
				value,
			} => {
				let value = register_ref(registers, *value)?;
				if !value.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				}
				let buffer = descriptors.buffer_mut(*slot)?;
				let index = match index {
					Some(index) => read_bound_buffer_array_index(registers, *index, *count, buffer, *stride)?,
					None => 0,
				};
				let address = *offset + *stride * index;
				let previous = buffer.read_value(address, value_type)?;
				let replacement = apply_atomic_operation(*operation, &previous, value)?;
				buffer.write_value(address, value_type, &replacement)?;
				registers[*register] = Some(previous);
			}
			BufferInstruction::AtomicCompareExchangeBuffer {
				register,
				slot,
				offset,
				stride,
				count,
				index,
				value_type,
				expected,
				desired,
			} => {
				let expected = register_ref(registers, *expected)?;
				let desired = register_ref(registers, *desired)?;
				if !expected.matches_type(value_type) || !desired.matches_type(value_type) {
					return Err(VmError::TypeMismatch {
						expected: value_type.name().to_string(),
						found: format!("{} and {}", expected.value_type().name(), desired.value_type().name()),
					});
				}
				let buffer = descriptors.buffer_mut(*slot)?;
				let index = match index {
					Some(index) => read_bound_buffer_array_index(registers, *index, *count, buffer, *stride)?,
					None => 0,
				};
				let address = *offset + *stride * index;
				let previous = buffer.read_value(address, value_type)?;
				if previous == *expected {
					buffer.write_value(address, value_type, desired)?;
				}
				registers[*register] = Some(previous);
			}
		}
		Ok(())
	}

	/// Executes texture fetch, sample, and size instructions.
	fn execute_texture_instruction(
		instruction: &TextureInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
	) -> Result<(), VmError> {
		match instruction {
			TextureInstruction::FetchTexture { register, slot, coord } => {
				let coord = register_ref(registers, *coord)?;
				let &Value::Vec2U(coord) = coord else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2U.name().to_string(),
						found: coord.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.texture_mut(slot)?.fetch(coord)?);
			}
			TextureInstruction::GatherTexture {
				register,
				slot,
				uv,
				layer,
			} => {
				let uv = register_ref(registers, *uv)?;
				let &Value::Vec2F(uv) = uv else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2F.name().to_string(),
						found: uv.value_type().name().to_string(),
					});
				};
				let layer = match layer {
					Some(layer) => {
						let layer = register_ref(registers, *layer)?;
						let &Value::U32(layer) = layer else {
							return Err(VmError::TypeMismatch {
								expected: ValueType::U32.name().to_string(),
								found: layer.value_type().name().to_string(),
							});
						};
						layer
					}
					None => 0,
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.texture_mut(slot)?.gather(uv, layer)?);
			}
			TextureInstruction::FetchTextureArray {
				register,
				slot,
				coord,
				layer,
			} => {
				let coord = register_ref(registers, *coord)?;
				let &Value::Vec2U(coord) = coord else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2U.name().to_string(),
						found: coord.value_type().name().to_string(),
					});
				};
				let layer = register_ref(registers, *layer)?;
				let &Value::U32(layer) = layer else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::U32.name().to_string(),
						found: layer.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.texture_mut(slot)?.fetch_array(coord, layer)?);
			}
			TextureInstruction::FetchTextureU32 { register, slot, coord } => {
				let coord = register_ref(registers, *coord)?;
				let &Value::Vec2U(coord) = coord else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2U.name().to_string(),
						found: coord.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.texture_mut(slot)?.fetch_u32(coord)?);
			}
			TextureInstruction::SampleTexture {
				register,
				slot,
				uv,
				layer,
				lod,
				reduction_mode,
			} => {
				let uv = register_ref(registers, *uv)?;
				let &Value::Vec2F(uv) = uv else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2F.name().to_string(),
						found: uv.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				let (texture, sampler) = descriptors.texture_and_sampler_mut(slot)?;
				let sampler = reduction_mode.map(Sampler::new).unwrap_or(sampler);
				let lod = if let Some(lod) = lod {
					let lod = register_ref(registers, *lod)?;
					let &Value::F32(lod) = lod else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::F32.name().to_string(),
							found: lod.value_type().name().to_string(),
						});
					};
					Some(lod)
				} else {
					None
				};
				let sampled = if let Some(layer) = layer {
					let layer = register_ref(registers, *layer)?;
					let &Value::U32(layer) = layer else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::U32.name().to_string(),
							found: layer.value_type().name().to_string(),
						});
					};
					texture.sample_array_lod_with_sampler(uv, layer, lod.unwrap_or(0.0), sampler)?
				} else if let Some(lod) = lod {
					texture.sample_lod_with_sampler(uv, lod, sampler)?
				} else {
					texture.sample_with_sampler(uv, sampler)?
				};
				registers[*register] = Some(sampled);
			}
			TextureInstruction::SampleTexture3D { register, slot, uvw } => {
				let uvw = register_ref(registers, *uvw)?;
				let &Value::Vec3F(uvw) = uvw else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec3F.name().to_string(),
						found: uvw.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.texture_mut(slot)?.sample_3d(uvw)?);
			}
			TextureInstruction::TextureSize { register, slot } => {
				let slot = resolve_resource_slot(*slot, registers)?;
				let texture = descriptors.texture_mut(slot)?;
				registers[*register] = Some(Value::Vec2U([texture.width, texture.height]));
			}
		}
		Ok(())
	}

	/// Executes image reads, writes, atomics, and guard termination.
	fn execute_image_instruction(
		instruction: &ImageInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
	) -> Result<InstructionProgress, VmError> {
		match instruction {
			ImageInstruction::ImageSize { register, slot } => {
				let slot = resolve_resource_slot(*slot, registers)?;
				let image = descriptors.image_mut(slot)?;
				registers[*register] = Some(Value::Vec2U([image.width, image.height]));
			}
			ImageInstruction::LoadImage { register, slot, coord } => {
				let coord = expect_vec2u(register_ref(registers, *coord)?)?;
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.image_mut(slot)?.fetch(coord)?);
			}
			ImageInstruction::LoadImageU32 { register, slot, coord } => {
				let coord = expect_vec2u(register_ref(registers, *coord)?)?;
				let slot = resolve_resource_slot(*slot, registers)?;
				registers[*register] = Some(descriptors.image_mut(slot)?.fetch_u32(coord)?);
			}
			ImageInstruction::GuardImageBounds { slot, coord } => {
				let coord = expect_vec2u(register_ref(registers, *coord)?)?;
				let slot = resolve_resource_slot(*slot, registers)?;
				if !descriptors.image_mut(slot)?.contains_2d(coord) {
					return Ok(InstructionProgress::Complete(None));
				}
			}
			ImageInstruction::ImageAtomicOr {
				register,
				slot,
				coord,
				value,
			} => {
				let coord = expect_vec2u(register_ref(registers, *coord)?)?;
				let value = expect_u32(register_ref(registers, *value)?)?;
				let slot = resolve_resource_slot(*slot, registers)?;
				let previous = descriptors.image_mut(slot)?.atomic_or(coord, value)?;
				registers[*register] = Some(Value::U32(previous));
			}
			ImageInstruction::WriteImage { slot, coord, value } => {
				let coord = register_ref(registers, *coord)?;
				let &Value::Vec2U(coord) = coord else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec2U.name().to_string(),
						found: coord.value_type().name().to_string(),
					});
				};
				let value = register_ref(registers, *value)?;
				let &Value::Vec4F(value) = value else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec4F.name().to_string(),
						found: value.value_type().name().to_string(),
					});
				};
				let slot = resolve_resource_slot(*slot, registers)?;
				descriptors.image_mut(slot)?.write(coord, value)?;
			}
		}
		Ok(InstructionProgress::Advance)
	}

	/// Executes control-flow, discard, call, and return instructions.
	fn execute_control_instruction(
		&self,
		instruction: &ControlInstruction,
		registers: &mut [Option<Value>],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
		collective_behavior: CollectiveBehavior,
	) -> Result<InstructionProgress, VmError> {
		match instruction {
			ControlInstruction::JumpIfZero { register, target } => {
				let value = register_ref(registers, *register)?;
				if is_zero_value(value)? {
					Ok(InstructionProgress::JumpTo(*target))
				} else {
					Ok(InstructionProgress::Advance)
				}
			}
			ControlInstruction::Jump { target } => Ok(InstructionProgress::JumpTo(*target)),
			ControlInstruction::Switch {
				register,
				cases,
				default,
			} => {
				let label = switch_label(register_ref(registers, *register)?)?;
				let target = cases
					.binary_search_by_key(&label, |&(case, _)| case)
					.map_or(*default, |case| cases[case].1);
				Ok(InstructionProgress::JumpTo(target))
			}
			ControlInstruction::Discard => {
				state.discarded = true;
				Ok(InstructionProgress::Complete(None))
			}
			ControlInstruction::Call {
				register,
				function,
				arguments,
			} => {
				// Scheduled task lanes cannot preserve a nested call stack across a rendezvous; ordinary invocations may ignore it.
				let nested_collective_behavior = match collective_behavior {
					CollectiveBehavior::Ignore => CollectiveBehavior::Ignore,
					CollectiveBehavior::Suspend | CollectiveBehavior::Reject => CollectiveBehavior::Reject,
				};
				let value = self.execute_function(
					*function,
					arguments,
					registers,
					descriptors,
					state,
					nested_collective_behavior,
				)?;
				if state.discarded {
					return Ok(InstructionProgress::Complete(None));
				}
				if let Some(register) = register {
					registers[*register] = value;
				}
				Ok(InstructionProgress::Advance)
			}
			ControlInstruction::Return { register } => match register {
				Some(register) => Ok(InstructionProgress::Complete(Some(
					register_ref(registers, *register)?.clone(),
				))),
				None => Ok(InstructionProgress::Complete(None)),
			},
		}
	}
}

impl InvocationBuiltin {
	/// Reads this builtin from the invocation coordinates of `config`.
	fn value(self, config: &ExecutionConfig) -> Value {
		match self {
			InvocationBuiltin::ThreadIdx => Value::U32(config.thread_idx()),
			InvocationBuiltin::ThreadPosition => Value::U32(config.thread_position()),
			InvocationBuiltin::ThreadId => Value::Vec2U(config.thread_id()),
			InvocationBuiltin::ThreadgroupPosition => Value::U32(config.threadgroup_position()),
			InvocationBuiltin::SubgroupLaneIndex => Value::U32(config.thread_idx() % config.subgroup_size()),
		}
	}
}

impl SubgroupMaskOperator {
	/// Reduces one 128-lane ballot mask to the scalar this operator names.
	fn apply(self, mask: [u32; 4]) -> Value {
		match self {
			SubgroupMaskOperator::Any => Value::Bool(mask.into_iter().any(|word| word != 0)),
			SubgroupMaskOperator::FindLsb => Value::U32(
				mask.into_iter()
					.enumerate()
					.find_map(|(word_index, word)| (word != 0).then(|| word_index as u32 * 32 + word.trailing_zeros()))
					.unwrap_or(u32::MAX),
			),
			SubgroupMaskOperator::Count => Value::U32(mask.into_iter().map(u32::count_ones).sum()),
		}
	}
}
