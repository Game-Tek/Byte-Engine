use super::*;

/// The `Value` enum stores the VM values that can move between registers, locals, and buffers.
#[derive(Debug, PartialEq)]
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
	Mat4F([f32; 16]),
	Mat4x3F([f32; 12]),
	Resource {
		slot: ResourceSlot,
		value_type: ValueType,
	},
	/// A struct or array value. Each copy owns its fields. The VM copies one into a register or local that already
	/// holds a struct with [`Clone::clone_from`], which reuses that struct's buffers, so a frame that runs again copies
	/// structs without allocating.
	Struct {
		value_type: ValueType,
		fields: Vec<Value>,
	},
}

impl Clone for Value {
	fn clone(&self) -> Self {
		match self {
			Value::Bool(value) => Value::Bool(*value),
			Value::U8(value) => Value::U8(*value),
			Value::U16(value) => Value::U16(*value),
			Value::U32(value) => Value::U32(*value),
			Value::I32(value) => Value::I32(*value),
			Value::F16(value) => Value::F16(*value),
			Value::F32(value) => Value::F32(*value),
			Value::Vec2U16(value) => Value::Vec2U16(*value),
			Value::Vec4U16(value) => Value::Vec4U16(*value),
			Value::Vec2I(value) => Value::Vec2I(*value),
			Value::Vec2U(value) => Value::Vec2U(*value),
			Value::Vec3U(value) => Value::Vec3U(*value),
			Value::Vec4U(value) => Value::Vec4U(*value),
			Value::Vec2F16(value) => Value::Vec2F16(*value),
			Value::Vec3F16(value) => Value::Vec3F16(*value),
			Value::Vec4F16(value) => Value::Vec4F16(*value),
			Value::Vec2F(value) => Value::Vec2F(*value),
			Value::Vec3F(value) => Value::Vec3F(*value),
			Value::Vec4F(value) => Value::Vec4F(*value),
			Value::Mat4F(value) => Value::Mat4F(*value),
			Value::Mat4x3F(value) => Value::Mat4x3F(*value),
			Value::Resource { slot, value_type } => Value::Resource {
				slot: *slot,
				value_type: value_type.clone(),
			},
			Value::Struct { value_type, fields } => Value::Struct {
				value_type: value_type.clone(),
				fields: fields.clone(),
			},
		}
	}

	/// Copies `source` into this value, reusing this value's buffers when both are structs: the type is copied only
	/// when it differs, and `Vec::clone_from` copies each field into the field already there, recursively.
	fn clone_from(&mut self, source: &Self) {
		match (&mut *self, source) {
			(
				Value::Struct { value_type, fields },
				Value::Struct {
					value_type: source_type,
					fields: source_fields,
				},
			) => {
				if value_type != source_type {
					value_type.clone_from(source_type);
				}
				fields.clone_from(source_fields);
			}
			(value, source) => *value = source.clone(),
		}
	}
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

#[cfg(test)]
mod tests {
	use super::*;

	/// Returns a struct of type `name` whose fields are `fields` and an inner struct holding `inner`.
	fn nested(name: &str, fields: [f32; 2], inner: f32) -> Value {
		let inner_type = ValueType::Struct {
			name: "Inner".to_string(),
			fields: Vec::new(),
			size: 4,
			alignment: 4,
		};
		Value::Struct {
			value_type: ValueType::Struct {
				name: name.to_string(),
				fields: Vec::new(),
				size: 12,
				alignment: 4,
			},
			fields: vec![
				Value::F32(fields[0]),
				Value::F32(fields[1]),
				Value::Struct {
					value_type: inner_type,
					fields: vec![Value::F32(inner)],
				},
			],
		}
	}

	/// Returns the address of the fields of `value`, a struct, and of its third field, an inner struct.
	fn buffers(value: &Value) -> [*const Value; 2] {
		let Value::Struct { fields, .. } = value else {
			panic!("the value is a struct");
		};
		let Value::Struct { fields: inner, .. } = &fields[2] else {
			panic!("the third field is a struct");
		};
		[fields.as_ptr(), inner.as_ptr()]
	}

	#[test]
	fn copying_a_struct_into_a_struct_reuses_its_buffers() {
		let mut held = nested("Pair", [1.0, 2.0], 3.0);
		let before = buffers(&held);

		let source = nested("Pair", [4.0, 5.0], 6.0);
		held.clone_from(&source);

		assert_eq!(held, source);
		assert_eq!(buffers(&held), before, "the copy should land in the buffers already there");

		// A struct of another type takes the source's type too.
		let other = nested("Other", [7.0, 8.0], 9.0);
		held.clone_from(&other);
		assert_eq!(held, other);
	}
}
