use super::*;

impl<'a> Compiler<'a> {
	// Intrinsic names form one exhaustive VM dispatch table whose arms document the portable instruction semantics.
	#[allow(clippy::too_many_lines)]
	pub(super) fn compile_intrinsic_call_expression(
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

				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let register = self.allocate_register();
				let (slot, layer) = if let Some((slot, layer)) =
					self.resolve_array_texture_layer_access(&arguments[0], RequiredAccess::Read, descriptor_layouts)?
				{
					(
						slot,
						Some(self.compile_value_expression(&layer, &ValueType::U32, descriptor_layouts)?),
					)
				} else {
					(
						self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?,
						None,
					)
				};
				self.instructions.push(Instruction::SampleTexture {
					register,
					slot,
					uv,
					layer,
					lod: None,
					reduction_mode: None,
				});
				Ok(register)
			}
			"texture_cube_array_lod" => {
				require_argument_count(arguments, 4)?;
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let direction = self.compile_value_expression(&arguments[1], &ValueType::Vec3F, descriptor_layouts)?;
				let _cube = self.compile_value_expression(&arguments[2], &ValueType::U32, descriptor_layouts)?;
				let _lod = self.compile_value_expression(&arguments[3], &ValueType::F32, descriptor_layouts)?;
				let register = self.allocate_register();
				// The VM has no cube-array storage model. Sampling the supplied direction preserves the shader seam's typed execution.
				self.instructions.push(Instruction::SampleTexture3D {
					register,
					slot,
					uvw: direction,
				});
				Ok(register)
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				if arguments.len() == 4 && name == "downsample_max" {
					let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
					let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
					let layer = self.compile_value_expression(&arguments[2], &ValueType::U32, descriptor_layouts)?;
					let lod = self.compile_value_expression(&arguments[3], &ValueType::F32, descriptor_layouts)?;
					let sample_register = self.allocate_register();
					self.instructions.push(Instruction::SampleTexture {
						register: sample_register,
						slot,
						uv,
						layer: Some(layer),
						lod: Some(lod),
						reduction_mode: Some(SamplerReductionMode::Max),
					});
					let register = self.allocate_register();
					self.instructions.push(Instruction::Extract {
						register,
						source: sample_register,
						index: 0,
						value_type: ValueType::F32,
					});
					return Ok(register);
				}
				if arguments.len() != 2 && arguments.len() != 3 {
					return Err(VmError::UnsupportedExpression {
						message: format!(
							"{name} requires a texture, UV coordinates, and an optional LOD; array maximum reduction also requires a layer."
						),
					});
				}
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord_type = self.infer_expression_type(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &coord_type, descriptor_layouts)?;
				let lod = arguments
					.get(2)
					.map(|lod| self.compile_value_expression(lod, &ValueType::F32, descriptor_layouts))
					.transpose()?;
				let sample_register = self.allocate_register();
				match coord_type {
					ValueType::Vec2F => self.instructions.push(Instruction::SampleTexture {
						register: sample_register,
						slot,
						uv: coord,
						layer: None,
						lod,
						reduction_mode: match name.as_str() {
							"downsample_min" => Some(SamplerReductionMode::Min),
							"downsample_max" => Some(SamplerReductionMode::Max),
							_ => None,
						},
					}),
					ValueType::Vec3F => self.instructions.push(Instruction::SampleTexture3D {
						register: sample_register,
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
				if name == "texture_lod" {
					Ok(sample_register)
				} else {
					// Conservative downsampling is scalar because both production pyramids reduce depth.
					// The instruction-level override keeps VM behavior independent of fixture sampler state.
					let register = self.allocate_register();
					self.instructions.push(Instruction::Extract {
						register,
						source: sample_register,
						index: 0,
						value_type: ValueType::F32,
					});
					Ok(register)
				}
			}
			"fetch" => {
				if arguments.len() != 2 && arguments.len() != 3 {
					return Err(VmError::UnsupportedExpression {
						message: "fetch requires a texture, texel coordinates, and an optional array layer.".to_string(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				if let Some(layer) = arguments.get(2) {
					let layer = self.compile_value_expression(layer, &ValueType::U32, descriptor_layouts)?;
					self.instructions.push(Instruction::FetchTextureArray {
						register,
						slot,
						coord,
						layer,
					});
				} else {
					self.instructions.push(Instruction::FetchTexture { register, slot, coord });
				}
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
				if !matches!(return_type, ValueType::U32 | ValueType::I32) {
					return Err(VmError::TypeMismatch {
						expected: "u32 or i32".to_string(),
						found: return_type.name().to_string(),
					});
				}
				if let Some(target) = resolve_workgroup_access(&arguments[0])? {
					return self.compile_workgroup_load(target, &return_type, descriptor_layouts);
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::ReadWrite, descriptor_layouts)?;
				if target.value_type != return_type {
					return Err(VmError::TypeMismatch {
						expected: return_type.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				self.compile_resolved_buffer_load(target, descriptor_layouts)
			}
			"atomic_exchange" | "atomic_add" | "atomic_sub" | "atomic_min" | "atomic_max" | "atomic_and" | "atomic_or"
			| "atomic_xor" => {
				require_argument_count(arguments, 2)?;
				if !matches!(return_type, ValueType::U32 | ValueType::I32) {
					return Err(VmError::TypeMismatch {
						expected: "u32 or i32".to_string(),
						found: return_type.name().to_string(),
					});
				}
				let operation = match name.as_str() {
					"atomic_exchange" => AtomicOperation::Exchange,
					"atomic_add" => AtomicOperation::Add,
					"atomic_sub" => AtomicOperation::Subtract,
					"atomic_min" => AtomicOperation::Min,
					"atomic_max" => AtomicOperation::Max,
					"atomic_and" => AtomicOperation::And,
					"atomic_or" => AtomicOperation::Or,
					"atomic_xor" => AtomicOperation::Xor,
					_ => unreachable!("Expected an atomic read-modify-write intrinsic"),
				};
				if let Some(target) = resolve_workgroup_access(&arguments[0])? {
					if target.value_type != return_type {
						return Err(VmError::TypeMismatch {
							expected: return_type.name().to_string(),
							found: target.value_type.name().to_string(),
						});
					}
					let index = target
						.index_expression
						.as_ref()
						.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
						.transpose()?;
					let value = self.compile_value_expression(&arguments[1], &return_type, descriptor_layouts)?;
					let register = self.allocate_register();
					self.instructions.push(Instruction::AtomicWorkgroup {
						register,
						operation,
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						value,
					});
					return Ok(register);
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::ReadWrite, descriptor_layouts)?;
				if target.value_type != return_type {
					return Err(VmError::TypeMismatch {
						expected: return_type.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let value = self.compile_value_expression(&arguments[1], &return_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::AtomicBuffer {
					register,
					operation,
					slot: target.slot,
					offset: target.offset,
					stride: target.stride,
					count: target.count,
					index: target.index,
					value_type: return_type,
					value,
				});
				Ok(register)
			}
			"atomic_compare_exchange" => {
				require_argument_count(arguments, 3)?;
				if !matches!(return_type, ValueType::U32 | ValueType::I32) {
					return Err(VmError::TypeMismatch {
						expected: "u32 or i32".to_string(),
						found: return_type.name().to_string(),
					});
				}
				if let Some(target) = resolve_workgroup_access(&arguments[0])? {
					if target.value_type != return_type {
						return Err(VmError::TypeMismatch {
							expected: return_type.name().to_string(),
							found: target.value_type.name().to_string(),
						});
					}
					let index = target
						.index_expression
						.as_ref()
						.map(|index| self.compile_value_expression(index, &ValueType::U32, descriptor_layouts))
						.transpose()?;
					let expected = self.compile_value_expression(&arguments[1], &return_type, descriptor_layouts)?;
					let desired = self.compile_value_expression(&arguments[2], &return_type, descriptor_layouts)?;
					let register = self.allocate_register();
					self.instructions.push(Instruction::AtomicCompareExchangeWorkgroup {
						register,
						name: target.name,
						index,
						count: target.count,
						value_type: target.value_type,
						expected,
						desired,
					});
					return Ok(register);
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::ReadWrite, descriptor_layouts)?;
				if target.value_type != return_type {
					return Err(VmError::TypeMismatch {
						expected: return_type.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let target = self.lower_buffer_access(target, descriptor_layouts)?;
				let expected = self.compile_value_expression(&arguments[1], &return_type, descriptor_layouts)?;
				let desired = self.compile_value_expression(&arguments[2], &return_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::AtomicCompareExchangeBuffer {
					register,
					slot: target.slot,
					offset: target.offset,
					stride: target.stride,
					count: target.count,
					index: target.index,
					value_type: return_type,
					expected,
					desired,
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

				let supported_type = [
					ValueType::Vec2F,
					ValueType::Vec3F,
					ValueType::Vec4F,
					ValueType::Vec2F16,
					ValueType::Vec3F16,
					ValueType::Vec4F16,
				]
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

				let supported_type = [
					ValueType::Vec2F,
					ValueType::Vec3F,
					ValueType::Vec4F,
					ValueType::Vec2F16,
					ValueType::Vec3F16,
					ValueType::Vec4F16,
				]
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

				let supported_type = [
					ValueType::Vec2F,
					ValueType::Vec3F,
					ValueType::Vec4F,
					ValueType::Vec2F16,
					ValueType::Vec3F16,
					ValueType::Vec4F16,
				]
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
			"is_nan" | "is_infinite" | "is_finite" | "is_normal" => {
				require_argument_count(arguments, 1)?;
				let source_type = resolve_intrinsic_parameter_type(intrinsic, 0)?;
				if !matches!(source_type, ValueType::F16 | ValueType::F32) {
					return Err(VmError::TypeMismatch {
						expected: "f16 or f32".to_string(),
						found: source_type.name().to_string(),
					});
				}
				let value = self.compile_value_expression(&arguments[0], &source_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FloatPredicate {
					register,
					predicate: match name.as_str() {
						"is_nan" => FloatPredicate::Nan,
						"is_infinite" => FloatPredicate::Infinite,
						"is_finite" => FloatPredicate::Finite,
						"is_normal" => FloatPredicate::Normal,
						_ => unreachable!("Expected a floating-point classification intrinsic"),
					},
					value,
				});
				Ok(register)
			}
			"abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "floor" | "round" | "fract" | "radians"
			| "inversesqrt" | "log2" | "fwidth" => {
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
						"asin" => ScalarUnaryOperator::Asin,
						"floor" => ScalarUnaryOperator::Floor,
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
			"sincos" => {
				require_argument_count(arguments, 1)?;
				let value = self.compile_value_expression(&arguments[0], &ValueType::F32, descriptor_layouts)?;
				let sine = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register: sine,
					operator: ScalarUnaryOperator::Sin,
					value,
				});
				let cosine = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register: cosine,
					operator: ScalarUnaryOperator::Cos,
					value,
				});
				let register = self.allocate_register();
				self.instructions.push(Instruction::Construct {
					register,
					value_type: ValueType::Vec2F,
					components: vec![sine, cosine],
				});
				Ok(register)
			}
			"round_to_i32" => {
				require_argument_count(arguments, 1)?;
				let value = self.compile_value_expression(&arguments[0], &ValueType::Vec2F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::RoundToVec2I { register, value });
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

				let source_type = self.infer_expression_type(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let operator = match source_type {
					ValueType::F16 => ScalarUnaryOperator::FromF16ToF32,
					ValueType::U32 => ScalarUnaryOperator::FromU32ToF32,
					ValueType::I32 => ScalarUnaryOperator::FromI32ToF32,
					ref other => {
						return Err(VmError::TypeMismatch {
							expected: "f16, u32, or i32".to_string(),
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
			"f16" => {
				require_argument_count(arguments, 1)?;
				if expected_type != &ValueType::F16 {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: ValueType::F16.name().to_string(),
					});
				}

				let source_type = self.infer_expression_type(&arguments[0], &ValueType::F32, descriptor_layouts)?;
				let operator = match source_type {
					ValueType::F32 => ScalarUnaryOperator::FromF32ToF16,
					ValueType::U32 => ScalarUnaryOperator::FromU32ToF16,
					ValueType::I32 => ScalarUnaryOperator::FromI32ToF16,
					ref other => {
						return Err(VmError::TypeMismatch {
							expected: "f32, u32, or i32".to_string(),
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
					ValueType::I32 => ScalarUnaryOperator::FromI32ToU32,
					ValueType::F16 => ScalarUnaryOperator::FromF16ToU32,
					ValueType::F32 => ScalarUnaryOperator::FromF32ToU32,
					ref other => {
						return Err(VmError::TypeMismatch {
							expected: "u8, u16, i32, f16, or f32".to_string(),
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
			"u16" => {
				require_argument_count(arguments, 1)?;
				if expected_type != &ValueType::U16 {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: ValueType::U16.name().to_string(),
					});
				}

				let source_type = self.infer_expression_type(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				if source_type == ValueType::U16 {
					return self.compile_value_expression(&arguments[0], &ValueType::U16, descriptor_layouts);
				}
				if source_type != ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: "u32".to_string(),
						found: source_type.name().to_string(),
					});
				}
				let value = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register,
					operator: ScalarUnaryOperator::FromU32ToU16,
					value,
				});
				Ok(register)
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" | "packed_vec4f" => {
				require_argument_count(arguments, 1)?;
				// The selected overload carries the source type. This also distinguishes
				// vec4f conversions from f16 and packed storage vectors.
				let source_type = resolve_intrinsic_parameter_type(intrinsic, 0)?;
				let target_type = return_type.clone();
				if expected_type != &target_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: target_type.name().to_string(),
					});
				}
				let value = self.compile_value_expression(&arguments[0], &source_type, descriptor_layouts)?;
				let register = self.allocate_register();
				// Constructors perform the precision conversion without adding a dedicated VM instruction.
				self.instructions.push(Instruction::Construct {
					register,
					value_type: target_type,
					components: vec![value],
				});
				Ok(register)
			}
			"min" | "max" | "pow" | "step" | "atan2" => {
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
						"atan2" => ScalarBinaryOperator::Atan2,
						_ => unreachable!("Expected binary intrinsic"),
					},
					left,
					right,
				});
				Ok(register)
			}
			"smoothstep" | "mix" | "clamp" | "fma" => {
				require_argument_count(arguments, 3)?;

				let argument_type = if name == "clamp" || name == "fma" {
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
						"fma" => ScalarTernaryOperator::Fma,
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
			"subgroup_lane_index" => {
				require_argument_count(arguments, 0)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupLaneIndex { register });
				Ok(register)
			}
			"thread_position" => {
				require_argument_count(arguments, 0)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadPosition { register });
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
			"subgroup_ballot" => {
				require_argument_count(arguments, 1)?;
				let predicate = self.compile_value_expression(&arguments[0], &ValueType::Bool, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBallot { register, predicate });
				Ok(register)
			}
			"subgroup_ballot_any" => {
				require_argument_count(arguments, 1)?;
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBallotAny { register, mask });
				Ok(register)
			}
			"subgroup_ballot_find_lsb" => {
				require_argument_count(arguments, 1)?;
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBallotFindLsb { register, mask });
				Ok(register)
			}
			"subgroup_ballot_count" => {
				require_argument_count(arguments, 1)?;
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBallotCount { register, mask });
				Ok(register)
			}
			"subgroup_ballot_and_not" => {
				require_argument_count(arguments, 2)?;
				let mask = self.compile_value_expression(&arguments[0], &ValueType::Vec4U, descriptor_layouts)?;
				let removed = self.compile_value_expression(&arguments[1], &ValueType::Vec4U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions
					.push(Instruction::SubgroupBallotAndNot { register, mask, removed });
				Ok(register)
			}
			"subgroup_broadcast_u32" => {
				require_argument_count(arguments, 2)?;
				let value = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let source_lane = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBroadcastU32 {
					register,
					value,
					source_lane,
				});
				Ok(register)
			}
			"subgroup_broadcast_f32" => {
				require_argument_count(arguments, 2)?;
				let value = self.compile_value_expression(&arguments[0], &ValueType::F32, descriptor_layouts)?;
				let source_lane = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SubgroupBroadcastF32 {
					register,
					value,
					source_lane,
				});
				Ok(register)
			}
			_ => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported intrinsic `{}`", name),
			}),
		}
	}
}
