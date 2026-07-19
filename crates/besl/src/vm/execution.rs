//! Bounded execution of compiled VM instructions against bound resources.

use super::*;

impl ExecutableProgram {
	/// Executes the compiled `main` function using the currently bound descriptor resources.
	pub fn run_main(&self, descriptors: &mut DescriptorBindings<'_>) -> Result<(), VmError> {
		self.run_main_with_config(descriptors, &ExecutionConfig::default())
	}

	/// Executes `main` with explicit execution limits and shader invocation coordinates.
	pub fn run_main_with_config(
		&self,
		descriptors: &mut DescriptorBindings<'_>,
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		let mut state = ExecutionState::new(config);
		let return_value = self.execute_function(self.main_function, &[], descriptors, &mut state)?;
		if return_value.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions must not return a value".to_string(),
			});
		}
		Ok(())
	}

	fn execute_function(
		&self,
		function_index: usize,
		arguments: &[Value],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
	) -> Result<Option<Value>, VmError> {
		state.enter_call()?;
		let result = self.execute_function_inner(function_index, arguments, descriptors, state);
		state.leave_call();
		result
	}

	/// Runs one function body while the caller maintains the shared execution limits.
	fn execute_function_inner(
		&self,
		function_index: usize,
		arguments: &[Value],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
	) -> Result<Option<Value>, VmError> {
		let function = self
			.functions
			.get(function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", function_index),
			})?;

		let mut registers = vec![None; function.register_count];
		let mut locals = vec![None; function.local_types.len()];
		if arguments.len() != function.parameter_count {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: arguments.len(),
			});
		}
		for (index, argument) in arguments.iter().enumerate() {
			locals[index] = Some(argument.clone());
		}

		let mut instruction_index = 0usize;
		while instruction_index < function.instructions.len() {
			state.consume_instruction()?;
			let instruction = &function.instructions[instruction_index];
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
						.map(|component| read_register(&registers, *component))
						.collect::<Result<Vec<_>, _>>()?;
					registers[*register] = Some(construct_value(value_type, &values)?);
				}
				Instruction::Extract {
					register,
					source,
					index,
					value_type,
				} => {
					let source = read_register(&registers, *source)?;
					registers[*register] = Some(extract_value(&source, *index, value_type)?);
				}
				Instruction::ExtractDynamic {
					register,
					source,
					index,
					count,
					value_type,
				} => {
					let source = read_register(&registers, *source)?;
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
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
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_arithmetic(*operator, &left, &right)?);
				}
				Instruction::Compare {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_comparison(*operator, &left, &right)?);
				}
				Instruction::JumpIfZero { register, target } => {
					let value = read_register(&registers, *register)?;
					if is_zero_value(&value)? {
						instruction_index = *target;
						continue;
					}
				}
				Instruction::Jump { target } => {
					instruction_index = *target;
					continue;
				}
				Instruction::DotProduct { register, left, right } => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_dot_product(&left, &right)?);
				}
				Instruction::CrossProduct { register, left, right } => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_cross_product(&left, &right)?);
				}
				Instruction::Length { register, value } => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_length(&value)?);
				}
				Instruction::Normalize { register, value } => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_normalize(&value)?);
				}
				Instruction::Reflect {
					register,
					incident,
					normal,
				} => {
					let incident = read_register(&registers, *incident)?;
					let normal = read_register(&registers, *normal)?;
					registers[*register] = Some(apply_reflect(&incident, &normal)?);
				}
				Instruction::UnaryScalar {
					register,
					operator,
					value,
				} => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_scalar_unary(*operator, &value)?);
				}
				Instruction::BinaryScalar {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_scalar_binary(*operator, &left, &right)?);
				}
				Instruction::TernaryScalar {
					register,
					operator,
					first,
					second,
					third,
				} => {
					let first = read_register(&registers, *first)?;
					let second = read_register(&registers, *second)?;
					let third = read_register(&registers, *third)?;
					registers[*register] = Some(apply_scalar_ternary(*operator, &first, &second, &third)?);
				}
				Instruction::ThreadIdx { register } => {
					registers[*register] = Some(Value::U32(state.config.thread_idx()));
				}
				Instruction::ThreadId { register } => {
					registers[*register] = Some(Value::Vec2U(state.config.thread_id()));
				}
				Instruction::ThreadgroupPosition { register } => {
					registers[*register] = Some(Value::U32(state.config.threadgroup_position()));
				}
				Instruction::LoadTaskPayload {
					register,
					name,
					index,
					count,
					value_type,
				} => {
					let index = read_buffer_array_index(&registers, *index, *count)?;
					let value = descriptors.task_payload_value(name, index)?;
					if !value.matches_type(value_type) {
						return Err(VmError::TypeMismatch {
							expected: value_type.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					}
					registers[*register] = Some(value);
				}
				Instruction::SetMeshOutputCounts {
					vertex_count,
					primitive_count,
				} => {
					let vertex_count = expect_u32(read_register(&registers, *vertex_count)?)?;
					let primitive_count = expect_u32(read_register(&registers, *primitive_count)?)?;
					descriptors.mesh_outputs_mut()?.set_counts(
						vertex_count,
						primitive_count,
						state.config.max_mesh_vertex_count(),
						state.config.max_mesh_primitive_count(),
						state.config.thread_idx() == 0,
					)?;
				}
				Instruction::SetMeshVertexPosition { index, position } => {
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
					let position = read_register(&registers, *position)?;
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
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
					let triangle = read_register(&registers, *triangle)?;
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
				Instruction::LoadLocal { register, local } => {
					let value = locals
						.get(*local)
						.and_then(Option::clone)
						.ok_or(VmError::UninitializedLocal { local: *local })?;
					registers[*register] = Some(value);
				}
				Instruction::StoreLocal { local, register } => {
					let value = read_register(&registers, *register)?;
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
					let index = read_buffer_array_index(&registers, *index, *count)?;
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
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch(coord)?);
				}
				Instruction::FetchTextureU32 { register, slot, coord } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::SampleTexture { register, slot, uv } => {
					let uv = read_register(&registers, *uv)?;
					let Value::Vec2F(uv) = uv else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2F.name().to_string(),
							found: uv.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample(uv)?);
				}
				Instruction::SampleTexture3D { register, slot, uvw } => {
					let uvw = read_register(&registers, *uvw)?;
					let Value::Vec3F(uvw) = uvw else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec3F.name().to_string(),
							found: uvw.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample_3d(uvw)?);
				}
				Instruction::TextureSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, &registers)?;
					let texture = descriptors.texture_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([texture.width, texture.height]));
				}
				Instruction::ImageSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, &registers)?;
					let image = descriptors.image_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([image.width, image.height]));
				}
				Instruction::LoadImage { register, slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch(coord)?);
				}
				Instruction::LoadImageU32 { register, slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::GuardImageBounds { slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					if !descriptors.image_mut(slot)?.contains_2d(coord) {
						return Ok(None);
					}
				}
				Instruction::ImageAtomicOr {
					register,
					slot,
					coord,
					value,
				} => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let value = expect_u32(read_register(&registers, *value)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					let previous = descriptors.image_mut(slot)?.atomic_or(coord, value)?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::WriteImage { slot, coord, value } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let value = read_register(&registers, *value)?;
					let Value::Vec4F(value) = value else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec4F.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					descriptors.image_mut(slot)?.write(coord, value)?;
				}
				Instruction::StoreBuffer {
					slot,
					offset,
					value_type,
					register,
				} => {
					let value = read_register(&registers, *register)?;
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
					let index = read_buffer_array_index(&registers, *index, *count)?;
					let value = read_register(&registers, *register)?;
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
						Some(index) => read_buffer_array_index(&registers, *index, *count)?,
						None => 0,
					};
					let value = expect_u32(read_register(&registers, *value)?)?;
					let buffer = descriptors.buffer_mut(*slot)?;
					let address = *offset + *stride * index;
					let previous = expect_u32(buffer.read_value(address, &ValueType::U32)?)?;
					buffer.write_value(address, &ValueType::U32, &Value::U32(previous.wrapping_add(value)))?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::Call {
					register,
					function,
					arguments,
				} => {
					let arguments = arguments
						.iter()
						.map(|argument| read_register(&registers, *argument))
						.collect::<Result<Vec<_>, _>>()?;
					let value = self.execute_function(*function, &arguments, descriptors, state)?;
					if let Some(register) = register {
						registers[*register] = value;
					}
				}
				Instruction::Return { register } => {
					return match register {
						Some(register) => Ok(Some(read_register(&registers, *register)?)),
						None => Ok(None),
					};
				}
			}

			instruction_index += 1;
		}

		match &function.return_type {
			Some(return_type) => Err(VmError::UnsupportedStatement {
				message: format!(
					"Function with return type `{}` ended without returning a value",
					return_type.name()
				),
			}),
			None => Ok(None),
		}
	}
}
