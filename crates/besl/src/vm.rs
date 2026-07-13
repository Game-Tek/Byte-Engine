//! The `vm` module executes lexed BESL programs against CPU-side resources for testing and host-side evaluation.

use std::collections::{HashMap, HashSet};

use crate::lexer::{BindingTypes, Expressions, NodeReference, Nodes, Operators};

/// The `DescriptorSlot` struct identifies the descriptor set and binding that a VM resource uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DescriptorSlot {
	set: u32,
	binding: u32,
}

impl DescriptorSlot {
	pub const fn new(set: u32, binding: u32) -> Self {
		Self { set, binding }
	}

	pub const fn set(&self) -> u32 {
		self.set
	}

	pub const fn binding(&self) -> u32 {
		self.binding
	}
}

const PUSH_CONSTANT_SLOT: DescriptorSlot = DescriptorSlot::new(u32::MAX, u32::MAX);
const DYNAMIC_RESOURCE_SET: u32 = u32::MAX - 4;
const BUILTIN_POSITION_INTERFACE_SET: u32 = u32::MAX - 3;
const INPUT_INTERFACE_SET: u32 = u32::MAX - 2;
const OUTPUT_INTERFACE_SET: u32 = u32::MAX - 1;

pub const fn input_slot(location: u8) -> DescriptorSlot {
	DescriptorSlot::new(INPUT_INTERFACE_SET, location as u32)
}

pub const fn output_slot(location: u8) -> DescriptorSlot {
	DescriptorSlot::new(OUTPUT_INTERFACE_SET, location as u32)
}

/// Returns the interface slot reserved for the vertex position builtin.
pub const fn builtin_position_slot() -> DescriptorSlot {
	DescriptorSlot::new(BUILTIN_POSITION_INTERFACE_SET, 0)
}

fn dynamic_resource_slot(register: usize) -> DescriptorSlot {
	DescriptorSlot::new(
		DYNAMIC_RESOURCE_SET,
		u32::try_from(register).expect("VM register indices fit in a descriptor slot"),
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

/// The `BufferMemberLayout` struct stores the packed VM layout information for one buffer member.
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

/// The `BufferLayout` struct stores the packed CPU memory layout that the VM uses for a buffer binding.
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

/// The `Buffer` struct stores mutable bytes together with the VM layout that gives those bytes meaning.
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

	fn read_value(&self, offset: usize, value_type: &ValueType) -> Result<ScalarValue, VmError> {
		let bytes = self.read_bytes(offset, value_type.size())?;

		let value = match value_type {
			ValueType::Bool => ScalarValue::Bool(bytes[0] != 0),
			ValueType::U8 => ScalarValue::U8(bytes[0]),
			ValueType::U16 => ScalarValue::U16(u16::from_ne_bytes(bytes.try_into().expect("Invalid u16 byte count"))),
			ValueType::U32 => ScalarValue::U32(u32::from_ne_bytes(bytes.try_into().expect("Invalid u32 byte count"))),
			ValueType::I32 => ScalarValue::I32(i32::from_ne_bytes(bytes.try_into().expect("Invalid i32 byte count"))),
			ValueType::F32 => ScalarValue::F32(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count"))),
			ValueType::Vec2U16 => ScalarValue::Vec2U16(read_u16_array::<2>(bytes)?),
			ValueType::Vec2I => ScalarValue::Vec2I(read_i32_array::<2>(bytes)?),
			ValueType::Vec2U => ScalarValue::Vec2U(read_u32_array::<2>(bytes)?),
			ValueType::Vec3U => ScalarValue::Vec3U(read_u32_array::<3>(bytes)?),
			ValueType::Vec4U => ScalarValue::Vec4U(read_u32_array::<4>(bytes)?),
			ValueType::Vec2F => ScalarValue::Vec2F(read_f32_array::<2>(bytes)?),
			ValueType::Vec3F => ScalarValue::Vec3F(read_f32_array::<3>(bytes)?),
			ValueType::Vec4F => ScalarValue::Vec4F(read_f32_array::<4>(bytes)?),
			ValueType::Mat4F => ScalarValue::Mat4F(read_f32_array::<16>(bytes)?),
			ValueType::Mat4x3F => ScalarValue::Mat4x3F(read_f32_array::<12>(bytes)?),
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
				ScalarValue::Struct {
					value_type: value_type.clone(),
					fields: values,
				}
			}
		};

		Ok(value)
	}

	fn write_value(&mut self, offset: usize, value_type: &ValueType, value: &ScalarValue) -> Result<(), VmError> {
		if !value.matches_type(value_type) {
			return Err(VmError::TypeMismatch {
				expected: value_type.name().to_string(),
				found: value.value_type().name().to_string(),
			});
		}

		match value {
			ScalarValue::Bool(value) => self.write_bytes(offset, &[u8::from(*value)]),
			ScalarValue::U8(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			ScalarValue::U16(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			ScalarValue::U32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			ScalarValue::I32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			ScalarValue::F32(value) => self.write_bytes(offset, &value.to_ne_bytes()),
			ScalarValue::Vec2U16(value) => write_u16_slice(self, offset, value),
			ScalarValue::Vec2I(value) => write_i32_slice(self, offset, value),
			ScalarValue::Vec2U(value) => write_u32_slice(self, offset, value),
			ScalarValue::Vec3U(value) => write_u32_slice(self, offset, value),
			ScalarValue::Vec4U(value) => write_u32_slice(self, offset, value),
			ScalarValue::Vec2F(value) => write_f32_slice(self, offset, value),
			ScalarValue::Vec3F(value) => write_f32_slice(self, offset, value),
			ScalarValue::Vec4F(value) => write_f32_slice(self, offset, value),
			ScalarValue::Mat4F(value) => write_f32_slice(self, offset, value),
			ScalarValue::Mat4x3F(value) => write_f32_slice(self, offset, value),
			ScalarValue::Resource { .. } => Err(VmError::UnsupportedBufferLayout {
				message: "Resource handles cannot be written into CPU buffer memory".to_string(),
			}),
			ScalarValue::Struct { fields, .. } => {
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
	texels: Vec<[f32; 4]>,
	u32_texels: Vec<u32>,
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

		let texel_count = (width as usize) * (height as usize) * (depth as usize);
		Ok(Self {
			width,
			height,
			depth,
			texels: vec![[0.0, 0.0, 0.0, 0.0]; texel_count],
			u32_texels: vec![0; texel_count],
		})
	}

	pub fn write(&mut self, coord: [u32; 2], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.texels[index] = value;
		Ok(())
	}

	/// Writes one texel in a three-dimensional CPU texture.
	pub fn write_3d(&mut self, coord: [u32; 3], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index] = value;
		Ok(())
	}

	/// Writes one unsigned integer texel for integer image and atomic tests.
	pub fn write_u32(&mut self, coord: [u32; 2], value: u32) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.u32_texels[index] = value;
		self.texels[index] = [value as f32, 0.0, 0.0, 1.0];
		Ok(())
	}

	/// Fetches one texel without interpolation.
	pub fn fetch(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		Ok(Value::Vec4F(self.fetch_texel([coord[0], coord[1], 0])?))
	}

	/// Fetches one unsigned integer texel without interpolation.
	pub fn fetch_u32(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		Ok(Value::U32(self.u32_texels[index]))
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
		Ok(self.texels[index])
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
		let previous = self.u32_texels[index];
		let updated = previous | value;
		self.u32_texels[index] = updated;
		self.texels[index] = [updated as f32, 0.0, 0.0, 1.0];
		Ok(previous)
	}
}

enum DescriptorBinding<'a> {
	Buffer(&'a mut Buffer),
	Texture(&'a mut Texture),
	Image(&'a mut Texture),
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

	fn set_counts(&mut self, vertex_count: u32, primitive_count: u32) {
		self.vertex_count = vertex_count;
		self.primitive_count = primitive_count;
		self.vertex_positions.resize(vertex_count as usize, [0.0; 4]);
		self.triangles.resize(primitive_count as usize, [0; 3]);
	}
}

/// The `DescriptorBindings` struct stores the mutable resources that a compiled BESL VM program can access.
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
			DescriptorBinding::Texture(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "buffer",
				found: "texture",
			}),
			DescriptorBinding::Image(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "buffer",
				found: "image",
			}),
		}
	}

	fn texture_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Texture(texture) => Ok(&mut **texture),
			DescriptorBinding::Buffer(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "texture",
				found: "buffer",
			}),
			DescriptorBinding::Image(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "texture",
				found: "image",
			}),
		}
	}

	fn image_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Image(image) => Ok(&mut **image),
			DescriptorBinding::Buffer(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "image",
				found: "buffer",
			}),
			DescriptorBinding::Texture(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "image",
				found: "texture",
			}),
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
	thread_id: [u32; 2],
	thread_idx: u32,
	threadgroup_position: u32,
}

impl Default for ExecutionConfig {
	fn default() -> Self {
		Self {
			instruction_limit: 1_000_000,
			call_depth_limit: 64,
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

/// The `ExecutableProgram` struct stores the runnable VM form of a lexed BESL program.
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
		let main = resolve_main_function(&program)?;
		let main_signature = extract_function_signature(&main)?;
		if !main_signature.params.is_empty() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions with parameters are not supported".to_string(),
			});
		}
		if main_signature.return_type.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: format!(
					"Main functions must return void, but found `{}`",
					main_signature.return_type.as_ref().map(ValueType::name).unwrap_or("void")
				),
			});
		}

		let function_nodes = collect_functions(&main);
		for function in &function_nodes {
			reject_raw_code_nodes(function)?;
		}
		// NodeReference hashing is pointer-identity based, so function lookup is stable even though nodes are RefCell-backed.
		let mut function_ids = HashMap::new();
		for (index, function) in function_nodes.iter().enumerate() {
			function_ids.insert(function.clone(), index);
		}

		let mut descriptor_layouts = HashMap::new();
		let mut functions = Vec::with_capacity(function_nodes.len());
		for function in &function_nodes {
			functions.push(Compiler::compile_function(
				function,
				&function_ids,
				&mut descriptor_layouts,
				specializations,
			)?);
		}

		Ok(Self {
			descriptor_layouts,
			functions,
			main_function: function_ids[&main],
		})
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

	/// Executes the compiled `main` function using the currently bound descriptor resources.
	pub fn run_main(&self, descriptors: &mut DescriptorBindings<'_>) -> Result<(), VmError> {
		self.run_main_with_config(descriptors, &ExecutionConfig::default())
	}

	/// Executes `main` with explicit execution limits and shader invocation coordinates.
	pub fn run_main_with_config(
		&self,
		descriptors: &mut DescriptorBindings<'_>,
		config: &ExecutionConfig,
	) -> Result<(), VmError> {
		let mut state = ExecutionState::new(config);
		let return_value = self.execute_function(self.main_function, &[], descriptors, &mut state)?;
		if return_value.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions must not return a value".to_string(),
			});
		}
		Ok(())
	}

	fn execute_function(
		&self,
		function_index: usize,
		arguments: &[ScalarValue],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
	) -> Result<Option<ScalarValue>, VmError> {
		state.enter_call()?;
		let result = self.execute_function_inner(function_index, arguments, descriptors, state);
		state.leave_call();
		result
	}

	/// Runs one function body while the caller maintains the shared execution limits.
	fn execute_function_inner(
		&self,
		function_index: usize,
		arguments: &[ScalarValue],
		descriptors: &mut DescriptorBindings<'_>,
		state: &mut ExecutionState<'_>,
	) -> Result<Option<ScalarValue>, VmError> {
		let function = self
			.functions
			.get(function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", function_index),
			})?;

		let mut registers = vec![None; function.register_count];
		let mut locals = vec![None; function.local_types.len()];
		if arguments.len() != function.parameter_count {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: arguments.len(),
			});
		}
		for (index, argument) in arguments.iter().enumerate() {
			locals[index] = Some(argument.clone());
		}

		let mut instruction_index = 0usize;
		while instruction_index < function.instructions.len() {
			state.consume_instruction()?;
			let instruction = &function.instructions[instruction_index];
			match instruction {
				Instruction::LoadLiteral { register, value } => {
					registers[*register] = Some(value.clone());
				}
				Instruction::Construct {
					register,
					value_type,
					components,
				} => {
					let values = components
						.iter()
						.map(|component| read_register(&registers, *component))
						.collect::<Result<Vec<_>, _>>()?;
					registers[*register] = Some(construct_value(value_type, &values)?);
				}
				Instruction::Extract {
					register,
					source,
					index,
					value_type,
				} => {
					let source = read_register(&registers, *source)?;
					registers[*register] = Some(extract_value(&source, *index, value_type)?);
				}
				Instruction::ExtractDynamic {
					register,
					source,
					index,
					count,
					value_type,
				} => {
					let source = read_register(&registers, *source)?;
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
					if index >= *count {
						return Err(VmError::BufferArrayIndexOutOfBounds { index, count: *count });
					}
					registers[*register] = Some(extract_value(&source, index, value_type)?);
				}
				Instruction::Arithmetic {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_arithmetic(*operator, &left, &right)?);
				}
				Instruction::CompareLessThan { register, left, right } => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_less_than(&left, &right)?);
				}
				Instruction::Compare {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_comparison(*operator, &left, &right)?);
				}
				Instruction::JumpIfZero { register, target } => {
					let value = read_register(&registers, *register)?;
					if is_zero_value(&value)? {
						instruction_index = *target;
						continue;
					}
				}
				Instruction::Jump { target } => {
					instruction_index = *target;
					continue;
				}
				Instruction::DotProduct { register, left, right } => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_dot_product(&left, &right)?);
				}
				Instruction::CrossProduct { register, left, right } => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_cross_product(&left, &right)?);
				}
				Instruction::Length { register, value } => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_length(&value)?);
				}
				Instruction::Normalize { register, value } => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_normalize(&value)?);
				}
				Instruction::Reflect {
					register,
					incident,
					normal,
				} => {
					let incident = read_register(&registers, *incident)?;
					let normal = read_register(&registers, *normal)?;
					registers[*register] = Some(apply_reflect(&incident, &normal)?);
				}
				Instruction::UnaryScalar {
					register,
					operator,
					value,
				} => {
					let value = read_register(&registers, *value)?;
					registers[*register] = Some(apply_scalar_unary(*operator, &value)?);
				}
				Instruction::BinaryScalar {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_scalar_binary(*operator, &left, &right)?);
				}
				Instruction::TernaryScalar {
					register,
					operator,
					first,
					second,
					third,
				} => {
					let first = read_register(&registers, *first)?;
					let second = read_register(&registers, *second)?;
					let third = read_register(&registers, *third)?;
					registers[*register] = Some(apply_scalar_ternary(*operator, &first, &second, &third)?);
				}
				Instruction::ThreadIdx { register } => {
					registers[*register] = Some(ScalarValue::U32(state.config.thread_idx()));
				}
				Instruction::ThreadId { register } => {
					registers[*register] = Some(ScalarValue::Vec2U(state.config.thread_id()));
				}
				Instruction::ThreadgroupPosition { register } => {
					registers[*register] = Some(ScalarValue::U32(state.config.threadgroup_position()));
				}
				Instruction::SetMeshOutputCounts {
					vertex_count,
					primitive_count,
				} => {
					let vertex_count = expect_u32(read_register(&registers, *vertex_count)?)?;
					let primitive_count = expect_u32(read_register(&registers, *primitive_count)?)?;
					descriptors.mesh_outputs_mut()?.set_counts(vertex_count, primitive_count);
				}
				Instruction::SetMeshVertexPosition { index, position } => {
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
					let position = read_register(&registers, *position)?;
					let Value::Vec4F(position) = position else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec4F.name().to_string(),
							found: position.value_type().name().to_string(),
						});
					};
					let outputs = descriptors.mesh_outputs_mut()?;
					let count = outputs.vertex_positions.len();
					let destination = outputs
						.vertex_positions
						.get_mut(index)
						.ok_or(VmError::MeshOutputIndexOutOfBounds {
							kind: "vertex",
							index,
							count,
						})?;
					*destination = position;
				}
				Instruction::SetMeshTriangle { index, triangle } => {
					let index = expect_u32(read_register(&registers, *index)?)? as usize;
					let triangle = read_register(&registers, *triangle)?;
					let Value::Vec3U(triangle) = triangle else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec3U.name().to_string(),
							found: triangle.value_type().name().to_string(),
						});
					};
					let outputs = descriptors.mesh_outputs_mut()?;
					let count = outputs.triangles.len();
					let destination = outputs.triangles.get_mut(index).ok_or(VmError::MeshOutputIndexOutOfBounds {
						kind: "primitive",
						index,
						count,
					})?;
					*destination = triangle;
				}
				Instruction::LoadLocal { register, local } => {
					let value = locals
						.get(*local)
						.and_then(Option::clone)
						.ok_or(VmError::UninitializedLocal { local: *local })?;
					registers[*register] = Some(value);
				}
				Instruction::StoreLocal { local, register } => {
					let value = read_register(&registers, *register)?;
					locals[*local] = Some(value.clone());
				}
				Instruction::LoadBuffer {
					register,
					slot,
					offset,
					value_type,
				} => {
					let value = if *slot == PUSH_CONSTANT_SLOT {
						descriptors.push_constant_mut()?.read_value(*offset, value_type)?
					} else {
						descriptors.buffer_mut(*slot)?.read_value(*offset, value_type)?
					};
					registers[*register] = Some(value);
				}
				Instruction::LoadBufferIndexed {
					register,
					slot,
					offset,
					stride,
					count,
					index,
					value_type,
				} => {
					let index = read_buffer_array_index(&registers, *index, *count)?;
					let value = if *slot == PUSH_CONSTANT_SLOT {
						descriptors
							.push_constant_mut()?
							.read_value(*offset + *stride * index, value_type)?
					} else {
						descriptors
							.buffer_mut(*slot)?
							.read_value(*offset + *stride * index, value_type)?
					};
					registers[*register] = Some(value);
				}
				Instruction::FetchTexture { register, slot, coord } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch(coord)?);
				}
				Instruction::FetchTextureU32 { register, slot, coord } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::SampleTexture { register, slot, uv } => {
					let uv = read_register(&registers, *uv)?;
					let Value::Vec2F(uv) = uv else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2F.name().to_string(),
							found: uv.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample(uv)?);
				}
				Instruction::SampleTexture3D { register, slot, uvw } => {
					let uvw = read_register(&registers, *uvw)?;
					let Value::Vec3F(uvw) = uvw else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec3F.name().to_string(),
							found: uvw.value_type().name().to_string(),
						});
					};
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.texture_mut(slot)?.sample_3d(uvw)?);
				}
				Instruction::TextureSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, &registers)?;
					let texture = descriptors.texture_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([texture.width, texture.height]));
				}
				Instruction::ImageSize { register, slot } => {
					let slot = resolve_resource_slot(*slot, &registers)?;
					let image = descriptors.image_mut(slot)?;
					registers[*register] = Some(Value::Vec2U([image.width, image.height]));
				}
				Instruction::LoadImage { register, slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch(coord)?);
				}
				Instruction::LoadImageU32 { register, slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					registers[*register] = Some(descriptors.image_mut(slot)?.fetch_u32(coord)?);
				}
				Instruction::GuardImageBounds { slot, coord } => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					if !descriptors.image_mut(slot)?.contains_2d(coord) {
						return Ok(None);
					}
				}
				Instruction::ImageAtomicOr {
					register,
					slot,
					coord,
					value,
				} => {
					let coord = expect_vec2u(read_register(&registers, *coord)?)?;
					let value = expect_u32(read_register(&registers, *value)?)?;
					let slot = resolve_resource_slot(*slot, &registers)?;
					let previous = descriptors.image_mut(slot)?.atomic_or(coord, value)?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::WriteImage { slot, coord, value } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					let value = read_register(&registers, *value)?;
					let Value::Vec4F(value) = value else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec4F.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					};

					let slot = resolve_resource_slot(*slot, &registers)?;
					descriptors.image_mut(slot)?.write(coord, value)?;
				}
				Instruction::StoreBuffer {
					slot,
					offset,
					value_type,
					register,
				} => {
					let value = read_register(&registers, *register)?;
					descriptors.buffer_mut(*slot)?.write_value(*offset, value_type, &value)?;
				}
				Instruction::StoreBufferIndexed {
					slot,
					offset,
					stride,
					count,
					index,
					value_type,
					register,
				} => {
					let index = read_buffer_array_index(&registers, *index, *count)?;
					let value = read_register(&registers, *register)?;
					descriptors
						.buffer_mut(*slot)?
						.write_value(*offset + *stride * index, value_type, &value)?;
				}
				Instruction::AtomicAddBuffer {
					register,
					slot,
					offset,
					stride,
					count,
					index,
					value,
				} => {
					let index = match index {
						Some(index) => read_buffer_array_index(&registers, *index, *count)?,
						None => 0,
					};
					let value = expect_u32(read_register(&registers, *value)?)?;
					let buffer = descriptors.buffer_mut(*slot)?;
					let address = *offset + *stride * index;
					let previous = expect_u32(buffer.read_value(address, &ValueType::U32)?)?;
					buffer.write_value(address, &ValueType::U32, &Value::U32(previous.wrapping_add(value)))?;
					registers[*register] = Some(Value::U32(previous));
				}
				Instruction::Call {
					register,
					function,
					arguments,
				} => {
					let arguments = arguments
						.iter()
						.map(|argument| read_register(&registers, *argument))
						.collect::<Result<Vec<_>, _>>()?;
					let value = self.execute_function(*function, &arguments, descriptors, state)?;
					registers[*register] = value;
				}
				Instruction::Return { register } => {
					return match register {
						Some(register) => Ok(Some(read_register(&registers, *register)?)),
						None => Ok(None),
					};
				}
			}

			instruction_index += 1;
		}

		match &function.return_type {
			Some(return_type) => Err(VmError::UnsupportedStatement {
				message: format!(
					"Function with return type `{}` ended without returning a value",
					return_type.name()
				),
			}),
			None => Ok(None),
		}
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

