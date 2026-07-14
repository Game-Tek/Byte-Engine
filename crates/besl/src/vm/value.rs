//! Portable VM value parsing, construction, encoding, and numeric semantics.

use super::*;

pub(super) fn parse_literal(value: &str, value_type: &ValueType) -> Result<Value, VmError> {
	let parsed = match value_type {
		ValueType::Bool => match value {
			"true" => Value::Bool(true),
			"false" => Value::Bool(false),
			_ => {
				return Err(VmError::InvalidLiteral {
					value: value.to_string(),
					value_type: value_type.name().to_string(),
				});
			}
		},
		ValueType::U8 => value.parse::<u8>().map(Value::U8).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::U16 => value.parse::<u16>().map(Value::U16).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::U32 => value.parse::<u32>().map(Value::U32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::I32 => value.parse::<i32>().map(Value::I32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::Vec2U16 | ValueType::Vec4U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
		ValueType::F32 => value.parse::<f32>().map(Value::F32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::Vec2F
		| ValueType::Vec3F
		| ValueType::Vec4F
		| ValueType::Mat4F
		| ValueType::Mat4x3F
		| ValueType::Texture2D
		| ValueType::Texture3D
		| ValueType::ArrayTexture2D
		| ValueType::Struct { .. } => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
	};

	Ok(parsed)
}

pub(super) fn construct_value(value_type: &ValueType, components: &[Value]) -> Result<Value, VmError> {
	match value_type {
		ValueType::Vec2U16 => Ok(Value::Vec2U16(extract_u16_components::<2>(components)?)),
		ValueType::Vec4U16 => Ok(Value::Vec4U16(extract_u16_components::<4>(components)?)),
		ValueType::Vec2I => Ok(Value::Vec2I(extract_i32_components::<2>(components)?)),
		ValueType::Vec2U => Ok(Value::Vec2U(extract_u32_components::<2>(components)?)),
		ValueType::Vec3U => Ok(Value::Vec3U(extract_u32_components::<3>(components)?)),
		ValueType::Vec4U => Ok(Value::Vec4U(extract_u32_components::<4>(components)?)),
		ValueType::Vec2F => Ok(Value::Vec2F(extract_f32_components::<2>(components)?)),
		ValueType::Vec3F => Ok(Value::Vec3F(extract_f32_components::<3>(components)?)),
		ValueType::Vec4F => Ok(Value::Vec4F(extract_f32_components::<4>(components)?)),
		ValueType::Mat4F => Ok(Value::Mat4F(extract_f32_components::<16>(components)?)),
		ValueType::Mat4x3F => Ok(Value::Mat4x3F(extract_f32_components::<12>(components)?)),
		ValueType::Struct { fields, .. } => {
			if fields.len() != components.len()
				|| !components
					.iter()
					.zip(fields)
					.all(|(component, field)| component.matches_type(field.value_type()))
			{
				return Err(VmError::TypeMismatch {
					expected: value_type.name().to_string(),
					found: "constructor fields".to_string(),
				});
			}
			Ok(Value::Struct {
				value_type: value_type.clone(),
				fields: components.to_vec(),
			})
		}
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` is not a constructor-backed VM value type", value_type.name()),
		}),
	}
}

pub(super) fn extract_f32_components<const N: usize>(components: &[Value]) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	let mut index = 0;
	for component in components {
		let slice: &[f32] = match component {
			Value::F32(value) => std::slice::from_ref(value),
			Value::Vec2F(value) => value,
			Value::Vec3F(value) => value,
			Value::Vec4F(value) => value,
			Value::Mat4F(value) => value,
			Value::Mat4x3F(value) => value,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: ValueType::F32.name().to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + slice.len() > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} f32 components", N),
			});
		}
		values[index..index + slice.len()].copy_from_slice(slice);
		index += slice.len();
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} f32 components, but found {}", N, index),
		});
	}

	Ok(values)
}

pub(super) fn extract_u32_components<const N: usize>(components: &[Value]) -> Result<[u32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[u32] = match component {
			Value::U32(value) => std::slice::from_ref(value),
			Value::Vec2U(value) => value,
			Value::Vec3U(value) => value,
			Value::Vec4U(value) => value,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U32.name().to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + slice.len() > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} u32 components", N),
			});
		}
		values[index..index + slice.len()].copy_from_slice(slice);
		index += slice.len();
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} u32 components, but found {}", N, index),
		});
	}

	Ok(values)
}

pub(super) fn extract_u16_components<const N: usize>(components: &[Value]) -> Result<[u16; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let component_count = match component {
			Value::U16(value) => {
				if index < N {
					values[index] = *value;
				}
				1
			}
			Value::U32(value) => {
				if index < N {
					values[index] = *value as u16;
				}
				1
			}
			Value::Vec2U16(value) => {
				if index + value.len() <= N {
					values[index..index + value.len()].copy_from_slice(value);
				}
				value.len()
			}
			Value::Vec4U16(value) => {
				if index + value.len() <= N {
					values[index..index + value.len()].copy_from_slice(value);
				}
				value.len()
			}
			Value::Vec2U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			Value::Vec3U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			Value::Vec4U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "u16 or u32".to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + component_count > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} u16 components", N),
			});
		}
		index += component_count;
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} u16 components, but found {}", N, index),
		});
	}
	Ok(values)
}

pub(super) fn extract_i32_components<const N: usize>(components: &[Value]) -> Result<[i32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[i32] = match component {
			Value::I32(value) => std::slice::from_ref(value),
			Value::Vec2I(value) => value,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: ValueType::I32.name().to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + slice.len() > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} i32 components", N),
			});
		}
		values[index..index + slice.len()].copy_from_slice(slice);
		index += slice.len();
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} i32 components, but found {}", N, index),
		});
	}
	Ok(values)
}

pub(super) fn read_f32_array<const N: usize>(bytes: &[u8]) -> Result<[f32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} f32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0.0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = f32::from_ne_bytes(chunk.try_into().expect("Invalid f32 byte count"));
	}
	Ok(values)
}

pub(super) fn read_u32_array<const N: usize>(bytes: &[u8]) -> Result<[u32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} u32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = u32::from_ne_bytes(chunk.try_into().expect("Invalid u32 byte count"));
	}
	Ok(values)
}

pub(super) fn read_u16_array<const N: usize>(bytes: &[u8]) -> Result<[u16; N], VmError> {
	if bytes.len() != N * 2 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} u16 values, but found {}", N * 2, N, bytes.len()),
		});
	}
	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(2).enumerate() {
		values[index] = u16::from_ne_bytes(chunk.try_into().expect("Invalid u16 byte count"));
	}
	Ok(values)
}

pub(super) fn read_i32_array<const N: usize>(bytes: &[u8]) -> Result<[i32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} i32 values, but found {}", N * 4, N, bytes.len()),
		});
	}
	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = i32::from_ne_bytes(chunk.try_into().expect("Invalid i32 byte count"));
	}
	Ok(values)
}

pub(super) fn write_f32_slice(buffer: &mut Buffer, offset: usize, values: &[f32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(super) fn write_u32_slice(buffer: &mut Buffer, offset: usize, values: &[u32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(super) fn write_u16_slice(buffer: &mut Buffer, offset: usize, values: &[u16]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 2, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(super) fn write_i32_slice(buffer: &mut Buffer, offset: usize, values: &[i32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(super) fn lerp_rgba(left: [f32; 4], right: [f32; 4], factor: f32) -> [f32; 4] {
	let mut value = [0.0; 4];
	for index in 0..4 {
		value[index] = left[index] + (right[index] - left[index]) * factor;
	}
	value
}

pub(super) fn normalized_linear_axis(uv: f32, size: u32) -> (u32, u32, f32) {
	let coordinate = uv.clamp(0.0, 1.0) * size as f32 - 0.5;
	let low = coordinate.floor();
	let high = low + 1.0;
	let maximum = size.saturating_sub(1) as f32;
	(
		low.clamp(0.0, maximum) as u32,
		high.clamp(0.0, maximum) as u32,
		coordinate - low,
	)
}

pub(super) fn arithmetic_operator(operator: &Operators) -> Option<ArithmeticOperator> {
	match operator {
		Operators::Plus => Some(ArithmeticOperator::Add),
		Operators::Minus => Some(ArithmeticOperator::Subtract),
		Operators::Multiply => Some(ArithmeticOperator::Multiply),
		Operators::Divide => Some(ArithmeticOperator::Divide),
		Operators::Modulo => Some(ArithmeticOperator::Modulo),
		Operators::ShiftLeft => Some(ArithmeticOperator::ShiftLeft),
		Operators::ShiftRight => Some(ArithmeticOperator::ShiftRight),
		Operators::BitwiseAnd => Some(ArithmeticOperator::BitwiseAnd),
		Operators::BitwiseOr => Some(ArithmeticOperator::BitwiseOr),
		Operators::LogicalAnd => Some(ArithmeticOperator::LogicalAnd),
		Operators::LogicalOr => Some(ArithmeticOperator::LogicalOr),
		Operators::Assignment
		| Operators::Equality
		| Operators::LessThan
		| Operators::Inequality
		| Operators::GreaterThan
		| Operators::LessThanOrEqual
		| Operators::GreaterThanOrEqual => None,
	}
}

pub(super) fn binary_result_type(
	operator: ArithmeticOperator,
	left: &ValueType,
	right: &ValueType,
) -> Result<ValueType, VmError> {
	if matches!(operator, ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr) {
		return Ok(ValueType::Bool);
	}
	if operator == ArithmeticOperator::Multiply {
		match (left, right) {
			(ValueType::Mat4F, ValueType::Vec4F) => return Ok(ValueType::Vec4F),
			(ValueType::Mat4F, ValueType::Mat4F) => return Ok(ValueType::Mat4F),
			(ValueType::Mat4x3F, ValueType::Vec4F) => return Ok(ValueType::Vec3F),
			_ => {}
		}
	}
	if left == right {
		return Ok(left.clone());
	}
	if supports_scalar_broadcast(left) && right == &ValueType::F32 {
		return Ok(left.clone());
	}
	if left == &ValueType::F32 && supports_scalar_broadcast(right) {
		return Ok(right.clone());
	}
	Err(VmError::TypeMismatch {
		expected: left.name().to_string(),
		found: right.name().to_string(),
	})
}

pub(super) fn comparison_operator(operator: &Operators) -> Option<ComparisonOperator> {
	match operator {
		Operators::Equality => Some(ComparisonOperator::Equal),
		Operators::Inequality => Some(ComparisonOperator::NotEqual),
		Operators::LessThan => Some(ComparisonOperator::LessThan),
		Operators::GreaterThan => Some(ComparisonOperator::GreaterThan),
		Operators::LessThanOrEqual => Some(ComparisonOperator::LessThanOrEqual),
		Operators::GreaterThanOrEqual => Some(ComparisonOperator::GreaterThanOrEqual),
		_ => None,
	}
}

pub(super) fn supports_scalar_broadcast(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F | ValueType::Mat4F | ValueType::Mat4x3F
	)
}

pub(super) fn apply_arithmetic(operator: ArithmeticOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
	if matches!(operator, ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr) {
		let left = !is_zero_value(left)?;
		let right = !is_zero_value(right)?;
		return Ok(Value::Bool(match operator {
			ArithmeticOperator::LogicalAnd => left && right,
			ArithmeticOperator::LogicalOr => left || right,
			_ => unreachable!("Logical operators are handled before arithmetic"),
		}));
	}
	if operator == ArithmeticOperator::Multiply {
		match (left, right) {
			(Value::Mat4F(matrix), Value::Vec4F(vector)) => {
				return Ok(Value::Vec4F(multiply_mat4_vec4(*matrix, *vector)));
			}
			(Value::Mat4F(left), Value::Mat4F(right)) => {
				return Ok(Value::Mat4F(multiply_mat4(*left, *right)));
			}
			(Value::Mat4x3F(matrix), Value::Vec4F(vector)) => {
				return Ok(Value::Vec3F(multiply_mat4x3_vec4(*matrix, *vector)));
			}
			_ => {}
		}
	}
	match (left, right) {
		(Value::U8(left), Value::U8(right)) => apply_integer_arithmetic(*left, *right, operator).map(Value::U8),
		(Value::U16(left), Value::U16(right)) => apply_integer_arithmetic(*left, *right, operator).map(Value::U16),
		(Value::U32(left), Value::U32(right)) => apply_integer_arithmetic(*left, *right, operator).map(Value::U32),
		(Value::I32(left), Value::I32(right)) => apply_integer_arithmetic(*left, *right, operator).map(Value::I32),
		(Value::F32(left), Value::F32(right)) => apply_float_arithmetic(*left, *right, operator).map(Value::F32),
		(Value::Vec2U16(left), Value::Vec2U16(right)) => {
			apply_integer_array_arithmetic::<u16, 2>(*left, *right, operator).map(Value::Vec2U16)
		}
		(Value::Vec4U16(left), Value::Vec4U16(right)) => {
			apply_integer_array_arithmetic::<u16, 4>(*left, *right, operator).map(Value::Vec4U16)
		}
		(Value::Vec2I(left), Value::Vec2I(right)) => {
			apply_integer_array_arithmetic::<i32, 2>(*left, *right, operator).map(Value::Vec2I)
		}
		(Value::Vec2U(left), Value::Vec2U(right)) => {
			apply_integer_array_arithmetic::<u32, 2>(*left, *right, operator).map(Value::Vec2U)
		}
		(Value::Vec3U(left), Value::Vec3U(right)) => {
			apply_integer_array_arithmetic::<u32, 3>(*left, *right, operator).map(Value::Vec3U)
		}
		(Value::Vec4U(left), Value::Vec4U(right)) => {
			apply_integer_array_arithmetic::<u32, 4>(*left, *right, operator).map(Value::Vec4U)
		}
		(Value::Vec2F(left), Value::Vec2F(right)) => {
			apply_float_array_arithmetic::<2>(*left, *right, operator).map(Value::Vec2F)
		}
		(Value::Vec3F(left), Value::Vec3F(right)) => {
			apply_float_array_arithmetic::<3>(*left, *right, operator).map(Value::Vec3F)
		}
		(Value::Vec4F(left), Value::Vec4F(right)) => {
			apply_float_array_arithmetic::<4>(*left, *right, operator).map(Value::Vec4F)
		}
		(Value::Mat4F(left), Value::Mat4F(right)) => {
			apply_float_array_arithmetic::<16>(*left, *right, operator).map(Value::Mat4F)
		}
		(Value::Mat4x3F(left), Value::Mat4x3F(right)) => {
			apply_float_array_arithmetic::<12>(*left, *right, operator).map(Value::Mat4x3F)
		}
		(Value::Vec2F(left), Value::F32(right)) => apply_float_scalar_broadcast::<2>(*left, *right, operator).map(Value::Vec2F),
		(Value::Vec3F(left), Value::F32(right)) => apply_float_scalar_broadcast::<3>(*left, *right, operator).map(Value::Vec3F),
		(Value::Vec4F(left), Value::F32(right)) => apply_float_scalar_broadcast::<4>(*left, *right, operator).map(Value::Vec4F),
		(Value::Mat4F(left), Value::F32(right)) => {
			apply_float_scalar_broadcast::<16>(*left, *right, operator).map(Value::Mat4F)
		}
		(Value::Mat4x3F(left), Value::F32(right)) => {
			apply_float_scalar_broadcast::<12>(*left, *right, operator).map(Value::Mat4x3F)
		}
		(Value::F32(left), Value::Vec2F(right)) => apply_scalar_float_broadcast::<2>(*left, *right, operator).map(Value::Vec2F),
		(Value::F32(left), Value::Vec3F(right)) => apply_scalar_float_broadcast::<3>(*left, *right, operator).map(Value::Vec3F),
		(Value::F32(left), Value::Vec4F(right)) => apply_scalar_float_broadcast::<4>(*left, *right, operator).map(Value::Vec4F),
		(Value::F32(left), Value::Mat4F(right)) => {
			apply_scalar_float_broadcast::<16>(*left, *right, operator).map(Value::Mat4F)
		}
		(Value::F32(left), Value::Mat4x3F(right)) => {
			apply_scalar_float_broadcast::<12>(*left, *right, operator).map(Value::Mat4x3F)
		}
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_comparison(operator: ComparisonOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
	match (left, right) {
		(Value::Bool(left), Value::Bool(right)) => Ok(Value::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "equality comparison for bool".to_string(),
					found: format!("{:?}", operator),
				});
			}
		})),
		(Value::U32(left), Value::U32(right)) => Ok(Value::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		})),
		(Value::I32(left), Value::I32(right)) => Ok(Value::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		})),
		(Value::F32(left), Value::F32(right)) => Ok(Value::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		})),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

pub(super) fn is_zero_value(value: &Value) -> Result<bool, VmError> {
	match value {
		Value::Bool(value) => Ok(!*value),
		Value::U32(value) => Ok(*value == 0),
		Value::I32(value) => Ok(*value == 0),
		Value::F32(value) => Ok(*value == 0.0),
		value => Err(VmError::TypeMismatch {
			expected: "u32, i32, or f32".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

/// The `VmInteger` trait keeps integer instruction semantics consistent across BESL scalar widths.
trait VmInteger: Copy + PartialEq + Default {
	fn wrapping_add(self, right: Self) -> Self;
	fn wrapping_sub(self, right: Self) -> Self;
	fn wrapping_mul(self, right: Self) -> Self;
	fn wrapping_div(self, right: Self) -> Self;
	fn wrapping_rem(self, right: Self) -> Self;
	fn wrapping_shl(self, right: Self) -> Self;
	fn wrapping_shr(self, right: Self) -> Self;
	fn bitand(self, right: Self) -> Self;
	fn bitor(self, right: Self) -> Self;
}

macro_rules! impl_vm_integer {
	($($type:ty),+ $(,)?) => {
		$(impl VmInteger for $type {
			fn wrapping_add(self, right: Self) -> Self { self.wrapping_add(right) }
			fn wrapping_sub(self, right: Self) -> Self { self.wrapping_sub(right) }
			fn wrapping_mul(self, right: Self) -> Self { self.wrapping_mul(right) }
			fn wrapping_div(self, right: Self) -> Self { self.wrapping_div(right) }
			fn wrapping_rem(self, right: Self) -> Self { self.wrapping_rem(right) }
			fn wrapping_shl(self, right: Self) -> Self { self.wrapping_shl(right as u32) }
			fn wrapping_shr(self, right: Self) -> Self { self.wrapping_shr(right as u32) }
			fn bitand(self, right: Self) -> Self { self & right }
			fn bitor(self, right: Self) -> Self { self | right }
		})+
	};
}

impl_vm_integer!(u8, u16, u32, i32);

fn apply_integer_arithmetic<T: VmInteger>(left: T, right: T, operator: ArithmeticOperator) -> Result<T, VmError> {
	let zero = T::default();
	match operator {
		ArithmeticOperator::Add => Ok(left.wrapping_add(right)),
		ArithmeticOperator::Subtract => Ok(left.wrapping_sub(right)),
		ArithmeticOperator::Multiply => Ok(left.wrapping_mul(right)),
		ArithmeticOperator::Divide => {
			if right == zero {
				return Err(VmError::ArithmeticError {
					message: "Division by zero".to_string(),
				});
			}
			Ok(left.wrapping_div(right))
		}
		ArithmeticOperator::Modulo => {
			if right == zero {
				return Err(VmError::ArithmeticError {
					message: "Modulo by zero".to_string(),
				});
			}
			Ok(left.wrapping_rem(right))
		}
		ArithmeticOperator::ShiftLeft => Ok(left.wrapping_shl(right)),
		ArithmeticOperator::ShiftRight => Ok(left.wrapping_shr(right)),
		ArithmeticOperator::BitwiseAnd => Ok(left.bitand(right)),
		ArithmeticOperator::BitwiseOr => Ok(left.bitor(right)),
		ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr => {
			unreachable!("Logical operations are evaluated before integer arithmetic")
		}
	}
}

fn apply_integer_array_arithmetic<T: VmInteger, const N: usize>(
	left: [T; N],
	right: [T; N],
	operator: ArithmeticOperator,
) -> Result<[T; N], VmError> {
	let mut values = [T::default(); N];
	for index in 0..N {
		values[index] = apply_integer_arithmetic(left[index], right[index], operator)?;
	}
	Ok(values)
}

pub(super) fn apply_float_arithmetic(left: f32, right: f32, operator: ArithmeticOperator) -> Result<f32, VmError> {
	match operator {
		ArithmeticOperator::Add => Ok(left + right),
		ArithmeticOperator::Subtract => Ok(left - right),
		ArithmeticOperator::Multiply => Ok(left * right),
		ArithmeticOperator::Divide => Ok(left / right),
		ArithmeticOperator::Modulo => Ok(left % right),
		ArithmeticOperator::ShiftLeft
		| ArithmeticOperator::ShiftRight
		| ArithmeticOperator::BitwiseAnd
		| ArithmeticOperator::BitwiseOr
		| ArithmeticOperator::LogicalAnd
		| ArithmeticOperator::LogicalOr => Err(VmError::TypeMismatch {
			expected: "integer operands".to_string(),
			found: ValueType::F32.name().to_string(),
		}),
	}
}

pub(super) fn apply_float_array_arithmetic<const N: usize>(
	left: [f32; N],
	right: [f32; N],
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left[index], right[index], operator)?;
	}
	Ok(values)
}

pub(super) fn apply_float_scalar_broadcast<const N: usize>(
	left: [f32; N],
	right: f32,
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left[index], right, operator)?;
	}
	Ok(values)
}

pub(super) fn apply_scalar_float_broadcast<const N: usize>(
	left: f32,
	right: [f32; N],
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left, right[index], operator)?;
	}
	Ok(values)
}

pub(super) fn apply_dot_product(left: &Value, right: &Value) -> Result<Value, VmError> {
	match (left, right) {
		(Value::Vec2F(left), Value::Vec2F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(Value::Vec3F(left), Value::Vec3F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(Value::Vec4F(left), Value::Vec4F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_cross_product(left: &Value, right: &Value) -> Result<Value, VmError> {
	match (left, right) {
		(Value::Vec3F(left), Value::Vec3F(right)) => Ok(Value::Vec3F(cross_product(*left, *right))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_length(value: &Value) -> Result<Value, VmError> {
	match value {
		Value::Vec2F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		Value::Vec3F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		Value::Vec4F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_normalize(value: &Value) -> Result<Value, VmError> {
	match value {
		Value::Vec2F(value) => normalize_vector(*value).map(Value::Vec2F),
		Value::Vec3F(value) => normalize_vector(*value).map(Value::Vec3F),
		Value::Vec4F(value) => normalize_vector(*value).map(Value::Vec4F),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_reflect(incident: &Value, normal: &Value) -> Result<Value, VmError> {
	match (incident, normal) {
		(Value::Vec2F(incident), Value::Vec2F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec2F),
		(Value::Vec3F(incident), Value::Vec3F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec3F),
		(Value::Vec4F(incident), Value::Vec4F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec4F),
		(incident, normal) => Err(VmError::TypeMismatch {
			expected: incident.value_type().name().to_string(),
			found: normal.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_scalar_unary(operator: ScalarUnaryOperator, value: &Value) -> Result<Value, VmError> {
	match operator {
		ScalarUnaryOperator::FromU32ToF32 => {
			let Value::U32(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U32.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};

			return Ok(Value::F32(*value as f32));
		}
		ScalarUnaryOperator::FromF32ToU32 => {
			let Value::F32(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::F32.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};

			return Ok(Value::U32(*value as u32));
		}
		ScalarUnaryOperator::FromU8ToU32 => {
			let Value::U8(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U8.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};
			return Ok(Value::U32(u32::from(*value)));
		}
		ScalarUnaryOperator::FromU16ToU32 => {
			let Value::U16(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U16.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};
			return Ok(Value::U32(u32::from(*value)));
		}
		_ => {}
	}

	map_float_value(value, |value| match operator {
		ScalarUnaryOperator::Abs => value.abs(),
		ScalarUnaryOperator::Sqrt => value.sqrt(),
		ScalarUnaryOperator::Exp => value.exp(),
		ScalarUnaryOperator::Sin => value.sin(),
		ScalarUnaryOperator::Cos => value.cos(),
		ScalarUnaryOperator::Tan => value.tan(),
		ScalarUnaryOperator::Round => value.round(),
		ScalarUnaryOperator::Fract => value - value.floor(),
		ScalarUnaryOperator::Radians => value.to_radians(),
		ScalarUnaryOperator::InverseSqrt => 1.0 / value.sqrt(),
		ScalarUnaryOperator::Log2 => value.log2(),
		ScalarUnaryOperator::Fwidth => 0.0,
		ScalarUnaryOperator::FromU32ToF32
		| ScalarUnaryOperator::FromF32ToU32
		| ScalarUnaryOperator::FromU8ToU32
		| ScalarUnaryOperator::FromU16ToU32 => unreachable!("conversion operators return early"),
	})
}

pub(super) fn map_float_value(value: &Value, map: impl Fn(f32) -> f32) -> Result<Value, VmError> {
	match value {
		Value::F32(value) => Ok(Value::F32(map(*value))),
		Value::Vec2F(value) => Ok(Value::Vec2F(value.map(&map))),
		Value::Vec3F(value) => Ok(Value::Vec3F(value.map(&map))),
		Value::Vec4F(value) => Ok(Value::Vec4F(value.map(&map))),
		value => Err(VmError::TypeMismatch {
			expected: "f32 or float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_scalar_binary(operator: ScalarBinaryOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
	fn apply(operator: ScalarBinaryOperator, left: f32, right: f32) -> f32 {
		match operator {
			ScalarBinaryOperator::Min => left.min(right),
			ScalarBinaryOperator::Max => left.max(right),
			ScalarBinaryOperator::Pow => left.powf(right),
			ScalarBinaryOperator::Step => f32::from(right >= left),
		}
	}
	match (left, right) {
		(Value::F32(left), Value::F32(right)) => Ok(Value::F32(apply(operator, *left, *right))),
		(Value::Vec2F(left), Value::Vec2F(right)) => Ok(Value::Vec2F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(Value::Vec3F(left), Value::Vec3F(right)) => Ok(Value::Vec3F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(Value::Vec4F(left), Value::Vec4F(right)) => Ok(Value::Vec4F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

pub(super) fn apply_scalar_ternary(
	operator: ScalarTernaryOperator,
	first: &Value,
	second: &Value,
	third: &Value,
) -> Result<Value, VmError> {
	fn apply(operator: ScalarTernaryOperator, first: f32, second: f32, third: f32) -> f32 {
		match operator {
			ScalarTernaryOperator::Mix => first + (second - first) * third,
			ScalarTernaryOperator::Clamp => first.clamp(second, third),
			ScalarTernaryOperator::Smoothstep => {
				let t = ((third - first) / (second - first)).clamp(0.0, 1.0);
				t * t * (3.0 - 2.0 * t)
			}
		}
	}
	match (first, second, third) {
		(Value::F32(first), Value::F32(second), Value::F32(third)) => Ok(Value::F32(apply(operator, *first, *second, *third))),
		(Value::Vec2F(first), Value::Vec2F(second), Value::Vec2F(third)) => Ok(Value::Vec2F(std::array::from_fn(|index| {
			apply(operator, first[index], second[index], third[index])
		}))),
		(Value::Vec3F(first), Value::Vec3F(second), Value::Vec3F(third)) => Ok(Value::Vec3F(std::array::from_fn(|index| {
			apply(operator, first[index], second[index], third[index])
		}))),
		(Value::Vec4F(first), Value::Vec4F(second), Value::Vec4F(third)) => Ok(Value::Vec4F(std::array::from_fn(|index| {
			apply(operator, first[index], second[index], third[index])
		}))),
		_ => Err(VmError::TypeMismatch {
			expected: first.value_type().name().to_string(),
			found: format!("{}, {}", second.value_type().name(), third.value_type().name()),
		}),
	}
}

pub(super) fn extract_value(value: &Value, index: usize, expected_type: &ValueType) -> Result<Value, VmError> {
	let extracted = match value {
		Value::Vec2U16(value) => value.get(index).copied().map(Value::U16),
		Value::Vec4U16(value) => value.get(index).copied().map(Value::U16),
		Value::Vec2I(value) => value.get(index).copied().map(Value::I32),
		Value::Vec2U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec3U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec4U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec2F(value) => value.get(index).copied().map(Value::F32),
		Value::Vec3F(value) => value.get(index).copied().map(Value::F32),
		Value::Vec4F(value) => value.get(index).copied().map(Value::F32),
		Value::Mat4F(value) if index < 4 => Some(Value::Vec4F(
			value[index * 4..index * 4 + 4].try_into().expect("Matrix column size"),
		)),
		Value::Mat4x3F(value) if index < 4 => Some(Value::Vec3F(
			value[index * 3..index * 3 + 3].try_into().expect("Matrix column size"),
		)),
		Value::Struct { fields, .. } => fields.get(index).cloned(),
		_ => None,
	}
	.ok_or_else(|| VmError::UnsupportedExpression {
		message: format!("Member index {} is invalid for `{}`", index, value.value_type().name()),
	})?;
	if !extracted.matches_type(expected_type) {
		return Err(VmError::TypeMismatch {
			expected: expected_type.name().to_string(),
			found: extracted.value_type().name().to_string(),
		});
	}
	Ok(extracted)
}

pub(super) fn vector_scalar_type(value_type: &ValueType) -> Option<ValueType> {
	match value_type {
		ValueType::Vec2U16 => Some(ValueType::U16),
		ValueType::Vec4U16 => Some(ValueType::U16),
		ValueType::Vec2I => Some(ValueType::I32),
		ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => Some(ValueType::U32),
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F => Some(ValueType::F32),
		_ => None,
	}
}

pub(super) fn multiply_mat4_vec4(matrix: [f32; 16], vector: [f32; 4]) -> [f32; 4] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 4 + row] * vector[column]).sum())
}

pub(super) fn multiply_mat4(left: [f32; 16], right: [f32; 16]) -> [f32; 16] {
	let mut value = [0.0; 16];
	for column in 0..4 {
		let product = multiply_mat4_vec4(
			left,
			right[column * 4..column * 4 + 4]
				.try_into()
				.expect("Matrix columns contain four values"),
		);
		value[column * 4..column * 4 + 4].copy_from_slice(&product);
	}
	value
}

pub(super) fn multiply_mat4x3_vec4(matrix: [f32; 12], vector: [f32; 4]) -> [f32; 3] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 3 + row] * vector[column]).sum())
}

pub(super) fn expect_vec2u(value: Value) -> Result<[u32; 2], VmError> {
	let Value::Vec2U(value) = value else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::Vec2U.name().to_string(),
			found: value.value_type().name().to_string(),
		});
	};
	Ok(value)
}

pub(super) fn expect_u32(value: Value) -> Result<u32, VmError> {
	let Value::U32(value) = value else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::U32.name().to_string(),
			found: value.value_type().name().to_string(),
		});
	};
	Ok(value)
}

pub(super) fn dot_product<const N: usize>(left: [f32; N], right: [f32; N]) -> f32 {
	let mut value = 0.0;
	for index in 0..N {
		value += left[index] * right[index];
	}
	value
}

pub(super) fn cross_product(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

pub(super) fn normalize_vector<const N: usize>(value: [f32; N]) -> Result<[f32; N], VmError> {
	let length = dot_product(value, value).sqrt();
	if length == 0.0 {
		return Err(VmError::ArithmeticError {
			message: "Cannot normalize a zero-length vector".to_string(),
		});
	}

	let mut normalized = [0.0; N];
	for index in 0..N {
		normalized[index] = value[index] / length;
	}
	Ok(normalized)
}

pub(super) fn reflect_vector<const N: usize>(incident: [f32; N], normal: [f32; N]) -> Result<[f32; N], VmError> {
	let scale = 2.0 * dot_product(incident, normal);
	let mut reflected = [0.0; N];
	for index in 0..N {
		reflected[index] = incident[index] - scale * normal[index];
	}
	Ok(reflected)
}

pub(super) fn read_register(registers: &[Option<Value>], register: usize) -> Result<Value, VmError> {
	registers
		.get(register)
		.and_then(Option::clone)
		.ok_or(VmError::UninitializedRegister { register })
}

pub(super) fn resolve_resource_slot(slot: DescriptorSlot, registers: &[Option<Value>]) -> Result<DescriptorSlot, VmError> {
	if !slot.is_dynamic_resource() {
		return Ok(slot);
	}
	match read_register(registers, slot.binding() as usize)? {
		Value::Resource { slot, .. } => Ok(slot),
		value => Err(VmError::TypeMismatch {
			expected: "resource handle".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(super) fn read_buffer_array_index(registers: &[Option<Value>], register: usize, count: usize) -> Result<usize, VmError> {
	let index = read_register(registers, register)?;
	let Value::U32(index) = index else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::U32.name().to_string(),
			found: index.value_type().name().to_string(),
		});
	};
	let index = index as usize;
	if index >= count {
		return Err(VmError::BufferArrayIndexOutOfBounds { index, count });
	}

	Ok(index)
}
