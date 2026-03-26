use crate::AccessPolicies;

/// Possible types of a shader source
pub enum Sources<'a> {
	/// SPIR-V binary
	SPIRV(&'a [u8]),
	/// Metal shading language source and entry-point name
	MTL { source: &'a str, entry_point: &'a str },
}

#[derive(Clone, Copy)]
pub struct BindingDescriptor {
	pub(crate) set: u32,
	pub(crate) binding: u32,
	pub(crate) access: AccessPolicies,
}

impl BindingDescriptor {
	pub fn new(set: u32, binding: u32, access: AccessPolicies) -> Self {
		Self { set, binding, access }
	}
}
