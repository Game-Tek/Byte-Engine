use super::*;
use crate::sampler::Reduction;

impl<'a> Compiler<'a> {
	/// Returns the name of the intrinsic a call selected, after checking that the call passes one argument per declared
	/// parameter, so every lowering arm can index its arguments.
	fn intrinsic_name(&self, intrinsic: &NodeId, arguments: &[NodeId]) -> Result<&'a str, VmError> {
		let program = self.program;
		let Nodes::Intrinsic { name, elements, .. } = program[intrinsic].node() else {
			return Err(VmError::UnsupportedExpression {
				message: format!("Expected an intrinsic, but found {}", describe_node(program[intrinsic].node())),
			});
		};
		let parameters = elements
			.iter()
			.filter(|element| matches!(program[*element].node(), Nodes::Parameter { .. }))
			.count();
		require_argument_count(arguments, parameters)?;
		Ok(program.text(*name))
	}

	// Intrinsic names form one exhaustive VM dispatch table whose arms document the portable instruction semantics.
	#[allow(clippy::too_many_lines)]
	pub(super) fn compile_intrinsic_call_expression(
		&mut self,
		intrinsic: &NodeId,
		arguments: &[NodeId],
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		let name = self.intrinsic_name(intrinsic, arguments)?;
		let return_type = resolve_callable_return_type(self.program, intrinsic)?;

		if name != "normalize" && name != "reflect" && &return_type != expected_type {
			return Err(VmError::mismatch(expected_type.name(), return_type.name()));
		}

		if let Some(builtin) = InvocationBuiltin::from_name(name) {
			return Ok(self.emit_to(|register| LocalInstruction::LoadBuiltin { register, builtin }));
		}

		match name {
			"sample" => {
				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F)?;
				let (slot, layer) = match self.resolve_array_texture_layer_access(&arguments[0], RequiredAccess::Read)? {
					Some((slot, layer)) => (slot, Some(self.compile_value_expression(&layer, &ValueType::U32)?)),
					None => (self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?, None),
				};
				let sampler = fixed_sampler(self.program, &arguments[0], name)?;
				Ok(self.emit_to(|register| TextureInstruction::SampleTexture {
					register,
					slot,
					uv,
					layer,
					lod: None,
					sampler,
				}))
			}
			"texture_cube_array_lod" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				let uvw = self.compile_value_expression(&arguments[1], &ValueType::Vec3F)?;
				self.compile_value_expression(&arguments[2], &ValueType::U32)?;
				self.compile_value_expression(&arguments[3], &ValueType::F32)?;
				// The VM has no cube-array storage model. Sampling the supplied direction preserves the shader seam's typed execution.
				Ok(self.emit_to(|register| TextureInstruction::SampleTexture3D { register, slot, uvw }))
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				let sampler = fixed_sampler(self.program, &arguments[0], name)?;
				let sample = if let [_, uv, layer, lod] = arguments {
					// Only the array form of `downsample_max` takes a layer.
					let uv = self.compile_value_expression(uv, &ValueType::Vec2F)?;
					let layer = Some(self.compile_value_expression(layer, &ValueType::U32)?);
					let lod = Some(self.compile_value_expression(lod, &ValueType::F32)?);
					self.emit_to(|register| TextureInstruction::SampleTexture {
						register,
						slot,
						uv,
						layer,
						lod,
						sampler,
					})
				} else {
					let coord_type = self.infer_expression_type(&arguments[1], &ValueType::Vec2F)?;
					let coord = self.compile_value_expression(&arguments[1], &coord_type)?;
					let lod = arguments
						.get(2)
						.map(|lod| self.compile_value_expression(lod, &ValueType::F32))
						.transpose()?;
					match coord_type {
						ValueType::Vec2F => self.emit_to(|register| TextureInstruction::SampleTexture {
							register,
							slot,
							uv: coord,
							layer: None,
							lod,
							sampler,
						}),
						ValueType::Vec3F => {
							self.emit_to(|register| TextureInstruction::SampleTexture3D { register, slot, uvw: coord })
						}
						other => return Err(VmError::mismatch("vec2f or vec3f", other.name())),
					}
				};
				if name == "texture_lod" {
					return Ok(sample);
				}
				// Conservative downsampling is scalar because both production pyramids reduce depth.
				// The fixed sampler keeps VM behavior independent of fixture sampler state.
				Ok(self.emit_to(|register| ValueInstruction::Extract {
					register,
					source: sample,
					index: 0,
					value_type: ValueType::F32,
				}))
			}
			"fetch" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				Ok(match arguments.get(2) {
					Some(layer) => {
						let layer = self.compile_value_expression(layer, &ValueType::U32)?;
						self.emit_to(|register| TextureInstruction::FetchTextureArray {
							register,
							slot,
							coord,
							layer,
						})
					}
					None => self.emit_to(|register| TextureInstruction::FetchTexture { register, slot, coord }),
				})
			}
			"gather" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F)?;
				let layer = arguments
					.get(2)
					.map(|layer| self.compile_value_expression(layer, &ValueType::U32))
					.transpose()?;
				Ok(self.emit_to(|register| TextureInstruction::GatherTexture {
					register,
					slot,
					uv,
					layer,
				}))
			}
			"fetch_u32" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				Ok(self.emit_to(|register| TextureInstruction::FetchTextureU32 { register, slot, coord }))
			}
			"image_load" | "image_load_u32" => {
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Read)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				Ok(self.emit_to(|register| match name {
					"image_load" => ImageInstruction::LoadImage { register, slot, coord },
					_ => ImageInstruction::LoadImageU32 { register, slot, coord },
				}))
			}
			"image_atomic_or" | "image_atomic_max" => {
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::ReadWrite)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				let value = self.compile_value_expression(&arguments[2], &ValueType::U32)?;
				let operation = if name == "image_atomic_or" {
					AtomicOperation::Or
				} else {
					AtomicOperation::Max
				};
				Ok(self.emit_to(|register| ImageInstruction::ImageAtomic {
					operation,
					register,
					slot,
					coord,
					value,
				}))
			}
			"atomic_load" | "atomic_exchange" | "atomic_add" | "atomic_sub" | "atomic_min" | "atomic_max" | "atomic_and"
			| "atomic_or" | "atomic_xor" | "atomic_compare_exchange" => self.compile_atomic(name, arguments, return_type),
			"texture_size" => {
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read)?;
				Ok(self.emit_to(|register| TextureInstruction::TextureSize { register, slot }))
			}
			"image_size" => {
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Any)?;
				Ok(self.emit_to(|register| ImageInstruction::ImageSize { register, slot }))
			}
			"dot" | "distance" => {
				let operand_type = self.float_vector_operand_type(arguments, name)?;
				let left = self.compile_value_expression(&arguments[0], &operand_type)?;
				let right = self.compile_value_expression(&arguments[1], &operand_type)?;
				if name == "dot" {
					return Ok(self.emit_to(|register| NumericInstruction::DotProduct { register, left, right }));
				}
				// `distance` is the length of the difference.
				let value = self.emit_to(|register| NumericInstruction::Arithmetic {
					register,
					operator: ArithmeticOperator::Subtract,
					left,
					right,
				});
				Ok(self.emit_to(|register| NumericInstruction::Length { register, value }))
			}
			"cross" => {
				let left = self.compile_value_expression(&arguments[0], &ValueType::Vec3F)?;
				let right = self.compile_value_expression(&arguments[1], &ValueType::Vec3F)?;
				Ok(self.emit_to(|register| NumericInstruction::CrossProduct { register, left, right }))
			}
			"length" => {
				let operand_type = self.float_vector_operand_type(arguments, name)?;
				let value = self.compile_value_expression(&arguments[0], &operand_type)?;
				Ok(self.emit_to(|register| NumericInstruction::Length { register, value }))
			}
			"normalize" | "reflect" => {
				// These return their operand type, which the arguments decide instead of the selected overload.
				let operand_type = self.float_vector_operand_type(arguments, name)?;
				if &operand_type != expected_type {
					return Err(VmError::mismatch(expected_type.name(), operand_type.name()));
				}
				let value = self.compile_value_expression(&arguments[0], &operand_type)?;
				if name == "normalize" {
					return Ok(self.emit_to(|register| NumericInstruction::Normalize { register, value }));
				}
				let normal = self.compile_value_expression(&arguments[1], &operand_type)?;
				Ok(self.emit_to(|register| NumericInstruction::Reflect {
					register,
					incident: value,
					normal,
				}))
			}
			"is_nan" | "is_infinite" | "is_finite" | "is_normal" => {
				let source_type = resolve_intrinsic_parameter_type(self.program, intrinsic, 0)?;
				if !matches!(source_type, ValueType::F16 | ValueType::F32) {
					return Err(VmError::mismatch("f16 or f32", source_type.name()));
				}
				let value = self.compile_value_expression(&arguments[0], &source_type)?;
				let predicate = match name {
					"is_nan" => FloatPredicate::Nan,
					"is_infinite" => FloatPredicate::Infinite,
					"is_finite" => FloatPredicate::Finite,
					_ => FloatPredicate::Normal,
				};
				Ok(self.emit_to(|register| NumericInstruction::FloatPredicate {
					register,
					predicate,
					value,
				}))
			}
			"abs" | "sign" | "sqrt" | "exp" | "exp2" | "sin" | "cos" | "tan" | "asin" | "floor" | "ceil" | "round"
			| "fract" | "radians" | "inversesqrt" | "log2" | "fwidth" | "find_lsb" => {
				let value = self.compile_value_expression(&arguments[0], &return_type)?;
				let operator = match name {
					"abs" => ScalarUnaryOperator::Abs,
					"sign" => ScalarUnaryOperator::Sign,
					"sqrt" => ScalarUnaryOperator::Sqrt,
					"exp" => ScalarUnaryOperator::Exp,
					"exp2" => ScalarUnaryOperator::Exp2,
					"sin" => ScalarUnaryOperator::Sin,
					"cos" => ScalarUnaryOperator::Cos,
					"tan" => ScalarUnaryOperator::Tan,
					"asin" => ScalarUnaryOperator::Asin,
					"floor" => ScalarUnaryOperator::Floor,
					"ceil" => ScalarUnaryOperator::Ceil,
					"round" => ScalarUnaryOperator::Round,
					"fract" => ScalarUnaryOperator::Fract,
					"radians" => ScalarUnaryOperator::Radians,
					"inversesqrt" => ScalarUnaryOperator::InverseSqrt,
					"log2" => ScalarUnaryOperator::Log2,
					"fwidth" => ScalarUnaryOperator::Fwidth,
					_ => ScalarUnaryOperator::FindLsb,
				};
				Ok(self.emit_to(|register| NumericInstruction::UnaryScalar {
					register,
					operator,
					value,
				}))
			}
			"sincos" => {
				let value = self.compile_value_expression(&arguments[0], &ValueType::F32)?;
				let components = [ScalarUnaryOperator::Sin, ScalarUnaryOperator::Cos]
					.map(|operator| {
						self.emit_to(|register| NumericInstruction::UnaryScalar {
							register,
							operator,
							value,
						})
					})
					.to_vec();
				Ok(self.emit_to(|register| ValueInstruction::Construct {
					register,
					value_type: ValueType::Vec2F,
					components,
				}))
			}
			"round_to_i32" => {
				let value = self.compile_value_expression(&arguments[0], &ValueType::Vec2F)?;
				Ok(self.emit_to(|register| NumericInstruction::RoundToVec2I { register, value }))
			}
			"f32" | "f16" | "u32" | "u16" | "u8" => {
				// Each target keeps the type untyped literals fall back to and the source list its mismatch reports.
				let (fallback_type, accepted_sources) = match name {
					"f32" => (ValueType::U32, "f16, u32, or i32"),
					"f16" => (ValueType::F32, "f32, u32, or i32"),
					"u32" => (ValueType::F32, "u8, u16, i32, f16, or f32"),
					// `u16` and `u8` keep the low bits of a `u32`.
					_ => (ValueType::U32, "u32"),
				};
				let source_type = self.infer_expression_type(&arguments[0], &fallback_type)?;
				// Integer conversions to their own type are the identity and emit no instruction.
				if source_type == return_type && matches!(return_type, ValueType::U32 | ValueType::U16 | ValueType::U8) {
					return self.compile_value_expression(&arguments[0], &return_type);
				}
				let operator = conversion_operator(&source_type, &return_type)
					.ok_or_else(|| VmError::mismatch(accepted_sources, source_type.name()))?;
				let value = self.compile_value_expression(&arguments[0], &source_type)?;
				Ok(self.emit_to(|register| NumericInstruction::UnaryScalar {
					register,
					operator,
					value,
				}))
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" => {
				// The selected overload carries the source type. This also distinguishes
				// vec4f conversions from f16 and packed storage vectors.
				let source_type = resolve_intrinsic_parameter_type(self.program, intrinsic, 0)?;
				let value = self.compile_value_expression(&arguments[0], &source_type)?;
				// Constructors perform the precision conversion without adding a dedicated VM instruction.
				Ok(self.emit_to(|register| ValueInstruction::Construct {
					register,
					value_type: return_type,
					components: vec![value],
				}))
			}
			"min" | "max" | "pow" | "step" | "atan2" => {
				let left = self.compile_value_expression(&arguments[0], &return_type)?;
				let right = self.compile_value_expression(&arguments[1], &return_type)?;
				let operator = match name {
					"min" => ScalarBinaryOperator::Min,
					"max" => ScalarBinaryOperator::Max,
					"pow" => ScalarBinaryOperator::Pow,
					"step" => ScalarBinaryOperator::Step,
					_ => ScalarBinaryOperator::Atan2,
				};
				Ok(self.emit_to(|register| NumericInstruction::BinaryScalar {
					register,
					operator,
					left,
					right,
				}))
			}
			"smoothstep" | "mix" | "clamp" | "fma" => {
				// Every ternary intrinsic takes operands of its result type; `mix` broadcasts its scalar factor below.
				let first = self.compile_value_expression(&arguments[0], &return_type)?;
				let second = self.compile_value_expression(&arguments[1], &return_type)?;
				// `mix` on a vector takes one scalar factor. The ternary instruction works component-wise on
				// same-typed operands, so the factor is broadcast into a vector before the blend.
				let lane_count = match return_type {
					ValueType::Vec2F => 2,
					ValueType::Vec3F => 3,
					ValueType::Vec4F => 4,
					_ => 0,
				};
				let third = if name == "mix" && lane_count > 0 {
					let factor = self.compile_value_expression(&arguments[2], &ValueType::F32)?;
					self.emit_to(|register| ValueInstruction::Construct {
						register,
						value_type: return_type.clone(),
						components: vec![factor; lane_count],
					})
				} else {
					self.compile_value_expression(&arguments[2], &return_type)?
				};
				let operator = match name {
					"smoothstep" => ScalarTernaryOperator::Smoothstep,
					"mix" => ScalarTernaryOperator::Mix,
					"clamp" => ScalarTernaryOperator::Clamp,
					_ => ScalarTernaryOperator::Fma,
				};
				Ok(self.emit_to(|register| NumericInstruction::TernaryScalar {
					register,
					operator,
					first,
					second,
					third,
				}))
			}
			"intersect" | "trace_occlusion" => {
				let slot = self.resolve_acceleration_structure_slot(&arguments[0])?;
				let ray = RayOperands {
					mask: self.compile_value_expression(&arguments[1], &ValueType::U32)?,
					origin: self.compile_value_expression(&arguments[2], &ValueType::Vec3F)?,
					direction: self.compile_value_expression(&arguments[3], &ValueType::Vec3F)?,
					t_min: self.compile_value_expression(&arguments[4], &ValueType::F32)?,
					t_max: self.compile_value_expression(&arguments[5], &ValueType::F32)?,
				};
				Ok(self.emit_to(|register| match name {
					"intersect" => RayTracingInstruction::Intersect {
						register,
						slot,
						ray,
						hit_type: return_type,
					},
					_ => RayTracingInstruction::TraceOcclusion { register, slot, ray },
				}))
			}
			"invoke" => {
				let hit_type = resolve_intrinsic_parameter_type(self.program, intrinsic, 0)?;
				let hit = self.compile_value_expression(&arguments[0], &hit_type)?;
				let payload = self.compile_value_expression(&arguments[1], &return_type)?;
				Ok(self.emit_to(|register| RayTracingInstruction::Invoke {
					register,
					hit,
					payload,
					payload_type: return_type,
				}))
			}
			"subgroup_ballot" => {
				let predicate = self.compile_value_expression(&arguments[0], &ValueType::Bool)?;
				Ok(self.emit_to(|register| Instruction::SubgroupBallot { register, predicate }))
			}
			"subgroup_ballot_any" | "subgroup_ballot_find_lsb" | "subgroup_ballot_count" => {
				let operator = match name {
					"subgroup_ballot_any" => SubgroupMaskOperator::Any,
					"subgroup_ballot_find_lsb" => SubgroupMaskOperator::FindLsb,
					_ => SubgroupMaskOperator::Count,
				};
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U)?;
				Ok(self.emit_to(|register| LocalInstruction::SubgroupMask {
					register,
					operator,
					mask,
				}))
			}
			"subgroup_ballot_and_not" => {
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U)?;
				let removed = self.compile_value_expression(&arguments[1], &ValueType::Vec4U)?;
				Ok(self.emit_to(|register| LocalInstruction::SubgroupBallotAndNot { register, mask, removed }))
			}
			"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => {
				let value = self.compile_value_expression(&arguments[0], &return_type)?;
				let source_lane = self.compile_value_expression(&arguments[1], &ValueType::U32)?;
				Ok(self.emit_to(|register| Instruction::SubgroupBroadcast {
					register,
					value,
					source_lane,
					value_type: return_type,
				}))
			}
			"subgroup_shuffle_xor_f32" => {
				let value = self.compile_value_expression(&arguments[0], &ValueType::F32)?;
				let mask = self.compile_value_expression(&arguments[1], &ValueType::U32)?;
				Ok(self.emit_to(|register| Instruction::SubgroupShuffleXor { register, value, mask }))
			}
			_ => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported intrinsic `{}`", name),
			}),
		}
	}

	/// Lowers one value-returning buffer or workgroup atomic, whose first argument names the target and whose result,
	/// the value the target held before, is a `u32` or an `i32`.
	fn compile_atomic(&mut self, name: &str, arguments: &[NodeId], return_type: ValueType) -> Result<usize, VmError> {
		if !matches!(return_type, ValueType::U32 | ValueType::I32) {
			return Err(VmError::mismatch("u32 or i32", return_type.name()));
		}
		let operation = match name {
			"atomic_exchange" => Some(AtomicOperation::Exchange),
			"atomic_add" => Some(AtomicOperation::Add),
			"atomic_sub" => Some(AtomicOperation::Subtract),
			"atomic_min" => Some(AtomicOperation::Min),
			"atomic_max" => Some(AtomicOperation::Max),
			"atomic_and" => Some(AtomicOperation::And),
			"atomic_or" => Some(AtomicOperation::Or),
			"atomic_xor" => Some(AtomicOperation::Xor),
			// `atomic_load` and `atomic_compare_exchange`.
			_ => None,
		};

		if let Some(target) = resolve_workgroup_access(self.program, &arguments[0])? {
			if name == "atomic_load" {
				return self.compile_workgroup_load(target, &return_type);
			}
			if target.value_type != return_type {
				return Err(VmError::mismatch(return_type.name(), target.value_type.name()));
			}
			let index = self.compile_index(target.index_expression.as_ref())?;
			let value = self.compile_value_expression(&arguments[1], &return_type)?;
			let (name, count, value_type) = (target.name, target.count, target.value_type);
			if let Some(operation) = operation {
				return Ok(self.emit_to(|register| WorkgroupInstruction::AtomicWorkgroup {
					register,
					operation,
					name,
					index,
					count,
					value_type,
					value,
				}));
			}
			let desired = self.compile_value_expression(&arguments[2], &return_type)?;
			return Ok(self.emit_to(|register| WorkgroupInstruction::AtomicCompareExchangeWorkgroup {
				register,
				name,
				index,
				count,
				value_type,
				expected: value,
				desired,
			}));
		}

		let target = self.resolve_memory_access(&arguments[0], RequiredAccess::ReadWrite)?;
		if target.value_type != return_type {
			return Err(VmError::mismatch(return_type.name(), target.value_type.name()));
		}
		if name == "atomic_load" {
			return self.compile_resolved_buffer_load(target);
		}
		let target = self.lower_buffer_access(target)?;
		let value = self.compile_value_expression(&arguments[1], &return_type)?;
		let LoweredBufferAccess {
			slot,
			offset,
			stride,
			count,
			index,
			value_type,
		} = target;
		if let Some(operation) = operation {
			return Ok(self.emit_to(|register| BufferInstruction::AtomicBuffer {
				register,
				operation,
				slot,
				offset,
				stride,
				count,
				index,
				value_type,
				value,
			}));
		}
		let desired = self.compile_value_expression(&arguments[2], &return_type)?;
		Ok(self.emit_to(|register| BufferInstruction::AtomicCompareExchangeBuffer {
			register,
			slot,
			offset,
			stride,
			count,
			index,
			value_type,
			expected: value,
			desired,
		}))
	}

	/// Returns the float vector type that every argument of a vector intrinsic has.
	///
	/// The arguments decide the type instead of the selected overload, because a graph built in Rust may bind any of
	/// an intrinsic's overloads to a call.
	fn float_vector_operand_type(&mut self, arguments: &[NodeId], intrinsic: &str) -> Result<ValueType, VmError> {
		[
			ValueType::Vec2F,
			ValueType::Vec3F,
			ValueType::Vec4F,
			ValueType::Vec2F16,
			ValueType::Vec3F16,
			ValueType::Vec4F16,
		]
		.into_iter()
		.find(|candidate| {
			arguments
				.iter()
				.all(|argument| self.infer_expression_type(argument, candidate).ok().as_ref() == Some(candidate))
		})
		.ok_or_else(|| VmError::UnsupportedExpression {
			message: format!(
				"`{intrinsic}` received arguments that aren't float vectors of one size. Pass vectors of the same type, such as two `vec3f` values."
			),
		})
	}

	/// Lowers the intrinsics that return nothing, and any other intrinsic whose result a statement discards.
	pub(super) fn compile_intrinsic_call_statement(&mut self, intrinsic: &NodeId, arguments: &[NodeId]) -> Result<(), VmError> {
		let name = self.intrinsic_name(intrinsic, arguments)?;
		let u32_argument = |compiler: &mut Self, index: usize| compiler.compile_value_expression(&arguments[index], &ValueType::U32);

		match name {
			"set_task_mesh_output_count" => {
				let count = u32_argument(self, 0)?;
				self.emit(WorkgroupInstruction::SetTaskMeshOutputCount { count });
			}
			// Preserve the barrier as an instruction so workgroup execution can rendezvous every lane.
			"workgroup_barrier" => self.emit(Instruction::WorkgroupBarrier),
			"set_mesh_output_counts" => {
				let vertex_count = u32_argument(self, 0)?;
				let primitive_count = u32_argument(self, 1)?;
				self.emit(MeshOutputInstruction::Counts {
					vertex_count,
					primitive_count,
				});
			}
			"set_mesh_vertex_position" => {
				let index = u32_argument(self, 0)?;
				let position = self.compile_value_expression(&arguments[1], &ValueType::Vec4F)?;
				self.emit(MeshOutputInstruction::VertexPosition { index, position });
			}
			"set_mesh_triangle" => {
				let index = u32_argument(self, 0)?;
				let triangle = self.compile_value_expression(&arguments[1], &ValueType::Vec3U)?;
				self.emit(MeshOutputInstruction::Triangle { index, triangle });
			}
			"set_mesh_primitive_render_target_array_index" => {
				let index = u32_argument(self, 0)?;
				let array_index = u32_argument(self, 1)?;
				self.emit(MeshOutputInstruction::PrimitiveRenderTargetArrayIndex { index, array_index });
			}
			// A tile shader writes one of its outputs, bound as an image at its output slot, where other stages write a
			// storage image.
			"write" => {
				let output_location = crate::referenced_output(self.program, &arguments[0]).and_then(|output| {
					match self.program[output].node() {
						Nodes::Output { location, .. } => Some(*location),
						_ => None,
					}
				});
				let slot = match output_location {
					Some(location) => {
						let slot = output_slot(location);
						self.claim_descriptor_layout(slot, DescriptorLayout::Image, "tile output")?;
						slot
					}
					None => self.resolve_image_slot(&arguments[0], RequiredAccess::Write)?,
				};
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				let value = self.compile_value_expression(&arguments[2], &ValueType::Vec4F)?;
				self.emit(if output_location.is_some() {
					ImageInstruction::WriteTileOutput { slot, coord, value }
				} else {
					ImageInstruction::WriteImage { slot, coord, value }
				});
			}
			"write_u32" => {
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Write)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				let value = u32_argument(self, 2)?;
				self.emit(ImageInstruction::WriteImageU32 { slot, coord, value });
			}
			"guard_image_bounds" => {
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Any)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U)?;
				self.emit(ImageInstruction::GuardImageBounds { slot, coord });
			}
			"atomic_store" => {
				if let Some(target) = resolve_workgroup_access(self.program, &arguments[0])? {
					if !matches!(target.value_type, ValueType::U32 | ValueType::I32) {
						return Err(VmError::mismatch("u32 or i32", target.value_type.name()));
					}
					let index = self.compile_index(target.index_expression.as_ref())?;
					let value = self.compile_value_expression(&arguments[1], &target.value_type)?;
					self.emit(WorkgroupInstruction::StoreWorkgroup {
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						value,
					});
					return Ok(());
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Write)?;
				if !matches!(target.value_type, ValueType::U32 | ValueType::I32) {
					return Err(VmError::mismatch("u32 or i32", target.value_type.name()));
				}
				let value_type = target.value_type.clone();
				let target = self.lower_buffer_access(target)?;
				let register = self.compile_value_expression(&arguments[1], &value_type)?;
				self.emit_buffer_store(target, register);
			}
			// Any other intrinsic, such as an atomic whose previous value is unused, runs for its effect.
			_ => {
				let return_type = resolve_callable_return_type(self.program, intrinsic)?;
				self.compile_intrinsic_call_expression(intrinsic, arguments, &return_type)?;
			}
		}
		Ok(())
	}
}

/// Returns the sampler a texture read fixes: the one the texture's descriptor declares, or else the reduction a
/// downsample intrinsic names. `None` reads through the sampler the application bound.
fn fixed_sampler(program: &Program, texture: &NodeId, intrinsic: &str) -> Result<Option<Sampler>, VmError> {
	if let Some(state) = crate::lexer::binding_sampler(program, texture) {
		return Ok(Some(Sampler::from(state)));
	}
	Ok(match intrinsic {
		"downsample_min" => Some(Sampler::new(Reduction::Min)),
		"downsample_max" => Some(Sampler::new(Reduction::Max)),
		_ => None,
	})
}
