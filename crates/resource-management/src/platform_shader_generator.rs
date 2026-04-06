use crate::{
	glsl_shader_generator::GLSLShaderGenerator,
	msl_shader_generator::MSLShaderGenerator,
	shader_generator::{ShaderGenerationSettings, ShaderGenerator},
};

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

/// The `GeneratedPlatformShader` struct stores the shader source emitted for the selected platform language.
pub struct GeneratedPlatformShader {
	language: PlatformShaderLanguage,
	source: String,
}

impl GeneratedPlatformShader {
	pub fn language(&self) -> PlatformShaderLanguage {
		self.language
	}

	pub fn source(&self) -> &str {
		&self.source
	}

	pub fn into_source(self) -> String {
		self.source
	}

	pub fn entry_point(&self) -> &'static str {
		self.language.entry_point()
	}
}

/// The `PlatformShaderGenerator` struct selects the shader source generator that matches the current platform.
pub struct PlatformShaderGenerator {
	glsl_shader_generator: GLSLShaderGenerator,
	msl_shader_generator: MSLShaderGenerator,
}

impl ShaderGenerator for PlatformShaderGenerator {}

impl PlatformShaderGenerator {
	pub fn new() -> Self {
		Self {
			glsl_shader_generator: GLSLShaderGenerator::new(),
			msl_shader_generator: MSLShaderGenerator::new(),
		}
	}

	/// Generates platform-native shader source for the provided BESL entry point.
	pub fn generate(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<GeneratedPlatformShader, ()> {
		self.generate_for_language(
			PlatformShaderLanguage::current_platform(),
			shader_generation_settings,
			main_function_node,
		)
	}

	pub fn generate_for_language(
		&mut self,
		language: PlatformShaderLanguage,
		shader_generation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<GeneratedPlatformShader, ()> {
		let source = match language {
			PlatformShaderLanguage::Glsl => self
				.glsl_shader_generator
				.generate(shader_generation_settings, main_function_node)?,
			PlatformShaderLanguage::Msl => self
				.msl_shader_generator
				.generate(shader_generation_settings, main_function_node)?,
		};

		Ok(GeneratedPlatformShader { language, source })
	}
}

#[cfg(test)]
mod tests {
	use super::{PlatformShaderGenerator, PlatformShaderLanguage};
	use crate::{
		glsl_shader_generator::GLSLShaderGenerator,
		msl_shader_generator::MSLShaderGenerator,
		shader_generator::{self, ShaderGenerationSettings},
	};

	#[test]
	fn current_platform_language_matches_target() {
		#[cfg(target_vendor = "apple")]
		assert_eq!(PlatformShaderLanguage::current_platform(), PlatformShaderLanguage::Msl);

		#[cfg(not(target_vendor = "apple"))]
		assert_eq!(PlatformShaderLanguage::current_platform(), PlatformShaderLanguage::Glsl);
	}

	#[test]
	fn generate_uses_current_platform_generator() {
		let main = shader_generator::tests::fragment_shader();
		let settings = ShaderGenerationSettings::fragment();
		let mut generator = PlatformShaderGenerator::new();
		let generated = generator
			.generate(&settings, &main)
			.expect("Failed to generate platform shader source");

		let expected = match PlatformShaderLanguage::current_platform() {
			PlatformShaderLanguage::Glsl => GLSLShaderGenerator::new()
				.generate(&settings, &main)
				.expect("Failed to generate expected GLSL shader"),
			PlatformShaderLanguage::Msl => MSLShaderGenerator::new()
				.generate(&settings, &main)
				.expect("Failed to generate expected MSL shader"),
		};

		assert_eq!(generated.language(), PlatformShaderLanguage::current_platform());
		assert_eq!(generated.source(), expected);
		assert_eq!(
			generated.entry_point(),
			PlatformShaderLanguage::current_platform().entry_point()
		);
	}
}