type ScalarValue = Value;

#[derive(Clone, Debug, PartialEq)]
enum Instruction {
	LoadLiteral {
		register: usize,
		value: ScalarValue,
	},
	Construct {
		register: usize,
		value_type: ValueType,
		components: Vec<usize>,
	},
	Extract {
		register: usize,
		source: usize,
		index: usize,
		value_type: ValueType,
	},
	ExtractDynamic {
		register: usize,
		source: usize,
		index: usize,
		count: usize,
		value_type: ValueType,
	},
	Arithmetic {
		register: usize,
		operator: ArithmeticOperator,
		left: usize,
		right: usize,
	},
	Compare {
		register: usize,
		operator: ComparisonOperator,
		left: usize,
		right: usize,
	},
	CompareLessThan {
		register: usize,
		left: usize,
		right: usize,
	},
	JumpIfZero {
		register: usize,
		target: usize,
	},
	Jump {
		target: usize,
	},
	DotProduct {
		register: usize,
		left: usize,
		right: usize,
	},
	CrossProduct {
		register: usize,
		left: usize,
		right: usize,
	},
	Length {
		register: usize,
		value: usize,
	},
	Normalize {
		register: usize,
		value: usize,
	},
	Reflect {
		register: usize,
		incident: usize,
		normal: usize,
	},
	UnaryScalar {
		register: usize,
		operator: ScalarUnaryOperator,
		value: usize,
	},
	BinaryScalar {
		register: usize,
		operator: ScalarBinaryOperator,
		left: usize,
		right: usize,
	},
	TernaryScalar {
		register: usize,
		operator: ScalarTernaryOperator,
		first: usize,
		second: usize,
		third: usize,
	},
	ThreadIdx {
		register: usize,
	},
	ThreadId {
		register: usize,
	},
	ThreadgroupPosition {
		register: usize,
	},
	SetMeshOutputCounts {
		vertex_count: usize,
		primitive_count: usize,
	},
	SetMeshVertexPosition {
		index: usize,
		position: usize,
	},
	SetMeshTriangle {
		index: usize,
		triangle: usize,
	},
	LoadLocal {
		register: usize,
		local: usize,
	},
	StoreLocal {
		local: usize,
		register: usize,
	},
	LoadBuffer {
		register: usize,
		slot: DescriptorSlot,
		offset: usize,
		value_type: ValueType,
	},
	LoadBufferIndexed {
		register: usize,
		slot: DescriptorSlot,
		offset: usize,
		stride: usize,
		count: usize,
		index: usize,
		value_type: ValueType,
	},
	FetchTexture {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
	},
	FetchTextureU32 {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
	},
	SampleTexture {
		register: usize,
		slot: DescriptorSlot,
		uv: usize,
	},
	SampleTexture3D {
		register: usize,
		slot: DescriptorSlot,
		uvw: usize,
	},
	TextureSize {
		register: usize,
		slot: DescriptorSlot,
	},
	ImageSize {
		register: usize,
		slot: DescriptorSlot,
	},
	LoadImage {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
	},
	LoadImageU32 {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
	},
	GuardImageBounds {
		slot: DescriptorSlot,
		coord: usize,
	},
	ImageAtomicOr {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
		value: usize,
	},
	WriteImage {
		slot: DescriptorSlot,
		coord: usize,
		value: usize,
	},
	StoreBuffer {
		slot: DescriptorSlot,
		offset: usize,
		value_type: ValueType,
		register: usize,
	},
	StoreBufferIndexed {
		slot: DescriptorSlot,
		offset: usize,
		stride: usize,
		count: usize,
		index: usize,
		value_type: ValueType,
		register: usize,
	},
	AtomicAddBuffer {
		register: usize,
		slot: DescriptorSlot,
		offset: usize,
		stride: usize,
		count: usize,
		index: Option<usize>,
		value: usize,
	},
	Call {
		register: usize,
		function: usize,
		arguments: Vec<usize>,
	},
	Return {
		register: Option<usize>,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArithmeticOperator {
	Add,
	Subtract,
	Multiply,
	Divide,
	Modulo,
	ShiftLeft,
	ShiftRight,
	BitwiseAnd,
	BitwiseOr,
	LogicalAnd,
	LogicalOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComparisonOperator {
	Equal,
	NotEqual,
	LessThan,
	GreaterThan,
	LessThanOrEqual,
	GreaterThanOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarUnaryOperator {
	Abs,
	Sqrt,
	Exp,
	Sin,
	Cos,
	Tan,
	Round,
	Fract,
	Radians,
	InverseSqrt,
	Log2,
	Fwidth,
	FromU32ToF32,
	FromF32ToU32,
	FromU8ToU32,
	FromU16ToU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarBinaryOperator {
	Min,
	Max,
	Pow,
	Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScalarTernaryOperator {
	Smoothstep,
	Mix,
	Clamp,
}

/// The `Compiler` struct lowers one BESL function into bounded register-machine instructions.
struct Compiler<'a> {
	function_ids: HashMap<NodeReference, usize>,
	specializations: &'a SpecializationValues,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: HashMap<NodeReference, usize>,
	register_count: usize,
	return_type: Option<ValueType>,
	parameter_count: usize,
	loop_continue_targets: Vec<usize>,
	loop_continue_patches: Vec<Vec<usize>>,
}

impl<'a> Compiler<'a> {
	#[allow(clippy::mutable_key_type)]
	fn compile_function(
		function: &NodeReference,
		function_ids: &HashMap<NodeReference, usize>,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
		specializations: &'a SpecializationValues,
	) -> Result<ExecutableFunction, VmError> {
		let signature = extract_function_signature(function)?;
		let mut compiler = Self {
			function_ids: function_ids.clone(),
			specializations,
			instructions: Vec::new(),
			local_types: Vec::new(),
			locals_by_reference: HashMap::new(),
			register_count: 0,
			return_type: signature.return_type.clone(),
			parameter_count: signature.params.len(),
			loop_continue_targets: Vec::new(),
			loop_continue_patches: Vec::new(),
		};

		for (index, param) in signature.params.iter().enumerate() {
			compiler.local_types.push(param.value_type.clone());
			compiler.locals_by_reference.insert(param.node.clone(), index);
		}

		for statement in &signature.statements {
			compiler.compile_statement(statement, descriptor_layouts)?;
		}

		if !matches!(compiler.instructions.last(), Some(Instruction::Return { .. })) {
			compiler.instructions.push(Instruction::Return { register: None });
		}

		Ok(ExecutableFunction {
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
			parameter_count: compiler.parameter_count,
			return_type: compiler.return_type,
		})
	}

	/// Compiles one BESL statement into bytecode while tracking locals and descriptors.
	fn compile_statement(
		&mut self,
		statement: &NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let borrowed = statement.borrow();
		let result = match borrowed.node() {
			Nodes::Conditional { condition, statements } => {
				let condition = condition.clone();
				let statements = statements.clone();
				drop(borrowed);
				self.compile_conditional(&condition, &statements, descriptor_layouts)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let initializer = initializer.clone();
				let condition = condition.clone();
				let update = update.clone();
				let statements = statements.clone();
				drop(borrowed);
				self.compile_for_loop(&initializer, &condition, &update, &statements, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				left,
				right,
			}) => {
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);
				self.compile_assignment(statement, left, right, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Return { value }) => {
				let value = value.clone();
				drop(borrowed);
				self.compile_return_statement(value.as_ref(), descriptor_layouts)
			}
			Nodes::Expression(Expressions::Continue) => {
				drop(borrowed);
				if self.loop_continue_targets.is_empty() {
					return Err(VmError::UnsupportedStatement {
						message: "`continue` must be used inside a loop".to_string(),
					});
				}
				let jump_index = self.instructions.len();
				let target = self
					.loop_continue_targets
					.last()
					.copied()
					.expect("Expected loop continue target");
				self.instructions.push(Instruction::Jump { target });
				self.loop_continue_patches
					.last_mut()
					.expect("Expected continue patch stack")
					.push(jump_index);
				Ok(())
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_call_statement(&function, &parameters, descriptor_layouts)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = intrinsic.clone();
				let arguments = arguments.clone();
				drop(borrowed);
				self.compile_intrinsic_call_statement(&intrinsic, &arguments, descriptor_layouts)
			}
			Nodes::Raw { .. } => Ok(()),
			Nodes::Expression(Expressions::Member { .. }) | Nodes::Expression(Expressions::Accessor { .. }) => Ok(()),
			Nodes::Expression(other) => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement node: {}", describe_node(node)),
			}),
		};

		result
	}

	fn compile_conditional(
		&mut self,
		condition: &NodeReference,
		statements: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let condition_register = self.compile_value_expression(condition, &ValueType::Bool, descriptor_layouts)?;
		let jump_if_zero_index = self.instructions.len();
		self.instructions.push(Instruction::JumpIfZero {
			register: condition_register,
			target: usize::MAX,
		});

		for statement in statements {
			self.compile_statement(statement, descriptor_layouts)?;
		}

		let conditional_end = self.instructions.len();
		match &mut self.instructions[jump_if_zero_index] {
			Instruction::JumpIfZero { target, .. } => *target = conditional_end,
			_ => unreachable!("Expected JumpIfZero placeholder"),
		}

		Ok(())
	}

	fn compile_for_loop(
		&mut self,
		initializer: &NodeReference,
		condition: &NodeReference,
		update: &NodeReference,
		statements: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		self.compile_statement(initializer, descriptor_layouts)?;

		let condition_start = self.instructions.len();
		let condition_register = self.compile_value_expression(condition, &ValueType::Bool, descriptor_layouts)?;
		let jump_if_zero_index = self.instructions.len();
		self.instructions.push(Instruction::JumpIfZero {
			register: condition_register,
			target: usize::MAX,
		});
		let loop_end_placeholder_index = jump_if_zero_index;

		let continue_target = usize::MAX;
		self.loop_continue_targets.push(continue_target);
		self.loop_continue_patches.push(Vec::new());
		for statement in statements {
			self.compile_statement(statement, descriptor_layouts)?;
		}
		self.loop_continue_targets.pop();

		let update_start = self.instructions.len();
		self.compile_statement(update, descriptor_layouts)?;
		for jump_index in self.loop_continue_patches.pop().expect("Expected continue patch list") {
			match &mut self.instructions[jump_index] {
				Instruction::Jump { target } => *target = update_start,
				_ => unreachable!("Expected continue jump placeholder"),
			}
		}
		self.instructions.push(Instruction::Jump { target: condition_start });

		let loop_end = self.instructions.len();
		match &mut self.instructions[loop_end_placeholder_index] {
			Instruction::JumpIfZero { target, .. } => *target = loop_end,
			_ => unreachable!("Expected JumpIfZero placeholder"),
		}

		Ok(())
	}

	fn compile_assignment(
		&mut self,
		statement: &NodeReference,
		left: NodeReference,
		right: NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let left_expression = left.borrow();

		match left_expression.node() {
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
				let name = name.clone();
				let value_type = resolve_value_type(r#type)?;
				drop(left_expression);

				let local = self.define_local(statement.clone(), left, &name, value_type.clone());
				let register = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
				self.instructions.push(Instruction::StoreLocal { local, register });
				Ok(())
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(left_expression);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					let value_type = self
						.local_types
						.get(local)
						.cloned()
						.ok_or(VmError::UninitializedLocal { local })?;
					let register = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreLocal { local, register });
					Ok(())
				} else {
					let target = self.resolve_output_access(&left, descriptor_layouts)?;
					let register = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
					self.instructions.push(Instruction::StoreBuffer {
						slot: target.slot,
						offset: target.offset,
						value_type: target.value_type,
						register,
					});
					Ok(())
				}
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(left_expression);

				let target = if accessor_references_output(&left) {
					self.resolve_output_array_access(&left, descriptor_layouts)?
				} else {
					self.resolve_memory_access(&left, RequiredAccess::Write, descriptor_layouts)?
				};
				let register = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
				if let Some(index) = target.index {
					self.instructions.push(Instruction::StoreBufferIndexed {
						slot: target.slot,
						offset: target.offset,
						stride: target.stride,
						count: target.count,
						index,
						value_type: target.value_type,
						register,
					});
				} else {
					self.instructions.push(Instruction::StoreBuffer {
						slot: target.slot,
						offset: target.offset,
						value_type: target.value_type,
						register,
					});
				}
				Ok(())
			}
			node => Err(VmError::UnsupportedAssignmentTarget {
				message: format!("Unsupported assignment target: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles a scalar BESL expression into one register-producing VM instruction sequence.
	fn compile_value_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0].clone();
				drop(borrowed);
				self.compile_value_expression(&inner, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_function_call_expression(&function, &parameters, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = intrinsic.clone();
				let arguments = arguments.clone();
				drop(borrowed);
				self.compile_intrinsic_call_expression(&intrinsic, &arguments, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				let comparison = comparison_operator(operator);
				let arithmetic = if comparison.is_none() {
					Some(arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported value operator: {:?}", operator),
					})?)
				} else {
					None
				};
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);

				let operand_hint = if comparison.is_some() {
					&ValueType::U32
				} else if matches!(
					arithmetic,
					Some(ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr)
				) {
					&ValueType::Bool
				} else {
					expected_type
				};
				let left_type = self.infer_expression_type(&left, operand_hint, descriptor_layouts)?;
				let right_type = self.infer_expression_type(&right, operand_hint, descriptor_layouts)?;
				let result_type = if comparison.is_some() {
					ValueType::Bool
				} else {
					binary_result_type(arithmetic.expect("Expected arithmetic operator"), &left_type, &right_type)?
				};
				if &result_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: result_type.name().to_string(),
					});
				}

				let left = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
				let right = self.compile_value_expression(&right, &right_type, descriptor_layouts)?;
				let register = self.allocate_register();
				if let Some(operator) = comparison {
					if operator == ComparisonOperator::LessThan {
						self.instructions.push(Instruction::CompareLessThan { register, left, right });
					} else {
						self.instructions.push(Instruction::Compare {
							register,
							operator,
							left,
							right,
						});
					}
				} else {
					self.instructions.push(Instruction::Arithmetic {
						register,
						operator: arithmetic.expect("Expected arithmetic operator"),
						left,
						right,
					});
				}
				Ok(register)
			}
			Nodes::Expression(Expressions::Literal { value }) => {
				let value = value.clone();
				drop(borrowed);

				let register = self.allocate_register();
				let value = parse_literal(&value, expected_type)?;
				self.instructions.push(Instruction::LoadLiteral { register, value });
				Ok(register)
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(borrowed);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					let actual_type = self.local_types.get(local).ok_or(VmError::UninitializedLocal { local })?;
					if actual_type != expected_type {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: actual_type.name().to_string(),
						});
					}

					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLocal { register, local });
					Ok(register)
				} else if matches!(source.borrow().node(), Nodes::Input { .. }) {
					let target = self.resolve_input_access(expression, descriptor_layouts)?;
					if &target.value_type != expected_type {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: target.value_type.name().to_string(),
						});
					}

					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadBuffer {
						register,
						slot: target.slot,
						offset: target.offset,
						value_type: target.value_type,
					});
					Ok(register)
				} else if is_resource_type(expected_type) && matches!(source.borrow().node(), Nodes::Binding { .. }) {
					let (slot, layout) = {
						let source_ref = source.borrow();
						let Nodes::Binding {
							set, binding, r#type, ..
						} = source_ref.node()
						else {
							unreachable!("Resource sources are checked before compiling the handle")
						};
						let layout = match r#type {
							BindingTypes::CombinedImageSampler { .. } => DescriptorLayout::Texture,
							BindingTypes::Image { .. } => DescriptorLayout::Image,
							BindingTypes::Buffer { .. } => {
								return Err(VmError::TypeMismatch {
									expected: expected_type.name().to_string(),
									found: "buffer".to_string(),
								});
							}
						};
						(DescriptorSlot::new(*set, *binding), layout)
					};
					match descriptor_layouts.get(&slot) {
						Some(existing) if existing != &layout => {
							return Err(VmError::UnsupportedDescriptor {
								slot,
								message: "Descriptor slot was reused with a different resource type".to_string(),
							});
						}
						Some(_) => {}
						None => {
							descriptor_layouts.insert(slot, layout);
						}
					}
					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLiteral {
						register,
						value: Value::Resource {
							slot,
							value_type: expected_type.clone(),
						},
					});
					Ok(register)
				} else {
					let source_value = {
						let source_ref = source.borrow();
						match source_ref.node() {
							Nodes::Specialization { name, r#type } => {
								let declared_type = resolve_value_type(r#type)?;
								let value = self
									.specializations
									.get(name)
									.ok_or_else(|| VmError::MissingSpecialization { name: name.clone() })?;
								if !value.matches_type(&declared_type) {
									return Err(VmError::TypeMismatch {
										expected: declared_type.name().to_string(),
										found: value.value_type().name().to_string(),
									});
								}
								Some(Ok(value.clone()))
							}
							Nodes::Const { value, .. } | Nodes::Literal { value, .. } => {
								let value = value.clone();
								drop(source_ref);
								return self.compile_value_expression(&value, expected_type, descriptor_layouts);
							}
							_ => None,
						}
					};
					let value = source_value.ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported member source: {}", describe_node(source.borrow().node())),
					})??;
					if !value.matches_type(expected_type) {
						return Err(VmError::TypeMismatch {
							expected: expected_type.name().to_string(),
							found: value.value_type().name().to_string(),
						});
					}
					let register = self.allocate_register();
					self.instructions.push(Instruction::LoadLiteral { register, value });
					Ok(register)
				}
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(borrowed);
				self.compile_accessor_expression(expression, expected_type, descriptor_layouts)
			}
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles either a buffer access chain or a projection from a temporary aggregate value.
	fn compile_accessor_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		if accessor_references_buffer(expression) {
			let target = self.resolve_memory_access(expression, RequiredAccess::Read, descriptor_layouts)?;
			if &target.value_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: target.value_type.name().to_string(),
				});
			}
			let register = self.allocate_register();
			if let Some(index) = target.index {
				self.instructions.push(Instruction::LoadBufferIndexed {
					register,
					slot: target.slot,
					offset: target.offset,
					stride: target.stride,
					count: target.count,
					index,
					value_type: target.value_type,
				});
			} else {
				self.instructions.push(Instruction::LoadBuffer {
					register,
					slot: target.slot,
					offset: target.offset,
					value_type: target.value_type,
				});
			}
			return Ok(register);
		}
		let (left, right) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedExpression {
					message: "Expected an aggregate accessor".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
		if let Ok(member_name) = extract_member_name(&right) {
			let (index, result_type) = aggregate_member(&left_type, &member_name)?;
			if &result_type != expected_type {
				return Err(VmError::TypeMismatch {
					expected: expected_type.name().to_string(),
					found: result_type.name().to_string(),
				});
			}
			let source = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
			let register = self.allocate_register();
			self.instructions.push(Instruction::Extract {
				register,
				source,
				index,
				value_type: result_type,
			});
			return Ok(register);
		}

		let (result_type, count) = array_element_type(&left_type)?;
		if &result_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: result_type.name().to_string(),
			});
		}
		let source = self.compile_value_expression(&left, &left_type, descriptor_layouts)?;
		let index = self.compile_value_expression(&right, &ValueType::U32, descriptor_layouts)?;
		let register = self.allocate_register();
		self.instructions.push(Instruction::ExtractDynamic {
			register,
			source,
			index,
			count,
			value_type: result_type,
		});
		Ok(register)
	}

	fn compile_resolved_buffer_load(&mut self, target: ResolvedBufferAccess) -> Result<usize, VmError> {
		let register = self.allocate_register();
		if let Some(index) = target.index {
			self.instructions.push(Instruction::LoadBufferIndexed {
				register,
				slot: target.slot,
				offset: target.offset,
				stride: target.stride,
				count: target.count,
				index,
				value_type: target.value_type,
			});
		} else {
			self.instructions.push(Instruction::LoadBuffer {
				register,
				slot: target.slot,
				offset: target.offset,
				value_type: target.value_type,
			});
		}
		Ok(register)
	}

	fn compile_intrinsic_call_expression(
		&mut self,
		intrinsic: &NodeReference,
		arguments: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
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
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SampleTexture { register, slot, uv });
				Ok(register)
			}
			"texture_lod" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord_type = self.infer_expression_type(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &coord_type, descriptor_layouts)?;
				let register = self.allocate_register();
				match coord_type {
					ValueType::Vec2F => self.instructions.push(Instruction::SampleTexture {
						register,
						slot,
						uv: coord,
					}),
					ValueType::Vec3F => self.instructions.push(Instruction::SampleTexture3D {
						register,
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
				Ok(register)
			}
			"fetch" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FetchTexture { register, slot, coord });
				Ok(register)
			}
			"fetch_u32" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FetchTextureU32 { register, slot, coord });
				Ok(register)
			}
			"image_load" | "image_load_u32" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
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
				if arguments.len() != 3 {
					return Err(VmError::CallArgumentMismatch {
						expected: 3,
						found: arguments.len(),
					});
				}
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
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
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				self.compile_resolved_buffer_load(target)
			}
			"atomic_add" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				if target.value_type != ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: ValueType::U32.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let value = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::AtomicAddBuffer {
					register,
					slot: target.slot,
					offset: target.offset,
					stride: target.stride,
					count: target.count,
					index: target.index,
					value,
				});
				Ok(register)
			}
			"texture_size" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::TextureSize { register, slot });
				Ok(register)
			}
			"image_size" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::ImageSize { register, slot });
				Ok(register)
			}
			"dot" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
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
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let left = self.compile_value_expression(&arguments[0], &ValueType::Vec3F, descriptor_layouts)?;
				let right = self.compile_value_expression(&arguments[1], &ValueType::Vec3F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::CrossProduct { register, left, right });
				Ok(register)
			}
			"length" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
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
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}

				let supported_type = [ValueType::Vec2F, ValueType::Vec3F, ValueType::Vec4F]
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
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

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
			"abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "round" | "fract" | "radians" | "inversesqrt" | "log2"
			| "fwidth" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}

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
			"f32" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}
				if expected_type != &ValueType::F32 {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: ValueType::F32.name().to_string(),
					});
				}

				let value = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::UnaryScalar {
					register,
					operator: ScalarUnaryOperator::FromU32ToF32,
					value,
				});
				Ok(register)
			}
			"u32" => {
				if arguments.len() != 1 {
					return Err(VmError::CallArgumentMismatch {
						expected: 1,
						found: arguments.len(),
					});
				}
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
					ValueType::F32 => ScalarUnaryOperator::FromF32ToU32,
					ref other => {
						return Err(VmError::TypeMismatch {
							expected: "u8, u16, or f32".to_string(),
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
			"min" | "max" | "pow" | "step" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
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
						_ => unreachable!("Expected binary intrinsic"),
					},
					left,
					right,
				});
				Ok(register)
			}
			"smoothstep" | "mix" | "clamp" => {
				let expected_argument_count = 3;
				if arguments.len() != expected_argument_count {
					return Err(VmError::CallArgumentMismatch {
						expected: expected_argument_count,
						found: arguments.len(),
					});
				}

				let argument_type = if name == "clamp" {
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
						_ => unreachable!("Expected scalar ternary intrinsic"),
					},
					first,
					second,
					third,
				});
				Ok(register)
			}
			"thread_idx" => {
				if !arguments.is_empty() {
					return Err(VmError::CallArgumentMismatch {
						expected: 0,
						found: arguments.len(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadIdx { register });
				Ok(register)
			}
			"thread_id" => {
				if !arguments.is_empty() {
					return Err(VmError::CallArgumentMismatch {
						expected: 0,
						found: arguments.len(),
					});
				}
				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadId { register });
				Ok(register)
			}
			"threadgroup_position" => {
				if !arguments.is_empty() {
					return Err(VmError::CallArgumentMismatch {
						expected: 0,
						found: arguments.len(),
					});
				}
				let register = self.allocate_register();
				self.instructions.push(Instruction::ThreadgroupPosition { register });
				Ok(register)
			}
			_ => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported intrinsic `{}`", name),
			}),
		}
	}

	fn compile_function_call_expression(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
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

				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				if parameters.len() != signature.params.len() {
					return Err(VmError::CallArgumentMismatch {
						expected: signature.params.len(),
						found: parameters.len(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::Call {
					register,
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

	fn compile_constructor_expression(
		&mut self,
		_function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		constructor_type: ValueType,
		fields: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
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
			for parameter in parameters {
				// Packed u16 constructors accept ordinary u32 coordinate arithmetic and
				// apply the shader backend's narrowing conversion per component.
				let parameter_hint = if constructor_type == ValueType::Vec2U16 {
					ValueType::U32
				} else {
					scalar_type.clone()
				};
				let parameter_type = self.infer_expression_type(parameter, &parameter_hint, descriptor_layouts)?;
				let parameter_scalar = vector_scalar_type(&parameter_type).unwrap_or_else(|| parameter_type.clone());
				let compatible = parameter_scalar == scalar_type
					|| (constructor_type == ValueType::Vec2U16 && parameter_scalar == ValueType::U32);
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

	fn infer_expression_type(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ValueType, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				let inner = elements[0].clone();
				drop(borrowed);
				self.infer_expression_type(&inner, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Literal { value }) => {
				// Decimal and scientific notation remain floating-point when comparisons
				// do not otherwise provide an operand type.
				if value.contains(['.', 'e', 'E']) || supports_scalar_broadcast(expected_type) {
					Ok(ValueType::F32)
				} else {
					Ok(expected_type.clone())
				}
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(borrowed);

				if let Some(local) = self.locals_by_reference.get(&source).copied() {
					self.local_types
						.get(local)
						.cloned()
						.ok_or(VmError::UninitializedLocal { local })
				} else if matches!(source.borrow().node(), Nodes::Input { .. }) {
					Ok(self.resolve_input_access(expression, descriptor_layouts)?.value_type)
				} else {
					resolve_referenced_value_type(&source)
				}
			}
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);
				if accessor_references_buffer(expression) {
					Ok(self
						.resolve_memory_access(expression, RequiredAccess::Read, descriptor_layouts)?
						.value_type)
				} else {
					let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
					if let Ok(member_name) = extract_member_name(&right) {
						Ok(aggregate_member(&left_type, &member_name)?.1)
					} else {
						Ok(array_element_type(&left_type)?.0)
					}
				}
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				let intrinsic = intrinsic.clone();
				drop(borrowed);
				resolve_callable_return_type(&intrinsic)
			}
			Nodes::Expression(Expressions::FunctionCall { function, .. }) => resolve_callable_return_type(function),
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				if comparison_operator(operator).is_some() {
					Ok(ValueType::Bool)
				} else {
					let operator = arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
						message: format!("Unsupported value operator: {:?}", operator),
					})?;
					if matches!(operator, ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr) {
						return Ok(ValueType::Bool);
					}
					let left = left.clone();
					let right = right.clone();
					drop(borrowed);
					let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
					let right_type = self.infer_expression_type(&right, expected_type, descriptor_layouts)?;
					binary_result_type(operator, &left_type, &right_type)
				}
			}
			Nodes::Expression(Expressions::Continue) => Err(VmError::UnsupportedExpression {
				message: "`continue` is only valid as a statement".to_string(),
			}),
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	fn define_local(
		&mut self,
		statement: NodeReference,
		declaration: NodeReference,
		_name: &str,
		value_type: ValueType,
	) -> usize {
		let local = self.local_types.len();
		self.local_types.push(value_type);
		self.locals_by_reference.insert(statement, local);
		self.locals_by_reference.insert(declaration, local);
		local
	}

	fn allocate_register(&mut self) -> usize {
		let register = self.register_count;
		self.register_count += 1;
		register
	}

	/// Resolves a BESL accessor into the descriptor slot and packed byte offset that the VM should access.
	fn resolve_memory_access(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (binding, selectors) = extract_access_chain(expression)?;
		let Some(AccessSelector::Member(member_name)) = selectors.first() else {
			return Err(VmError::UnsupportedExpression {
				message: "Buffer access must select a named member first".to_string(),
			});
		};

		let binding_ref = binding.borrow();
		let (slot, layout) = match binding_ref.node() {
			Nodes::Binding {
				set,
				binding,
				read,
				write,
				r#type,
				..
			} => {
				match access {
					RequiredAccess::Read if !read => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "read",
						});
					}
					RequiredAccess::Write if !write => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "write",
						});
					}
					_ => {}
				}

				let slot = DescriptorSlot::new(*set, *binding);
				let layout = match r#type {
					BindingTypes::Buffer { members } => compile_buffer_layout(members)?,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only buffer descriptors are supported".to_string(),
						});
					}
				};

				(slot, layout)
			}
			Nodes::PushConstant { members } => {
				if matches!(access, RequiredAccess::Write) {
					return Err(VmError::UnsupportedAssignmentTarget {
						message: "Push constant members are read-only".to_string(),
					});
				}

				(PUSH_CONSTANT_SLOT, compile_buffer_layout(members)?)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		let descriptor_layout = if slot == PUSH_CONSTANT_SLOT {
			DescriptorLayout::PushConstant(layout.clone())
		} else {
			DescriptorLayout::Buffer(layout.clone())
		};

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &descriptor_layout => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, descriptor_layout);
			}
		}

		let member = layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;
		let mut offset = member.offset();
		let mut current_stride = member.value_type().size();
		let mut current_count = member.count();
		let mut value_type = member.value_type().clone();
		let mut index = None;
		let mut indexed_stride = current_stride;
		let mut indexed_count = current_count;
		for selector in selectors.iter().skip(1) {
			match selector {
				AccessSelector::Index(index_expression) => {
					if index.is_some() {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` cannot use more than one dynamic index", member_name),
						});
					}
					indexed_stride = current_stride;
					indexed_count = current_count;
					index = Some(self.compile_value_expression(index_expression, &ValueType::U32, descriptor_layouts)?);
					current_count = 1;
				}
				AccessSelector::Member(field_name) => {
					if current_count > 1 {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` is an array and requires an element index", member_name),
						});
					}
					let (field_offset, field_type, field_count) = aggregate_member_layout(&value_type, field_name)?;
					offset += field_offset;
					value_type = field_type;
					current_stride = value_type.size();
					current_count = field_count;
				}
			}
		}
		if current_count > 1 {
			return Err(VmError::UnsupportedExpression {
				message: format!("Buffer member `{}` is an array and requires an element index", member_name),
			});
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset,
			stride: indexed_stride,
			count: indexed_count,
			index,
			value_type,
		})
	}

	fn resolve_texture_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<DescriptorSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
				if !matches!(
					value_type,
					ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D
				) {
					return Err(VmError::TypeMismatch {
						expected: "texture resource".to_string(),
						found: value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				set,
				binding,
				read,
				write,
				r#type,
				..
			} => {
				match access {
					RequiredAccess::Read if !read => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "read",
						});
					}
					RequiredAccess::Write if !write => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "write",
						});
					}
					_ => {}
				}

				let slot = DescriptorSlot::new(*set, *binding);
				match r#type {
					BindingTypes::CombinedImageSampler { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only texture descriptors can be sampled or fetched".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Texture => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Texture);
				Ok(slot)
			}
		}
	}

	fn resolve_image_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<DescriptorSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
				if value_type != ValueType::Texture2D {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Texture2D.name().to_string(),
						found: value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				set,
				binding,
				read,
				write,
				r#type,
				..
			} => {
				match access {
					RequiredAccess::Read if !read => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "read",
						});
					}
					RequiredAccess::Write if !write => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "write",
						});
					}
					_ => {}
				}

				let slot = DescriptorSlot::new(*set, *binding);
				match r#type {
					BindingTypes::Image { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only image descriptors can be written through `write`".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Image => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Image);
				Ok(slot)
			}
		}
	}

	fn resolve_output_access(
		&mut self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, output_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Output {
				name,
				format,
				location,
				count,
			} => {
				if name != &output_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct output assignment is supported for `{}`", output_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				(
					if output_name == "position" {
						builtin_position_slot()
					} else {
						output_slot(*location)
					},
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: output_name.clone(),
							offset: 0,
							value_type: value_type.clone(),
							count,
						}],
						size: value_type.size() * count,
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.members()[0].value_type().size(),
			count: layout.members()[0].count(),
			index: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}

	/// Resolves one dynamically indexed mesh output-array write.
	fn resolve_output_array_access(
		&mut self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (left, index_expression) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedAssignmentTarget {
					message: "Expected an indexed output array".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let mut target = self.resolve_output_access(&left, descriptor_layouts)?;
		target.index = Some(self.compile_value_expression(&index_expression, &ValueType::U32, descriptor_layouts)?);
		Ok(target)
	}

	fn resolve_input_access(
		&mut self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, input_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Input { name, format, location } => {
				if name != &input_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct input reads are supported for `{}`", input_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				(
					input_slot(*location),
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: input_name.clone(),
							offset: 0,
							value_type: value_type.clone(),
							count: 1,
						}],
						size: value_type.size(),
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.size(),
			count: 1,
			index: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}

	fn compile_call_statement(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let function_ref = function.borrow();
		match function_ref.node() {
			Nodes::Function { .. } => {
				let signature = extract_function_signature(function)?;
				drop(function_ref);
				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				if parameters.len() != signature.params.len() {
					return Err(VmError::CallArgumentMismatch {
						expected: signature.params.len(),
						found: parameters.len(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::Call {
					register,
					function: *self
						.function_ids
						.get(function)
						.ok_or_else(|| VmError::UnsupportedExpression {
							message: "Unknown function reference".to_string(),
						})?,
					arguments,
				});
				Ok(())
			}
			node => Err(VmError::UnsupportedStatement {
				message: format!("Expected a function call statement, but found {}", describe_node(node)),
			}),
		}
	}

	fn compile_return_statement(
		&mut self,
		value: Option<&NodeReference>,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		match (self.return_type.clone(), value) {
			(None, None) => {
				self.instructions.push(Instruction::Return { register: None });
				Ok(())
			}
			(None, Some(_)) => Err(VmError::UnsupportedStatement {
				message: "Void functions cannot return a value".to_string(),
			}),
			(Some(return_type), Some(value)) => {
				let register = self.compile_value_expression(value, &return_type, descriptor_layouts)?;
				self.instructions.push(Instruction::Return {
					register: Some(register),
				});
				Ok(())
			}
			(Some(return_type), None) => Err(VmError::UnsupportedStatement {
				message: format!("Function with return type `{}` must return a value", return_type.name()),
			}),
		}
	}

	fn compile_intrinsic_call_statement(
		&mut self,
		intrinsic: &NodeReference,
		arguments: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let intrinsic_ref = intrinsic.borrow();
		let name = match intrinsic_ref.node() {
			Nodes::Intrinsic { name, .. } => name.clone(),
			node => {
				return Err(VmError::UnsupportedStatement {
					message: format!("Expected an intrinsic, but found {}", describe_node(node)),
				});
			}
		};
		drop(intrinsic_ref);

		match name.as_str() {
			"set_mesh_output_counts" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let vertex_count = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let primitive_count = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshOutputCounts {
					vertex_count,
					primitive_count,
				});
				Ok(())
			}
			"set_mesh_vertex_position" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let index = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let position = self.compile_value_expression(&arguments[1], &ValueType::Vec4F, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshVertexPosition { index, position });
				Ok(())
			}
			"set_mesh_triangle" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let index = self.compile_value_expression(&arguments[0], &ValueType::U32, descriptor_layouts)?;
				let triangle = self.compile_value_expression(&arguments[1], &ValueType::Vec3U, descriptor_layouts)?;
				self.instructions.push(Instruction::SetMeshTriangle { index, triangle });
				Ok(())
			}
			"write" => {
				if arguments.len() != 3 {
					return Err(VmError::CallArgumentMismatch {
						expected: 3,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let value = self.compile_value_expression(&arguments[2], &ValueType::Vec4F, descriptor_layouts)?;
				self.instructions.push(Instruction::WriteImage { slot, coord, value });
				Ok(())
			}
			"guard_image_bounds" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let slot = self.resolve_image_slot(&arguments[0], RequiredAccess::Any, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				self.instructions.push(Instruction::GuardImageBounds { slot, coord });
				Ok(())
			}
			"atomic_store" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}
				let target = self.resolve_memory_access(&arguments[0], RequiredAccess::Write, descriptor_layouts)?;
				if target.value_type != ValueType::U32 {
					return Err(VmError::TypeMismatch {
						expected: ValueType::U32.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(&arguments[1], &ValueType::U32, descriptor_layouts)?;
				if let Some(index) = target.index {
					self.instructions.push(Instruction::StoreBufferIndexed {
						slot: target.slot,
						offset: target.offset,
						stride: target.stride,
						count: target.count,
						index,
						value_type: ValueType::U32,
						register,
					});
				} else {
					self.instructions.push(Instruction::StoreBuffer {
						slot: target.slot,
						offset: target.offset,
						value_type: ValueType::U32,
						register,
					});
				}
				Ok(())
			}
			"atomic_add" | "image_atomic_or" => {
				self.compile_intrinsic_call_expression(intrinsic, arguments, &ValueType::U32, descriptor_layouts)?;
				Ok(())
			}
			_ => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported intrinsic statement `{}`", name),
			}),
		}
	}
}

