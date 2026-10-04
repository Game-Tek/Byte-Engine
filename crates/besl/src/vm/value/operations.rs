use super::*;

pub(crate) fn lerp_rgba(left: [f32; 4], right: [f32; 4], factor: f32) -> [f32; 4] {
	std::array::from_fn(|index| left[index] + (right[index] - left[index]) * factor)
}

/// Reports that a `found` value was used where an `expected` one is required.
pub(crate) fn type_mismatch(expected: &ValueType, found: &ValueType) -> VmError {
	VmError::TypeMismatch {
		expected: expected.name().to_string(),
		found: found.name().to_string(),
	}
}

pub(crate) fn normalized_linear_axis(uv: f32, size: u32) -> (u32, u32, f32) {
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

/// Selects the scalar instruction that converts `source` to `target`, or `None` when the VM has no such conversion.
pub(crate) fn conversion_operator(source: &ValueType, target: &ValueType) -> Option<ScalarUnaryOperator> {
	Some(match (source, target) {
		(ValueType::F16, ValueType::F32) => ScalarUnaryOperator::FromF16ToF32,
		(ValueType::U32, ValueType::F32) => ScalarUnaryOperator::FromU32ToF32,
		(ValueType::I32, ValueType::F32) => ScalarUnaryOperator::FromI32ToF32,
		(ValueType::F32, ValueType::F16) => ScalarUnaryOperator::FromF32ToF16,
		(ValueType::U32, ValueType::F16) => ScalarUnaryOperator::FromU32ToF16,
		(ValueType::I32, ValueType::F16) => ScalarUnaryOperator::FromI32ToF16,
		(ValueType::U8, ValueType::U32) => ScalarUnaryOperator::FromU8ToU32,
		(ValueType::U16, ValueType::U32) => ScalarUnaryOperator::FromU16ToU32,
		(ValueType::I32, ValueType::U32) => ScalarUnaryOperator::FromI32ToU32,
		(ValueType::F16, ValueType::U32) => ScalarUnaryOperator::FromF16ToU32,
		(ValueType::F32, ValueType::U32) => ScalarUnaryOperator::FromF32ToU32,
		(ValueType::U32, ValueType::U16) => ScalarUnaryOperator::FromU32ToU16,
		_ => return None,
	})
}

pub(crate) fn arithmetic_operator(operator: &Operators) -> Option<ArithmeticOperator> {
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
		Operators::BitwiseXor => Some(ArithmeticOperator::BitwiseXor),
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

pub(crate) fn binary_result_type(
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
	if supports_scalar_broadcast(left) && vector_scalar_type(left).as_ref() == Some(right) {
		return Ok(left.clone());
	}
	if supports_scalar_broadcast(right) && vector_scalar_type(right).as_ref() == Some(left) {
		return Ok(right.clone());
	}
	Err(type_mismatch(left, right))
}

pub(crate) fn comparison_operator(operator: &Operators) -> Option<ComparisonOperator> {
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

pub(crate) fn supports_scalar_broadcast(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Vec2F16
			| ValueType::Vec3F16
			| ValueType::Vec4F16
			| ValueType::Vec2F
			| ValueType::Vec3F
			| ValueType::Vec4F
			| ValueType::Mat4F
			| ValueType::Mat4x3F
	)
}

pub(crate) fn apply_arithmetic(operator: ArithmeticOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
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
		(Value::F16(left), Value::F16(right)) => apply_f16_arithmetic(*left, *right, operator).map(Value::F16),
		(Value::F32(left), Value::F32(right)) => apply_float_arithmetic(*left, *right, operator).map(Value::F32),
		(Value::Vec2U16(left), Value::Vec2U16(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec2U16)
		}
		(Value::Vec4U16(left), Value::Vec4U16(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec4U16)
		}
		(Value::Vec2I(left), Value::Vec2I(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec2I)
		}
		(Value::Vec2U(left), Value::Vec2U(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec2U)
		}
		(Value::Vec3U(left), Value::Vec3U(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec3U)
		}
		(Value::Vec4U(left), Value::Vec4U(right)) => {
			apply_lanes(*left, *right, operator, apply_integer_arithmetic).map(Value::Vec4U)
		}
		(Value::Vec2F16(left), Value::Vec2F16(right)) => {
			apply_lanes(*left, *right, operator, apply_f16_arithmetic).map(Value::Vec2F16)
		}
		(Value::Vec3F16(left), Value::Vec3F16(right)) => {
			apply_lanes(*left, *right, operator, apply_f16_arithmetic).map(Value::Vec3F16)
		}
		(Value::Vec4F16(left), Value::Vec4F16(right)) => {
			apply_lanes(*left, *right, operator, apply_f16_arithmetic).map(Value::Vec4F16)
		}
		(Value::Vec2F(left), Value::Vec2F(right)) => {
			apply_lanes(*left, *right, operator, apply_float_arithmetic).map(Value::Vec2F)
		}
		(Value::Vec3F(left), Value::Vec3F(right)) => {
			apply_lanes(*left, *right, operator, apply_float_arithmetic).map(Value::Vec3F)
		}
		(Value::Vec4F(left), Value::Vec4F(right)) => {
			apply_lanes(*left, *right, operator, apply_float_arithmetic).map(Value::Vec4F)
		}
		(Value::Mat4F(left), Value::Mat4F(right)) => {
			apply_lanes(*left, *right, operator, apply_float_arithmetic).map(Value::Mat4F)
		}
		(Value::Mat4x3F(left), Value::Mat4x3F(right)) => {
			apply_lanes(*left, *right, operator, apply_float_arithmetic).map(Value::Mat4x3F)
		}
		(Value::Vec2F16(left), Value::F16(right)) => {
			apply_lanes(*left, [*right; 2], operator, apply_f16_arithmetic).map(Value::Vec2F16)
		}
		(Value::Vec3F16(left), Value::F16(right)) => {
			apply_lanes(*left, [*right; 3], operator, apply_f16_arithmetic).map(Value::Vec3F16)
		}
		(Value::Vec4F16(left), Value::F16(right)) => {
			apply_lanes(*left, [*right; 4], operator, apply_f16_arithmetic).map(Value::Vec4F16)
		}
		(Value::Vec2F(left), Value::F32(right)) => {
			apply_lanes(*left, [*right; 2], operator, apply_float_arithmetic).map(Value::Vec2F)
		}
		(Value::Vec3F(left), Value::F32(right)) => {
			apply_lanes(*left, [*right; 3], operator, apply_float_arithmetic).map(Value::Vec3F)
		}
		(Value::Vec4F(left), Value::F32(right)) => {
			apply_lanes(*left, [*right; 4], operator, apply_float_arithmetic).map(Value::Vec4F)
		}
		(Value::Mat4F(left), Value::F32(right)) => {
			apply_lanes(*left, [*right; 16], operator, apply_float_arithmetic).map(Value::Mat4F)
		}
		(Value::Mat4x3F(left), Value::F32(right)) => {
			apply_lanes(*left, [*right; 12], operator, apply_float_arithmetic).map(Value::Mat4x3F)
		}
		(Value::F16(left), Value::Vec2F16(right)) => {
			apply_lanes([*left; 2], *right, operator, apply_f16_arithmetic).map(Value::Vec2F16)
		}
		(Value::F16(left), Value::Vec3F16(right)) => {
			apply_lanes([*left; 3], *right, operator, apply_f16_arithmetic).map(Value::Vec3F16)
		}
		(Value::F16(left), Value::Vec4F16(right)) => {
			apply_lanes([*left; 4], *right, operator, apply_f16_arithmetic).map(Value::Vec4F16)
		}
		(Value::F32(left), Value::Vec2F(right)) => {
			apply_lanes([*left; 2], *right, operator, apply_float_arithmetic).map(Value::Vec2F)
		}
		(Value::F32(left), Value::Vec3F(right)) => {
			apply_lanes([*left; 3], *right, operator, apply_float_arithmetic).map(Value::Vec3F)
		}
		(Value::F32(left), Value::Vec4F(right)) => {
			apply_lanes([*left; 4], *right, operator, apply_float_arithmetic).map(Value::Vec4F)
		}
		(Value::F32(left), Value::Mat4F(right)) => {
			apply_lanes([*left; 16], *right, operator, apply_float_arithmetic).map(Value::Mat4F)
		}
		(Value::F32(left), Value::Mat4x3F(right)) => {
			apply_lanes([*left; 12], *right, operator, apply_float_arithmetic).map(Value::Mat4x3F)
		}
		(left, right) => Err(type_mismatch(&left.value_type(), &right.value_type())),
	}
}

pub(crate) fn apply_comparison(operator: ComparisonOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
	/// Orders two scalars of one type, with IEEE semantics for floats.
	fn compare<T: PartialOrd>(operator: ComparisonOperator, left: T, right: T) -> bool {
		match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		}
	}
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
		(Value::U32(left), Value::U32(right)) => Ok(Value::Bool(compare(operator, left, right))),
		(Value::I32(left), Value::I32(right)) => Ok(Value::Bool(compare(operator, left, right))),
		(Value::F16(left), Value::F16(right)) => Ok(Value::Bool(compare(operator, left, right))),
		(Value::F32(left), Value::F32(right)) => Ok(Value::Bool(compare(operator, left, right))),
		(left, right) => Err(type_mismatch(&left.value_type(), &right.value_type())),
	}
}

