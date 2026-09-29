#[cfg(target_os = "windows")]
use crate::shader::besl::backends::hlsl::HLSLTranspiler;
#[cfg(target_os = "linux")]
use crate::shader::besl::backends::spirv::SPIRVCompiler;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::shader::besl::evaluation::ProgramEvaluation;
use crate::shader::generator::{CompiledShaderBinding, ShaderGenerationSettings, ShaderGenerator};
#[cfg(target_vendor = "apple")]
use crate::shader::msl_shader_compiler::MSLShaderCompiler;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformShaderLanguage {
	Glsl,
	Hlsl,
	Msl,
}

impl PlatformShaderLanguage {
	pub const fn current_platform() -> Self {
		if cfg!(target_vendor = "apple") {
			Self::Msl
		} else if cfg!(target_os = "windows") {
			Self::Hlsl
		} else if cfg!(target_os = "linux") {
			Self::Glsl
		} else {
			Self::Glsl
		}
	}

	pub const fn entry_point(self) -> &'static str {
		match self {
			Self::Glsl => "main",
			Self::Hlsl => "besl_main",
			Self::Msl => crate::shader::besl::backends::msl::MSL_ENTRY_POINT,
		}
	}

	pub const fn is_glsl(self) -> bool {
		matches!(self, Self::Glsl)
	}

	pub const fn is_msl(self) -> bool {
		matches!(self, Self::Msl)
	}

	pub const fn is_hlsl(self) -> bool {
		matches!(self, Self::Hlsl)
	}
}

/// The `GeneratedCompiledPlatformShader` struct stores compiled shader bytes and reflection metadata for the active platform.
pub struct GeneratedCompiledPlatformShader {
	binary: Box<[u8]>,
	bindings: Vec<CompiledShaderBinding>,
	extent: Option<utils::Extent>,
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
}

/// The `Generator` struct selects the compiled shader backend that matches the current platform.
pub struct Generator {
	#[cfg(not(target_vendor = "apple"))]
	#[cfg(target_os = "linux")]
	spirv_compiler: SPIRVCompiler,
	#[cfg(target_os = "windows")]
	hlsl_transpiler: HLSLTranspiler,
	#[cfg(target_vendor = "apple")]
	msl_shader_compiler: MSLShaderCompiler,
}

impl ShaderGenerator for Generator {}

impl Default for Generator {
	fn default() -> Self {
		Self::new()
	}
}

impl Generator {
	pub fn new() -> Self {
		Self {
			#[cfg(target_os = "linux")]
			spirv_compiler: SPIRVCompiler::new(),
			#[cfg(target_os = "windows")]
			hlsl_transpiler: HLSLTranspiler::new(),
			#[cfg(target_vendor = "apple")]
			msl_shader_compiler: MSLShaderCompiler::new(),
		}
	}