/// The `ResolvedBufferAccess` struct carries a validated packed-memory target into instruction lowering.
struct ResolvedBufferAccess {
	slot: DescriptorSlot,
	offset: usize,
	stride: usize,
	count: usize,
	index: Option<usize>,
	value_type: ValueType,
}

/// The `FunctionParameter` struct links one lexical parameter identity to its portable VM value type.
struct FunctionParameter {
	node: NodeReference,
	value_type: ValueType,
}

/// The `FunctionSignature` struct supplies parameter, return, and body information while lowering function calls.
struct FunctionSignature {
	params: Vec<FunctionParameter>,
	return_type: Option<ValueType>,
	statements: Vec<NodeReference>,
}

#[derive(Clone, Copy)]
enum RequiredAccess {
	Read,
	Write,
	Any,
}

fn resolve_main_function(program: &NodeReference) -> Result<NodeReference, VmError> {
	let function = {
		let node = program.borrow();
		match node.node() {
			Nodes::Function { name, .. } if name == "main" => Some(program.clone()),
			_ => None,
		}
	};

	if let Some(function) = function {
		return Ok(function);
	}

	program.get_main().ok_or(VmError::MissingMainFunction)
}

fn collect_functions(main: &NodeReference) -> Vec<NodeReference> {
	let mut functions = Vec::new();
	let mut seen = HashSet::new();
	collect_reachable_function(main, &mut seen, &mut functions);
	functions
}