/// Classifies one scalar floating-point value without changing its precision.
pub(crate) fn apply_float_predicate(predicate: FloatPredicate, value: &Value) -> Result<Value, VmError> {
	let result = match value {
		Value::F16(value) => match predicate {
			FloatPredicate::Nan => value.is_nan(),
			FloatPredicate::Infinite => value.is_infinite(),
			FloatPredicate::Finite => value.is_finite(),
			FloatPredicate::Normal => value.is_normal(),
		},
		Value::F32(value) => match predicate {
			FloatPredicate::Nan => value.is_nan(),
			FloatPredicate::Infinite => value.is_infinite(),
			FloatPredicate::Finite => value.is_finite(),
			FloatPredicate::Normal => value.is_normal(),
		},
		value => {
			return Err(VmError::TypeMismatch {
				expected: "f16 or f32".to_string(),
				found: value.value_type().name().to_string(),
			});
		}
	};
	Ok(Value::Bool(result))
}

/// Applies one relaxed scalar integer read-modify-write operation and returns the replacement value.
pub(crate) fn apply_atomic_operation(operation: AtomicOperation, previous: &Value, operand: &Value) -> Result<Value, VmError> {
	fn apply<T: VmInteger + Ord>(operation: AtomicOperation, previous: T, operand: T) -> T {
		match operation {
			AtomicOperation::Exchange => operand,
			AtomicOperation::Add => previous.wrapping_add(operand),
			AtomicOperation::Subtract => previous.wrapping_sub(operand),
			AtomicOperation::Min => previous.min(operand),
			AtomicOperation::Max => previous.max(operand),
			AtomicOperation::And => previous & operand,
			AtomicOperation::Or => previous | operand,
			AtomicOperation::Xor => previous ^ operand,
		}
	}
	match (previous, operand) {
		(Value::U32(previous), Value::U32(operand)) => Ok(Value::U32(apply(operation, *previous, *operand))),
		(Value::I32(previous), Value::I32(operand)) => Ok(Value::I32(apply(operation, *previous, *operand))),
		(previous, operand) => Err(type_mismatch(&previous.value_type(), &operand.value_type())),
	}
}

