#[cfg(not(target_vendor = "apple"))]
use crate::shader::besl::backends::spirv::SPIRVShaderGenerator;
use crate::shader::generator::{CompiledShaderBinding, ShaderGenerationSettings, ShaderGenerator};
#[cfg(target_vendor = "apple")]
use crate::shader::msl_shader_compiler::MSLShaderCompiler;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformShaderLanguage {
	Glsl,
	Msl,
}

impl PlatformShaderLanguage {
	pub const fn current_platform() -> Self {
		if cfg!(target_vendor = "apple") {
			Self::Msl
		} else {
			Self::Glsl
		}
	}

	pub const fn entry_point(self) -> &'static str {
		match self {
			Self::Glsl => "main",
			Self::Msl => "besl_main",
		}
	}

	pub const fn is_glsl(self) -> bool {
		matches!(self, Self::Glsl)
	}

	pub const fn is_msl(self) -> bool {
		matches!(self, Self::Msl)
	}
}

/// The `GeneratedCompiledPlatformShader` struct stores compiled shader bytes and reflection metadata for the active platform.
pub struct GeneratedCompiledPlatformShader {
	binary: Box<[u8]>,
	bindings: Vec<CompiledShaderBinding>,
	extent: Option<utils::Extent>,
	entry_point: Option<&'static str>,
}

impl GeneratedCompiledPlatformShader {
	pub fn binary(&self) -> &[u8] {
		&self.binary
	}

	pub fn into_binary(self) -> Box<[u8]> {
		self.binary
	}

	pub fn bindings(&self) -> &[CompiledShaderBinding] {
		&self.bindings
	}

	pub fn extent(&self) -> Option<utils::Extent> {
		self.extent
	}

	pub fn entry_point(&self) -> Option<&'static str> {
		self.entry_point
	}
}

/// The `Generator` struct selects the compiled shader backend that matches the current platform.
pub struct Generator {
	#[cfg(not(target_vendor = "apple"))]
	spirv_shader_generator: SPIRVShaderGenerator,
	#[cfg(target_vendor = "apple")]
	msl_shader_compiler: MSLShaderCompiler,
}

impl ShaderGenerator for Generator {}

impl Generator {
	pub fn new() -> Self {
		Self {
			#[cfg(not(target_vendor = "apple"))]
			spirv_shader_generator: SPIRVShaderGenerator::new(),
			#[cfg(target_vendor = "apple")]
			msl_shader_compiler: MSLShaderCompiler::new(),
		}
	}

	/// Generates a compiled shader artifact for the current platform.
	pub fn generate(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<GeneratedCompiledPlatformShader, String> {
		self.generate_for_language(
			PlatformShaderLanguage::current_platform(),
			shader_generation_settings,
			main_function_node,
		)
	}

	/// Generates a compiled shader artifact for the backend associated with `language`.
	pub fn generate_for_language(
		&mut self,
		language: PlatformShaderLanguage,
		shader_generation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<GeneratedCompiledPlatformShader, String> {
		match language {
			#[cfg(not(target_vendor = "apple"))]
			PlatformShaderLanguage::Glsl => {
				let (binary, bindings, extent) = self
					.spirv_shader_generator
					.generate(shader_generation_settings, main_function_node)?
					.into_parts();

				Ok(GeneratedCompiledPlatformShader {
					binary,
					bindings,
					extent,
					entry_point: None,
				})
			}
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => {
				let (binary, bindings, extent) = self
					.msl_shader_compiler
					.generate(shader_generation_settings, main_function_node)?
					.into_parts();

				Ok(GeneratedCompiledPlatformShader {
					binary,
					bindings,
					extent,
					entry_point: Some(PlatformShaderLanguage::Msl.entry_point()),
				})
			}
			_ => Err(
				"Unsupported platform shader language. The most likely cause is that this compiler backend is gated off for the current target platform."
					.to_string(),
			),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Generator, PlatformShaderLanguage};
	use crate::shader::generator::{self, ShaderGenerationSettings};

	#[test]
	fn current_platform_language_matches_target() {
		#[cfg(target_vendor = "apple")]
		assert_eq!(PlatformShaderLanguage::current_platform(), PlatformShaderLanguage::Msl);

		#[cfg(not(target_vendor = "apple"))]
		assert_eq!(PlatformShaderLanguage::current_platform(), PlatformShaderLanguage::Glsl);
	}

	#[test]
	fn generate_uses_current_platform_compiler() {
		let main = generator::tests::fragment_shader();
		let settings = ShaderGenerationSettings::fragment();
		let mut generator = Generator::new();
		let generated = generator
			.generate(&settings, &main)
			.expect("Failed to generate compiled platform shader");

		if cfg!(target_vendor = "apple") {
			assert_eq!(generated.entry_point(), Some(PlatformShaderLanguage::Msl.entry_point()));
		} else {
			assert_eq!(generated.entry_point(), None);
		}
		assert!(!generated.binary().is_empty());
	}
}

pub use Generator as PlatformShaderGenerator;
