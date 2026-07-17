use utils::Extent;

use crate::{AccessPolicies, TextureViewTypes};

/// Possible types of a shader source
pub enum Sources<'a> {
	/// SPIR-V binary
	SPIRV(&'a [u8]),
	/// DirectX Intermediate Language bytecode for DX12 backends.
	DXIL(&'a [u8]),
	/// HLSL source and entry-point name for DX12 backends.
	HLSL { source: &'a str, entry_point: &'a str },
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
	#[cfg(target_os = "linux")]
	Glsl(&'a str),
	/// MSL source code used directly on Metal.
	Msl { source: &'a str, entry_point: &'a str },
	/// HLSL source code compiled for DX12.
	Hlsl { source: &'a str, entry_point: &'a str },
	/// Paired GLSL and MSL sources; [`compile`] selects the appropriate variant for the current platform.
	Platform {
		glsl: &'a str,
		msl: &'a str,
		msl_entry_point: &'a str,
	},
	/// Paired GLSL, MSL, and HLSL sources; [`compile`] selects the native variant for the active backend.
	PlatformNative {
		glsl: &'a str,
		msl: &'a str,
		msl_entry_point: &'a str,
		hlsl: &'a str,
		hlsl_entry_point: &'a str,
	},
}

/// The `CompiledShaderSource` enum stores shader source after platform selection and compilation.
pub enum CompiledShaderSource {
	/// SPIR-V binary compiled from GLSL.
	SPIRV(Vec<u8>),
	/// HLSL source and entry-point name.
	HLSL { source: String, entry_point: String },
	/// Metal shading language source and entry-point name.
	MTL { source: String, entry_point: String },
}

impl CompiledShaderSource {
	pub fn as_source(&self) -> Sources<'_> {
		match self {
			Self::SPIRV(binary) => Sources::SPIRV(binary.as_slice()),
			Self::HLSL { source, entry_point } => Sources::HLSL {
				source: source.as_str(),
				entry_point: entry_point.as_str(),
			},
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
		#[cfg(target_os = "linux")]
		ShaderSource::Glsl(source) => compile_glsl(name, source),
		ShaderSource::Hlsl { source, entry_point } => Ok(CompiledShaderSource::HLSL {
			source: source.to_string(),
			entry_point: entry_point.to_string(),
		}),
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
			} else if crate::implementation::USES_VULKAN {
				compile_glsl(name, glsl)
			} else {
				Err("Platform shader source does not include a native backend for this OS. The most likely cause is using GLSL/MSL-only source on DX12.".to_string())
			}
		}
		ShaderSource::PlatformNative {
			glsl,
			msl,
			msl_entry_point,
			hlsl,
			hlsl_entry_point,
		} => {
			if crate::implementation::USES_DX12 {
				Ok(CompiledShaderSource::HLSL {
					source: hlsl.to_string(),
					entry_point: hlsl_entry_point.to_string(),
				})
			} else if crate::implementation::USES_METAL {
				Ok(CompiledShaderSource::MTL {
					source: msl.to_string(),
					entry_point: msl_entry_point.to_string(),
				})
			} else if crate::implementation::USES_VULKAN {
				compile_glsl(name, glsl)
			} else {
				Err("Platform-native shader source does not include a supported backend for this OS. The most likely cause is compiling on an unsupported operating system.".to_string())
			}
		}
	}
}

#[cfg(target_os = "linux")]
fn compile_glsl(name: &str, source: &str) -> Result<CompiledShaderSource, String> {
	resource_management::shader::glsl_compile::compile(source, name)
		.map(|artifact| CompiledShaderSource::SPIRV(artifact.as_ref().to_vec()))
}

#[cfg(not(target_os = "linux"))]
fn compile_glsl(_name: &str, _source: &str) -> Result<CompiledShaderSource, String> {
	Err(
		"GLSL shader compilation requires a Vulkan backend (Linux only). Use a BESL shader for cross-platform support."
			.to_string(),
	)
}

