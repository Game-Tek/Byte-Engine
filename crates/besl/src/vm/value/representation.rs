use super::*;

/// The `Value` enum stores the VM values that can move between registers, locals, and buffers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
	Bool(bool),
	U8(u8),
	U16(u16),
	U32(u32),
	I32(i32),
	F16(f16),
	F32(f32),
	Vec2U16([u16; 2]),
	Vec4U16([u16; 4]),
	Vec2I([i32; 2]),
	Vec2U([u32; 2]),
	Vec3U([u32; 3]),
	Vec4U([u32; 4]),
	Vec2F16([f16; 2]),
	Vec3F16([f16; 3]),
	Vec4F16([f16; 4]),
	Vec2F([f32; 2]),
	Vec3F([f32; 3]),
	Vec4F([f32; 4]),
	PackedVec4F([f32; 4]),
	Mat4F([f32; 16]),
	Mat4x3F([f32; 12]),
	Resource { slot: ResourceSlot, value_type: ValueType },
	Struct { value_type: ValueType, fields: Vec<Value> },
}

impl Value {
	pub(crate) fn value_type(&self) -> ValueType {
		match self {
			Value::Bool(_) => ValueType::Bool,
			Value::U8(_) => ValueType::U8,
			Value::U16(_) => ValueType::U16,
			Value::U32(_) => ValueType::U32,
			Value::I32(_) => ValueType::I32,
			Value::F16(_) => ValueType::F16,
			Value::F32(_) => ValueType::F32,
			Value::Vec2U16(_) => ValueType::Vec2U16,
			Value::Vec4U16(_) => ValueType::Vec4U16,
			Value::Vec2I(_) => ValueType::Vec2I,
			Value::Vec2U(_) => ValueType::Vec2U,
			Value::Vec3U(_) => ValueType::Vec3U,
			Value::Vec4U(_) => ValueType::Vec4U,
			Value::Vec2F16(_) => ValueType::Vec2F16,
			Value::Vec3F16(_) => ValueType::Vec3F16,
			Value::Vec4F16(_) => ValueType::Vec4F16,
			Value::Vec2F(_) => ValueType::Vec2F,
			Value::Vec3F(_) => ValueType::Vec3F,
			Value::Vec4F(_) => ValueType::Vec4F,
			Value::PackedVec4F(_) => ValueType::PackedVec4F,
			Value::Mat4F(_) => ValueType::Mat4F,
			Value::Mat4x3F(_) => ValueType::Mat4x3F,
			Value::Resource { value_type, .. } => value_type.clone(),
			Value::Struct { value_type, .. } => value_type.clone(),
		}
	}

	pub(crate) fn matches_type(&self, expected: &ValueType) -> bool {
		match (self, expected) {
			(
				Value::Struct { value_type, fields },
				ValueType::Struct {
					fields: expected_fields, ..
				},
			) => {
				value_type == expected
					&& fields.len() == expected_fields.len()
					&& fields
						.iter()
						.zip(expected_fields)
						.all(|(field, expected_field)| field.matches_type(expected_field.value_type()))
			}
			_ => self.value_type() == *expected,
		}
	}
}
