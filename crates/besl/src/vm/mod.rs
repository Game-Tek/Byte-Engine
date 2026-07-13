//! The `vm` module compiles and executes lexed BESL programs for deterministic host-side evaluation.

use std::collections::HashMap;

use crate::lexer::{BindingTypes, Expressions, NodeReference, Nodes, Operators};

mod compiler;
mod error;
mod execution;
mod instruction;
mod value;

pub use error::VmError;
use instruction::*;
use value::*;

/// The `DescriptorSlot` struct provides a stable lookup key for host resources and VM interface resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DescriptorSlot {
	set: u32,
	binding: u32,
	// The kind keeps internal VM namespaces distinct from host descriptors that use the same numeric coordinates.
	kind: DescriptorSlotKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DescriptorSlotKind {
	Descriptor,
	PushConstant,
	DynamicResource,
	BuiltinPosition,
	Input,
	Output,
}

impl DescriptorSlot {
	pub const fn new(set: u32, binding: u32) -> Self {
		Self {
			set,
			binding,
			kind: DescriptorSlotKind::Descriptor,
		}
	}

	pub const fn set(&self) -> u32 {
		self.set
	}

	pub const fn binding(&self) -> u32 {
		self.binding
	}

	const fn virtual_slot(set: u32, binding: u32, kind: DescriptorSlotKind) -> Self {
		Self { set, binding, kind }
	}

	const fn is_dynamic_resource(&self) -> bool {
		matches!(self.kind, DescriptorSlotKind::DynamicResource)
	}
}

const PUSH_CONSTANT_SLOT: DescriptorSlot = DescriptorSlot::virtual_slot(u32::MAX, u32::MAX, DescriptorSlotKind::PushConstant);
const DYNAMIC_RESOURCE_SET: u32 = u32::MAX - 4;
const BUILTIN_POSITION_INTERFACE_SET: u32 = u32::MAX - 3;
const INPUT_INTERFACE_SET: u32 = u32::MAX - 2;
const OUTPUT_INTERFACE_SET: u32 = u32::MAX - 1;

pub const fn input_slot(location: u8) -> DescriptorSlot {
	DescriptorSlot::virtual_slot(INPUT_INTERFACE_SET, location as u32, DescriptorSlotKind::Input)
}

pub const fn output_slot(location: u8) -> DescriptorSlot {
	DescriptorSlot::virtual_slot(OUTPUT_INTERFACE_SET, location as u32, DescriptorSlotKind::Output)
}

/// Returns the interface slot reserved for the vertex position builtin.
pub const fn builtin_position_slot() -> DescriptorSlot {
	DescriptorSlot::virtual_slot(BUILTIN_POSITION_INTERFACE_SET, 0, DescriptorSlotKind::BuiltinPosition)
}

fn dynamic_resource_slot(register: usize) -> DescriptorSlot {
	DescriptorSlot::virtual_slot(
		DYNAMIC_RESOURCE_SET,
		u32::try_from(register).expect("VM register indices fit in a descriptor slot"),
		DescriptorSlotKind::DynamicResource,
	)
}

/// The `ValueType` enum describes portable BESL values and resource handles used by VM layouts and registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
	Bool,
	U8,
	U16,
	U32,
	I32,
	F32,
	Vec2U16,
	Vec2I,
	Vec3U,
	Vec2U,
	Vec4U,
	Vec2F,
	Vec3F,
	Vec4F,
	Mat4F,
	Mat4x3F,
	Texture2D,
	Texture3D,
	ArrayTexture2D,
	Struct {
		name: String,
		fields: Vec<BufferMemberLayout>,
		size: usize,
	},
}

