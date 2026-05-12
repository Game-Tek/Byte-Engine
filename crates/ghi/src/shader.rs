use utils::Extent;

use crate::AccessPolicies;

/// Possible types of a shader source
pub enum Sources<'a> {
	/// SPIR-V binary
	SPIRV(&'a [u8]),
	/// Compiled Metal library bytes and entry-point name
	MTLB {
		binary: &'a [u8],
		entry_point: &'a str,
		threadgroup_size: Option<Extent>,
	},
	/// Metal shading language source and entry-point name
	MTL { source: &'a str, entry_point: &'a str },
}

/// The `ShaderSource` enum represents platform-specific shader source for cross-platform compilation.
///
/// It exists to let callers express the GLSL and/or MSL variants of a shader in one value and let
/// [`compile`] pick the correct path for the active backend.
#[derive(Clone, Copy)]
pub enum ShaderSource<'a> {
	/// GLSL source code to be compiled to SPIR-V for Vulkan backends.
	Glsl(&'a str),
	/// MSL source code used directly on Metal.
	Msl { source: &'a str, entry_point: &'a str },
	/// Paired GLSL and MSL sources; [`compile`] selects the appropriate variant for the current platform.
	Platform {
		glsl: &'a str,
		msl: &'a str,
		msl_entry_point: &'a str,
	},
}

/// The `CompiledShaderSource` enum stores shader source after platform selection and compilation.
pub enum CompiledShaderSource {
	/// SPIR-V binary compiled from GLSL.
	SPIRV(Vec<u8>),
	/// Metal shading language source and entry-point name.
	MTL { source: String, entry_point: String },
}

impl CompiledShaderSource {
	pub fn as_source(&self) -> Sources<'_> {
		match self {
			Self::SPIRV(binary) => Sources::SPIRV(binary.as_slice()),
			Self::MTL { source, entry_point } => Sources::MTL {
				source: source.as_str(),
				entry_point: entry_point.as_str(),
			},
		}
	}
}

/// Compiles a platform-specific shader source into the representation expected by a device.
pub fn compile(name: &str, source: ShaderSource) -> Result<CompiledShaderSource, String> {
	match source {
		ShaderSource::Glsl(source) => compile_glsl(name, source),
		ShaderSource::Msl { source, entry_point } => Ok(CompiledShaderSource::MTL {
			source: source.to_string(),
			entry_point: entry_point.to_string(),
		}),
		ShaderSource::Platform {
			glsl,
			msl,
			msl_entry_point,
		} => {
			if crate::implementation::USES_METAL {
				Ok(CompiledShaderSource::MTL {
					source: msl.to_string(),
					entry_point: msl_entry_point.to_string(),
				})
			} else {
				compile_glsl(name, glsl)
			}
		}
	}
}

fn compile_glsl(name: &str, source: &str) -> Result<CompiledShaderSource, String> {
	resource_management::glsl::compile(source, name).map(|artifact| CompiledShaderSource::SPIRV(artifact.as_ref().to_vec()))
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