/// The `ResourceSlot` struct identifies one resource in a shader's flat resource namespace.
///
/// Slots are global to the pipeline interface. Descriptor sets may group resources by lifetime,
/// but they never introduce another shader-visible coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ResourceSlot(u32);

impl ResourceSlot {
	pub const fn new(slot: u32) -> Self {
		Self(slot)
	}

	pub const fn index(self) -> u32 {
		self.0
	}
}

impl From<u32> for ResourceSlot {
	fn from(slot: u32) -> Self {
		Self::new(slot)
	}
}

/// The `ResourceKind` enum describes the native resource category expected at a flat shader slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
	UniformBuffer,
	StorageBuffer,
	SampledImage,
	CombinedImageSampler,
	StorageImage,
	InputAttachment,
	Sampler,
	AccelerationStructure,
}

/// The `ShaderResourceDescriptor` struct defines the complete retained-resource contract used to build a pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderResourceDescriptor {
	pub(crate) slot: ResourceSlot,
	pub(crate) kind: ResourceKind,
	pub(crate) count: u32,
	pub(crate) access: AccessPolicies,
	pub(crate) texture_view_type: TextureViewTypes,
	pub(crate) buffer_stride: u32,
}

impl ShaderResourceDescriptor {
	pub const fn new(slot: ResourceSlot, kind: ResourceKind, count: u32, access: AccessPolicies) -> Self {
		assert!(
			count > 0,
			"Invalid shader resource count. The most likely cause is that a shader declared an empty resource array."
		);
		assert!(
			slot.index().checked_add(count).is_some(),
			"Invalid shader resource slot range. The most likely cause is that a resource array extends beyond the flat slot namespace."
		);
		Self {
			slot,
			kind,
			count,
			access,
			texture_view_type: TextureViewTypes::Texture2D,
			buffer_stride: 4,
		}
	}

	pub const fn single(slot: ResourceSlot, kind: ResourceKind, access: AccessPolicies) -> Self {
		Self::new(slot, kind, 1, access)
	}

	pub const fn texture_view_type(mut self, texture_view_type: TextureViewTypes) -> Self {
		self.texture_view_type = texture_view_type;
		self
	}

	pub const fn buffer_stride(mut self, buffer_stride: u32) -> Self {
		self.buffer_stride = buffer_stride;
		self
	}

	pub const fn slot(self) -> ResourceSlot {
		self.slot
	}

	pub const fn kind(self) -> ResourceKind {
		self.kind
	}

	pub const fn count(self) -> u32 {
		self.count
	}

	pub const fn access(self) -> AccessPolicies {
		self.access
	}

	pub const fn texture_view(self) -> TextureViewTypes {
		self.texture_view_type
	}

	pub const fn buffer_element_stride(self) -> u32 {
		self.buffer_stride
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn platform_native_selects_backend_specific_shader_source() {
		let compiled = compile(
			"platform-native",
			ShaderSource::PlatformNative {
				glsl: "#version 450\n#pragma shader_stage(compute)\nlayout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;\nvoid main() {}",
				msl: "kernel void main0() {}",
				msl_entry_point: "main0",
				hlsl: "[numthreads(1, 1, 1)] void main() {}",
				hlsl_entry_point: "main",
			},
		)
		.expect("Expected platform-native shader selection to compile.");

		if crate::implementation::USES_DX12 {
			assert!(matches!(
				compiled,
				CompiledShaderSource::HLSL {
					source,
					entry_point
				} if source.contains("numthreads") && entry_point == "main"
			));
		} else if crate::implementation::USES_METAL {
			assert!(matches!(
				compiled,
				CompiledShaderSource::MTL {
					source,
					entry_point
				} if source.contains("main0") && entry_point == "main0"
			));
		} else {
			assert!(matches!(compiled, CompiledShaderSource::SPIRV(binary) if !binary.is_empty()));
		}
	}
}