impl ValueType {
	pub const fn size(&self) -> usize {
		match self {
			ValueType::Bool => 1,
			ValueType::U8 => 1,
			ValueType::U16 => 2,
			ValueType::U32 | ValueType::I32 | ValueType::F32 => 4,
			ValueType::Vec2U16 => 4,
			ValueType::Vec2I => 8,
			ValueType::Vec2U | ValueType::Vec2F => 8,
			ValueType::Vec3U => 12,
			ValueType::Vec4U | ValueType::Vec4F => 16,
			ValueType::Vec3F => 12,
			ValueType::Mat4F => 64,
			ValueType::Mat4x3F => 48,
			ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D => 0,
			ValueType::Struct { size, .. } => *size,
		}
	}

	fn name(&self) -> &str {
		match self {
			ValueType::Bool => "bool",
			ValueType::U8 => "u8",
			ValueType::U16 => "u16",
			ValueType::U32 => "u32",
			ValueType::I32 => "i32",
			ValueType::F32 => "f32",
			ValueType::Vec2U16 => "vec2u16",
			ValueType::Vec2I => "vec2i",
			ValueType::Vec3U => "vec3u",
			ValueType::Vec2U => "vec2u",
			ValueType::Vec4U => "vec4u",
			ValueType::Vec2F => "vec2f",
			ValueType::Vec3F => "vec3f",
			ValueType::Vec4F => "vec4f",
			ValueType::Mat4F => "mat4f",
			ValueType::Mat4x3F => "mat4x3f",
			ValueType::Texture2D => "Texture2D",
			ValueType::Texture3D => "Texture3D",
			ValueType::ArrayTexture2D => "ArrayTexture2D",
			ValueType::Struct { name, .. } => name,
		}
	}

	fn field(&self, name: &str) -> Option<&BufferMemberLayout> {
		match self {
			ValueType::Struct { fields, .. } => fields.iter().find(|field| field.name() == name),
			_ => None,
		}
	}
}

/// The `BufferMemberLayout` struct defines how host code addresses one named member in packed VM memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferMemberLayout {
	name: String,
	offset: usize,
	value_type: ValueType,
	count: usize,
}

impl BufferMemberLayout {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub const fn offset(&self) -> usize {
		self.offset
	}

	pub fn value_type(&self) -> &ValueType {
		&self.value_type
	}

	pub const fn count(&self) -> usize {
		self.count
	}

	fn element_offset(&self, index: usize) -> Result<usize, VmError> {
		if index >= self.count {
			return Err(VmError::BufferArrayIndexOutOfBounds {
				index,
				count: self.count,
			});
		}
		Ok(self.offset + self.value_type.size() * index)
	}
}

/// The `BufferLayout` struct provides the host-visible packed memory contract for one VM buffer binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferLayout {
	members: Vec<BufferMemberLayout>,
	size: usize,
}

impl BufferLayout {
	pub fn members(&self) -> &[BufferMemberLayout] {
		&self.members
	}

	pub const fn size(&self) -> usize {
		self.size
	}

	fn member(&self, name: &str) -> Option<&BufferMemberLayout> {
		self.members.iter().find(|member| member.name == name)
	}
}

