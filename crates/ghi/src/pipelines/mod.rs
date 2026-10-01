use crate::{DataTypes, ShaderHandle, ShaderTypes, pod};

pub mod compute;

pub mod raster;
pub mod ray_tracing;

#[derive(Clone, Hash)]
pub struct VertexElement<'a> {
	pub(crate) name: &'a str,
	pub(crate) format: DataTypes,
	pub(crate) binding: u32,
}

impl<'a> VertexElement<'a> {
	pub const fn new(name: &'a str, format: DataTypes, binding: u32) -> Self {
		Self { name, format, binding }
	}
}

#[derive(Clone, Copy)]
pub struct ShaderParameter<'a> {
	pub(crate) handle: &'a ShaderHandle,
	pub(crate) stage: ShaderTypes,
	pub(crate) specialization_map: &'a [SpecializationMapEntry],
}

impl<'a> ShaderParameter<'a> {
	pub fn new(handle: &'a ShaderHandle, stage: ShaderTypes) -> Self {
		Self {
			handle,
			stage,
			specialization_map: &[],
		}
	}

	pub fn with_specialization_map(mut self, specialization_map: &'a [SpecializationMapEntry]) -> Self {
		self.specialization_map = specialization_map;
		self
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PushConstantRange {
	pub(crate) offset: u32,
	pub(crate) size: u32,
}

impl PushConstantRange {
	pub fn new(offset: u32, size: u32) -> Self {
		Self { offset, size }
	}
}

/// A value a shader can take as a specialization constant, named by its shader type.
pub trait SpecializationConstant: bytemuck::NoUninit {
	const TYPE: &'static str;
}

macro_rules! specialization_constant {
	($($value:ty => $name:literal),+ $(,)?) => {
		$(impl SpecializationConstant for $value {
			const TYPE: &'static str = $name;
		})+
	};
}

// Specialization constants are passed by value rather than read from a buffer, so `bool` travels as Rust's one-byte bool.
specialization_constant!(
	bool => "bool",
	pod::I32 => "i32",
	pod::U32 => "u32",
	pod::F32 => "f32",
	pod::Vec2f => "vec2f",
	pod::Vec3f => "vec3f",
	pod::Vec4f => "vec4f",
);

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SpecializationMapEntry {
	pub(crate) r#type: &'static str,
	pub(crate) constant_id: u32,
	pub(crate) value: Box<[u8]>,
}

impl SpecializationMapEntry {
	pub fn new<T: SpecializationConstant>(constant_id: u32, value: T) -> Self {
		Self {
			r#type: T::TYPE,
			constant_id,
			value: bytemuck::bytes_of(&value).into(),
		}
	}

	pub fn get_constant_id(&self) -> u32 {
		self.constant_id
	}

	pub fn get_type(&self) -> String {
		self.r#type.to_string()
	}

	/// Returns the byte size of the constant's value.
	pub fn get_size(&self) -> usize {
		self.value.len()
	}

	pub fn get_data(&self) -> &[u8] {
		// SAFETY: We know that the data is valid for the lifetime of the specialization map entry.
		self.value.as_ref()
	}
}
