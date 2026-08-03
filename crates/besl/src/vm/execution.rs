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

/// The `ExecutionFrame` struct lets one invocation resume after a scheduled barrier without replaying earlier instructions.
struct ExecutionFrame {
	function_index: usize,
	registers: Vec<Option<Value>>,
	locals: Vec<Option<Value>>,
	instruction_index: usize,
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
		let mut frame = self.create_frame(self.main_function, &[])?;
		let outcome = self.execute_frame(&mut frame, descriptors, &mut state, CollectiveBehavior::Ignore);
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
		let mut lanes = configs
			.iter()
			.map(|config| {
				let mut state = ExecutionState::new(config);
				state.enter_call()?;
				Ok(WorkgroupLane {
					frame: self.create_frame(self.main_function, &[])?,
					state,
					status: WorkgroupLaneStatus::Running,
				})
			})
			.collect::<Result<Vec<_>, VmError>>()?;

		loop {
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
		}
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
			.cloned()
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
					let predicate = expect_bool(read_register(&lanes[*lane_index].frame.registers, predicate)?)?;
					if predicate {
						let local_lane = lanes[*lane_index].state.config.thread_idx() % subgroup_size;
						mask[(local_lane / 32) as usize] |= 1 << (local_lane % 32);
					}
				}
				for lane_index in subgroup {
					lanes[*lane_index].frame.registers[register] = Some(Value::Vec4U(mask));
				}
			}
			Instruction::SubgroupBroadcastU32 {
				register,
				value,
				source_lane,
			} => {
				let expected_source_lane = expect_u32(read_register(&lanes[first_lane].frame.registers, source_lane)?)?;
				for lane_index in subgroup {
					let found_source_lane = expect_u32(read_register(&lanes[*lane_index].frame.registers, source_lane)?)?;
					if found_source_lane != expected_source_lane {
						return Err(VmError::DivergentSubgroupBroadcastLane {
							lane: *lane_index,
							expected: expected_source_lane,
							found: found_source_lane,
						});
					}
				}
				if expected_source_lane >= subgroup_size {
					return Err(VmError::SubgroupBroadcastLaneOutOfRange {
						source_lane: expected_source_lane,
						subgroup_size,
					});
				}
				let source = subgroup
					.iter()
					.copied()
					.find(|lane_index| lanes[*lane_index].state.config.thread_idx() % subgroup_size == expected_source_lane)
					.ok_or(VmError::SubgroupBroadcastLaneOutOfRange {
						source_lane: expected_source_lane,
						subgroup_size,
					})?;
				let value = expect_u32(read_register(&lanes[source].frame.registers, value)?)?;
				for lane_index in subgroup {
					lanes[*lane_index].frame.registers[register] = Some(Value::U32(value));
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
		arguments: &[Value],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
		collective_behavior: CollectiveBehavior,
	) -> Result<Option<Value>, VmError> {
		state.enter_call()?;
		let result = (|| {
			let mut frame = self.create_frame(function_index, arguments)?;
			match self.execute_frame(&mut frame, descriptors, state, collective_behavior)? {
				FrameOutcome::Complete(value) => Ok(value),
				FrameOutcome::WorkgroupBarrier(_) | FrameOutcome::SubgroupCollective(_) => unreachable!(
					"Unexpected nested collective suspension. The most likely cause is that nested execution stopped rejecting shader collectives."
				),
			}
		})();
		state.leave_call();
		result
	}

	/// Creates a resumable function frame with initialized argument locals.
	fn create_frame(&self, function_index: usize, arguments: &[Value]) -> Result<ExecutionFrame, VmError> {
		let function = self
			.functions
			.get(function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", function_index),
			})?;
		if arguments.len() != function.parameter_count {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: arguments.len(),
			});
		}
		let mut locals = vec![None; function.local_types.len()];
		for (index, argument) in arguments.iter().enumerate() {
			locals[index] = Some(argument.clone());
		}
		Ok(ExecutionFrame {
			function_index,
			registers: vec![None; function.register_count],
			locals,
			instruction_index: 0,
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
		let registers = &mut frame.registers;
		let locals = &mut frame.locals;

		while frame.instruction_index < function.instructions.len() {
			state.consume_instruction()?;
			let instruction = &function.instructions[frame.instruction_index];
			match instruction {
				Instruction::LoadLiteral { register, value } => {
					registers[*register] = Some(value.clone());
				}
				Instruction::Construct {
					register,
					value_type,
					components,
				} => {
					let values = components
						.iter()
						.map(|component| read_register(registers, *component))
						.collect::<Result<Vec<_>, _>>()?;
					registers[*register] = Some(construct_value(value_type, &values)?);
				}
				Instruction::Extract {
					register,
					source,
					index,
					value_type,
				} => {
					let source = read_register(registers, *source)?;
					registers[*register] = Some(extract_value(&source, *index, value_type)?);
				}
				Instruction::ExtractDynamic {
					register,
					source,
					index,
					count,
					value_type,
				} => {
					let source = read_register(registers, *source)?;
					let index = expect_u32(read_register(registers, *index)?)? as usize;
					if index >= *count {
						return Err(VmError::BufferArrayIndexOutOfBounds { index, count: *count });
					}
					registers[*register] = Some(extract_value(&source, index, value_type)?);
				}
				Instruction::Arithmetic {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(registers, *left)?;
					let right = read_register(registers, *right)?;
					registers[*register] = Some(apply_arithmetic(*operator, &left, &right)?);
				}
				Instruction::Compare {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(registers, *left)?;
					let right = read_register(registers, *right)?;
					registers[*register] = Some(apply_comparison(*operator, &left, &right)?);
				}
				Instruction::JumpIfZero { register, target } => {
					let value = read_register(registers, *register)?;
					if is_zero_value(&value)? {
						frame.instruction_index = *target;
						continue;
					}
				}
				Instruction::Jump { target } => {
					frame.instruction_index = *target;
					continue;
				}
				Instruction::DotProduct { register, left, right } => {
					let left = read_register(registers, *left)?;
					let right = read_register(registers, *right)?;
					registers[*register] = Some(apply_dot_product(&left, &right)?);
				}
				Instruction::CrossProduct { register, left, right } => {
					let left = read_register(registers, *left)?;
					let right = read_register(registers, *right)?;
					registers[*register] = Some(apply_cross_product(&left, &right)?);
				}
				Instruction::Length { register, value } => {
					let value = read_register(registers, *value)?;
					registers[*register] = Some(apply_length(&value)?);
				}
				Instruction::Normalize { register, value } => {
					let value = read_register(registers, *value)?;
					registers[*register] = Some(apply_normalize(&value)?);
				}
				Instruction::Reflect {
					register,
					incident,
					normal,
				} => {
					let incident = read_register(registers, *incident)?;
					let normal = read_register(registers, *normal)?;
					registers[*register] = Some(apply_reflect(&incident, &normal)?);
				}
				Instruction::UnaryScalar {
					register,
					operator,
					value,
				} => {
					let value = read_register(registers, *value)?;
					registers[*register] = Some(apply_scalar_unary(*operator, &value)?);
				}
				Instruction::RoundToVec2I { register, value } => {
					let value = read_register(registers, *value)?;
					let Value::Vec2F(value) = value else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2F.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					};
					registers[*register] = Some(Value::Vec2I(value.map(|component| component.round() as i32)));
				}
				Instruction::BinaryScalar {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(registers, *left)?;
					let right = read_register(registers, *right)?;
					registers[*register] = Some(apply_scalar_binary(*operator, &left, &right)?);
				}
				Instruction::TernaryScalar {
					register,
					operator,
					first,
					second,
					third,
				} => {
					let first = read_register(registers, *first)?;
					let second = read_register(registers, *second)?;
					let third = read_register(registers, *third)?;
					registers[*register] = Some(apply_scalar_ternary(*operator, &first, &second, &third)?);
				}
				Instruction::ThreadIdx { register } => {
					registers[*register] = Some(Value::U32(state.config.thread_idx()));
				}
				Instruction::ThreadPosition { register } => {
					registers[*register] = Some(Value::U32(state.config.thread_position()));
				}
				Instruction::ThreadId { register } => {
					registers[*register] = Some(Value::Vec2U(state.config.thread_id()));
				}
				Instruction::ThreadgroupPosition { register } => {
					registers[*register] = Some(Value::U32(state.config.threadgroup_position()));
				}
				Instruction::SubgroupBallot { register, predicate } => match collective_behavior {
					CollectiveBehavior::Ignore => {
						let predicate = expect_bool(read_register(registers, *predicate)?)?;
						registers[*register] = Some(Value::Vec4U([predicate as u32, 0, 0, 0]));
					}
					CollectiveBehavior::Suspend => {
						return Ok(FrameOutcome::SubgroupCollective(frame.instruction_index));
					}
					CollectiveBehavior::Reject => {
						return Err(VmError::UnsupportedStatement {
							message: "Subgroup collectives inside called functions cannot participate in workgroup rendezvous"
								.to_string(),
						});
					}
				},
				Instruction::SubgroupBallotAny { register, mask } => {
					let mask = expect_vec4u(read_register(registers, *mask)?)?;
					registers[*register] = Some(Value::Bool(mask.into_iter().any(|word| word != 0)));
				}
				Instruction::SubgroupBallotFindLsb { register, mask } => {
					let mask = expect_vec4u(read_register(registers, *mask)?)?;
					let first_lane = mask
						.into_iter()
						.enumerate()
						.find_map(|(word_index, word)| (word != 0).then(|| word_index as u32 * 32 + word.trailing_zeros()))
						.unwrap_or(u32::MAX);
					registers[*register] = Some(Value::U32(first_lane));
				}
				Instruction::SubgroupBallotCount { register, mask } => {
					let mask = expect_vec4u(read_register(registers, *mask)?)?;
					registers[*register] = Some(Value::U32(mask.into_iter().map(u32::count_ones).sum()));
				}
				Instruction::SubgroupBallotAndNot { register, mask, removed } => {
					let mask = expect_vec4u(read_register(registers, *mask)?)?;
					let removed = expect_vec4u(read_register(registers, *removed)?)?;
					registers[*register] = Some(Value::Vec4U(std::array::from_fn(|index| mask[index] & !removed[index])));
				}
				Instruction::SubgroupBroadcastU32 { .. } => match collective_behavior {
					CollectiveBehavior::Ignore => {
						return Err(VmError::UnsupportedStatement {
							message: "Subgroup broadcasts require run_workgroup so the VM can supply peer lanes".to_string(),
						});
					}
					CollectiveBehavior::Suspend => {
						return Ok(FrameOutcome::SubgroupCollective(frame.instruction_index));
					}
					CollectiveBehavior::Reject => {
						return Err(VmError::UnsupportedStatement {
							message: "Subgroup collectives inside called functions cannot participate in workgroup rendezvous"
								.to_string(),
						});
					}
				},
				Instruction::LoadTaskPayload {
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
				Instruction::StoreTaskPayload {
					name,
					index,
					count,
					value_type,
					value,
				} => {
					let index = expect_u32(read_register(registers, *index)?)? as usize;
					let value = read_register(registers, *value)?;
					if !value.matches_type(value_type) {
						return Err(VmError::TypeMismatch {
							expected: value_type.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					}
					descriptors.task_outputs_mut()?.write_payload(name, index, *count, value)?;
				}
				Instruction::LoadWorkgroup {
					register,
					name,
					index,
					count,
					value_type,
				} => {
					let index = index
						.map(|index| read_register(registers, index).and_then(expect_u32))
						.transpose()?
						.unwrap_or(0) as usize;
					let value = descriptors.workgroup_state_mut()?.load(name, index, *count, value_type)?;
					registers[*register] = Some(value);
				}
				Instruction::StoreWorkgroup {
					name,
					index,
					count,
					value_type,
					value,
				} => {
					let index = index
						.map(|index| read_register(registers, index).and_then(expect_u32))
						.transpose()?
						.unwrap_or(0) as usize;
					let value = read_register(registers, *value)?;
					descriptors
						.workgroup_state_mut()?
						.store(name, index, *count, value_type, value)?;
				}
				Instruction::AtomicAddWorkgroup {
					register,
					name,
					index,
					count,
					value,
				} => {
					let index = index
						.map(|index| read_register(registers, index).and_then(expect_u32))
						.transpose()?
						.unwrap_or(0) as usize;
					let value = expect_u32(read_register(registers, *value)?)?;
					let previous = descriptors
						.workgroup_state_mut()?
						.atomic_add_u32(name, index, *count, value)?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::AtomicCompareExchangeWorkgroup {
					register,
					name,
					index,
					count,
					expected,
					desired,
				} => {
					let index = index
						.map(|index| read_register(registers, index).and_then(expect_u32))
						.transpose()?
						.unwrap_or(0) as usize;
					let expected = expect_u32(read_register(registers, *expected)?)?;
					let desired = expect_u32(read_register(registers, *desired)?)?;
					let previous = descriptors
						.workgroup_state_mut()?
						.atomic_compare_exchange_u32(name, index, *count, expected, desired)?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::WorkgroupBarrier => match collective_behavior {
					CollectiveBehavior::Ignore => {
						// A single invocation has no peers to await, so ordinary execution preserves program order.
					}
					CollectiveBehavior::Suspend => {
						let barrier_instruction = frame.instruction_index;
						frame.instruction_index += 1;
						return Ok(FrameOutcome::WorkgroupBarrier(barrier_instruction));
					}
					CollectiveBehavior::Reject => {
						return Err(VmError::UnsupportedStatement {
							message: "Workgroup barriers inside called functions cannot participate in workgroup rendezvous"
								.to_string(),
						});
					}
				},
				Instruction::SetTaskMeshOutputCount { count } => {
					let count = expect_u32(read_register(registers, *count)?)?;
					if count > state.config.max_task_mesh_output_count() {
						return Err(VmError::TaskMeshOutputCountLimitExceeded {
							requested: count,
							limit: state.config.max_task_mesh_output_count(),
						});
					}
					descriptors.task_outputs_mut()?.set_mesh_output_count(count);
				}
				Instruction::SetMeshOutputCounts {
					vertex_count,
					primitive_count,
				} => {
					let vertex_count = expect_u32(read_register(registers, *vertex_count)?)?;
					let primitive_count = expect_u32(read_register(registers, *primitive_count)?)?;
					descriptors.mesh_outputs_mut()?.set_counts(
						vertex_count,
						primitive_count,
						state.config.max_mesh_vertex_count(),
						state.config.max_mesh_primitive_count(),
						state.config.thread_idx() == 0,
					)?;
				}
				Instruction::SetMeshVertexPosition { index, position } => {
					let index = expect_u32(read_register(registers, *index)?)? as usize;
					let position = read_register(registers, *position)?;
					let Value::Vec4F(position) = position else {
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
				Instruction::SetMeshTriangle { index, triangle } => {
					let index = expect_u32(read_register(registers, *index)?)? as usize;
					let triangle = read_register(registers, *triangle)?;
					let Value::Vec3U(triangle) = triangle else {
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
				Instruction::SetMeshPrimitiveRenderTargetArrayIndex { index, array_index } => {
					let index = expect_u32(read_register(registers, *index)?)? as usize;
					let array_index = expect_u32(read_register(registers, *array_index)?)?;
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
				Instruction::LoadLocal { register, local } => {
					let value = locals
						.get(*local)
						.and_then(Option::clone)
						.ok_or(VmError::UninitializedLocal { local: *local })?;
					registers[*register] = Some(value);
				}
				Instruction::StoreLocal { local, register } => {
					let value = read_register(registers, *register)?;
					locals[*local] = Some(value.clone());
				}
				Instruction::LoadBuffer {
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
				Instruction::LoadBufferIndexed {
					register,
					slot,
					offset,
					stride,
					count,
					index,
					value_type,
				} => {
					let index = read_buffer_array_index(registers, *index, *count)?;
					let value = if *slot == PUSH_CONSTANT_SLOT {
						descriptors
							.push_constant_mut()?
							.read_value(*offset + *stride * index, value_type)?
					} else {
						descriptors
							.buffer_mut(*slot)?
							.read_value(*offset + *stride * index, value_type)?
					};
					registers[*register] = Some(value);
				}
				Instruction::FetchTexture { register, slot, coord } => {
					let coord = read_register(registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch(coord)?);
				}
				Instruction::FetchTextureU32 { register, slot, coord } => {
					let coord = read_register(registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::SampleTexture { register, slot, uv } => {
					let uv = read_register(registers, *uv)?;
					let Value::Vec2F(uv) = uv else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2F.name().to_string(),
							found: uv.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample(uv)?);
				}
				Instruction::SampleTexture3D { register, slot, uvw } => {
					let uvw = read_register(registers, *uvw)?;
					let Value::Vec3F(uvw) = uvw else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec3F.name().to_string(),
							found: uvw.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample_3d(uvw)?);
				}
				Instruction::TextureSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, registers)?;
					let texture = descriptors.texture_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([texture.width, texture.height]));
				}
				Instruction::ImageSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, registers)?;
					let image = descriptors.image_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([image.width, image.height]));
				}
				Instruction::LoadImage { register, slot, coord } => {
					let coord = expect_vec2u(read_register(registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch(coord)?);
				}
				Instruction::LoadImageU32 { register, slot, coord } => {
					let coord = expect_vec2u(read_register(registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::GuardImageBounds { slot, coord } => {
					let coord = expect_vec2u(read_register(registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, registers)?;
					if !descriptors.image_mut(slot)?.contains_2d(coord) {
						return Ok(FrameOutcome::Complete(None));
					}
				}
				Instruction::ImageAtomicOr {
					register,
					slot,
					coord,
					value,
				} => {
					let coord = expect_vec2u(read_register(registers, *coord)?)?;
					let value = expect_u32(read_register(registers, *value)?)?;
					let slot = resolve_resource_slot(*slot, registers)?;
					let previous = descriptors.image_mut(slot)?.atomic_or(coord, value)?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::WriteImage { slot, coord, value } => {
					let coord = read_register(registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let value = read_register(registers, *value)?;
					let Value::Vec4F(value) = value else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec4F.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, registers)?;
					descriptors.image_mut(slot)?.write(coord, value)?;
				}
				Instruction::StoreBuffer {
					slot,
					offset,
					value_type,
					register,
				} => {
					let value = read_register(registers, *register)?;
					descriptors.buffer_mut(*slot)?.write_value(*offset, value_type, &value)?;
				}
				Instruction::StoreBufferIndexed {
					slot,
					offset,
					stride,
					count,
					index,
					value_type,
					register,
				} => {
					let index = read_buffer_array_index(registers, *index, *count)?;
					let value = read_register(registers, *register)?;
					descriptors
						.buffer_mut(*slot)?
						.write_value(*offset + *stride * index, value_type, &value)?;
				}
				Instruction::AtomicAddBuffer {
					register,
					slot,
					offset,
					stride,
					count,
					index,
					value,
				} => {
					let index = match index {
						Some(index) => read_buffer_array_index(registers, *index, *count)?,
						None => 0,
					};
					let value = expect_u32(read_register(registers, *value)?)?;
					let buffer = descriptors.buffer_mut(*slot)?;
					let address = *offset + *stride * index;
					let previous = expect_u32(buffer.read_value(address, &ValueType::U32)?)?;
					buffer.write_value(address, &ValueType::U32, &Value::U32(previous.wrapping_add(value)))?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::AtomicCompareExchangeBuffer {
					register,
					slot,
					offset,
					stride,
					count,
					index,
					expected,
					desired,
				} => {
					let index = match index {
						Some(index) => read_buffer_array_index(registers, *index, *count)?,
						None => 0,
					};
					let expected = expect_u32(read_register(registers, *expected)?)?;
					let desired = expect_u32(read_register(registers, *desired)?)?;
					let buffer = descriptors.buffer_mut(*slot)?;
					let address = *offset + *stride * index;
					let previous = expect_u32(buffer.read_value(address, &ValueType::U32)?)?;
					if previous == expected {
						buffer.write_value(address, &ValueType::U32, &Value::U32(desired))?;
					}
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::Call {
					register,
					function,
					arguments,
				} => {
					let arguments = arguments
						.iter()
						.map(|argument| read_register(registers, *argument))
						.collect::<Result<Vec<_>, _>>()?;
					// Scheduled task lanes cannot preserve a nested call stack across a rendezvous; ordinary invocations may ignore it.
					let nested_collective_behavior = match collective_behavior {
						CollectiveBehavior::Ignore => CollectiveBehavior::Ignore,
						CollectiveBehavior::Suspend | CollectiveBehavior::Reject => CollectiveBehavior::Reject,
					};
					let value = self.execute_function(*function, &arguments, descriptors, state, nested_collective_behavior)?;
					if let Some(register) = register {
						registers[*register] = value;
					}
				}
				Instruction::Return { register } => {
					return match register {
						Some(register) => Ok(FrameOutcome::Complete(Some(read_register(registers, *register)?))),
						None => Ok(FrameOutcome::Complete(None)),
					};
				}
			}

			frame.instruction_index += 1;
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
}
