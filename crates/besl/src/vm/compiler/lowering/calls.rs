use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn compile_function_call_expression(
		&mut self,
		function: &NodeId,
		parameters: &[NodeId],
		expected_type: &ValueType,
	) -> Result<usize, VmError> {
		let function_ref = &self.program[function];
		match function_ref.node() {
			Nodes::Struct { fields, .. } => {
				let constructor_type = resolve_value_type(self.program, function)?;
				let fields = fields.clone();
				self.compile_constructor_expression(function, parameters, expected_type, constructor_type, &fields)
			}
			Nodes::Function { .. } => {
				let signature = extract_function_signature(self.program, function)?;
				let return_type = signature.return_type.clone().ok_or_else(|| VmError::UnsupportedExpression {
					message: "Void functions cannot be used as value expressions".to_string(),
				})?;
				if &return_type != expected_type {
					return Err(VmError::mismatch(expected_type.name(), return_type.name()));
				}
				let (function, arguments) = self.compile_call_arguments(function, &signature, parameters)?;
				Ok(self.emit_to(|register| ControlInstruction::Call {
					register: Some(register),
					function,
					arguments,
				}))
			}
			node => Err(VmError::UnsupportedExpression {
				message: format!("Expected a callable value, but found {}", describe_node(node)),
			}),
		}
	}

	/// Compiles the arguments of a call to the function `function`, whose signature is `signature`, and returns the
	/// function's index with the registers that hold its arguments.
	pub(super) fn compile_call_arguments(
		&mut self,
		function: &NodeId,
		signature: &FunctionSignature,
		parameters: &[NodeId],
	) -> Result<(usize, Vec<usize>), VmError> {
		require_argument_count(parameters, signature.params.len())?;
		let mut arguments = Vec::with_capacity(parameters.len());
		for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
			arguments.push(self.compile_value_expression(parameter, &signature_parameter.value_type)?);
		}
		// Every function a call reaches is compiled, so it has an index.
		Ok((self.function_ids[function], arguments))
	}

	pub(super) fn compile_constructor_expression(
		&mut self,
		_function: &NodeId,
		parameters: &[NodeId],
		expected_type: &ValueType,
		constructor_type: ValueType,
		fields: &[NodeId],
	) -> Result<usize, VmError> {
		if &constructor_type != expected_type {
			return Err(VmError::mismatch(expected_type.name(), constructor_type.name()));
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
				let field_type = match self.program[field].node() {
					Nodes::Member { r#type, .. } => resolve_value_type(self.program, r#type)?,
					node => {
						return Err(VmError::UnsupportedExpression {
							message: format!("Expected a constructor field, but found {}", describe_node(node)),
						});
					}
				};
				components.push(self.compile_value_expression(parameter, &field_type)?);
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
				let parameter_type = self.infer_expression_type(parameter, &parameter_hint)?;
				let parameter_scalar = vector_scalar_type(&parameter_type).unwrap_or_else(|| parameter_type.clone());
				let compatible = parameter_scalar == scalar_type
					|| packed_u16 && parameter_scalar == ValueType::U32
					|| packed_f16 && parameter_scalar == ValueType::F32;
				if !compatible {
					return Err(VmError::mismatch(scalar_type.name(), parameter_type.name()));
				}
				components.push(self.compile_value_expression(parameter, &parameter_type)?);
			}
		}

		Ok(self.emit_to(|register| ValueInstruction::Construct {
			register,
			value_type: constructor_type,
			components,
		}))
	}
}