/// Adds one function and every function referenced by its executable expressions.
fn collect_reachable_function(function: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
	if !seen.insert(function.identity()) {
		return;
	}
	functions.push(function.clone());
	let statements = match function.borrow().node() {
		Nodes::Function { statements, .. } => statements.clone(),
		_ => return,
	};
	for statement in statements {
		collect_function_references(&statement, seen, functions);
	}
}

fn collect_function_references(node: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
	let (called_function, children) = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				(Some(function.clone()), parameters.clone())
			}
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => (None, arguments.clone()),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => (None, vec![left.clone(), right.clone()]),
			Nodes::Expression(Expressions::Expression { elements }) => (None, elements.clone()),
			Nodes::Expression(Expressions::Return { value }) => (None, value.iter().cloned().collect()),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => (None, vec![value.clone()]),
			_ => (None, Vec::new()),
		}
	};
	if let Some(function) = called_function {
		if matches!(function.borrow().node(), Nodes::Function { .. }) {
			collect_reachable_function(&function, seen, functions);
		}
	}
	for child in children {
		collect_function_references(&child, seen, functions);
	}
}

fn reject_raw_code_nodes(node: &NodeReference) -> Result<(), VmError> {
	let children = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Raw { glsl, hlsl, msl, .. } => {
				let has_code = [glsl.as_deref(), hlsl.as_deref(), msl.as_deref()]
					.into_iter()
					.flatten()
					.any(|code| !code.trim().is_empty());
				if has_code {
					return Err(VmError::UnsupportedRawCode);
				}
				Vec::new()
			}
			Nodes::Function { statements, .. } => statements.clone(),
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::Expression(Expressions::FunctionCall { parameters, .. }) => parameters.clone(),
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => arguments.clone(),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => vec![left.clone(), right.clone()],
			Nodes::Expression(Expressions::Expression { elements }) => elements.clone(),
			Nodes::Expression(Expressions::Return { value }) => value.iter().cloned().collect(),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => vec![value.clone()],
			_ => Vec::new(),
		}
	};

	for child in children {
		reject_raw_code_nodes(&child)?;
	}

	Ok(())
}

fn extract_function_signature(function: &NodeReference) -> Result<FunctionSignature, VmError> {
	let function_ref = function.borrow();
	let (params, return_type, statements) = match function_ref.node() {
		Nodes::Function {
			params,
			return_type,
			statements,
			..
		} => (params.clone(), return_type.clone(), statements.clone()),
		node => {
			return Err(VmError::UnsupportedExpression {
				message: format!("Expected a function, but found {}", describe_node(node)),
			});
		}
	};
	drop(function_ref);

	let mut compiled_params = Vec::with_capacity(params.len());
	for param in params {
		let param_ref = param.borrow();
		let value_type = match param_ref.node() {
			Nodes::Parameter { r#type, .. } => resolve_value_type(r#type)?,
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a parameter, but found {}", describe_node(node)),
				});
			}
		};
		drop(param_ref);
		compiled_params.push(FunctionParameter { node: param, value_type });
	}

	let return_type = resolve_function_return_type(&return_type)?;
	Ok(FunctionSignature {
		params: compiled_params,
		return_type,
		statements,
	})
}

fn resolve_function_return_type(return_type: &NodeReference) -> Result<Option<ValueType>, VmError> {
	if return_type.borrow().get_name() == Some("void") {
		Ok(None)
	} else {
		Ok(Some(resolve_value_type(return_type)?))
	}
}

fn resolve_callable_return_type(callable: &NodeReference) -> Result<ValueType, VmError> {
	let callable_ref = callable.borrow();
	match callable_ref.node() {
		Nodes::Struct { .. } => resolve_value_type(callable),
		Nodes::Intrinsic { r#return, .. } => {
			let return_type = r#return.clone();
			drop(callable_ref);
			resolve_value_type(&return_type)
		}
		Nodes::Function { return_type, .. } => {
			let return_type = return_type.clone();
			drop(callable_ref);
			resolve_function_return_type(&return_type)?.ok_or_else(|| VmError::UnsupportedExpression {
				message: "Void functions cannot be used as value expressions".to_string(),
			})
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a callable value, but found {}", describe_node(node)),
		}),
	}
}

fn resolve_value_type(node: &NodeReference) -> Result<ValueType, VmError> {
	let type_name = node
		.borrow()
		.get_name()
		.map(str::to_string)
		.unwrap_or_else(|| "unknown".to_string());

	match type_name.as_str() {
		"bool" => Ok(ValueType::Bool),
		"u8" => Ok(ValueType::U8),
		"u16" => Ok(ValueType::U16),
		"u32" => Ok(ValueType::U32),
		"i32" => Ok(ValueType::I32),
		"f32" => Ok(ValueType::F32),
		"atomicu32" => Ok(ValueType::U32),
		"vec2u16" => Ok(ValueType::Vec2U16),
		"vec2i" => Ok(ValueType::Vec2I),
		"vec2u" => Ok(ValueType::Vec2U),
		"vec3u" => Ok(ValueType::Vec3U),
		"vec4u" => Ok(ValueType::Vec4U),
		"vec2f" => Ok(ValueType::Vec2F),
		"vec3f" => Ok(ValueType::Vec3F),
		"vec4f" => Ok(ValueType::Vec4F),
		"mat4f" => Ok(ValueType::Mat4F),
		"mat4x3f" => Ok(ValueType::Mat4x3F),
		"Texture2D" => Ok(ValueType::Texture2D),
		"Texture3D" => Ok(ValueType::Texture3D),
		"ArrayTexture2D" => Ok(ValueType::ArrayTexture2D),
		_ => {
			let fields = match node.borrow().node() {
				Nodes::Struct { fields, .. } => fields.clone(),
				_ => return Err(VmError::UnsupportedType { type_name }),
			};
			let (fields, size) = compile_member_layouts(&fields)?;
			Ok(ValueType::Struct {
				name: type_name,
				fields,
				size,
			})
		}
	}
}

fn is_resource_type(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Texture2D | ValueType::Texture3D | ValueType::ArrayTexture2D
	)
}

fn compile_buffer_layout(members: &[NodeReference]) -> Result<BufferLayout, VmError> {
	let (compiled_members, offset) = compile_member_layouts(members)?;

	Ok(BufferLayout {
		members: compiled_members,
		size: offset,
	})
}

fn compile_member_layouts(members: &[NodeReference]) -> Result<(Vec<BufferMemberLayout>, usize), VmError> {
	let mut offset = 0;
	let mut compiled_members = Vec::with_capacity(members.len());
	for member in members {
		let member = member.borrow();
		match member.node() {
			Nodes::Member { name, r#type, count } => {
				let value_type = resolve_value_type(r#type)?;
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				compiled_members.push(BufferMemberLayout {
					name: name.clone(),
					offset,
					value_type: value_type.clone(),
					count,
				});
				offset += value_type.size() * count;
			}
			node => {
				return Err(VmError::UnsupportedBufferLayout {
					message: format!("Unsupported buffer member node: {}", describe_node(node)),
				});
			}
		}
	}
	Ok((compiled_members, offset))
}

enum AccessSelector {
	Member(String),
	Index(NodeReference),
}

fn extract_access_chain(expression: &NodeReference) -> Result<(NodeReference, Vec<AccessSelector>), VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			let inner = elements[0].clone();
			drop(borrowed);
			extract_access_chain(&inner)
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			let left = left.clone();
			let selector = match right.borrow().node() {
				Nodes::Expression(Expressions::Member { name, .. }) => AccessSelector::Member(name.clone()),
				_ => AccessSelector::Index(right.clone()),
			};
			drop(borrowed);
			let (binding, mut selectors) = extract_access_chain(&left)?;
			selectors.push(selector);
			Ok((binding, selectors))
		}
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);
			if matches!(source.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }) {
				Ok((source, Vec::new()))
			} else {
				Err(VmError::UnsupportedExpression {
					message: "Accessor is not rooted in a buffer binding".to_string(),
				})
			}
		}
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok((expression.clone(), Vec::new())),
		Nodes::Expression(expression) => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a buffer accessor, but found {}",
				match expression {
					Expressions::Return { .. } => "return",
					Expressions::Continue => "continue",
					Expressions::Member { .. } => "member",
					Expressions::Expression { .. } => "multi-element expression group",
					Expressions::Literal { .. } => "literal",
					Expressions::FunctionCall { .. } => "function call",
					Expressions::IntrinsicCall { .. } => "intrinsic call",
					Expressions::Operator { .. } => "operator",
					Expressions::VariableDeclaration { .. } => "variable declaration",
					Expressions::Accessor { .. } => "accessor",
					Expressions::Macro { .. } => "macro",
				}
			),
		}),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer accessor, but found {}", describe_node(node)),
		}),
	}
}

