//! CPU buffer storage that implements VM packed-memory layouts.

use super::*;

/// The `Buffer` struct provides mutable CPU storage for binding structured host data to a VM invocation.
#[derive(Debug)]
pub struct Buffer {
	layout: BufferLayout,
	data: Vec<u8>,
}

impl Buffer {
	pub fn new(layout: BufferLayout) -> Self {
		Self {
			data: vec![0; layout.size()],
			layout,
		}
	}

	pub fn layout(&self) -> &BufferLayout {
		&self.layout
	}

	pub fn bytes(&self) -> &[u8] {
		&self.data
	}

	/// Reads a VM value from the buffer layout by member name.
	pub fn read(&self, member_name: &str) -> Result<Value, VmError> {
		let member = self.member_layout(member_name)?;
		if member.count() != 1 {
			return Err(VmError::UnsupportedBufferLayout {
				message: format!("Array member `{}` requires an element index", member_name),
			});
		}

		self.read_value(member.offset, &member.value_type)
	}

	/// Reads one array element from a VM buffer member.
	pub fn read_indexed(&self, member_name: &str, index: usize) -> Result<Value, VmError> {
		let member = self.member_layout(member_name)?;
		let offset = member.element_offset(index)?;
		self.read_value(offset, member.value_type())
	}

	/// Reads one field from a struct-valued VM buffer member.
	pub fn read_field(&self, member_name: &str, field_name: &str) -> Result<Value, VmError> {
		let member = self.member_layout(member_name)?;
		if member.count() != 1 {
			return Err(VmError::UnsupportedBufferLayout {
				message: format!("Array member `{}` requires an element index", member_name),
			});
		}
		self.read_indexed_field(member_name, 0, field_name)
	}

	/// Reads one field from a struct array element in a VM buffer member.
	pub fn read_indexed_field(&self, member_name: &str, index: usize, field_name: &str) -> Result<Value, VmError> {
		let member = self.member_layout(member_name)?;
		let field = member
			.value_type()
			.field(field_name)
			.ok_or_else(|| VmError::UnknownBufferMember {
				member: format!("{}.{}", member_name, field_name),
			})?;
		let offset = member.element_offset(index)? + field.offset();
		self.read_value(offset, field.value_type())
	}

	/// Writes a VM value into the buffer layout by member name.
	pub fn write(&mut self, member_name: &str, value: Value) -> Result<(), VmError> {
		let (offset, value_type) = {
			let member = self.member_layout(member_name)?;
			if member.count() != 1 {
				return Err(VmError::UnsupportedBufferLayout {
					message: format!("Array member `{}` requires an element index", member_name),
				});
			}
			(member.offset, member.value_type.clone())
		};

		self.write_value(offset, &value_type, &value)
	}

	/// Writes one array element in a VM buffer member.
	pub fn write_indexed(&mut self, member_name: &str, index: usize, value: Value) -> Result<(), VmError> {
		let (offset, value_type) = {
			let member = self.member_layout(member_name)?;
			(member.element_offset(index)?, member.value_type().clone())
		};
		self.write_value(offset, &value_type, &value)
	}

	/// Writes one field in a struct-valued VM buffer member.
	pub fn write_field(&mut self, member_name: &str, field_name: &str, value: Value) -> Result<(), VmError> {
		let member = self.member_layout(member_name)?;
		if member.count() != 1 {
			return Err(VmError::UnsupportedBufferLayout {
				message: format!("Array member `{}` requires an element index", member_name),
			});
		}
		self.write_indexed_field(member_name, 0, field_name, value)
	}

	/// Writes one field in a struct array element in a VM buffer member.
	pub fn write_indexed_field(
		&mut self,
		member_name: &str,
		index: usize,
		field_name: &str,
		value: Value,
	) -> Result<(), VmError> {
		let (offset, value_type) = {
			let member = self.member_layout(member_name)?;
			let field = member
				.value_type()
				.field(field_name)
				.ok_or_else(|| VmError::UnknownBufferMember {
					member: format!("{}.{}", member_name, field_name),
				})?;
			(member.element_offset(index)? + field.offset(), field.value_type().clone())
		};
		self.write_value(offset, &value_type, &value)
	}

	/// Reads an `f32` member from the buffer layout by name.
	pub fn read_f32(&self, member_name: &str) -> Result<f32, VmError> {
		match self.read(member_name)? {
			Value::F32(value) => Ok(value),
			value => Err(VmError::TypeMismatch {
				expected: "f32".to_string(),
				found: value.value_type().name().to_string(),
			}),
		}
	}

	/// Reads an `f16` member from the buffer layout by name.
	pub fn read_f16(&self, member_name: &str) -> Result<f16, VmError> {
		match self.read(member_name)? {
			Value::F16(value) => Ok(value),
			value => Err(VmError::TypeMismatch {
				expected: "f16".to_string(),
				found: value.value_type().name().to_string(),
			}),
		}
	}

