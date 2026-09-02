use crate::{DataTypes, ShaderHandle, ShaderTypes};

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

#[derive(Clone)]
pub struct SpecializationMapEntry {
	pub(crate) r#type: String,
	pub(crate) constant_id: u32,
	pub(crate) value: Box<[u8]>,
}

impl SpecializationMapEntry {
	pub fn new<T: bytemuck::NoUninit + 'static>(constant_id: u32, r#type: String, value: T) -> Self {
		if r#type == "vec4f" {
			assert_eq!(std::mem::size_of::<T>(), 16);
		}

		Self {
			r#type,
			constant_id,
			value: bytemuck::bytes_of(&value).into(),
		}
	}

	pub fn get_constant_id(&self) -> u32 {
		self.constant_id
	}

	pub fn get_type(&self) -> String {
		self.r#type.clone()
	}

	pub fn get_size(&self) -> usize {
		std::mem::size_of_val(&self.value)
	}

	pub fn get_data(&self) -> &[u8] {
		// SAFETY: We know that the data is valid for the lifetime of the specialization map entry.
		self.value.as_ref()
	}
}