/// The `DescriptorLayout` enum stores the VM resource layout required by one descriptor slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorLayout {
	Buffer(BufferLayout),
	Texture,
	Image,
	PushConstant(BufferLayout),
}

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

	fn read_value(&self, offset: usize, value_type: &ValueType) -> Result<Value, VmError> {
		let bytes = self.read_bytes(offset, value_type.size())?;

		let value = match value_type {
			ValueType::Bool => Value::Bool(bytes[0] != 0),
			ValueType::U8 => Value::U8(bytes[0]),
			ValueType::U16 => Value::U16(u16::from_ne_bytes(bytes.try_into().expect("Invalid u16 byte count"))),
			ValueType::U32 => Value::U32(u32::from_ne_bytes(bytes.try_into().expect("Invalid u32 byte count"))),
			ValueType::I32 => Value::I32(i32::from_ne_bytes(bytes.try_into().expect("Invalid i32 byte count"))),
			ValueType::F32 => Value::F32(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count"))),
			ValueType::Vec2U16 => Value::Vec2U16(read_u16_array::<2>(bytes)?),
			ValueType::Vec2I => Value::Vec2I(read_i32_array::<2>(bytes)?),
			ValueType::Vec2U => Value::Vec2U(read_u32_array::<2>(bytes)?),
			ValueType::Vec3U => Value::Vec3U(read_u32_array::<3>(bytes)?),
			ValueType::Vec4U => Value::Vec4U(read_u32_array::<4>(bytes)?),
			ValueType::Vec2F => Value::Vec2F(read_f32_array::<2>(bytes)?),
			ValueType::Vec3F => Value::Vec3F(read_f32_array::<3>(bytes)?),
			ValueType::Vec4F => Value::Vec4F(read_f32_array::<4>(bytes)?),
			ValueType::Mat4F => Value::Mat4F(read_f32_array::<16>(bytes)?),
			ValueType::Mat4x3F => Value::Mat4x3F(read_f32_array::<12>(bytes)?),
			ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D => {
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

	fn write_value(&mut self, offset: usize, value_type: &ValueType, value: &Value) -> Result<(), VmError> {
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
			Value::F32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			Value::Vec2U16(value) => write_u16_slice(self, offset, value),
			Value::Vec2I(value) => write_i32_slice(self, offset, value),
			Value::Vec2U(value) => write_u32_slice(self, offset, value),
			Value::Vec3U(value) => write_u32_slice(self, offset, value),
			Value::Vec4U(value) => write_u32_slice(self, offset, value),
			Value::Vec2F(value) => write_f32_slice(self, offset, value),
			Value::Vec3F(value) => write_f32_slice(self, offset, value),
			Value::Vec4F(value) => write_f32_slice(self, offset, value),
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

	fn write_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), VmError> {
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

/// The `Texture` struct provides deterministic CPU texels for shader sampling, image access, and atomic assertions.
#[derive(Debug)]
pub struct Texture {
	width: u32,
	height: u32,
	depth: u32,
	texels: Vec<Texel>,
}

#[derive(Clone, Copy, Debug)]
enum Texel {
	Zero,
	Float([f32; 4]),
	U32(u32),
}

impl Texel {
	const fn kind(self) -> &'static str {
		match self {
			Self::Zero => "untyped zero",
			Self::Float(_) => "float RGBA",
			Self::U32(_) => "u32",
		}
	}

	fn float(self) -> Result<[f32; 4], VmError> {
		match self {
			Self::Zero => Ok([0.0; 4]),
			Self::Float(value) => Ok(value),
			value => Err(VmError::TextureFormatMismatch {
				expected: "float RGBA",
				found: value.kind(),
			}),
		}
	}

	fn u32(self) -> Result<u32, VmError> {
		match self {
			Self::Zero => Ok(0),
			Self::U32(value) => Ok(value),
			value => Err(VmError::TextureFormatMismatch {
				expected: "u32",
				found: value.kind(),
			}),
		}
	}
}

impl Texture {
	pub fn new(width: u32, height: u32) -> Result<Self, VmError> {
		Self::new_3d(width, height, 1)
	}

	/// Creates a CPU texture with three-dimensional addressing for VM texture tests.
	pub fn new_3d(width: u32, height: u32, depth: u32) -> Result<Self, VmError> {
		if width == 0 || height == 0 || depth == 0 {
			return Err(VmError::InvalidTextureDimensions { width, height, depth });
		}

		let texel_count = (width as usize)
			.checked_mul(height as usize)
			.and_then(|area| area.checked_mul(depth as usize))
			.ok_or(VmError::TextureTexelCountOverflow { width, height, depth })?;
		texel_count
			.checked_mul(std::mem::size_of::<Texel>())
			.filter(|byte_count| *byte_count <= isize::MAX as usize)
			.ok_or(VmError::TextureTexelCountOverflow { width, height, depth })?;

		// Fallible reservation keeps hostile or accidental dimensions on the VM error path.
		let mut texels = Vec::new();
		texels
			.try_reserve_exact(texel_count)
			.map_err(|_| VmError::TextureTexelCountOverflow { width, height, depth })?;
		texels.resize(texel_count, Texel::Zero);
		Ok(Self {
			width,
			height,
			depth,
			texels,
		})
	}

	pub fn write(&mut self, coord: [u32; 2], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.texels[index] = Texel::Float(value);
		Ok(())
	}

	/// Writes one texel in a three-dimensional CPU texture.
	pub fn write_3d(&mut self, coord: [u32; 3], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index] = Texel::Float(value);
		Ok(())
	}

	/// Writes one unsigned integer texel for integer image and atomic tests.
	pub fn write_u32(&mut self, coord: [u32; 2], value: u32) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.texels[index] = Texel::U32(value);
		Ok(())
	}

	/// Fetches one texel without interpolation.
	pub fn fetch(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		Ok(Value::Vec4F(self.fetch_texel([coord[0], coord[1], 0])?))
	}

	/// Fetches one unsigned integer texel without interpolation.
	pub fn fetch_u32(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		Ok(Value::U32(self.texels[index].u32()?))
	}

	/// Samples one texel using bilinear interpolation in normalized UV space.
	pub fn sample(&self, uv: [f32; 2]) -> Result<Value, VmError> {
		let (x0, x1, tx) = normalized_linear_axis(uv[0], self.width);
		let (y0, y1, ty) = normalized_linear_axis(uv[1], self.height);

		let top = lerp_rgba(self.fetch_texel([x0, y0, 0])?, self.fetch_texel([x1, y0, 0])?, tx);
		let bottom = lerp_rgba(self.fetch_texel([x0, y1, 0])?, self.fetch_texel([x1, y1, 0])?, tx);

		Ok(Value::Vec4F(lerp_rgba(top, bottom, ty)))
	}

	/// Samples a three-dimensional texture using trilinear interpolation.
	pub fn sample_3d(&self, uvw: [f32; 3]) -> Result<Value, VmError> {
		let x = normalized_linear_axis(uvw[0], self.width);
		let y = normalized_linear_axis(uvw[1], self.height);
		let z = normalized_linear_axis(uvw[2], self.depth);
		let low = [x.0, y.0, z.0];
		let high = [x.1, y.1, z.1];
		let factor = [x.2, y.2, z.2];
		let low_plane = lerp_rgba(
			lerp_rgba(
				self.fetch_texel([low[0], low[1], low[2]])?,
				self.fetch_texel([high[0], low[1], low[2]])?,
				factor[0],
			),
			lerp_rgba(
				self.fetch_texel([low[0], high[1], low[2]])?,
				self.fetch_texel([high[0], high[1], low[2]])?,
				factor[0],
			),
			factor[1],
		);
		let high_plane = lerp_rgba(
			lerp_rgba(
				self.fetch_texel([low[0], low[1], high[2]])?,
				self.fetch_texel([high[0], low[1], high[2]])?,
				factor[0],
			),
			lerp_rgba(
				self.fetch_texel([low[0], high[1], high[2]])?,
				self.fetch_texel([high[0], high[1], high[2]])?,
				factor[0],
			),
			factor[1],
		);
		Ok(Value::Vec4F(lerp_rgba(low_plane, high_plane, factor[2])))
	}

	fn fetch_texel(&self, coord: [u32; 3]) -> Result<[f32; 4], VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index].float()
	}

	fn texel_index(&self, coord: [u32; 3]) -> Result<usize, VmError> {
		let [x, y, z] = coord;
		if x >= self.width || y >= self.height || z >= self.depth {
			return Err(VmError::TextureAccessOutOfBounds {
				x,
				y,
				z,
				width: self.width,
				height: self.height,
				depth: self.depth,
			});
		}

		Ok(((z as usize) * self.height as usize + y as usize) * self.width as usize + x as usize)
	}

	fn contains_2d(&self, coord: [u32; 2]) -> bool {
		coord[0] < self.width && coord[1] < self.height
	}

	fn atomic_or(&mut self, coord: [u32; 2], value: u32) -> Result<u32, VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		let previous = self.texels[index].u32()?;
		let updated = previous | value;
		self.texels[index] = Texel::U32(updated);
		Ok(previous)
	}
}