fn accessor_references_buffer(expression: &NodeReference) -> bool {
	extract_access_chain(expression)
		.ok()
		.is_some_and(|(binding, _)| matches!(binding.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }))
}

fn accessor_references_output(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Accessor { left, .. }) => output_member_references_interface(left),
		_ => false,
	}
}

fn output_member_references_interface(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			output_member_references_interface(&elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. }) => matches!(source.borrow().node(), Nodes::Output { .. }),
		_ => false,
	}
}

fn extract_binding_reference(expression: &NodeReference) -> Result<NodeReference, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(expression.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);

			let result = match source.borrow().node() {
				Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(source.clone()),
				Nodes::Expression(Expressions::Member { .. }) => extract_binding_reference(&source),
				_ => Err(VmError::UnsupportedExpression {
					message: format!(
						"Only direct binding or push constant member access is supported, but found {}",
						describe_node(source.borrow().node())
					),
				}),
			};

			result
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a binding or push constant reference, but found {}",
				describe_node(node)
			),
		}),
	}
}

fn extract_member_name(expression: &NodeReference) -> Result<String, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { name, .. }) => Ok(name.clone()),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer member name, but found {}", describe_node(node)),
		}),
	}
}

fn aggregate_member(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType), VmError> {
	match value_type {
		ValueType::Struct { fields, .. } => fields
			.iter()
			.enumerate()
			.find(|(_, field)| field.name() == member_name)
			.map(|(index, field)| (index, field.value_type().clone()))
			.ok_or_else(|| VmError::UnknownBufferMember {
				member: member_name.to_string(),
			}),
		ValueType::Vec2U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec2F => {
			vector_member(value_type, member_name, 2)
		}
		ValueType::Vec3U | ValueType::Vec3F => vector_member(value_type, member_name, 3),
		ValueType::Vec4U | ValueType::Vec4F => vector_member(value_type, member_name, 4),
		ValueType::Mat4F => matrix_member(member_name, ValueType::Vec4F),
		ValueType::Mat4x3F => matrix_member(member_name, ValueType::Vec3F),
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` has no selectable members", value_type.name()),
		}),
	}
}

fn array_element_type(value_type: &ValueType) -> Result<(ValueType, usize), VmError> {
	let ValueType::Struct { fields, .. } = value_type else {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` cannot be indexed as an aggregate value", value_type.name()),
		});
	};
	let first = fields.first().ok_or_else(|| VmError::UnsupportedExpression {
		message: "Cannot index an empty aggregate value".to_string(),
	})?;
	if fields.iter().enumerate().any(|(index, field)| {
		field
			.name()
			.strip_prefix("value_")
			.and_then(|suffix| suffix.parse::<usize>().ok())
			!= Some(index)
			|| field.value_type() != first.value_type()
	}) {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` is a struct, not an indexable array value", value_type.name()),
		});
	}
	Ok((first.value_type().clone(), fields.len()))
}

fn aggregate_member_layout(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType, usize), VmError> {
	let (index, field_type) = aggregate_member(value_type, member_name)?;
	let offset = match value_type {
		ValueType::Struct { fields, .. } => fields[index].offset(),
		ValueType::Mat4F | ValueType::Mat4x3F => index * field_type.size(),
		_ => index * field_type.size(),
	};
	let count = match value_type {
		ValueType::Struct { fields, .. } => fields[index].count(),
		_ => 1,
	};
	Ok((offset, field_type, count))
}

fn vector_member(value_type: &ValueType, member_name: &str, component_count: usize) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name)
		.filter(|index| *index < component_count)
		.ok_or_else(|| VmError::UnsupportedExpression {
			message: format!("`{}` is not a component of `{}`", member_name, value_type.name()),
		})?;
	let scalar = vector_scalar_type(value_type).expect("Vector types have scalar components");
	Ok((index, scalar))
}

fn matrix_member(member_name: &str, column_type: ValueType) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name).ok_or_else(|| VmError::UnsupportedExpression {
		message: format!("`{}` is not a matrix column", member_name),
	})?;
	Ok((index, column_type))
}

fn component_index(name: &str) -> Option<usize> {
	match name {
		"x" | "r" => Some(0),
		"y" | "g" => Some(1),
		"z" | "b" => Some(2),
		"w" | "a" => Some(3),
		_ => None,
	}
}

fn resolve_referenced_value_type(source: &NodeReference) -> Result<ValueType, VmError> {
	match source.borrow().node() {
		Nodes::Member { r#type, .. }
		| Nodes::Parameter { r#type, .. }
		| Nodes::Specialization { r#type, .. }
		| Nodes::Const { r#type, .. } => resolve_value_type(r#type),
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => resolve_value_type(format),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => resolve_value_type(r#type),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Cannot resolve a value type from {}", describe_node(node)),
		}),
	}
}

fn describe_node(node: &Nodes) -> &'static str {
	match node {
		Nodes::Null => "null",
		Nodes::Scope { .. } => "scope",
		Nodes::Struct { .. } => "struct",
		Nodes::Member { .. } => "member",
		Nodes::Function { .. } => "function",
		Nodes::Conditional { .. } => "conditional",
		Nodes::ForLoop { .. } => "for loop",
		Nodes::Specialization { .. } => "specialization",
		Nodes::Expression(_) => "expression",
		Nodes::Raw { .. } => "raw",
		Nodes::Binding { .. } => "binding",
		Nodes::PushConstant { .. } => "push constant",
		Nodes::Intrinsic { .. } => "intrinsic",
		Nodes::Input { .. } => "input",
		Nodes::Output { .. } => "output",
		Nodes::Parameter { .. } => "parameter",
		Nodes::Literal { .. } => "literal",
		Nodes::Const { .. } => "const",
	}
}

fn parse_literal(value: &str, value_type: &ValueType) -> Result<ScalarValue, VmError> {
	let parsed = match value_type {
		ValueType::Bool => match value {
			"true" => ScalarValue::Bool(true),
			"false" => ScalarValue::Bool(false),
			_ => {
				return Err(VmError::InvalidLiteral {
					value: value.to_string(),
					value_type: value_type.name().to_string(),
				});
			}
		},
		ValueType::U8 => value
			.parse::<u8>()
			.map(ScalarValue::U8)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::U16 => value
			.parse::<u16>()
			.map(ScalarValue::U16)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::U32 => value
			.parse::<u32>()
			.map(ScalarValue::U32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::I32 => value
			.parse::<i32>()
			.map(ScalarValue::I32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::Vec2U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
		ValueType::F32 => value
			.parse::<f32>()
			.map(ScalarValue::F32)
			.map_err(|_| VmError::InvalidLiteral {
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

fn construct_value(value_type: &ValueType, components: &[ScalarValue]) -> Result<ScalarValue, VmError> {
	match value_type {
		ValueType::Vec2U16 => Ok(ScalarValue::Vec2U16(extract_u16_components::<2>(components)?)),
		ValueType::Vec2I => Ok(ScalarValue::Vec2I(extract_i32_components::<2>(components)?)),
		ValueType::Vec2U => Ok(ScalarValue::Vec2U(extract_u32_components::<2>(components)?)),
		ValueType::Vec3U => Ok(ScalarValue::Vec3U(extract_u32_components::<3>(components)?)),
		ValueType::Vec4U => Ok(ScalarValue::Vec4U(extract_u32_components::<4>(components)?)),
		ValueType::Vec2F => Ok(ScalarValue::Vec2F(extract_f32_components::<2>(components)?)),
		ValueType::Vec3F => Ok(ScalarValue::Vec3F(extract_f32_components::<3>(components)?)),
		ValueType::Vec4F => Ok(ScalarValue::Vec4F(extract_f32_components::<4>(components)?)),
		ValueType::Mat4F => Ok(ScalarValue::Mat4F(extract_f32_components::<16>(components)?)),
		ValueType::Mat4x3F => Ok(ScalarValue::Mat4x3F(extract_f32_components::<12>(components)?)),
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
			Ok(ScalarValue::Struct {
				value_type: value_type.clone(),
				fields: components.to_vec(),
			})
		}
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` is not a constructor-backed VM value type", value_type.name()),
		}),
	}
}