	/// Generates a compiled shader artifact for the current platform.
	pub async fn generate(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<GeneratedCompiledPlatformShader, String> {
		self.generate_for_language(
			PlatformShaderLanguage::current_platform(),
			shader_generation_settings,
			program,
		)
		.await
	}

	/// Generates a compiled shader artifact for the backend associated with `language`.
	pub async fn generate_for_language(
		&mut self,
		language: PlatformShaderLanguage,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<GeneratedCompiledPlatformShader, String> {
		match language {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				let (binary, _, extent) = self
					.spirv_compiler
					.generate(shader_generation_settings, &main)?
					.into_parts();
				let bindings = ProgramEvaluation::from_program(program)?
					.into_bindings()
					.into_iter()
					.map(CompiledShaderBinding::from)
					.collect();

				Ok(GeneratedCompiledPlatformShader {
					binary,
					bindings,
					extent,
				})
			}
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => {
				let (binary, bindings, extent) = self
					.msl_shader_compiler
					.generate(shader_generation_settings, program)
					.await?
					.into_parts();

				Ok(GeneratedCompiledPlatformShader {
					binary,
					bindings,
					extent,
				})
			}
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				let source = self.hlsl_transpiler.generate(shader_generation_settings, &main).map_err(|_| {
					"Failed to generate HLSL shader source. The most likely cause is that the BESL program uses unsupported HLSL constructs."
						.to_string()
				})?;
				let evaluation = ProgramEvaluation::from_program(program)?;
				Ok(GeneratedCompiledPlatformShader {
					binary: source.into_bytes().into_boxed_slice(),
					bindings: evaluation.into_bindings().into_iter().map(CompiledShaderBinding::from).collect(),
					extent: match shader_generation_settings.stage {
						crate::shader::generator::Stages::Compute { local_size }
						| crate::shader::generator::Stages::Task { local_size, .. }
						| crate::shader::generator::Stages::Mesh { local_size, .. } => Some(local_size),
						_ => None,
					},
				})
			}
			_ => Err(
				"Unsupported platform shader language. The most likely cause is that this compiler backend is gated off for the current target platform."
					.to_string(),
			),
		}
	}
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn missing_main_error() -> String {
	"Main function not found. The program description likely does not define a `main` function.".to_string()
}

#[cfg(test)]
mod tests {
	use super::Generator;
	use crate::shader::generator::ShaderGenerationSettings;

	/// Verifies bit-scan and scalar logarithm intrinsics compile with the real platform shader compiler.
	#[compio::test]
	async fn find_lsb_and_scalar_log2_compile_for_the_platform() {
		let root = besl::compile_to_besl(
			r#"
			Result: struct {
				values: u32[4],
				logarithm: f32,
			}
			result: descriptor<{ type: Result, binding: 43, access: read_write }>;

			main: fn () -> void {
				let empty: u32 = 0;
				let top: u32 = 1;
				top = top << 31;
				result.values[0] = find_lsb(empty);
				result.values[1] = find_lsb(top);
				result.values[2] = find_lsb(40);
				result.values[3] = find_lsb(1);
				result.logarithm = log2(8.0);
			}
			"#,
			None,
		)
		.expect("Expected the find_lsb fixture to link");
		let settings = ShaderGenerationSettings::compute(utils::Extent::line(1)).name("find_lsb".to_string());

		Generator::new()
			.generate(&settings, &root)
			.await
			.expect("Expected find_lsb and scalar log2 to compile for the platform shader language");
	}

	/// Verifies value arrays, array parameters, vector components selected at runtime, and unsigned `min`, `max`, and
	/// `clamp` with literal arguments compile with the real platform shader compiler.
	#[compio::test]
	async fn value_arrays_and_unsigned_extrema_compile_for_the_platform() {
		let root = besl::compile_to_besl(
			r#"
			Result: struct {
				values: u32[4],
			}
			result: descriptor<{ type: Result, binding: 43, access: read_write }>;

			sum: fn (values: u32[8], count: u32) -> u32 {
				let copy: u32[8] = values;
				let total: u32 = 0;
				for (let i: u32 = 0; i < min(count, 8); i = i + 1) {
					total = total + copy[i];
				}
				return total;
			}

			main: fn () -> void {
				let values: u32[8] = u32[8](1, 2, 3, 4, 5, 6, 7, 8);
				values[result.values[0] % 8] = max(result.values[1], 3);
				let words: vec4u = vec4u(0, 0, 0, 0);
				words[result.values[2] & 3] = clamp(sum(values, result.values[3]), 1, 100);
				result.values[0] = words.x;
			}
			"#,
			None,
		)
		.expect("Expected the array fixture to link");
		let settings = ShaderGenerationSettings::compute(utils::Extent::line(1)).name("value_arrays".to_string());

		Generator::new()
			.generate(&settings, &root)
			.await
			.expect("Expected arrays and unsigned extrema to compile for the platform shader language");
	}
}

pub use Generator as PlatformShaderCompiler;