enum DescriptorBinding<'a> {
	Buffer(&'a mut Buffer),
	Texture(&'a mut Texture),
	Image(&'a mut Texture),
}

impl DescriptorBinding<'_> {
	const fn kind(&self) -> &'static str {
		match self {
			Self::Buffer(_) => "buffer",
			Self::Texture(_) => "texture",
			Self::Image(_) => "image",
		}
	}

	fn type_mismatch(&self, slot: DescriptorSlot, expected: &'static str) -> VmError {
		VmError::DescriptorTypeMismatch {
			slot,
			expected,
			found: self.kind(),
		}
	}
}

/// The `MeshOutputs` struct captures mesh-stage topology and positions for VM assertions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshOutputs {
	vertex_count: u32,
	primitive_count: u32,
	vertex_positions: Vec<[f32; 4]>,
	triangles: Vec<[u32; 3]>,
}

impl MeshOutputs {
	/// Creates an empty capture that can be bound before a mesh shader invocation.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the vertex count declared by the most recent mesh invocation.
	pub const fn vertex_count(&self) -> u32 {
		self.vertex_count
	}

	/// Returns the primitive count declared by the most recent mesh invocation.
	pub const fn primitive_count(&self) -> u32 {
		self.primitive_count
	}

	/// Returns one captured mesh vertex position when the shader wrote that declared slot.
	pub fn vertex_position(&self, index: usize) -> Option<[f32; 4]> {
		self.vertex_positions.get(index).copied()
	}

