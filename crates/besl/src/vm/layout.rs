//! VM resource-slot and packed-memory layout contracts.

use super::*;

/// The `ResourceSlot` struct provides a stable flat lookup key for host resources and VM interface resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceSlot {
	slot: u32,
	// The kind keeps internal VM namespaces distinct from host resources that use the same numeric slot.
	kind: ResourceSlotKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ResourceSlotKind {
	Resource,
	PushConstant,
	DynamicResource,
	BuiltinPosition,
	BuiltinVertexIndex,
	BuiltinInstanceIndex,
	Input,
	Output,
}

impl ResourceSlot {
	pub const fn new(slot: u32) -> Self {
		Self {
			slot,
			kind: ResourceSlotKind::Resource,
		}
	}

	pub const fn slot(&self) -> u32 {
		self.slot
	}

	const fn virtual_slot(slot: u32, kind: ResourceSlotKind) -> Self {
		Self { slot, kind }
	}

	pub(super) const fn is_dynamic_resource(&self) -> bool {
		matches!(self.kind, ResourceSlotKind::DynamicResource)
	}
}

pub(super) const PUSH_CONSTANT_SLOT: ResourceSlot = ResourceSlot::virtual_slot(0, ResourceSlotKind::PushConstant);

pub const fn input_slot(location: u8) -> ResourceSlot {
	ResourceSlot::virtual_slot(location as u32, ResourceSlotKind::Input)
}

pub const fn output_slot(location: u8) -> ResourceSlot {
	ResourceSlot::virtual_slot(location as u32, ResourceSlotKind::Output)
}

/// Returns the interface slot reserved for the vertex position builtin.
pub const fn builtin_position_slot() -> ResourceSlot {
	ResourceSlot::virtual_slot(0, ResourceSlotKind::BuiltinPosition)
}

/// Returns the VM slot that supplies the implicit `vertex_index` value.
pub const fn builtin_vertex_index_slot() -> ResourceSlot {
	ResourceSlot::virtual_slot(0, ResourceSlotKind::BuiltinVertexIndex)
}

/// Returns the VM slot that supplies the implicit `instance_index` value.
pub const fn builtin_instance_index_slot() -> ResourceSlot {
	ResourceSlot::virtual_slot(0, ResourceSlotKind::BuiltinInstanceIndex)
}

