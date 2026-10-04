#[cfg(target_os = "linux")]
use crate::shader::besl::backends::glsl::GLSLTranspiler;
#[cfg(target_os = "windows")]
use crate::shader::besl::backends::hlsl::HLSLTranspiler;
#[cfg(target_vendor = "apple")]
use crate::shader::besl::backends::msl::MSLTranspiler;
use crate::shader::{
	besl::evaluation::collect_bindings,
	generator::{CompiledShader, CompiledShaderBinding, ShaderGenerationSettings, Stages},
};

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

/// The `Generator` struct selects the compiled shader backend that matches the current platform.
pub struct Generator {
	#[cfg(target_os = "linux")]
	glsl_transpiler: GLSLTranspiler,
	#[cfg(target_os = "windows")]
	hlsl_transpiler: HLSLTranspiler,
	#[cfg(target_vendor = "apple")]
	msl_transpiler: MSLTranspiler,
}

impl Default for Generator {
	fn default() -> Self {
		Self::new()
	}
}

impl Generator {
	pub fn new() -> Self {
		Self {
			#[cfg(target_os = "linux")]
			glsl_transpiler: GLSLTranspiler::new(),
			#[cfg(target_os = "windows")]
			hlsl_transpiler: HLSLTranspiler::new(),
			#[cfg(target_vendor = "apple")]
			msl_transpiler: MSLTranspiler::new(),
		}
	}

	/// Generates a compiled shader artifact for the current platform.
	///
	/// This is [`Self::lower`] followed by [`LoweredPlatformShader::compile`]. Call those separately to skip compilation
	/// when a binary for the same [`LoweredPlatformShader::source`] and [`Self::compiler_identity`] is already stored.
	pub async fn generate(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<CompiledShader, String> {
		self.lower(shader_generation_settings, program)?.compile().await
	}

	/// Lowers a linked BESL program to the source text the current platform compiler consumes.
	///
	/// Lowering is cheap next to compilation. Next, hash the result with [`Self::compiler_identity`] to look up a
	/// stored binary, or call [`LoweredPlatformShader::compile`].
	pub fn lower(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<LoweredPlatformShader, String> {
		let source = match PlatformShaderLanguage::current_platform() {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				self.glsl_transpiler
					.generate(shader_generation_settings, &main)
					.map_err(|_| "Failed to generate initial GLSL shader".to_string())?
			}
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => self
				.msl_transpiler
				.generate_program(shader_generation_settings, program)
				.map_err(|_| "Failed to generate MSL shader source. The MSL transpiler returned an error.".to_string())?,
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				self.hlsl_transpiler
					.generate(shader_generation_settings, &main)
					.map_err(|_| {
						"Failed to generate HLSL shader source. The most likely cause is that the BESL program uses unsupported HLSL constructs."
						.to_string()
					})?
			}
			_ => return Err(unsupported_language_error()),
		};
		let extent = match shader_generation_settings.stage {
			Stages::Compute { local_size } => Some(local_size),
			// SPIR-V reflection reports a workgroup only for compute shaders.
			_ if PlatformShaderLanguage::current_platform().is_glsl() => None,
			stage => stage.local_size(),
		};

		Ok(LoweredPlatformShader {
			name: shader_generation_settings.name.clone(),
			source,
			bindings: collect_bindings(program)?
				.into_iter()
				.map(CompiledShaderBinding::from)
				.collect(),
			extent,
		})
	}

	/// Describes the platform compiler toolchain, including its version and the flags every compile passes.
	///
	/// Two compiles of the same [`LoweredPlatformShader::source`] under the same identity produce equivalent binaries,
	/// so asset bakers hash both to decide whether a stored binary can be reused. The toolchain query runs once per
	/// process.
	pub async fn compiler_identity() -> Result<&'static str, String> {
		static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

		if let Some(identity) = IDENTITY.get() {
			return Ok(identity);
		}

		let identity = match PlatformShaderLanguage::current_platform() {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => crate::shader::besl::backends::spirv::spirv_compiler_identity()?,
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => crate::shader::msl_shader_compiler::metal_compiler_identity().await?,
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => crate::shader::hlsl_shader_compiler::dxc_compiler_identity()?,
			_ => return Err(unsupported_language_error()),
		};

		Ok(IDENTITY.get_or_init(|| identity))
	}
}

/// The `LoweredPlatformShader` struct holds one shader between cheap lowering and expensive platform compilation.
///
/// Asset bakers use its source as part of the key that decides whether a stored binary is still valid. Create it with
/// [`Generator::lower`], then call [`Self::compile`].
pub struct LoweredPlatformShader {
	/// The diagnostic name the platform compiler receives.
	pub name: String,
	/// The GLSL, MSL, or HLSL text the platform compiler consumes.
	pub source: String,
	bindings: Vec<CompiledShaderBinding>,
	extent: Option<utils::Extent>,
}

impl LoweredPlatformShader {
	/// Runs the platform compiler on the lowered source.
	///
	/// On Windows the payload stays HLSL text; the asset baker compiles it to DXIL when it finalizes the artifact.
	pub async fn compile(self) -> Result<CompiledShader, String> {
		let binary = match PlatformShaderLanguage::current_platform() {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => crate::shader::besl::backends::spirv::compile_glsl_to_spirv(&self.source, &self.name)?,
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => {
				crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&self.source, &self.name).await?
			}
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => self.source.into_bytes().into_boxed_slice(),
			_ => return Err(unsupported_language_error()),
		};

		Ok(CompiledShader {
			binary,
			bindings: self.bindings,
			extent: self.extent,
		})
	}
}

fn unsupported_language_error() -> String {
	"Unsupported platform shader language. The most likely cause is that this compiler backend is gated off for the current target platform."
		.to_string()
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