	/// Returns one captured mesh triangle when the shader wrote that declared slot.
	pub fn triangle(&self, index: usize) -> Option<[u32; 3]> {
		self.triangles.get(index).copied()
	}

	/// Prepares mesh output ranges after validating shader-controlled counts.
	fn set_counts(
		&mut self,
		vertex_count: u32,
		primitive_count: u32,
		max_vertex_count: u32,
		max_primitive_count: u32,
		clear: bool,
	) -> Result<(), VmError> {
		if vertex_count > max_vertex_count {
			return Err(VmError::MeshOutputCountLimitExceeded {
				kind: "vertex",
				requested: vertex_count,
				limit: max_vertex_count,
			});
		}
		if primitive_count > max_primitive_count {
			return Err(VmError::MeshOutputCountLimitExceeded {
				kind: "primitive",
				requested: primitive_count,
				limit: max_primitive_count,
			});
		}

		if clear {
			self.begin_invocation();
		}
		self.vertex_count = vertex_count;
		self.primitive_count = primitive_count;
		self.vertex_positions.resize(vertex_count as usize, [0.0; 4]);
		self.triangles.resize(primitive_count as usize, [0; 3]);
		Ok(())
	}

	fn begin_invocation(&mut self) {
		// The first lane clears the shared capture once; later workgroup lanes retain earlier lane writes.
		self.vertex_positions.fill([0.0; 4]);
		self.triangles.fill([0; 3]);
	}
}

/// The `DescriptorBindings` struct provides invocation-scoped host resources to a compiled BESL program.
pub struct DescriptorBindings<'a> {
	bindings: HashMap<DescriptorSlot, DescriptorBinding<'a>>,
	push_constant: Option<&'a mut Buffer>,
	mesh_outputs: Option<&'a mut MeshOutputs>,
}