	pub(super) fn read_value(&self, offset: usize, value_type: &ValueType) -> Result<Value, VmError> {
		let bytes = self.read_bytes(offset, value_type.size())?;

		let value = match value_type {
			ValueType::Bool => Value::Bool(bytes[0] != 0),
			ValueType::U8 => Value::U8(bytes[0]),
			ValueType::U16 => Value::U16(u16::from_ne_bytes(bytes.try_into().expect("Invalid u16 byte count"))),
			ValueType::U32 => Value::U32(u32::from_ne_bytes(bytes.try_into().expect("Invalid u32 byte count"))),
			ValueType::I32 => Value::I32(i32::from_ne_bytes(bytes.try_into().expect("Invalid i32 byte count"))),
			ValueType::F16 => Value::F16(f16::from_bits(u16::from_ne_bytes(
				bytes.try_into().expect("Invalid f16 byte count"),
			))),
			ValueType::F32 => Value::F32(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count"))),
			ValueType::Vec2U16 => Value::Vec2U16(read_u16_array::<2>(bytes)?),
			ValueType::Vec4U16 => Value::Vec4U16(read_u16_array::<4>(bytes)?),
			ValueType::Vec2I => Value::Vec2I(read_i32_array::<2>(bytes)?),
			ValueType::Vec2U => Value::Vec2U(read_u32_array::<2>(bytes)?),
			ValueType::Vec3U => Value::Vec3U(read_u32_array::<3>(bytes)?),
			ValueType::Vec4U => Value::Vec4U(read_u32_array::<4>(bytes)?),
			ValueType::Vec2F16 => Value::Vec2F16(read_f16_array::<2>(bytes)?),
			ValueType::Vec3F16 => Value::Vec3F16(read_f16_array::<3>(bytes)?),
			ValueType::Vec4F16 => Value::Vec4F16(read_f16_array::<4>(bytes)?),
			ValueType::Vec2F => Value::Vec2F(read_f32_array::<2>(bytes)?),
			ValueType::Vec3F => Value::Vec3F(read_f32_array::<3>(bytes)?),
			ValueType::Vec4F => Value::Vec4F(read_f32_array::<4>(bytes)?),
			ValueType::PackedVec4F => Value::PackedVec4F(read_f32_array::<4>(bytes)?),
			ValueType::Mat4F => Value::Mat4F(read_f32_array::<16>(bytes)?),
			ValueType::Mat4x3F => Value::Mat4x3F(read_f32_array::<12>(bytes)?),
			ValueType::Texture2D
			| ValueType::Texture3D
			| ValueType::TextureCube
			| ValueType::TextureCubeArray
			| ValueType::ArrayTexture2D => {
				return Err(VmError::UnsupportedBufferLayout {
					message: "Resource handles cannot be stored in CPU buffer memory".to_string(),
				});
			}
			ValueType::Struct { fields, .. } => {
				let mut values = Vec::with_capacity(fields.len());
				for field in fields {
					values.push(self.read_value(offset + field.offset(), field.value_type())?);
				}
				Value::Struct {
					value_type: value_type.clone(),
					fields: values,
				}
			}
		};

		Ok(value)
	}

	pub(super) fn write_value(&mut self, offset: usize, value_type: &ValueType, value: &Value) -> Result<(), VmError> {
		if !value.matches_type(value_type) {
			return Err(VmError::TypeMismatch {
				expected: value_type.name().to_string(),
				found: value.value_type().name().to_string(),
			});
		}

		match value {
			Value::Bool(value) => self.write_bytes(offset, &[u8::from(*value)]),
			Value::U8(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::U16(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::U32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::I32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::F16(value) => self.write_bytes(offset, &value.to_bits().to_ne_bytes()),
			Value::F32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::Vec2U16(value) => write_u16_slice(self, offset, value),
			Value::Vec4U16(value) => write_u16_slice(self, offset, value),
			Value::Vec2I(value) => write_i32_slice(self, offset, value),
			Value::Vec2U(value) => write_u32_slice(self, offset, value),
			Value::Vec3U(value) => write_u32_slice(self, offset, value),
			Value::Vec4U(value) => write_u32_slice(self, offset, value),
			Value::Vec2F16(value) => write_f16_slice(self, offset, value),
			Value::Vec3F16(value) => write_f16_slice(self, offset, value),
			Value::Vec4F16(value) => write_f16_slice(self, offset, value),
			Value::Vec2F(value) => write_f32_slice(self, offset, value),
			Value::Vec3F(value) => write_f32_slice(self, offset, value),
			Value::Vec4F(value) => write_f32_slice(self, offset, value),
			Value::PackedVec4F(value) => write_f32_slice(self, offset, value),
			Value::Mat4F(value) => write_f32_slice(self, offset, value),
			Value::Mat4x3F(value) => write_f32_slice(self, offset, value),
			Value::Resource { .. } => Err(VmError::UnsupportedBufferLayout {
				message: "Resource handles cannot be written into CPU buffer memory".to_string(),
			}),
			Value::Struct { fields, .. } => {
				let ValueType::Struct {
					fields: field_layouts, ..
				} = value_type
				else {
					unreachable!("Struct values are validated before writing")
				};
				for (field, field_layout) in fields.iter().zip(field_layouts) {
					self.write_value(offset + field_layout.offset(), field_layout.value_type(), field)?;
				}
				Ok(())
			}
		}
	}

	fn read_bytes(&self, offset: usize, size: usize) -> Result<&[u8], VmError> {
		self.data.get(offset..offset + size).ok_or(VmError::BufferAccessOutOfBounds {
			offset,
			size,
			buffer_size: self.data.len(),
		})
	}

	pub(super) fn write_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), VmError> {
		let buffer_size = self.data.len();
		let slice = self
			.data
			.get_mut(offset..offset + bytes.len())
			.ok_or(VmError::BufferAccessOutOfBounds {
				offset,
				size: bytes.len(),
				buffer_size,
			})?;

		slice.copy_from_slice(bytes);

		Ok(())
	}

	fn member_layout(&self, member_name: &str) -> Result<&BufferMemberLayout, VmError> {
		self.layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.to_string(),
		})
	}
}