pub(crate) fn is_zero_value(value: &Value) -> Result<bool, VmError> {
	match value {
		Value::Bool(value) => Ok(!*value),
		Value::U32(value) => Ok(*value == 0),
		Value::I32(value) => Ok(*value == 0),
		Value::F16(value) => Ok(*value == f16::from_f32(0.0)),
		Value::F32(value) => Ok(*value == 0.0),
		value => Err(VmError::TypeMismatch {
			expected: "u32, i32, f16, or f32".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

/// Returns the 32-bit pattern that a `switch` compares against its case labels.
/// `bool` values are `0` and `1`, and `i32` values keep their two's complement bits.
pub(crate) fn switch_label(value: &Value) -> Result<u32, VmError> {
	match *value {
		Value::Bool(value) => Ok(value.into()),
		Value::U8(value) => Ok(value.into()),
		Value::U16(value) => Ok(value.into()),
		Value::U32(value) => Ok(value),
		Value::I32(value) => Ok(value.cast_unsigned()),
		ref value => Err(VmError::TypeMismatch {
			expected: "bool, u8, u16, u32, or i32".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

/// The `VmInteger` trait keeps integer instruction semantics consistent across BESL scalar widths.
trait VmInteger:
	Copy + PartialEq + Default + std::ops::BitAnd<Output = Self> + std::ops::BitOr<Output = Self> + std::ops::BitXor<Output = Self>
{
	fn wrapping_add(self, right: Self) -> Self;
	fn wrapping_sub(self, right: Self) -> Self;
	fn wrapping_mul(self, right: Self) -> Self;
	fn wrapping_div(self, right: Self) -> Self;
	fn wrapping_rem(self, right: Self) -> Self;
	fn wrapping_shl(self, right: Self) -> Self;
	fn wrapping_shr(self, right: Self) -> Self;
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
		ArithmeticOperator::Divide if right == zero => Err(VmError::ArithmeticError {
			message: "Division by zero".to_string(),
		}),
		ArithmeticOperator::Modulo if right == zero => Err(VmError::ArithmeticError {
			message: "Modulo by zero".to_string(),
		}),
		ArithmeticOperator::Divide => Ok(left.wrapping_div(right)),
		ArithmeticOperator::Modulo => Ok(left.wrapping_rem(right)),
		ArithmeticOperator::ShiftLeft => Ok(left.wrapping_shl(right)),
		ArithmeticOperator::ShiftRight => Ok(left.wrapping_shr(right)),
		ArithmeticOperator::BitwiseAnd => Ok(left & right),
		ArithmeticOperator::BitwiseOr => Ok(left | right),
		ArithmeticOperator::BitwiseXor => Ok(left ^ right),
		ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr => {
			unreachable!("Logical operations are evaluated before integer arithmetic")
		}
	}
}

/// Applies `apply` lane by lane, as BESL vector arithmetic does. Pass `[scalar; N]` to broadcast a scalar operand.
fn apply_lanes<T: Copy + Default, const N: usize>(
	left: [T; N],
	right: [T; N],
	operator: ArithmeticOperator,
	apply: impl Fn(T, T, ArithmeticOperator) -> Result<T, VmError>,
) -> Result<[T; N], VmError> {
	let mut values = [T::default(); N];
	for index in 0..N {
		values[index] = apply(left[index], right[index], operator)?;
	}
	Ok(values)
}

fn apply_f16_arithmetic(left: f16, right: f16, operator: ArithmeticOperator) -> Result<f16, VmError> {
	let value = match operator {
		ArithmeticOperator::Add => left.to_f32() + right.to_f32(),
		ArithmeticOperator::Subtract => left.to_f32() - right.to_f32(),
		ArithmeticOperator::Multiply => left.to_f32() * right.to_f32(),
		ArithmeticOperator::Divide => left.to_f32() / right.to_f32(),
		ArithmeticOperator::Modulo => left.to_f32() % right.to_f32(),
		ArithmeticOperator::ShiftLeft
		| ArithmeticOperator::ShiftRight
		| ArithmeticOperator::BitwiseAnd
		| ArithmeticOperator::BitwiseOr
		| ArithmeticOperator::BitwiseXor
		| ArithmeticOperator::LogicalAnd
		| ArithmeticOperator::LogicalOr => {
			return Err(VmError::TypeMismatch {
				expected: "integer operands".to_string(),
				found: ValueType::F16.name().to_string(),
			});
		}
	};
	Ok(f16::from_f32(value))
}

pub(crate) fn apply_float_arithmetic(left: f32, right: f32, operator: ArithmeticOperator) -> Result<f32, VmError> {
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
		| ArithmeticOperator::BitwiseXor
		| ArithmeticOperator::LogicalAnd
		| ArithmeticOperator::LogicalOr => Err(VmError::TypeMismatch {
			expected: "integer operands".to_string(),
			found: ValueType::F32.name().to_string(),
		}),
	}
}

pub(crate) fn apply_dot_product(left: &Value, right: &Value) -> Result<Value, VmError> {
	match (left, right) {
		(Value::Vec2F(left), Value::Vec2F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(Value::Vec3F(left), Value::Vec3F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(Value::Vec4F(left), Value::Vec4F(right)) => Ok(Value::F32(dot_product(*left, *right))),
		(Value::Vec2F16(left), Value::Vec2F16(right)) => Ok(Value::F16(f16::from_f32(dot_product(
			left.map(f16::to_f32),
			right.map(f16::to_f32),
		)))),
		(Value::Vec3F16(left), Value::Vec3F16(right)) => Ok(Value::F16(f16::from_f32(dot_product(
			left.map(f16::to_f32),
			right.map(f16::to_f32),
		)))),
		(Value::Vec4F16(left), Value::Vec4F16(right)) => Ok(Value::F16(f16::from_f32(dot_product(
			left.map(f16::to_f32),
			right.map(f16::to_f32),
		)))),
		(left, right) => Err(type_mismatch(&left.value_type(), &right.value_type())),
	}
}

pub(crate) fn apply_cross_product(left: &Value, right: &Value) -> Result<Value, VmError> {
	match (left, right) {
		(Value::Vec3F(left), Value::Vec3F(right)) => Ok(Value::Vec3F(cross_product(*left, *right))),
		(left, right) => Err(type_mismatch(&left.value_type(), &right.value_type())),
	}
}

pub(crate) fn apply_length(value: &Value) -> Result<Value, VmError> {
	match value {
		Value::Vec2F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		Value::Vec3F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		Value::Vec4F(value) => Ok(Value::F32(dot_product(*value, *value).sqrt())),
		Value::Vec2F16(value) => Ok(Value::F16(f16::from_f32(
			dot_product(value.map(f16::to_f32), value.map(f16::to_f32)).sqrt(),
		))),
		Value::Vec3F16(value) => Ok(Value::F16(f16::from_f32(
			dot_product(value.map(f16::to_f32), value.map(f16::to_f32)).sqrt(),
		))),
		Value::Vec4F16(value) => Ok(Value::F16(f16::from_f32(
			dot_product(value.map(f16::to_f32), value.map(f16::to_f32)).sqrt(),
		))),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(crate) fn apply_normalize(value: &Value) -> Result<Value, VmError> {
	match value {
		Value::Vec2F(value) => normalize_vector(*value).map(Value::Vec2F),
		Value::Vec3F(value) => normalize_vector(*value).map(Value::Vec3F),
		Value::Vec4F(value) => normalize_vector(*value).map(Value::Vec4F),
		Value::Vec2F16(value) => normalize_vector(value.map(f16::to_f32)).map(|value| Value::Vec2F16(value.map(f16::from_f32))),
		Value::Vec3F16(value) => normalize_vector(value.map(f16::to_f32)).map(|value| Value::Vec3F16(value.map(f16::from_f32))),
		Value::Vec4F16(value) => normalize_vector(value.map(f16::to_f32)).map(|value| Value::Vec4F16(value.map(f16::from_f32))),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(crate) fn apply_reflect(incident: &Value, normal: &Value) -> Result<Value, VmError> {
	match (incident, normal) {
		(Value::Vec2F(incident), Value::Vec2F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec2F),
		(Value::Vec3F(incident), Value::Vec3F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec3F),
		(Value::Vec4F(incident), Value::Vec4F(normal)) => reflect_vector(*incident, *normal).map(Value::Vec4F),
		(incident, normal) => Err(type_mismatch(&incident.value_type(), &normal.value_type())),
	}
}

pub(crate) fn apply_scalar_unary(operator: ScalarUnaryOperator, value: &Value) -> Result<Value, VmError> {
	use ScalarUnaryOperator as Unary;
	// A conversion or `find_lsb` reports the scalar type its operator reads when given any other value.
	let mismatch = |expected: ValueType| Err(type_mismatch(&expected, &value.value_type()));
	match (operator, value) {
		(Unary::FromF16ToF32, Value::F16(value)) => Ok(Value::F32(value.to_f32())),
		(Unary::FromU32ToF32, Value::U32(value)) => Ok(Value::F32(*value as f32)),
		(Unary::FromI32ToF32, Value::I32(value)) => Ok(Value::F32(*value as f32)),
		(Unary::FromF32ToF16, Value::F32(value)) => Ok(Value::F16(f16::from_f32(*value))),
		(Unary::FromU32ToF16, Value::U32(value)) => Ok(Value::F16(f16::from_f32(*value as f32))),
		(Unary::FromI32ToF16, Value::I32(value)) => Ok(Value::F16(f16::from_f32(*value as f32))),
		(Unary::FromF32ToU32, Value::F32(value)) => Ok(Value::U32(*value as u32)),
		(Unary::FromF16ToU32, Value::F16(value)) => Ok(Value::U32(value.to_f32() as u32)),
		(Unary::FromU8ToU32, Value::U8(value)) => Ok(Value::U32(u32::from(*value))),
		(Unary::FromU16ToU32, Value::U16(value)) => Ok(Value::U32(u32::from(*value))),
		(Unary::FromU32ToU16, Value::U32(value)) => Ok(Value::U16(*value as u16)),
		(Unary::FindLsb, Value::U32(value)) => Ok(Value::U32(if *value == 0 { u32::MAX } else { value.trailing_zeros() })),
		// Signed-to-unsigned shader casts preserve the low 32 bits for negative inputs.
		(Unary::FromI32ToU32, Value::I32(value)) => Ok(Value::U32(*value as u32)),
		(Unary::FromF16ToF32 | Unary::FromF16ToU32, _) => mismatch(ValueType::F16),
		(Unary::FromF32ToF16 | Unary::FromF32ToU32, _) => mismatch(ValueType::F32),
		(Unary::FromU8ToU32, _) => mismatch(ValueType::U8),
		(Unary::FromU16ToU32, _) => mismatch(ValueType::U16),
		(Unary::FromU32ToF32 | Unary::FromU32ToF16 | Unary::FromU32ToU16 | Unary::FindLsb, _) => mismatch(ValueType::U32),
		(Unary::FromI32ToF32 | Unary::FromI32ToF16 | Unary::FromI32ToU32, _) => mismatch(ValueType::I32),
		_ => map_float_value(value, |value| match operator {
			Unary::Abs => value.abs(),
			Unary::Sqrt => value.sqrt(),
			Unary::Exp => value.exp(),
			Unary::Sin => value.sin(),
			Unary::Cos => value.cos(),
			Unary::Tan => value.tan(),
			Unary::Asin => value.asin(),
			Unary::Floor => value.floor(),
			Unary::Round => value.round(),
			Unary::Fract => value - value.floor(),
			Unary::Radians => value.to_radians(),
			Unary::InverseSqrt => 1.0 / value.sqrt(),
			Unary::Log2 => value.log2(),
			Unary::Fwidth => 0.0,
			_ => unreachable!("Conversions and find_lsb are matched above"),
		}),
	}
}

pub(crate) fn map_float_value(value: &Value, map: impl Fn(f32) -> f32) -> Result<Value, VmError> {
	match value {
		Value::F16(value) => Ok(Value::F16(f16::from_f32(map(value.to_f32())))),
		Value::F32(value) => Ok(Value::F32(map(*value))),
		Value::Vec2F16(value) => Ok(Value::Vec2F16(value.map(|value| f16::from_f32(map(value.to_f32()))))),
		Value::Vec3F16(value) => Ok(Value::Vec3F16(value.map(|value| f16::from_f32(map(value.to_f32()))))),
		Value::Vec4F16(value) => Ok(Value::Vec4F16(value.map(|value| f16::from_f32(map(value.to_f32()))))),
		Value::Vec2F(value) => Ok(Value::Vec2F(value.map(&map))),
		Value::Vec3F(value) => Ok(Value::Vec3F(value.map(&map))),
		Value::Vec4F(value) => Ok(Value::Vec4F(value.map(&map))),
		value => Err(VmError::TypeMismatch {
			expected: "f16, f32, or float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(crate) fn apply_scalar_binary(operator: ScalarBinaryOperator, left: &Value, right: &Value) -> Result<Value, VmError> {
	fn apply(operator: ScalarBinaryOperator, left: f32, right: f32) -> f32 {
		match operator {
			ScalarBinaryOperator::Min => left.min(right),
			ScalarBinaryOperator::Max => left.max(right),
			ScalarBinaryOperator::Pow => left.powf(right),
			ScalarBinaryOperator::Step => f32::from(right >= left),
			ScalarBinaryOperator::Atan2 => left.atan2(right),
		}
	}
	// Integers only support the ordering operators. The lexer's overload table admits no other integer form.
	match (operator, left, right) {
		(ScalarBinaryOperator::Min, Value::I32(left), Value::I32(right)) => return Ok(Value::I32(*left.min(right))),
		(ScalarBinaryOperator::Max, Value::I32(left), Value::I32(right)) => return Ok(Value::I32(*left.max(right))),
		(ScalarBinaryOperator::Min, Value::U32(left), Value::U32(right)) => return Ok(Value::U32(*left.min(right))),
		(ScalarBinaryOperator::Max, Value::U32(left), Value::U32(right)) => return Ok(Value::U32(*left.max(right))),
		_ => {}
	}
	// Half-precision lanes compute in f32 and round the result back to f16.
	let half = |left: f16, right: f16| f16::from_f32(apply(operator, left.to_f32(), right.to_f32()));
	match (left, right) {
		(Value::F16(left), Value::F16(right)) => Ok(Value::F16(half(*left, *right))),
		(Value::F32(left), Value::F32(right)) => Ok(Value::F32(apply(operator, *left, *right))),
		(Value::Vec2F16(left), Value::Vec2F16(right)) => Ok(Value::Vec2F16(std::array::from_fn(|i| half(left[i], right[i])))),
		(Value::Vec3F16(left), Value::Vec3F16(right)) => Ok(Value::Vec3F16(std::array::from_fn(|i| half(left[i], right[i])))),
		(Value::Vec4F16(left), Value::Vec4F16(right)) => Ok(Value::Vec4F16(std::array::from_fn(|i| half(left[i], right[i])))),
		(Value::Vec2F(left), Value::Vec2F(right)) => {
			Ok(Value::Vec2F(std::array::from_fn(|i| apply(operator, left[i], right[i]))))
		}
		(Value::Vec3F(left), Value::Vec3F(right)) => {
			Ok(Value::Vec3F(std::array::from_fn(|i| apply(operator, left[i], right[i]))))
		}
		(Value::Vec4F(left), Value::Vec4F(right)) => {
			Ok(Value::Vec4F(std::array::from_fn(|i| apply(operator, left[i], right[i]))))
		}
		(left, right) => Err(type_mismatch(&left.value_type(), &right.value_type())),
	}
}

pub(crate) fn apply_scalar_ternary(
	operator: ScalarTernaryOperator,
	first: &Value,
	second: &Value,
	third: &Value,
) -> Result<Value, VmError> {
	/// Rounds an unsigned significand after discarding `shift` low bits.
	fn round_shift_right_to_even(value: u64, shift: u32) -> u64 {
		if shift == 0 {
			return value;
		}
		if shift >= u64::BITS {
			return 0;
		}
		let truncated = value >> shift;
		let remainder_mask = (1_u64 << shift) - 1;
		let remainder = value & remainder_mask;
		let halfway = 1_u64 << (shift - 1);
		truncated + u64::from(remainder > halfway || (remainder == halfway && truncated & 1 != 0))
	}

	/// Converts binary64 directly to binary16 with round-to-nearest, ties-to-even.
	fn f16_from_f64_round_to_even(value: f64) -> f16 {
		let bits = value.to_bits();
		let sign = ((bits >> 48) & 0x8000) as u16;
		let exponent = ((bits >> 52) & 0x7ff) as i32;
		let fraction = bits & 0x000f_ffff_ffff_ffff;

		if exponent == 0x7ff {
			if fraction == 0 {
				return f16::from_bits(sign | 0x7c00);
			}
			let payload = ((fraction >> 42) as u16 | 0x0200) & 0x03ff;
			return f16::from_bits(sign | 0x7c00 | payload);
		}
		if exponent == 0 {
			return f16::from_bits(sign);
		}

		let unbiased_exponent = exponent - 1023;
		let significand = (1_u64 << 52) | fraction;
		if unbiased_exponent >= -14 {
			if unbiased_exponent > 15 {
				return f16::from_bits(sign | 0x7c00);
			}
			let mut half_exponent = unbiased_exponent + 15;
			let mut rounded_significand = round_shift_right_to_even(significand, 42);
			if rounded_significand == 0x0800 {
				rounded_significand = 0x0400;
				half_exponent += 1;
			}
			if half_exponent >= 31 {
				return f16::from_bits(sign | 0x7c00);
			}
			return f16::from_bits(sign | ((half_exponent as u16) << 10) | (rounded_significand as u16 & 0x03ff));
		}

		// One binary16 subnormal step is 2^-24. Shift the exact binary64
		// significand into that scale before applying the same rounding rule.
		let shift = (28 - unbiased_exponent) as u32;
		let rounded_fraction = round_shift_right_to_even(significand, shift);
		if rounded_fraction >= 0x0400 {
			return f16::from_bits(sign | 0x0400);
		}
		f16::from_bits(sign | rounded_fraction as u16)
	}

	fn apply(operator: ScalarTernaryOperator, first: f32, second: f32, third: f32) -> f32 {
		match operator {
			ScalarTernaryOperator::Mix => first + (second - first) * third,
			ScalarTernaryOperator::Clamp => first.clamp(second, third),
			ScalarTernaryOperator::Fma => first.mul_add(second, third),
			ScalarTernaryOperator::Smoothstep => {
				let t = ((third - first) / (second - first)).clamp(0.0, 1.0);
				t * t * (3.0 - 2.0 * t)
			}
		}
	}
	// Binary64 retains enough guard precision to decide the final binary16
	// rounding. Using the f32 path can land on a binary32 midpoint first.
	fn apply_f16(operator: ScalarTernaryOperator, first: f16, second: f16, third: f16) -> f16 {
		if operator == ScalarTernaryOperator::Fma {
			f16_from_f64_round_to_even(first.to_f64().mul_add(second.to_f64(), third.to_f64()))
		} else {
			f16::from_f32(apply(operator, first.to_f32(), second.to_f32(), third.to_f32()))
		}
	}
	// Integers only support `clamp`. The lexer's overload table admits no other integer form.
	match (operator, first, second, third) {
		(ScalarTernaryOperator::Clamp, Value::I32(value), Value::I32(minimum), Value::I32(maximum)) => {
			return Ok(Value::I32(*value.clamp(minimum, maximum)));
		}
		(ScalarTernaryOperator::Clamp, Value::U32(value), Value::U32(minimum), Value::U32(maximum)) => {
			return Ok(Value::U32(*value.clamp(minimum, maximum)));
		}
		_ => {}
	}
	match (first, second, third) {
		(Value::F16(first), Value::F16(second), Value::F16(third)) => {
			Ok(Value::F16(apply_f16(operator, *first, *second, *third)))
		}
		(Value::F32(first), Value::F32(second), Value::F32(third)) => Ok(Value::F32(apply(operator, *first, *second, *third))),
		(Value::Vec2F16(first), Value::Vec2F16(second), Value::Vec2F16(third)) => {
			Ok(Value::Vec2F16(std::array::from_fn(|i| {
				apply_f16(operator, first[i], second[i], third[i])
			})))
		}
		(Value::Vec3F16(first), Value::Vec3F16(second), Value::Vec3F16(third)) => {
			Ok(Value::Vec3F16(std::array::from_fn(|i| {
				apply_f16(operator, first[i], second[i], third[i])
			})))
		}
		(Value::Vec4F16(first), Value::Vec4F16(second), Value::Vec4F16(third)) => {
			Ok(Value::Vec4F16(std::array::from_fn(|i| {
				apply_f16(operator, first[i], second[i], third[i])
			})))
		}
		(Value::Vec2F(first), Value::Vec2F(second), Value::Vec2F(third)) => Ok(Value::Vec2F(std::array::from_fn(|i| {
			apply(operator, first[i], second[i], third[i])
		}))),
		(Value::Vec3F(first), Value::Vec3F(second), Value::Vec3F(third)) => Ok(Value::Vec3F(std::array::from_fn(|i| {
			apply(operator, first[i], second[i], third[i])
		}))),
		(Value::Vec4F(first), Value::Vec4F(second), Value::Vec4F(third)) => Ok(Value::Vec4F(std::array::from_fn(|i| {
			apply(operator, first[i], second[i], third[i])
		}))),
		_ => Err(VmError::TypeMismatch {
			expected: first.value_type().name().to_string(),
			found: format!("{}, {}", second.value_type().name(), third.value_type().name()),
		}),
	}
}

pub(crate) fn extract_value(value: &Value, index: usize, expected_type: &ValueType) -> Result<Value, VmError> {
	let extracted = match value {
		Value::Vec2U16(value) => value.get(index).copied().map(Value::U16),
		Value::Vec4U16(value) => value.get(index).copied().map(Value::U16),
		Value::Vec2I(value) => value.get(index).copied().map(Value::I32),
		Value::Vec2U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec3U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec4U(value) => value.get(index).copied().map(Value::U32),
		Value::Vec2F16(value) => value.get(index).copied().map(Value::F16),
		Value::Vec3F16(value) => value.get(index).copied().map(Value::F16),
		Value::Vec4F16(value) => value.get(index).copied().map(Value::F16),
		Value::Vec2F(value) => value.get(index).copied().map(Value::F32),
		Value::Vec3F(value) => value.get(index).copied().map(Value::F32),
		Value::Vec4F(value) => value.get(index).copied().map(Value::F32),
		Value::PackedVec4F(value) => value.get(index).copied().map(Value::F32),
		Value::Mat4F(value) => value.as_chunks::<4>().0.get(index).copied().map(Value::Vec4F),
		Value::Mat4x3F(value) => value.as_chunks::<3>().0.get(index).copied().map(Value::Vec3F),
		Value::Struct { fields, .. } => fields.get(index).cloned(),
		_ => None,
	}
	.ok_or_else(|| VmError::UnsupportedExpression {
		message: format!("Member index {} is invalid for `{}`", index, value.value_type().name()),
	})?;
	if !extracted.matches_type(expected_type) {
		return Err(type_mismatch(expected_type, &extracted.value_type()));
	}
	Ok(extracted)
}

/// Replaces the member at `index` of `aggregate`, the inverse of [`extract_value`].
pub(crate) fn insert_value(aggregate: &mut Value, index: usize, member: Value) -> Result<(), VmError> {
	/// Writes one element and reports whether `index` was in bounds.
	fn set<T>(slots: &mut [T], index: usize, value: T) -> bool {
		slots.get_mut(index).map(|slot| *slot = value).is_some()
	}

	let member_type = member.value_type();
	let inserted = match (&mut *aggregate, member) {
		(Value::Vec2U16(slots), Value::U16(value)) => set(slots, index, value),
		(Value::Vec4U16(slots), Value::U16(value)) => set(slots, index, value),
		(Value::Vec2I(slots), Value::I32(value)) => set(slots, index, value),
		(Value::Vec2U(slots), Value::U32(value)) => set(slots, index, value),
		(Value::Vec3U(slots), Value::U32(value)) => set(slots, index, value),
		(Value::Vec4U(slots), Value::U32(value)) => set(slots, index, value),
		(Value::Vec2F16(slots), Value::F16(value)) => set(slots, index, value),
		(Value::Vec3F16(slots), Value::F16(value)) => set(slots, index, value),
		(Value::Vec4F16(slots), Value::F16(value)) => set(slots, index, value),
		(Value::Vec2F(slots), Value::F32(value)) => set(slots, index, value),
		(Value::Vec3F(slots), Value::F32(value)) => set(slots, index, value),
		(Value::Vec4F(slots) | Value::PackedVec4F(slots), Value::F32(value)) => set(slots, index, value),
		(Value::Mat4F(slots), Value::Vec4F(column)) => set(slots.as_chunks_mut::<4>().0, index, column),
		(Value::Mat4x3F(slots), Value::Vec3F(column)) => set(slots.as_chunks_mut::<3>().0, index, column),
		(Value::Struct { fields, .. }, value) => match fields.get_mut(index) {
			Some(field) if value.matches_type(&field.value_type()) => {
				*field = value;
				true
			}
			_ => false,
		},
		_ => false,
	};

	if inserted {
		Ok(())
	} else {
		Err(VmError::TypeMismatch {
			expected: format!("member {} of `{}`", index, aggregate.value_type().name()),
			found: member_type.name().to_string(),
		})
	}
}

pub(crate) fn vector_scalar_type(value_type: &ValueType) -> Option<ValueType> {
	match value_type {
		ValueType::Vec2U16 | ValueType::Vec4U16 => Some(ValueType::U16),
		ValueType::Vec2I => Some(ValueType::I32),
		ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => Some(ValueType::U32),
		ValueType::Vec2F16 | ValueType::Vec3F16 | ValueType::Vec4F16 => Some(ValueType::F16),
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F | ValueType::PackedVec4F => Some(ValueType::F32),
		_ => None,
	}
}

pub(crate) fn multiply_mat4_vec4(matrix: [f32; 16], vector: [f32; 4]) -> [f32; 4] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 4 + row] * vector[column]).sum())
}

pub(crate) fn multiply_mat4(left: [f32; 16], right: [f32; 16]) -> [f32; 16] {
	let mut value = [0.0; 16];
	for (product, column) in value.as_chunks_mut::<4>().0.iter_mut().zip(right.as_chunks::<4>().0) {
		*product = multiply_mat4_vec4(left, *column);
	}
	value
}

pub(crate) fn multiply_mat4x3_vec4(matrix: [f32; 12], vector: [f32; 4]) -> [f32; 3] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 3 + row] * vector[column]).sum())
}

pub(crate) fn expect_vec2u(value: &Value) -> Result<[u32; 2], VmError> {
	let &Value::Vec2U(value) = value else {
		return Err(type_mismatch(&ValueType::Vec2U, &value.value_type()));
	};
	Ok(value)
}

pub(crate) fn expect_vec4u(value: &Value) -> Result<[u32; 4], VmError> {
	let &Value::Vec4U(value) = value else {
		return Err(type_mismatch(&ValueType::Vec4U, &value.value_type()));
	};
	Ok(value)
}

pub(crate) fn expect_bool(value: &Value) -> Result<bool, VmError> {
	let &Value::Bool(value) = value else {
		return Err(type_mismatch(&ValueType::Bool, &value.value_type()));
	};
	Ok(value)
}

pub(crate) fn expect_u32(value: &Value) -> Result<u32, VmError> {
	let &Value::U32(value) = value else {
		return Err(type_mismatch(&ValueType::U32, &value.value_type()));
	};
	Ok(value)
}

pub(crate) fn dot_product<const N: usize>(left: [f32; N], right: [f32; N]) -> f32 {
	let mut value = 0.0;
	for index in 0..N {
		value += left[index] * right[index];
	}
	value
}

pub(crate) fn cross_product(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

pub(crate) fn normalize_vector<const N: usize>(value: [f32; N]) -> Result<[f32; N], VmError> {
	let length = dot_product(value, value).sqrt();
	if length == 0.0 {
		return Err(VmError::ArithmeticError {
			message: "Cannot normalize a zero-length vector".to_string(),
		});
	}
	Ok(value.map(|component| component / length))
}

pub(crate) fn reflect_vector<const N: usize>(incident: [f32; N], normal: [f32; N]) -> Result<[f32; N], VmError> {
	let scale = 2.0 * dot_product(incident, normal);
	Ok(std::array::from_fn(|index| incident[index] - scale * normal[index]))
}