impl<'a> Default for DescriptorBindings<'a> {
	fn default() -> Self {
		Self::new()
	}
}

impl<'a> DescriptorBindings<'a> {
	pub fn new() -> Self {
		Self {
			bindings: HashMap::new(),
			push_constant: None,
			mesh_outputs: None,
		}
	}

	pub fn bind_buffer(&mut self, slot: DescriptorSlot, buffer: &'a mut Buffer) {
		self.bindings.insert(slot, DescriptorBinding::Buffer(buffer));
	}

	pub fn bind_texture(&mut self, slot: DescriptorSlot, texture: &'a mut Texture) {
		self.bindings.insert(slot, DescriptorBinding::Texture(texture));
	}

	pub fn bind_image(&mut self, slot: DescriptorSlot, image: &'a mut Texture) {
		self.bindings.insert(slot, DescriptorBinding::Image(image));
	}

	pub fn bind_push_constant(&mut self, push_constant: &'a mut Buffer) {
		self.push_constant = Some(push_constant);
	}

	/// Binds the capture used by mesh output-count, position, and triangle intrinsics.
	pub fn bind_mesh_outputs(&mut self, mesh_outputs: &'a mut MeshOutputs) {
		self.mesh_outputs = Some(mesh_outputs);
	}

	fn buffer_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Buffer, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Buffer(buffer) => Ok(&mut **buffer),
			descriptor => Err(descriptor.type_mismatch(slot, "buffer")),
		}
	}

	fn texture_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Texture(texture) => Ok(&mut **texture),
			descriptor => Err(descriptor.type_mismatch(slot, "texture")),
		}
	}

	fn image_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Image(image) => Ok(&mut **image),
			descriptor => Err(descriptor.type_mismatch(slot, "image")),
		}
	}

	fn push_constant_mut(&mut self) -> Result<&mut Buffer, VmError> {
		self.push_constant.as_deref_mut().ok_or(VmError::MissingPushConstant)
	}

	fn mesh_outputs_mut(&mut self) -> Result<&mut MeshOutputs, VmError> {
		self.mesh_outputs.as_deref_mut().ok_or(VmError::MissingMeshOutputs)
	}
}

/// The `SpecializationValues` struct supplies host-selected values for BESL specialization declarations.
#[derive(Clone, Debug, Default)]
pub struct SpecializationValues {
	values: HashMap<String, Value>,
}

impl SpecializationValues {
	/// Creates an empty specialization map for programs that use only defaults or no specializations.
	pub fn new() -> Self {
		Self::default()
	}

	/// Supplies one named specialization value before compiling an executable program.
	pub fn set(&mut self, name: impl Into<String>, value: Value) -> Option<Value> {
		self.values.insert(name.into(), value)
	}

	/// Returns a previously supplied specialization value by declaration name.
	pub fn get(&self, name: &str) -> Option<&Value> {
		self.values.get(name)
	}
}

/// The `ExecutionConfig` struct bounds a VM invocation and supplies its shader-visible thread coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionConfig {
	instruction_limit: usize,
	call_depth_limit: usize,
	max_mesh_vertex_count: u32,
	max_mesh_primitive_count: u32,
	thread_id: [u32; 2],
	thread_idx: u32,
	threadgroup_position: u32,
}

impl Default for ExecutionConfig {
	fn default() -> Self {
		Self {
			instruction_limit: 1_000_000,
			call_depth_limit: 64,
			max_mesh_vertex_count: 256,
			max_mesh_primitive_count: 256,
			thread_id: [0, 0],
			thread_idx: 0,
			threadgroup_position: 0,
		}
	}
}

impl ExecutionConfig {
	/// Creates an invocation config with an explicit instruction budget and default coordinates.
	pub fn new(instruction_limit: usize) -> Self {
		Self {
			instruction_limit,
			..Self::default()
		}
	}

