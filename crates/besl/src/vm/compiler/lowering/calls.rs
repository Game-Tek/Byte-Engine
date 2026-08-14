use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn compile_function_call_expression(
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

	pub(super) fn compile_constructor_expression(
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
			let packed_f16 = matches!(constructor_type, ValueType::Vec2F16 | ValueType::Vec3F16 | ValueType::Vec4F16);
			for parameter in parameters {
				// Packed u16 constructors accept ordinary u32 coordinate arithmetic and
				// f16 constructors accept f32 arithmetic, then narrow each component.
				let parameter_hint = if packed_u16 {
					ValueType::U32
				} else if packed_f16 {
					ValueType::F32
				} else {
					scalar_type.clone()
				};
				let parameter_type = self.infer_expression_type(parameter, &parameter_hint, descriptor_layouts)?;
				let parameter_scalar = vector_scalar_type(&parameter_type).unwrap_or_else(|| parameter_type.clone());
				let compatible = parameter_scalar == scalar_type
					|| packed_u16 && parameter_scalar == ValueType::U32
					|| packed_f16 && parameter_scalar == ValueType::F32;
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
}