fn extract_f32_components<const N: usize>(components: &[ScalarValue]) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	let mut index = 0;
	for component in components {
		let slice: &[f32] = match component {
			ScalarValue::F32(value) => std::slice::from_ref(value),
			ScalarValue::Vec2F(value) => value,
			ScalarValue::Vec3F(value) => value,
			ScalarValue::Vec4F(value) => value,
			ScalarValue::Mat4F(value) => value,
			ScalarValue::Mat4x3F(value) => value,
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

fn extract_u32_components<const N: usize>(components: &[ScalarValue]) -> Result<[u32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[u32] = match component {
			ScalarValue::U32(value) => std::slice::from_ref(value),
			ScalarValue::Vec2U(value) => value,
			ScalarValue::Vec3U(value) => value,
			ScalarValue::Vec4U(value) => value,
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

fn extract_u16_components<const N: usize>(components: &[ScalarValue]) -> Result<[u16; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let component_count = match component {
			ScalarValue::U16(value) => {
				if index < N {
					values[index] = *value;
				}
				1
			}
			ScalarValue::U32(value) => {
				if index < N {
					values[index] = *value as u16;
				}
				1
			}
			ScalarValue::Vec2U16(value) => {
				if index + value.len() <= N {
					values[index..index + value.len()].copy_from_slice(value);
				}
				value.len()
			}
			ScalarValue::Vec2U(value) => {
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

fn extract_i32_components<const N: usize>(components: &[ScalarValue]) -> Result<[i32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[i32] = match component {
			ScalarValue::I32(value) => std::slice::from_ref(value),
			ScalarValue::Vec2I(value) => value,
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

fn read_f32_array<const N: usize>(bytes: &[u8]) -> Result<[f32; N], VmError> {
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

fn read_u32_array<const N: usize>(bytes: &[u8]) -> Result<[u32; N], VmError> {
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

fn read_u16_array<const N: usize>(bytes: &[u8]) -> Result<[u16; N], VmError> {
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

fn read_i32_array<const N: usize>(bytes: &[u8]) -> Result<[i32; N], VmError> {
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

fn write_f32_slice(buffer: &mut Buffer, offset: usize, values: &[f32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

fn write_u32_slice(buffer: &mut Buffer, offset: usize, values: &[u32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

fn write_u16_slice(buffer: &mut Buffer, offset: usize, values: &[u16]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 2, &value.to_ne_bytes())?;
	}
	Ok(())
}

fn write_i32_slice(buffer: &mut Buffer, offset: usize, values: &[i32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

fn lerp_rgba(left: [f32; 4], right: [f32; 4], factor: f32) -> [f32; 4] {
	let mut value = [0.0; 4];
	for index in 0..4 {
		value[index] = left[index] + (right[index] - left[index]) * factor;
	}
	value
}

fn normalized_linear_axis(uv: f32, size: u32) -> (u32, u32, f32) {
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

fn arithmetic_operator(operator: &Operators) -> Option<ArithmeticOperator> {
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

fn binary_result_type(operator: ArithmeticOperator, left: &ValueType, right: &ValueType) -> Result<ValueType, VmError> {
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

fn comparison_operator(operator: &Operators) -> Option<ComparisonOperator> {
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

fn supports_scalar_broadcast(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F | ValueType::Mat4F | ValueType::Mat4x3F
	)
}

fn apply_arithmetic(operator: ArithmeticOperator, left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	if matches!(operator, ArithmeticOperator::LogicalAnd | ArithmeticOperator::LogicalOr) {
		let left = !is_zero_value(left)?;
		let right = !is_zero_value(right)?;
		return Ok(ScalarValue::Bool(match operator {
			ArithmeticOperator::LogicalAnd => left && right,
			ArithmeticOperator::LogicalOr => left || right,
			_ => unreachable!("Logical operators are handled before arithmetic"),
		}));
	}
	if operator == ArithmeticOperator::Multiply {
		match (left, right) {
			(ScalarValue::Mat4F(matrix), ScalarValue::Vec4F(vector)) => {
				return Ok(ScalarValue::Vec4F(multiply_mat4_vec4(*matrix, *vector)));
			}
			(ScalarValue::Mat4F(left), ScalarValue::Mat4F(right)) => {
				return Ok(ScalarValue::Mat4F(multiply_mat4(*left, *right)));
			}
			(ScalarValue::Mat4x3F(matrix), ScalarValue::Vec4F(vector)) => {
				return Ok(ScalarValue::Vec3F(multiply_mat4x3_vec4(*matrix, *vector)));
			}
			_ => {}
		}
	}
	match (left, right) {
		(ScalarValue::U8(left), ScalarValue::U8(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U8)
		}
		(ScalarValue::U16(left), ScalarValue::U16(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U16)
		}
		(ScalarValue::U32(left), ScalarValue::U32(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U32)
		}
		(ScalarValue::I32(left), ScalarValue::I32(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::I32)
		}
		(ScalarValue::F32(left), ScalarValue::F32(right)) => {
			apply_float_arithmetic(*left, *right, operator).map(ScalarValue::F32)
		}
		(ScalarValue::Vec2U16(left), ScalarValue::Vec2U16(right)) => {
			apply_integer_array_arithmetic::<u16, 2>(*left, *right, operator).map(ScalarValue::Vec2U16)
		}
		(ScalarValue::Vec2I(left), ScalarValue::Vec2I(right)) => {
			apply_integer_array_arithmetic::<i32, 2>(*left, *right, operator).map(ScalarValue::Vec2I)
		}
		(ScalarValue::Vec2U(left), ScalarValue::Vec2U(right)) => {
			apply_integer_array_arithmetic::<u32, 2>(*left, *right, operator).map(ScalarValue::Vec2U)
		}
		(ScalarValue::Vec3U(left), ScalarValue::Vec3U(right)) => {
			apply_integer_array_arithmetic::<u32, 3>(*left, *right, operator).map(ScalarValue::Vec3U)
		}
		(ScalarValue::Vec4U(left), ScalarValue::Vec4U(right)) => {
			apply_integer_array_arithmetic::<u32, 4>(*left, *right, operator).map(ScalarValue::Vec4U)
		}
		(ScalarValue::Vec2F(left), ScalarValue::Vec2F(right)) => {
			apply_float_array_arithmetic::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::Vec3F(left), ScalarValue::Vec3F(right)) => {
			apply_float_array_arithmetic::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::Vec4F(left), ScalarValue::Vec4F(right)) => {
			apply_float_array_arithmetic::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::Mat4F(left), ScalarValue::Mat4F(right)) => {
			apply_float_array_arithmetic::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(ScalarValue::Mat4x3F(left), ScalarValue::Mat4x3F(right)) => {
			apply_float_array_arithmetic::<12>(*left, *right, operator).map(ScalarValue::Mat4x3F)
		}
		(ScalarValue::Vec2F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::Vec3F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::Vec4F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::Mat4F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(ScalarValue::Mat4x3F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<12>(*left, *right, operator).map(ScalarValue::Mat4x3F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec2F(right)) => {
			apply_scalar_float_broadcast::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec3F(right)) => {
			apply_scalar_float_broadcast::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec4F(right)) => {
			apply_scalar_float_broadcast::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::F32(left), ScalarValue::Mat4F(right)) => {
			apply_scalar_float_broadcast::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(ScalarValue::F32(left), ScalarValue::Mat4x3F(right)) => {
			apply_scalar_float_broadcast::<12>(*left, *right, operator).map(ScalarValue::Mat4x3F)
		}
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_less_than(left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (left, right) {
		(ScalarValue::U32(left), ScalarValue::U32(right)) => Ok(ScalarValue::Bool(left < right)),
		(ScalarValue::I32(left), ScalarValue::I32(right)) => Ok(ScalarValue::Bool(left < right)),
		(ScalarValue::F32(left), ScalarValue::F32(right)) => Ok(ScalarValue::Bool(left < right)),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_comparison(operator: ComparisonOperator, left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (left, right) {
		(ScalarValue::Bool(left), ScalarValue::Bool(right)) => Ok(ScalarValue::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "equality comparison for bool".to_string(),
					found: format!("{:?}", operator),
				});
			}
		})),
		(ScalarValue::U32(left), ScalarValue::U32(right)) => Ok(ScalarValue::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		})),
		(ScalarValue::I32(left), ScalarValue::I32(right)) => Ok(ScalarValue::Bool(match operator {
			ComparisonOperator::Equal => left == right,
			ComparisonOperator::NotEqual => left != right,
			ComparisonOperator::LessThan => left < right,
			ComparisonOperator::GreaterThan => left > right,
			ComparisonOperator::LessThanOrEqual => left <= right,
			ComparisonOperator::GreaterThanOrEqual => left >= right,
		})),
		(ScalarValue::F32(left), ScalarValue::F32(right)) => Ok(ScalarValue::Bool(match operator {
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

fn is_zero_value(value: &ScalarValue) -> Result<bool, VmError> {
	match value {
		ScalarValue::Bool(value) => Ok(!*value),
		ScalarValue::U32(value) => Ok(*value == 0),
		ScalarValue::I32(value) => Ok(*value == 0),
		ScalarValue::F32(value) => Ok(*value == 0.0),
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

fn apply_float_arithmetic(left: f32, right: f32, operator: ArithmeticOperator) -> Result<f32, VmError> {
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

fn apply_float_array_arithmetic<const N: usize>(
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

fn apply_float_scalar_broadcast<const N: usize>(
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

fn apply_scalar_float_broadcast<const N: usize>(
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

fn apply_dot_product(left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (left, right) {
		(ScalarValue::Vec2F(left), ScalarValue::Vec2F(right)) => Ok(ScalarValue::F32(dot_product(*left, *right))),
		(ScalarValue::Vec3F(left), ScalarValue::Vec3F(right)) => Ok(ScalarValue::F32(dot_product(*left, *right))),
		(ScalarValue::Vec4F(left), ScalarValue::Vec4F(right)) => Ok(ScalarValue::F32(dot_product(*left, *right))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_cross_product(left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (left, right) {
		(ScalarValue::Vec3F(left), ScalarValue::Vec3F(right)) => Ok(ScalarValue::Vec3F(cross_product(*left, *right))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_length(value: &ScalarValue) -> Result<ScalarValue, VmError> {
	match value {
		ScalarValue::Vec2F(value) => Ok(ScalarValue::F32(dot_product(*value, *value).sqrt())),
		ScalarValue::Vec3F(value) => Ok(ScalarValue::F32(dot_product(*value, *value).sqrt())),
		ScalarValue::Vec4F(value) => Ok(ScalarValue::F32(dot_product(*value, *value).sqrt())),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

fn apply_normalize(value: &ScalarValue) -> Result<ScalarValue, VmError> {
	match value {
		ScalarValue::Vec2F(value) => normalize_vector(*value).map(ScalarValue::Vec2F),
		ScalarValue::Vec3F(value) => normalize_vector(*value).map(ScalarValue::Vec3F),
		ScalarValue::Vec4F(value) => normalize_vector(*value).map(ScalarValue::Vec4F),
		value => Err(VmError::TypeMismatch {
			expected: "float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

fn apply_reflect(incident: &ScalarValue, normal: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (incident, normal) {
		(ScalarValue::Vec2F(incident), ScalarValue::Vec2F(normal)) => {
			reflect_vector(*incident, *normal).map(ScalarValue::Vec2F)
		}
		(ScalarValue::Vec3F(incident), ScalarValue::Vec3F(normal)) => {
			reflect_vector(*incident, *normal).map(ScalarValue::Vec3F)
		}
		(ScalarValue::Vec4F(incident), ScalarValue::Vec4F(normal)) => {
			reflect_vector(*incident, *normal).map(ScalarValue::Vec4F)
		}
		(incident, normal) => Err(VmError::TypeMismatch {
			expected: incident.value_type().name().to_string(),
			found: normal.value_type().name().to_string(),
		}),
	}
}

fn apply_scalar_unary(operator: ScalarUnaryOperator, value: &ScalarValue) -> Result<ScalarValue, VmError> {
	match operator {
		ScalarUnaryOperator::FromU32ToF32 => {
			let ScalarValue::U32(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U32.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};

			return Ok(ScalarValue::F32(*value as f32));
		}
		ScalarUnaryOperator::FromF32ToU32 => {
			let ScalarValue::F32(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::F32.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};

			return Ok(ScalarValue::U32(*value as u32));
		}
		ScalarUnaryOperator::FromU8ToU32 => {
			let ScalarValue::U8(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U8.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};
			return Ok(ScalarValue::U32(u32::from(*value)));
		}
		ScalarUnaryOperator::FromU16ToU32 => {
			let ScalarValue::U16(value) = value else {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U16.name().to_string(),
					found: value.value_type().name().to_string(),
				});
			};
			return Ok(ScalarValue::U32(u32::from(*value)));
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

fn map_float_value(value: &ScalarValue, map: impl Fn(f32) -> f32) -> Result<ScalarValue, VmError> {
	match value {
		ScalarValue::F32(value) => Ok(ScalarValue::F32(map(*value))),
		ScalarValue::Vec2F(value) => Ok(ScalarValue::Vec2F(value.map(&map))),
		ScalarValue::Vec3F(value) => Ok(ScalarValue::Vec3F(value.map(&map))),
		ScalarValue::Vec4F(value) => Ok(ScalarValue::Vec4F(value.map(&map))),
		value => Err(VmError::TypeMismatch {
			expected: "f32 or float vector".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

fn apply_scalar_binary(
	operator: ScalarBinaryOperator,
	left: &ScalarValue,
	right: &ScalarValue,
) -> Result<ScalarValue, VmError> {
	fn apply(operator: ScalarBinaryOperator, left: f32, right: f32) -> f32 {
		match operator {
			ScalarBinaryOperator::Min => left.min(right),
			ScalarBinaryOperator::Max => left.max(right),
			ScalarBinaryOperator::Pow => left.powf(right),
			ScalarBinaryOperator::Step => f32::from(right >= left),
		}
	}
	match (left, right) {
		(ScalarValue::F32(left), ScalarValue::F32(right)) => Ok(ScalarValue::F32(apply(operator, *left, *right))),
		(ScalarValue::Vec2F(left), ScalarValue::Vec2F(right)) => Ok(ScalarValue::Vec2F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(ScalarValue::Vec3F(left), ScalarValue::Vec3F(right)) => Ok(ScalarValue::Vec3F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(ScalarValue::Vec4F(left), ScalarValue::Vec4F(right)) => Ok(ScalarValue::Vec4F(std::array::from_fn(|index| {
			apply(operator, left[index], right[index])
		}))),
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_scalar_ternary(
	operator: ScalarTernaryOperator,
	first: &ScalarValue,
	second: &ScalarValue,
	third: &ScalarValue,
) -> Result<ScalarValue, VmError> {
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
		(ScalarValue::F32(first), ScalarValue::F32(second), ScalarValue::F32(third)) => {
			Ok(ScalarValue::F32(apply(operator, *first, *second, *third)))
		}
		(ScalarValue::Vec2F(first), ScalarValue::Vec2F(second), ScalarValue::Vec2F(third)) => {
			Ok(ScalarValue::Vec2F(std::array::from_fn(|index| {
				apply(operator, first[index], second[index], third[index])
			})))
		}
		(ScalarValue::Vec3F(first), ScalarValue::Vec3F(second), ScalarValue::Vec3F(third)) => {
			Ok(ScalarValue::Vec3F(std::array::from_fn(|index| {
				apply(operator, first[index], second[index], third[index])
			})))
		}
		(ScalarValue::Vec4F(first), ScalarValue::Vec4F(second), ScalarValue::Vec4F(third)) => {
			Ok(ScalarValue::Vec4F(std::array::from_fn(|index| {
				apply(operator, first[index], second[index], third[index])
			})))
		}
		_ => Err(VmError::TypeMismatch {
			expected: first.value_type().name().to_string(),
			found: format!("{}, {}", second.value_type().name(), third.value_type().name()),
		}),
	}
}

fn extract_value(value: &ScalarValue, index: usize, expected_type: &ValueType) -> Result<ScalarValue, VmError> {
	let extracted = match value {
		ScalarValue::Vec2U16(value) => value.get(index).copied().map(ScalarValue::U16),
		ScalarValue::Vec2I(value) => value.get(index).copied().map(ScalarValue::I32),
		ScalarValue::Vec2U(value) => value.get(index).copied().map(ScalarValue::U32),
		ScalarValue::Vec3U(value) => value.get(index).copied().map(ScalarValue::U32),
		ScalarValue::Vec4U(value) => value.get(index).copied().map(ScalarValue::U32),
		ScalarValue::Vec2F(value) => value.get(index).copied().map(ScalarValue::F32),
		ScalarValue::Vec3F(value) => value.get(index).copied().map(ScalarValue::F32),
		ScalarValue::Vec4F(value) => value.get(index).copied().map(ScalarValue::F32),
		ScalarValue::Mat4F(value) if index < 4 => Some(ScalarValue::Vec4F(
			value[index * 4..index * 4 + 4].try_into().expect("Matrix column size"),
		)),
		ScalarValue::Mat4x3F(value) if index < 4 => Some(ScalarValue::Vec3F(
			value[index * 3..index * 3 + 3].try_into().expect("Matrix column size"),
		)),
		ScalarValue::Struct { fields, .. } => fields.get(index).cloned(),
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

fn vector_scalar_type(value_type: &ValueType) -> Option<ValueType> {
	match value_type {
		ValueType::Vec2U16 => Some(ValueType::U16),
		ValueType::Vec2I => Some(ValueType::I32),
		ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => Some(ValueType::U32),
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F => Some(ValueType::F32),
		_ => None,
	}
}

fn multiply_mat4_vec4(matrix: [f32; 16], vector: [f32; 4]) -> [f32; 4] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 4 + row] * vector[column]).sum())
}

fn multiply_mat4(left: [f32; 16], right: [f32; 16]) -> [f32; 16] {
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

fn multiply_mat4x3_vec4(matrix: [f32; 12], vector: [f32; 4]) -> [f32; 3] {
	std::array::from_fn(|row| (0..4).map(|column| matrix[column * 3 + row] * vector[column]).sum())
}

fn expect_vec2u(value: ScalarValue) -> Result<[u32; 2], VmError> {
	let ScalarValue::Vec2U(value) = value else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::Vec2U.name().to_string(),
			found: value.value_type().name().to_string(),
		});
	};
	Ok(value)
}

fn expect_u32(value: ScalarValue) -> Result<u32, VmError> {
	let ScalarValue::U32(value) = value else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::U32.name().to_string(),
			found: value.value_type().name().to_string(),
		});
	};
	Ok(value)
}

fn dot_product<const N: usize>(left: [f32; N], right: [f32; N]) -> f32 {
	let mut value = 0.0;
	for index in 0..N {
		value += left[index] * right[index];
	}
	value
}

fn cross_product(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

fn normalize_vector<const N: usize>(value: [f32; N]) -> Result<[f32; N], VmError> {
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

fn reflect_vector<const N: usize>(incident: [f32; N], normal: [f32; N]) -> Result<[f32; N], VmError> {
	let scale = 2.0 * dot_product(incident, normal);
	let mut reflected = [0.0; N];
	for index in 0..N {
		reflected[index] = incident[index] - scale * normal[index];
	}
	Ok(reflected)
}

fn read_register(registers: &[Option<ScalarValue>], register: usize) -> Result<ScalarValue, VmError> {
	registers
		.get(register)
		.and_then(Option::clone)
		.ok_or(VmError::UninitializedRegister { register })
}

fn resolve_resource_slot(slot: DescriptorSlot, registers: &[Option<ScalarValue>]) -> Result<DescriptorSlot, VmError> {
	if slot.set() != DYNAMIC_RESOURCE_SET {
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

fn read_buffer_array_index(registers: &[Option<ScalarValue>], register: usize, count: usize) -> Result<usize, VmError> {
	let index = read_register(registers, register)?;
	let ScalarValue::U32(index) = index else {
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

#[derive(Debug, PartialEq, Eq)]
pub enum VmError {
	MissingMainFunction,
	UnsupportedRawCode,
	UnsupportedMainSignature {
		message: String,
	},
	UnsupportedType {
		type_name: String,
	},
	UnsupportedStatement {
		message: String,
	},
	UnsupportedAssignmentTarget {
		message: String,
	},
	UnsupportedExpression {
		message: String,
	},
	UnsupportedBufferLayout {
		message: String,
	},
	UnsupportedDescriptor {
		slot: DescriptorSlot,
		message: String,
	},
	DescriptorAccessDenied {
		slot: DescriptorSlot,
		access: &'static str,
	},
	DescriptorTypeMismatch {
		slot: DescriptorSlot,
		expected: &'static str,
		found: &'static str,
	},
	UnknownBufferMember {
		member: String,
	},
	UnboundDescriptor {
		slot: DescriptorSlot,
	},
	MissingPushConstant,
	MissingMeshOutputs,
	MeshOutputIndexOutOfBounds {
		kind: &'static str,
		index: usize,
		count: usize,
	},
	MissingSpecialization {
		name: String,
	},
	InstructionLimitExceeded {
		limit: usize,
	},
	CallDepthLimitExceeded {
		limit: usize,
	},
	CallArgumentMismatch {
		expected: usize,
		found: usize,
	},
	BufferAccessOutOfBounds {
		offset: usize,
		size: usize,
		buffer_size: usize,
	},
	BufferArrayIndexOutOfBounds {
		index: usize,
		count: usize,
	},
	TextureAccessOutOfBounds {
		x: u32,
		y: u32,
		z: u32,
		width: u32,
		height: u32,
		depth: u32,
	},
	InvalidTextureDimensions {
		width: u32,
		height: u32,
		depth: u32,
	},
	InvalidLiteral {
		value: String,
		value_type: String,
	},
	ArithmeticError {
		message: String,
	},
	TypeMismatch {
		expected: String,
		found: String,
	},
	UninitializedRegister {
		register: usize,
	},
	UninitializedLocal {
		local: usize,
	},
}

impl std::fmt::Display for VmError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			VmError::MissingMainFunction => {
				write!(
					f,
					"Missing main function. The most likely cause is that the lexed BESL program does not define `main`."
				)
			}
			VmError::UnsupportedRawCode => write!(
				f,
				"Raw code blocks are not supported. The most likely cause is that a reachable BESL function contains non-empty platform shader code with no portable VM semantics."
			),
			VmError::UnsupportedMainSignature { message } => write!(
				f,
				"Unsupported main signature: {}. The most likely cause is that the VM only accepts `main: fn () -> void` right now.",
				message
			),
			VmError::UnsupportedType { type_name } => write!(
				f,
				"Unsupported type `{}`. The most likely cause is that the BESL type has no portable VM value or resource representation.",
				type_name
			),
			VmError::UnsupportedStatement { message } => write!(
				f,
				"Unsupported statement. {}. The most likely cause is that this statement form has not been lowered into VM control-flow or side-effect instructions.",
				message
			),
			VmError::UnsupportedAssignmentTarget { message } => write!(
				f,
				"Unsupported assignment target. {}. The most likely cause is that the target is not a writable local, buffer member, output, or image operation.",
				message
			),
			VmError::UnsupportedExpression { message } => write!(
				f,
				"Unsupported expression. {}. The most likely cause is that this BESL syntax or operand-type combination has no VM lowering yet.",
				message
			),
			VmError::UnsupportedBufferLayout { message } => write!(
				f,
				"Unsupported buffer layout. {}. The most likely cause is that a member cannot be represented in the VM's packed CPU buffer layout.",
				message
			),
			VmError::UnsupportedDescriptor { slot, message } => write!(
				f,
				"Unsupported descriptor at set {} binding {}. {}. The most likely cause is a descriptor-kind mismatch, incompatible reused slot, or unsupported resource access.",
				slot.set(),
				slot.binding(),
				message
			),
			VmError::DescriptorAccessDenied { slot, access } => write!(
				f,
				"Descriptor access denied at set {} binding {}. The most likely cause is that the BESL binding was not declared with `{}` access.",
				slot.set(),
				slot.binding(),
				access
			),
			VmError::DescriptorTypeMismatch { slot, expected, found } => write!(
				f,
				"Descriptor type mismatch at set {} binding {}: expected `{}` but found `{}`. The most likely cause is that the host bound a different resource kind than the compiled BESL program requires.",
				slot.set(),
				slot.binding(),
				expected,
				found
			),
			VmError::UnknownBufferMember { member } => write!(
				f,
				"Unknown buffer member `{}`. The most likely cause is that the BESL accessor does not match the bound buffer layout.",
				member
			),
			VmError::UnboundDescriptor { slot } => write!(
				f,
				"Unbound descriptor at set {} binding {}. The most likely cause is that no buffer was bound into the descriptor slot before execution.",
				slot.set(),
				slot.binding()
			),
			VmError::MissingPushConstant => write!(
				f,
				"Missing push constant binding. The most likely cause is that the BESL program reads `push_constant` but the host did not bind any push constant data before execution."
			),
			VmError::MissingMeshOutputs => write!(
				f,
				"Missing mesh output capture. The most likely cause is that the BESL mesh shader ran without binding `MeshOutputs`."
			),
			VmError::MeshOutputIndexOutOfBounds { kind, index, count } => write!(
				f,
				"Mesh {kind} output index {index} exceeds {count} declared outputs. The most likely cause is that the shader wrote beyond the counts supplied to `set_mesh_output_counts`."
			),
			VmError::MissingSpecialization { name } => write!(
				f,
				"Missing specialization `{}`. The most likely cause is that the host did not provide a value for a specialization used by the BESL program.",
				name
			),
			VmError::InstructionLimitExceeded { limit } => write!(
				f,
				"VM instruction limit {} exceeded. The most likely cause is that the BESL program contains an unbounded loop or needs a larger explicit execution budget.",
				limit
			),
			VmError::CallDepthLimitExceeded { limit } => write!(
				f,
				"VM call-depth limit {} exceeded. The most likely cause is that the BESL program recurses without reaching a base case.",
				limit
			),
			VmError::CallArgumentMismatch { expected, found } => write!(
				f,
				"Function call argument mismatch: expected {} arguments but found {}. The most likely cause is that the BESL function call does not match the declared parameter list.",
				expected, found
			),
			VmError::BufferAccessOutOfBounds {
				offset,
				size,
				buffer_size,
			} => write!(
				f,
				"Buffer access out of bounds at byte {} for {} bytes in a {} byte buffer. The most likely cause is that the bound buffer does not match the compiled BESL buffer layout.",
				offset, size, buffer_size
			),
			VmError::BufferArrayIndexOutOfBounds { index, count } => write!(
				f,
				"Buffer array index {} is out of bounds for {} elements. The most likely cause is that the BESL program indexed a buffer array member outside its declared length.",
				index, count
			),
			VmError::TextureAccessOutOfBounds {
				x,
				y,
				z,
				width,
				height,
				depth,
			} => write!(
				f,
				"Texture access out of bounds at ({}, {}, {}) in a {}x{}x{} texture. The most likely cause is that the BESL program fetched a texel outside the bound texture dimensions.",
				x, y, z, width, height, depth
			),
			VmError::InvalidTextureDimensions { width, height, depth } => write!(
				f,
				"Invalid texture dimensions {}x{}x{}. The most likely cause is that the host created a texture with a zero dimension.",
				width, height, depth
			),
			VmError::InvalidLiteral { value, value_type } => write!(
				f,
				"Invalid literal `{}` for `{}`. The most likely cause is that the literal cannot be parsed as the target BESL scalar type.",
				value, value_type
			),
			VmError::ArithmeticError { message } => write!(
				f,
				"Invalid arithmetic operation. {}. The most likely cause is that the BESL program evaluated an unsupported numeric operation such as division or modulo by zero.",
				message
			),
			VmError::TypeMismatch { expected, found } => write!(
				f,
				"Type mismatch: expected `{}` but found `{}`. The most likely cause is that the BESL assignment mixes incompatible scalar types.",
				expected, found
			),
			VmError::UninitializedRegister { register } => write!(
				f,
				"Uninitialized register {}. The most likely cause is that the VM tried to use a register before any instruction wrote a value into it.",
				register
			),
			VmError::UninitializedLocal { local } => write!(
				f,
				"Uninitialized local {}. The most likely cause is that the BESL program read a local variable before assigning a value to it.",
				local
			),
		}
	}
}

impl std::error::Error for VmError {}

#[cfg(test)]
mod tests {
	use super::{
		input_slot, output_slot, reflect_vector, Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram,
		ExecutionConfig, MeshOutputs, SpecializationValues, Texture, Value, VmError,
	};
	use crate::{compile_to_besl, BindingTypes, Expressions, Node, NodeReference, Operators};

	fn read_f32s(buffer: &Buffer, count: usize) -> Vec<f32> {
		buffer
			.bytes()
			.chunks_exact(4)
			.take(count)
			.map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("Expected four bytes")))
			.collect()
	}

	fn read_u32s(buffer: &Buffer, count: usize) -> Vec<u32> {
		buffer
			.bytes()
			.chunks_exact(4)
			.take(count)
			.map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("Expected four bytes")))
			.collect()
	}

	fn compile_test_program(script: &str, root: Option<Node>) -> ExecutableProgram {
		let program = compile_to_besl(script, root).expect("Expected lexed program");
		ExecutableProgram::compile(program).expect("Expected runnable program")
	}

	fn compile_test_root_program(root: NodeReference) -> ExecutableProgram {
		ExecutableProgram::compile(root).expect("Expected runnable program")
	}

	fn buffer_for_slot(executable: &ExecutableProgram, slot: DescriptorSlot) -> Buffer {
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		Buffer::new(layout)
	}

	fn interface_buffer_for_input(executable: &ExecutableProgram, location: u8) -> Buffer {
		let layout = executable.input_layout(location).expect("Expected input layout").clone();
		Buffer::new(layout)
	}

	fn interface_buffer_for_output(executable: &ExecutableProgram, location: u8) -> Buffer {
		let layout = executable.output_layout(location).expect("Expected output layout").clone();
		Buffer::new(layout)
	}

	fn run_with_buffer(executable: &ExecutableProgram, slot: DescriptorSlot, buffer: &mut Buffer) {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	fn write_texture(texture: &mut Texture, texels: &[([u32; 2], [f32; 4])]) {
		for (coord, value) in texels {
			texture.write(*coord, *value).expect("Expected texture write to succeed");
		}
	}

	#[test]
	fn executable_program_runs_main_and_writes_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			buff.value = 42.0;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				0,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 0);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 42.0);
	}

	#[test]
	fn executable_program_reads_locals_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = 7.5;
			buff.value = value;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				1,
				3,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(1, 3);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
	}

	#[test]
	fn executable_program_reads_a_bound_buffer_inside_main() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = input.value;
			output.value = value;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"input",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type.clone()).into()],
				},
				0,
				7,
				true,
				false,
			)
			.into(),
		);

		root.add_child(
			Node::binding(
				"output",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				8,
				false,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let input_slot = DescriptorSlot::new(0, 7);
		let output_slot = DescriptorSlot::new(0, 8);
		let mut input = buffer_for_slot(&executable, input_slot);
		let mut output = buffer_for_slot(&executable, output_slot);
		input
			.write("value", Value::F32(9.25))
			.expect("Expected host buffer write to succeed");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot, &mut input);
			descriptors.bind_buffer(output_slot, &mut output);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(output.read_f32("value").expect("Expected f32 member"), 9.25);
	}

	#[test]
	fn executable_program_evaluates_addition_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = 7.5;
			buff.value = value + 4.5;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				1,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 1);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 12.0);
	}

	#[test]
	fn apply_arithmetic_supports_all_basic_scalar_operations() {
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::U32(2),
				&super::ScalarValue::U32(3)
			)
			.expect("Expected addition to succeed"),
			super::ScalarValue::U32(5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Subtract,
				&super::ScalarValue::I32(9),
				&super::ScalarValue::I32(4)
			)
			.expect("Expected subtraction to succeed"),
			super::ScalarValue::I32(5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Multiply,
				&super::ScalarValue::U16(6),
				&super::ScalarValue::U16(7)
			)
			.expect("Expected multiplication to succeed"),
			super::ScalarValue::U16(42)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Divide,
				&super::ScalarValue::F32(9.0),
				&super::ScalarValue::F32(2.0)
			)
			.expect("Expected division to succeed"),
			super::ScalarValue::F32(4.5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Modulo,
				&super::ScalarValue::U8(20),
				&super::ScalarValue::U8(6)
			)
			.expect("Expected modulo to succeed"),
			super::ScalarValue::U8(2)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::Vec3F([1.0, 2.0, 3.0]),
				&super::ScalarValue::Vec3F([4.0, 5.0, 6.0])
			)
			.expect("Expected vec3f addition to succeed"),
			super::ScalarValue::Vec3F([5.0, 7.0, 9.0])
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Multiply,
				&super::ScalarValue::Vec4F([1.0, 2.0, 3.0, 4.0]),
				&super::ScalarValue::F32(2.0)
			)
			.expect("Expected vec4f scalar broadcast to succeed"),
			super::ScalarValue::Vec4F([2.0, 4.0, 6.0, 8.0])
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::Mat4F([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,]),
				&super::ScalarValue::F32(1.0)
			)
			.expect("Expected mat4f scalar broadcast to succeed"),
			super::ScalarValue::Mat4F([2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,])
		);
	}

	#[test]
	fn executable_program_evaluates_vec3f_arithmetic_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let lhs: vec3f = vec3f(1.0, 2.0, 3.0);
			let rhs: vec3f = vec3f(4.0, 5.0, 6.0);
			buff.value = lhs + rhs;
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type.clone()).into()],
				},
				0,
				2,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 2);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 3), vec![5.0, 7.0, 9.0]);
	}

	#[test]
	fn executable_program_evaluates_vec4f_scalar_broadcast_arithmetic() {
		let script = r#"
		main: fn () -> void {
			let value: vec4f = vec4f(1.0, 2.0, 3.0, 4.0);
			buff.value = value * 2.0;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				3,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 3);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 4), vec![2.0, 4.0, 6.0, 8.0]);
	}

	#[test]
	fn executable_program_evaluates_mat4f_arithmetic_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let lhs: mat4f = mat4f(
				vec4f(1.0, 0.0, 0.0, 0.0),
				vec4f(0.0, 1.0, 0.0, 0.0),
				vec4f(0.0, 0.0, 1.0, 0.0),
				vec4f(0.0, 0.0, 0.0, 1.0)
			);
			let rhs: mat4f = mat4f(
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0)
			);
			buff.value = lhs + rhs;
		}
		"#;

		let mut root = Node::root();
		let mat4f_type = root.get_child("mat4f").expect("Expected mat4f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", mat4f_type).into()],
				},
				0,
				4,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 4);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(
			read_f32s(&buffer, 16),
			vec![2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,]
		);
	}

	#[test]
	fn executable_program_calls_function_with_parameters_and_return_value() {
		let script = r#"
		add: fn (lhs: f32, rhs: f32) -> f32 {
			return lhs + rhs;
		}

		main: fn () -> void {
			buff.value = add(3.0, 4.5);
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				5,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 5);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
	}

	#[test]
	fn executable_program_calls_function_and_returns_vec3f() {
		let script = r#"
		double: fn (value: vec3f) -> vec3f {
			return value * 2.0;
		}

		main: fn () -> void {
			buff.value = double(vec3f(1.0, 2.0, 3.0));
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type.clone()).into()],
				},
				0,
				6,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 6);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 3), vec![2.0, 4.0, 6.0]);
	}

	#[test]
	fn executable_program_fetches_texture_texels_into_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 0);
			buff.value = fetch(texture, coord);
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"texture",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				7,
				true,
				false,
			)
			.into(),
		);
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				8,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let texture_slot = DescriptorSlot::new(0, 7);
		let buffer_slot = DescriptorSlot::new(0, 8);
		let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
		let mut buffer = buffer_for_slot(&executable, buffer_slot);
		write_texture(
			&mut texture,
			&[
				([0, 0], [1.0, 0.0, 0.0, 1.0]),
				([1, 0], [0.0, 1.0, 0.0, 1.0]),
				([0, 1], [0.0, 0.0, 1.0, 1.0]),
				([1, 1], [1.0, 1.0, 1.0, 1.0]),
			],
		);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(texture_slot, &mut texture);
			descriptors.bind_buffer(buffer_slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 4), vec![0.0, 1.0, 0.0, 1.0]);
	}

	#[test]
	fn executable_program_samples_textures_inside_arithmetic_expressions() {
		let script = r#"
		main: fn () -> void {
			let color: vec4f = sample(texture_sampler, vec2f(0.5, 0.5));
			buff.value = color * 2.0;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"texture_sampler",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				9,
				true,
				false,
			)
			.into(),
		);
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				10,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let texture_slot = DescriptorSlot::new(0, 9);
		let buffer_slot = DescriptorSlot::new(0, 10);
		let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
		let mut buffer = buffer_for_slot(&executable, buffer_slot);
		write_texture(
			&mut texture,
			&[
				([0, 0], [0.0, 0.0, 0.0, 1.0]),
				([1, 0], [1.0, 0.0, 0.0, 1.0]),
				([0, 1], [0.0, 1.0, 0.0, 1.0]),
				([1, 1], [1.0, 1.0, 0.0, 1.0]),
			],
		);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(texture_slot, &mut texture);
			descriptors.bind_buffer(buffer_slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 4), vec![1.0, 1.0, 0.0, 2.0]);
	}

	#[test]
	fn executable_program_writes_a_pixel_to_a_bound_image() {
		let script = r#"
		main: fn () -> void {
			write(image, vec2u(1, 0), vec4f(0.25, 0.5, 0.75, 1.0));
		}
		"#;

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"image",
				BindingTypes::Image {
					format: "rgba8".to_string(),
				},
				0,
				11,
				false,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let image_slot = DescriptorSlot::new(0, 11);
		let mut image = Texture::new(2, 2).expect("Expected texture allocation");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_image(image_slot, &mut image);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(
			image.fetch([1, 0]).expect("Expected image fetch"),
			Value::Vec4F([0.25, 0.5, 0.75, 1.0])
		);
	}

	#[test]
	fn executable_program_reads_and_writes_buffer_array_elements() {
		let script = r#"
		main: fn () -> void {
			let index: u32 = 1;
			buff.values[index] = 7.5;
			buff.value = buff.values[index];
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![
						Node::array("values", float_type.clone(), 3),
						Node::member("value", float_type).into(),
					],
				},
				0,
				12,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 12);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout.clone());
		let values_member = layout.member("values").expect("Expected values member");
		buffer
			.write_value(
				values_member.offset() + values_member.value_type().size(),
				values_member.value_type(),
				&Value::F32(2.5),
			)
			.expect("Expected array element write to succeed");

		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 4), vec![0.0, 7.5, 0.0, 7.5]);
	}

	#[test]
	fn executable_program_reads_and_writes_same_named_buffer_members() {
		let script = r#"
		main: fn () -> void {
			pixel_mapping.pixel_mapping[0] = meshes.meshes[1];
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");

		root.add_children(vec![
			Node::binding(
				"meshes",
				BindingTypes::Buffer {
					members: vec![Node::array("meshes", u32_type.clone(), 2)],
				},
				0,
				24,
				true,
				false,
			)
			.into(),
			Node::binding(
				"pixel_mapping",
				BindingTypes::Buffer {
					members: vec![Node::array("pixel_mapping", u32_type, 2)],
				},
				0,
				25,
				false,
				true,
			)
			.into(),
		]);

		let executable = compile_test_program(script, Some(root));

		let input_slot = DescriptorSlot::new(0, 24);
		let output_slot = DescriptorSlot::new(0, 25);
		let input_layout = executable
			.buffer_layout(input_slot)
			.expect("Expected input buffer layout")
			.clone();
		let mut input = Buffer::new(input_layout.clone());
		let meshes_member = input_layout.member("meshes").expect("Expected meshes member");
		input
			.write_value(
				meshes_member.offset() + meshes_member.value_type().size(),
				meshes_member.value_type(),
				&Value::U32(42),
			)
			.expect("Expected array element write to succeed");

		let mut output = buffer_for_slot(&executable, output_slot);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot, &mut input);
			descriptors.bind_buffer(output_slot, &mut output);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_u32s(&output, 2), vec![42, 0]);
	}

	#[test]
	fn executable_program_compile_rejects_raw_code_blocks() {
		let script = r#"
		main: fn () -> void {}
		"#;

		let program = compile_to_besl(script, None).expect("Expected lexed program");
		let main = program.get_descendant("main").expect("Expected main function");
		main.borrow_mut()
			.add_child(Node::raw(Some("gl_Position = vec4(0);".to_string()), None, None, vec![], vec![]).into());

		match ExecutableProgram::compile(program) {
			Err(error) => assert_eq!(error, super::VmError::UnsupportedRawCode),
			Ok(_) => panic!("Expected raw code rejection"),
		}
	}

	#[test]
	fn executable_program_reads_push_constant_members() {
		let script = r#"
		main: fn () -> void {
			buff.value = push_constant.material_id;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(Node::push_constant(vec![Node::member("material_id", float_type.clone()).into()]).into());
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				14,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 14);
		let push_constant_layout = executable
			.push_constant_layout()
			.expect("Expected push constant layout")
			.clone();
		let mut buffer = buffer_for_slot(&executable, slot);
		let mut push_constant = Buffer::new(push_constant_layout);
		push_constant
			.write("material_id", Value::F32(3.5))
			.expect("Expected push constant write");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			descriptors.bind_push_constant(&mut push_constant);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read_f32("value").expect("Expected f32"), 3.5);
	}

	#[test]
	fn executable_program_requires_bound_push_constant() {
		let script = r#"
		main: fn () -> void {
			buff.value = push_constant.material_id;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(Node::push_constant(vec![Node::member("material_id", float_type.clone()).into()]).into());
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				15,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 15);
		let mut buffer = buffer_for_slot(&executable, slot);

		let error = {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable
				.run_main(&mut descriptors)
				.expect_err("Expected missing push constant error")
		};

		assert_eq!(error, VmError::MissingPushConstant);
	}

	#[test]
	fn executable_program_writes_vertex_shader_output_interfaces() {
		let script = r#"
		main: fn () -> void {
			out_position = vec4f(1.0, 2.0, 3.0, 1.0);
			out_instance_index = 7;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(Node::output("out_position", vec4f_type, 0).into());
		root.add_child(Node::output("out_instance_index", u32_type, 1).into());

		let executable = compile_test_program(script, Some(root));

		let mut position = interface_buffer_for_output(&executable, 0);
		let mut instance = interface_buffer_for_output(&executable, 1);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(output_slot(0), &mut position);
			descriptors.bind_buffer(output_slot(1), &mut instance);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&position, 4), vec![1.0, 2.0, 3.0, 1.0]);
		assert_eq!(
			instance.read("out_instance_index").expect("Expected output value"),
			Value::U32(7)
		);
	}

	#[test]
	fn executable_program_reads_vertex_shader_input_interfaces() {
		let script = r#"
		main: fn () -> void {
			out_color = in_color;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
		root.add_child(Node::input("in_color", vec4f_type.clone(), 0).into());
		root.add_child(Node::output("out_color", vec4f_type, 0).into());

		let executable = compile_test_program(script, Some(root));

		let mut input = interface_buffer_for_input(&executable, 0);
		let mut output = interface_buffer_for_output(&executable, 0);
		input
			.write("in_color", Value::Vec4F([0.1, 0.2, 0.3, 1.0]))
			.expect("Expected input write to succeed");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut input);
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(
			output.read("out_color").expect("Expected output value"),
			Value::Vec4F([0.1, 0.2, 0.3, 1.0])
		);
	}

	#[test]
	fn executable_program_rejects_writing_to_input_interfaces() {
		let script = r#"
		main: fn () -> void {
			in_color = vec4f(1.0, 0.0, 0.0, 1.0);
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
		root.add_child(Node::input("in_color", vec4f_type, 0).into());

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let error = match ExecutableProgram::compile(program) {
			Err(error) => error,
			Ok(_) => panic!("Expected input write rejection"),
		};

		assert!(matches!(
			error,
			VmError::UnsupportedAssignmentTarget { .. } | VmError::UnsupportedExpression { .. }
		));
	}

	#[test]
	fn executable_program_rejects_reading_from_output_interfaces() {
		let script = r#"
		main: fn () -> void {
			let color: vec4f = out_color;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
		root.add_child(Node::output("out_color", vec4f_type, 0).into());

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let error = match ExecutableProgram::compile(program) {
			Err(error) => error,
			Ok(_) => panic!("Expected output read rejection"),
		};

		assert!(matches!(error, VmError::UnsupportedExpression { .. }));
	}

	#[test]
	fn executable_program_supports_vertex_to_fragment_interface_workflows() {
		let vertex_script = r#"
		main: fn () -> void {
			out_color = in_color * 0.5;
		}
		"#;
		let fragment_script = r#"
		main: fn () -> void {
			out_color = in_color + vec4f(0.25, 0.0, 0.0, 0.0);
		}
		"#;

		let mut vertex_root = Node::root();
		let vertex_vec4f = vertex_root.get_child("vec4f").expect("Expected vec4f");
		vertex_root.add_child(Node::input("in_color", vertex_vec4f.clone(), 0).into());
		vertex_root.add_child(Node::output("out_color", vertex_vec4f, 0).into());

		let mut fragment_root = Node::root();
		let fragment_vec4f = fragment_root.get_child("vec4f").expect("Expected vec4f");
		fragment_root.add_child(Node::input("in_color", fragment_vec4f.clone(), 0).into());
		fragment_root.add_child(Node::output("out_color", fragment_vec4f, 0).into());

		let vertex_executable = compile_test_program(vertex_script, Some(vertex_root));
		let fragment_executable = compile_test_program(fragment_script, Some(fragment_root));

		let mut vertex_input = interface_buffer_for_input(&vertex_executable, 0);
		let mut vertex_output = interface_buffer_for_output(&vertex_executable, 0);
		let mut fragment_output = interface_buffer_for_output(&fragment_executable, 0);

		vertex_input
			.write("in_color", Value::Vec4F([0.8, 0.4, 0.2, 1.0]))
			.expect("Expected vertex input write");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut vertex_input);
			descriptors.bind_buffer(output_slot(0), &mut vertex_output);
			vertex_executable
				.run_main(&mut descriptors)
				.expect("Expected vertex execution to succeed");
		}

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut vertex_output);
			descriptors.bind_buffer(output_slot(0), &mut fragment_output);
			fragment_executable
				.run_main(&mut descriptors)
				.expect("Expected fragment execution to succeed");
		}

		assert_eq!(
			fragment_output.read("out_color").expect("Expected fragment output"),
			Value::Vec4F([0.65, 0.2, 0.1, 0.5])
		);
	}

	#[test]
	fn executable_program_supports_fragment_style_input_attachments() {
		let script = r#"
		main: fn () -> void {
			out_color = fetch(input_attachment, vec2u(1, 0));
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
		root.add_child(
			Node::binding(
				"input_attachment",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				16,
				true,
				false,
			)
			.into(),
		);
		root.add_child(Node::output("out_color", vec4f_type, 0).into());

		let executable = compile_test_program(script, Some(root));

		let input_attachment_slot = DescriptorSlot::new(0, 16);
		let mut input_attachment = Texture::new(2, 1).expect("Expected attachment allocation");
		let mut output = interface_buffer_for_output(&executable, 0);
		write_texture(
			&mut input_attachment,
			&[([0, 0], [0.1, 0.2, 0.3, 1.0]), ([1, 0], [0.9, 0.4, 0.2, 1.0])],
		);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(input_attachment_slot, &mut input_attachment);
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Expected fragment-style execution to succeed");
		}

		assert_eq!(
			output.read("out_color").expect("Expected output value"),
			Value::Vec4F([0.9, 0.4, 0.2, 1.0])
		);
	}

	#[test]
	fn executable_program_evaluates_dot_intrinsics() {
		let script = r#"
		main: fn () -> void {
			buff.value = dot(vec3f(1.0, 2.0, 3.0), vec3f(4.0, 5.0, 6.0));
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				17,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 17);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 32.0);
	}

	#[test]
	fn executable_program_evaluates_cross_intrinsics() {
		let script = r#"
		main: fn () -> void {
			buff.value = cross(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type.clone()).into()],
				},
				0,
				18,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 18);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 3), vec![0.0, 0.0, 1.0]);
	}

	#[test]
	fn executable_program_evaluates_length_intrinsics() {
		let script = r#"
		main: fn () -> void {
			buff.value = length(vec3f(3.0, 4.0, 0.0));
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				19,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 19);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 5.0);
	}

	#[test]
	fn executable_program_evaluates_normalize_intrinsics() {
		let script = r#"
		main: fn () -> void {
			buff.value = normalize(vec3f(3.0, 4.0, 0.0));
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type.clone()).into()],
				},
				0,
				20,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 20);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 3), vec![0.6, 0.8, 0.0]);
	}

	#[test]
	fn executable_program_evaluates_reflect_intrinsics() {
		let mut root = Node::root();
		let void_type = root.get_child("void").expect("Expected void");
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
		let reflect = root.get_child("reflect").expect("Expected reflect intrinsic");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type.clone()).into()],
				},
				0,
				21,
				true,
				true,
			)
			.into(),
		);
		root.add_child(
			Node::function(
				"main",
				Vec::new(),
				void_type,
				vec![Node::expression(Expressions::Operator {
					operator: Operators::Assignment,
					left: Node::expression(Expressions::Accessor {
						left: Node::expression(Expressions::Member {
							name: "buff".to_string(),
							source: root.get_child("buff").expect("Expected buff binding"),
						})
						.into(),
						right: Node::expression(Expressions::Member {
							name: "value".to_string(),
							source: root.get_child("buff").expect("Expected buff binding"),
						})
						.into(),
					})
					.into(),
					right: Node::expression(Expressions::IntrinsicCall {
						intrinsic: reflect,
						arguments: vec![
							Node::expression(Expressions::FunctionCall {
								function: vec3f_type.clone(),
								parameters: vec![
									Node::expression(Expressions::Literal {
										value: "1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "-1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
								],
							})
							.into(),
							Node::expression(Expressions::FunctionCall {
								function: vec3f_type.clone(),
								parameters: vec![
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
								],
							})
							.into(),
						],
						elements: vec![],
					})
					.into(),
				})
				.into()],
			)
			.into(),
		);

		let executable = compile_test_root_program(root.into());

		let slot = DescriptorSlot::new(0, 21);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(read_f32s(&buffer, 3), vec![1.0, 1.0, 0.0]);
	}

	#[test]
	fn executable_program_reads_thread_idx_for_compute_style_workflows() {
		let script = r#"
		main: fn () -> void {
			buff.thread = thread_idx();
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("thread", u32_type).into()],
				},
				0,
				22,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 22);
		let mut buffer = buffer_for_slot(&executable, slot);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read("thread").expect("Expected thread value"), Value::U32(0));
	}

	#[test]
	fn executable_program_reads_thread_idx_for_mesh_style_workflows() {
		let script = r#"
		main: fn () -> void {
			payload.thread = thread_idx();
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(
			Node::binding(
				"payload",
				BindingTypes::Buffer {
					members: vec![Node::member("thread", u32_type).into()],
				},
				0,
				23,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 23);
		let mut buffer = buffer_for_slot(&executable, slot);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read("thread").expect("Expected thread value"), Value::U32(0));
	}

	#[test]
	fn executable_program_executes_for_loops() {
		let script = r#"
		main: fn () -> void {
			let sum: u32 = 0;
			for (let i: u32 = 0; i < 4; i = i + 1) {
				sum = sum + i;
			}
			buff.sum = sum;
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("sum", u32_type).into()],
				},
				0,
				24,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 24);
		let mut buffer = buffer_for_slot(&executable, slot);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read("sum").expect("Expected sum value"), Value::U32(6));
	}

	#[test]
	fn executable_program_executes_continue_and_comparisons() {
		let script = r#"
		main: fn () -> void {
			let sum: u32 = 0;
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
				sum = sum + i;
			}
			buff.sum = sum;
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("sum", u32_type).into()],
				},
				0,
				25,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));

		let slot = DescriptorSlot::new(0, 25);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		assert_eq!(buffer.read("sum").expect("Expected sum value"), Value::U32(1));
	}

	#[test]
	fn executable_program_evaluates_scalar_math_intrinsics() {
		let script = r#"
		main: fn () -> void {
			buff.abs_value = abs(0.0 - 2.5);
			buff.sqrt_value = sqrt(9.0);
			buff.exp_value = exp(1.0);
			buff.sin_value = sin(0.0);
			buff.cos_value = cos(0.0);
			buff.tan_value = tan(0.0);
			buff.fract_value = fract(1.25);
			buff.radians_value = radians(180.0);
			buff.inverse_sqrt_value = inversesqrt(4.0);
			buff.smoothstep_value = smoothstep(0.0, 1.0, 0.5);
			buff.mix_value = mix(2.0, 4.0, 0.25);
		}
		"#;

		let mut root = Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![
						Node::member("abs_value", f32_type.clone()).into(),
						Node::member("sqrt_value", f32_type.clone()).into(),
						Node::member("exp_value", f32_type.clone()).into(),
						Node::member("sin_value", f32_type.clone()).into(),
						Node::member("cos_value", f32_type.clone()).into(),
						Node::member("tan_value", f32_type.clone()).into(),
						Node::member("fract_value", f32_type.clone()).into(),
						Node::member("radians_value", f32_type.clone()).into(),
						Node::member("inverse_sqrt_value", f32_type.clone()).into(),
						Node::member("smoothstep_value", f32_type.clone()).into(),
						Node::member("mix_value", f32_type).into(),
					],
				},
				0,
				26,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));
		let slot = DescriptorSlot::new(0, 26);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		let values = read_f32s(&buffer, 11);
		assert!((values[0] - 2.5).abs() < 1e-6);
		assert!((values[1] - 3.0).abs() < 1e-6);
		assert!((values[2] - std::f32::consts::E).abs() < 1e-5);
		assert!(values[3].abs() < 1e-6);
		assert!((values[4] - 1.0).abs() < 1e-6);
		assert!(values[5].abs() < 1e-6);
		assert!((values[6] - 0.25).abs() < 1e-6);
		assert!((values[7] - std::f32::consts::PI).abs() < 1e-6);
		assert!((values[8] - 0.5).abs() < 1e-6);
		assert!((values[9] - 0.5).abs() < 1e-6);
		assert!((values[10] - 2.5).abs() < 1e-6);
	}

	#[test]
	fn executable_program_evaluates_scalar_max_and_clamp() {
		let script = r#"
		main: fn () -> void {
			buff.max_value = max(1.5, 2.5);
			buff.clamp_value = clamp(1.5, 0.0, 1.0);
		}
		"#;

		let mut root = Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![
						Node::member("max_value", f32_type.clone()).into(),
						Node::member("clamp_value", f32_type).into(),
					],
				},
				0,
				27,
				true,
				true,
			)
			.into(),
		);

		let executable = compile_test_program(script, Some(root));
		let slot = DescriptorSlot::new(0, 27);
		let mut buffer = buffer_for_slot(&executable, slot);
		run_with_buffer(&executable, slot, &mut buffer);

		let values = read_f32s(&buffer, 2);
		assert!((values[0] - 2.5).abs() < 1e-6);
		assert!((values[1] - 1.0).abs() < 1e-6);
	}

	#[test]
	fn execution_limit_stops_an_infinite_loop() {
		let executable = compile_test_program(
			r#"
			main: fn () -> void {
				for (let i: u32 = 0; i >= 0; i = i + 1) {
					i = i;
				}
			}
			"#,
			None,
		);
		let mut descriptors = DescriptorBindings::new();
		let error = executable
			.run_main_with_config(&mut descriptors, &ExecutionConfig::new(32))
			.expect_err("An infinite loop must exhaust its explicit instruction budget");
		assert_eq!(error, VmError::InstructionLimitExceeded { limit: 32 });
	}

	#[test]
	fn reflect_preserves_the_exact_non_unit_normal_semantics() {
		assert_eq!(
			reflect_vector([1.0, 2.0], [2.0, 0.0]).expect("Reflect is defined for every finite normal"),
			[-7.0, 2.0]
		);
	}

	#[test]
	fn texture_descriptor_handles_flow_through_function_parameters() {
		let script = r#"
		read_source: fn (source: Texture2D) -> vec4f {
			return fetch(source, vec2u(0, 0));
		}
		main: fn () -> void {
			result.color = read_source(source_texture);
		}
		"#;
		let mut root = Node::root();
		let vec4f = root.get_child("vec4f").expect("Expected vec4f");
		root.add_children(vec![
			Node::binding(
				"source_texture",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				30,
				true,
				false,
			)
			.into(),
			Node::binding(
				"result",
				BindingTypes::Buffer {
					members: vec![Node::member("color", vec4f).into()],
				},
				0,
				31,
				true,
				true,
			)
			.into(),
		]);
		let executable = compile_test_program(script, Some(root));
		let mut texture = Texture::new(1, 1).expect("Expected texture");
		texture.write([0, 0], [0.25, 0.5, 0.75, 1.0]).expect("Expected texel write");
		let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 31));
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(DescriptorSlot::new(0, 30), &mut texture);
			descriptors.bind_buffer(DescriptorSlot::new(0, 31), &mut result);
			executable
				.run_main(&mut descriptors)
				.expect("Expected descriptor-handle execution");
		}
		assert_eq!(
			result.read("color").expect("Expected color"),
			Value::Vec4F([0.25, 0.5, 0.75, 1.0])
		);
	}

	#[test]
	fn dynamic_const_array_indices_select_runtime_elements() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.25, 0.5, 0.75);
		main: fn () -> void {
			let index: u32 = 2;
			result.value = WEIGHTS[index];
		}
		"#;
		let mut root = Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"result",
				BindingTypes::Buffer {
					members: vec![Node::member("value", f32_type).into()],
				},
				0,
				32,
				true,
				true,
			)
			.into(),
		);
		let executable = compile_test_program(script, Some(root));
		let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 32));
		run_with_buffer(&executable, DescriptorSlot::new(0, 32), &mut result);
		assert_eq!(result.read("value").expect("Expected selected weight"), Value::F32(0.75));
	}

	#[test]
	fn mesh_intrinsics_capture_geometry_and_indexed_outputs() {
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(1, 1);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 0, 0));
			out_index[0] = 17;
		}
		"#;
		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		root.add_child(Node::output_array("out_index", u32_type, 0, 1).into());
		let executable = compile_test_program(script, Some(root));
		let mut output = interface_buffer_for_output(&executable, 0);
		let mut mesh_outputs = MeshOutputs::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(output_slot(0), &mut output);
			descriptors.bind_mesh_outputs(&mut mesh_outputs);
			executable
				.run_main(&mut descriptors)
				.expect("Expected mesh capture execution");
		}
		assert_eq!(mesh_outputs.vertex_count(), 1);
		assert_eq!(mesh_outputs.primitive_count(), 1);
		assert_eq!(mesh_outputs.vertex_position(0), Some([1.0, 2.0, 3.0, 1.0]));
		assert_eq!(mesh_outputs.triangle(0), Some([0, 0, 0]));
		assert_eq!(
			output.read_indexed("out_index", 0).expect("Expected indexed output"),
			Value::U32(17)
		);
	}

	#[test]
	fn specialization_values_select_x_and_y_components() {
		for (axis, expected) in [([1.0, 0.0], 1.0), ([0.0, 1.0], 2.0)] {
			let script = r#"
			main: fn () -> void {
				result.value = axis.x + axis.y * 2.0;
			}
			"#;
			let mut root = Node::root();
			let vec2f = root.get_child("vec2f").expect("Expected vec2f");
			let f32_type = root.get_child("f32").expect("Expected f32");
			root.add_children(vec![
				Node::specialization("axis", vec2f).into(),
				Node::binding(
					"result",
					BindingTypes::Buffer {
						members: vec![Node::member("value", f32_type).into()],
					},
					0,
					33,
					true,
					true,
				)
				.into(),
			]);
			let program = compile_to_besl(script, Some(root)).expect("Expected lexed specialization program");
			let mut specializations = SpecializationValues::new();
			specializations.set("axis", Value::Vec2F(axis));
			let executable = ExecutableProgram::compile_with_specializations(program, &specializations)
				.expect("Expected specialized executable");
			let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 33));
			run_with_buffer(&executable, DescriptorSlot::new(0, 33), &mut result);
			assert_eq!(
				result.read("value").expect("Expected specialization result"),
				Value::F32(expected)
			);
		}
	}
}