	/// Returns the maximum number of instructions shared by the invocation's call tree.
	pub const fn instruction_limit(&self) -> usize {
		self.instruction_limit
	}

	/// Returns the maximum nested BESL function-call depth.
	pub const fn call_depth_limit(&self) -> usize {
		self.call_depth_limit
	}

	/// Returns the maximum vertex count a mesh invocation may request.
	pub const fn max_mesh_vertex_count(&self) -> u32 {
		self.max_mesh_vertex_count
	}

	/// Returns the maximum primitive count a mesh invocation may request.
	pub const fn max_mesh_primitive_count(&self) -> u32 {
		self.max_mesh_primitive_count
	}

	/// Returns the two-dimensional compute invocation coordinate.
	pub const fn thread_id(&self) -> [u32; 2] {
		self.thread_id
	}

	/// Returns the mesh or workgroup-local invocation index.
	pub const fn thread_idx(&self) -> u32 {
		self.thread_idx
	}

	/// Returns the mesh workgroup position visible to the shader.
	pub const fn threadgroup_position(&self) -> u32 {
		self.threadgroup_position
	}

	/// Selects an explicit nested function-call limit for this invocation.
	pub fn with_call_depth_limit(mut self, limit: usize) -> Self {
		self.call_depth_limit = limit;
		self
	}

	/// Selects the maximum vertex count accepted from mesh output-count intrinsics.
	pub fn with_max_mesh_vertex_count(mut self, limit: u32) -> Self {
		self.max_mesh_vertex_count = limit;
		self
	}

	/// Selects the maximum primitive count accepted from mesh output-count intrinsics.
	pub fn with_max_mesh_primitive_count(mut self, limit: u32) -> Self {
		self.max_mesh_primitive_count = limit;
		self
	}

	/// Selects the two-dimensional compute invocation coordinate.
	pub fn with_thread_id(mut self, thread_id: [u32; 2]) -> Self {
		self.thread_id = thread_id;
		self
	}

	/// Selects the mesh or workgroup-local invocation index.
	pub fn with_thread_idx(mut self, thread_idx: u32) -> Self {
		self.thread_idx = thread_idx;
		self
	}

	/// Selects the mesh workgroup position visible to the shader.
	pub fn with_threadgroup_position(mut self, position: u32) -> Self {
		self.threadgroup_position = position;
		self
	}
}

/// The `ExecutionState` struct shares invocation limits and coordinates across nested VM calls.
struct ExecutionState<'a> {
	config: &'a ExecutionConfig,
	remaining_instructions: usize,
	call_depth: usize,
}

impl<'a> ExecutionState<'a> {
	fn new(config: &'a ExecutionConfig) -> Self {
		Self {
			config,
			remaining_instructions: config.instruction_limit(),
			call_depth: 0,
		}
	}

	fn consume_instruction(&mut self) -> Result<(), VmError> {
		if self.remaining_instructions == 0 {
			return Err(VmError::InstructionLimitExceeded {
				limit: self.config.instruction_limit(),
			});
		}
		self.remaining_instructions -= 1;
		Ok(())
	}

	fn enter_call(&mut self) -> Result<(), VmError> {
		if self.call_depth >= self.config.call_depth_limit() {
			return Err(VmError::CallDepthLimitExceeded {
				limit: self.config.call_depth_limit(),
			});
		}
		self.call_depth += 1;
		Ok(())
	}

	fn leave_call(&mut self) {
		self.call_depth -= 1;
	}
}

/// The `ExecutableProgram` struct provides a reusable host-side execution form for one lexed BESL program.
pub struct ExecutableProgram {
	descriptor_layouts: HashMap<DescriptorSlot, DescriptorLayout>,
	functions: Vec<ExecutableFunction>,
	main_function: usize,
}

/// The `ExecutableFunction` struct isolates one compiled BESL call target for bounded VM execution.
struct ExecutableFunction {
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	register_count: usize,
	parameter_count: usize,
	return_type: Option<ValueType>,
}