pub(super) fn dynamic_resource_slot(register: usize) -> ResourceSlot {
	ResourceSlot::virtual_slot(
		u32::try_from(register).expect(
			"Invalid VM resource register. The most likely cause is that compilation produced more registers than the flat slot representation can address.",
		),
		ResourceSlotKind::DynamicResource,
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
	F16,
	F32,
	Vec2U16,
	Vec4U16,
	Vec2I,
	Vec3U,
	Vec2U,
	Vec4U,
	Vec2F16,
	Vec3F16,
	Vec4F16,
	Vec2F,
	Vec3F,
	Vec4F,
	Mat4F,
	Mat4x3F,
	Texture2D,
	Texture3D,
	TextureCube,
	TextureCubeArray,
	ArrayTexture2D,
	Struct {
		name: String,
		fields: Vec<BufferMemberLayout>,
		size: usize,
	},
}

/// Pairs each fieldless [`ValueType`] with its BESL type name. [`ValueType::name`] and [`ValueType::builtin`] both
/// read it, so the two directions cannot drift apart. Add new built-in value types here.
static BUILTIN_VALUE_TYPES: [(&str, ValueType); 26] = [
	("bool", ValueType::Bool),
	("u8", ValueType::U8),
	("u16", ValueType::U16),
	("u32", ValueType::U32),
	("i32", ValueType::I32),
	("f16", ValueType::F16),
	("f32", ValueType::F32),
	("vec2u16", ValueType::Vec2U16),
	("vec4u16", ValueType::Vec4U16),
	("vec2i", ValueType::Vec2I),
	("vec3u", ValueType::Vec3U),
	("vec2u", ValueType::Vec2U),
	("vec4u", ValueType::Vec4U),
	("vec2f16", ValueType::Vec2F16),
	("vec3f16", ValueType::Vec3F16),
	("vec4f16", ValueType::Vec4F16),
	("vec2f", ValueType::Vec2F),
	("vec3f", ValueType::Vec3F),
	("vec4f", ValueType::Vec4F),
	("mat4f", ValueType::Mat4F),
	("mat4x3f", ValueType::Mat4x3F),
	("Texture2D", ValueType::Texture2D),
	("Texture3D", ValueType::Texture3D),
	("TextureCube", ValueType::TextureCube),
	("TextureCubeArray", ValueType::TextureCubeArray),
	("ArrayTexture2D", ValueType::ArrayTexture2D),
];

impl ValueType {
	pub const fn size(&self) -> usize {
		match self {
			ValueType::Bool => 1,
			ValueType::U8 => 1,
			ValueType::U16 => 2,
			ValueType::F16 => 2,
			ValueType::U32 | ValueType::I32 | ValueType::F32 => 4,
			ValueType::Vec2U16 => 4,
			ValueType::Vec4U16 => 8,
			ValueType::Vec2I => 8,
			ValueType::Vec2U | ValueType::Vec2F => 8,
			ValueType::Vec3U => 12,
			ValueType::Vec4U | ValueType::Vec4F => 16,
			ValueType::Vec2F16 => 4,
			ValueType::Vec3F16 => 6,
			ValueType::Vec4F16 => 8,
			ValueType::Vec3F => 12,
			ValueType::Mat4F => 64,
			ValueType::Mat4x3F => 48,
			ValueType::Texture2D
			| ValueType::Texture3D
			| ValueType::TextureCube
			| ValueType::TextureCubeArray
			| ValueType::ArrayTexture2D => 0,
			ValueType::Struct { size, .. } => *size,
		}
	}

	/// Names this type the way BESL source spells it.
	pub(super) fn name(&self) -> &str {
		match self {
			ValueType::Struct { name, .. } => name,
			builtin => {
				BUILTIN_VALUE_TYPES
					.iter()
					.find(|(_, value_type)| value_type == builtin)
					.expect("Every fieldless ValueType has a BUILTIN_VALUE_TYPES entry")
					.0
			}
		}
	}

	/// Resolves a built-in BESL type name, or returns `None` for struct and unknown names.
	pub(super) fn builtin(name: &str) -> Option<Self> {
		BUILTIN_VALUE_TYPES
			.iter()
			.find(|(builtin_name, _)| *builtin_name == name)
			.map(|(_, value_type)| value_type.clone())
	}

	pub(super) fn field(&self, name: &str) -> Option<&BufferMemberLayout> {
		match self {
			ValueType::Struct { fields, .. } => fields.iter().find(|field| field.name() == name),
			_ => None,
		}
	}
}

/// The `BufferMemberLayout` struct defines how host code addresses one named member in packed VM memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferMemberLayout {
	pub(super) name: String,
	pub(super) offset: usize,
	pub(super) value_type: ValueType,
	pub(super) count: usize,
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

	pub(super) fn element_offset(&self, index: usize) -> Result<usize, VmError> {
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
	pub(super) members: Vec<BufferMemberLayout>,
	pub(super) size: usize,
	/// The element type of a scalar or vector array, such as `u16[]`, which has no named members.
	pub(super) element: Option<ValueType>,
	/// Elements a fixed-size array buffer holds. [`super::Buffer::new`] allocates this many; runtime arrays use
	/// [`super::Buffer::new_array`].
	pub(super) element_count: Option<usize>,
}

impl BufferLayout {
	/// Describes one element of a scalar or vector runtime array.
	pub(super) fn scalar_element(value_type: ValueType) -> Self {
		Self {
			members: Vec::new(),
			size: value_type.size(),
			element: Some(value_type),
			element_count: None,
		}
	}

	pub fn members(&self) -> &[BufferMemberLayout] {
		&self.members
	}

	pub const fn size(&self) -> usize {
		self.size
	}

	pub(super) fn member(&self, name: &str) -> Option<&BufferMemberLayout> {
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