impl ExecutableProgram {
	/// Compiles a lexed BESL program into a runnable VM program.
	#[allow(clippy::mutable_key_type)]
	pub fn compile(program: NodeReference) -> Result<Self, VmError> {
		Self::compile_with_specializations(program, &SpecializationValues::new())
	}

	/// Compiles a lexed BESL program using host-provided specialization values.
	#[allow(clippy::mutable_key_type)]
	pub fn compile_with_specializations(
		program: NodeReference,
		specializations: &SpecializationValues,
	) -> Result<Self, VmError> {
		compiler::compile(program, specializations)
	}

	pub fn descriptor_layout(&self, slot: DescriptorSlot) -> Option<&DescriptorLayout> {
		self.descriptor_layouts.get(&slot)
	}

	pub fn buffer_layout(&self, slot: DescriptorSlot) -> Option<&BufferLayout> {
		match self.descriptor_layouts.get(&slot) {
			Some(DescriptorLayout::Buffer(layout)) => Some(layout),
			Some(DescriptorLayout::Texture) => None,
			Some(DescriptorLayout::Image) => None,
			Some(DescriptorLayout::PushConstant(_)) => None,
			None => None,
		}
	}

	pub fn push_constant_layout(&self) -> Option<&BufferLayout> {
		self.descriptor_layouts.values().find_map(|layout| match layout {
			DescriptorLayout::PushConstant(layout) => Some(layout),
			_ => None,
		})
	}

	pub fn input_layout(&self, location: u8) -> Option<&BufferLayout> {
		self.buffer_layout(input_slot(location))
	}

	pub fn output_layout(&self, location: u8) -> Option<&BufferLayout> {
		self.buffer_layout(output_slot(location))
	}

	pub fn builtin_position_layout(&self) -> Option<&BufferLayout> {
		self.buffer_layout(builtin_position_slot())
	}
}

/// The `Value` enum stores the VM values that can move between registers, locals, and buffers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
	Bool(bool),
	U8(u8),
	U16(u16),
	U32(u32),
	I32(i32),
	F32(f32),
	Vec2U16([u16; 2]),
	Vec2I([i32; 2]),
	Vec2U([u32; 2]),
	Vec3U([u32; 3]),
	Vec4U([u32; 4]),
	Vec2F([f32; 2]),
	Vec3F([f32; 3]),
	Vec4F([f32; 4]),
	Mat4F([f32; 16]),
	Mat4x3F([f32; 12]),
	Resource { slot: DescriptorSlot, value_type: ValueType },
	Struct { value_type: ValueType, fields: Vec<Value> },
}

impl Value {
	fn value_type(&self) -> ValueType {
		match self {
			Value::Bool(_) => ValueType::Bool,
			Value::U8(_) => ValueType::U8,
			Value::U16(_) => ValueType::U16,
			Value::U32(_) => ValueType::U32,
			Value::I32(_) => ValueType::I32,
			Value::F32(_) => ValueType::F32,
			Value::Vec2U16(_) => ValueType::Vec2U16,
			Value::Vec2I(_) => ValueType::Vec2I,
			Value::Vec2U(_) => ValueType::Vec2U,
			Value::Vec3U(_) => ValueType::Vec3U,
			Value::Vec4U(_) => ValueType::Vec4U,
			Value::Vec2F(_) => ValueType::Vec2F,
			Value::Vec3F(_) => ValueType::Vec3F,
			Value::Vec4F(_) => ValueType::Vec4F,
			Value::Mat4F(_) => ValueType::Mat4F,
			Value::Mat4x3F(_) => ValueType::Mat4x3F,
			Value::Resource { value_type, .. } => value_type.clone(),
			Value::Struct { value_type, .. } => value_type.clone(),
		}
	}

	fn matches_type(&self, expected: &ValueType) -> bool {
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
mod tests;
