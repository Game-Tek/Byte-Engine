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
	///
	/// This is [`Self::lower`] followed by [`Self::compile`]. Call those separately to skip compilation when a binary
	/// for the same [`LoweredPlatformShader::source`] and [`Self::compiler_identity`] is already stored.
	pub async fn generate(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<GeneratedCompiledPlatformShader, String> {
		let lowered = self.lower(shader_generation_settings, program)?;

		self.compile(lowered).await
	}

	/// Lowers a linked BESL program to the source text the current platform compiler consumes.
	///
	/// Lowering is cheap next to compilation. Next, hash the result with [`Self::compiler_identity`] to look up a
	/// stored binary, or pass it to [`Self::compile`].
	pub fn lower(
		&mut self,
		shader_generation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<LoweredPlatformShader, String> {
		let language = PlatformShaderLanguage::current_platform();

		let (source, bindings, extent) = match language {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				let source = self.spirv_compiler.transpile(shader_generation_settings, &main)?;
				let bindings = ProgramEvaluation::from_program(program)?
					.into_bindings()
					.into_iter()
					.map(CompiledShaderBinding::from)
					.collect();
				// SPIR-V reflection reports a workgroup only for compute shaders.
				let extent = match shader_generation_settings.stage {
					crate::shader::generator::Stages::Compute { local_size } => Some(local_size),
					_ => None,
				};

				(source, bindings, extent)
			}
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => {
				let source = self.msl_shader_compiler.transpile(shader_generation_settings, program)?;
				let bindings = crate::shader::besl::evaluation::collect_bindings::<CompiledShaderBinding>(program)?;

				(source, bindings, workgroup_extent(shader_generation_settings))
			}
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => {
				let main = program.get_main().ok_or_else(missing_main_error)?;
				let source = self
					.hlsl_transpiler
					.generate(shader_generation_settings, &main)
					.map_err(|_| {
						"Failed to generate HLSL shader source. The most likely cause is that the BESL program uses unsupported HLSL constructs."
						.to_string()
					})?;
				let bindings = ProgramEvaluation::from_program(program)?
					.into_bindings()
					.into_iter()
					.map(CompiledShaderBinding::from)
					.collect();

				(source, bindings, workgroup_extent(shader_generation_settings))
			}
			_ => return Err(unsupported_language_error()),
		};

		Ok(LoweredPlatformShader {
			name: shader_generation_settings.name.clone(),
			source,
			bindings,
			extent,
		})
	}

	/// Runs the platform compiler on lowered source.
	///
	/// On Windows the payload stays HLSL text; the asset baker compiles it to DXIL when it finalizes the artifact.
	pub async fn compile(&mut self, lowered: LoweredPlatformShader) -> Result<GeneratedCompiledPlatformShader, String> {
		let LoweredPlatformShader {
			name,
			source,
			bindings,
			extent,
		} = lowered;

		let binary = match PlatformShaderLanguage::current_platform() {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => crate::shader::besl::backends::spirv::compile_glsl_to_spirv(&source, &name)?,
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, &name).await?,
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => {
				let _ = name;
				source.into_bytes().into_boxed_slice()
			}
			_ => return Err(unsupported_language_error()),
		};

		Ok(GeneratedCompiledPlatformShader {
			binary,
			bindings,
			extent,
		})
	}

	/// Describes the platform compiler toolchain, including its version and the flags every compile passes.
	///
	/// Two compiles of the same [`LoweredPlatformShader::source`] under the same identity produce equivalent binaries,
	/// so asset bakers hash both to decide whether a stored binary can be reused.
	pub async fn compiler_identity() -> Result<String, String> {
		let identity = match PlatformShaderLanguage::current_platform() {
			#[cfg(target_os = "linux")]
			PlatformShaderLanguage::Glsl => crate::shader::besl::backends::spirv::spirv_compiler_identity()?,
			#[cfg(target_vendor = "apple")]
			PlatformShaderLanguage::Msl => crate::shader::msl_shader_compiler::metal_compiler_identity().await?,
			#[cfg(target_os = "windows")]
			PlatformShaderLanguage::Hlsl => crate::shader::hlsl_shader_compiler::dxc_compiler_identity()?,
			_ => return Err(unsupported_language_error()),
		};

		Ok(identity)
	}
}

/// The `LoweredPlatformShader` struct holds one shader between cheap lowering and expensive platform compilation.
///
/// Asset bakers use its source as part of the key that decides whether a stored binary is still valid. Create it with
/// [`Generator::lower`], then pass it to [`Generator::compile`].
pub struct LoweredPlatformShader {
	name: String,
	source: String,
	bindings: Vec<CompiledShaderBinding>,
	extent: Option<utils::Extent>,
}

impl LoweredPlatformShader {
	/// Returns the GLSL, MSL, or HLSL text the platform compiler consumes.
	pub fn source(&self) -> &str {
		&self.source
	}

	/// Returns the diagnostic name the platform compiler receives.
	pub fn name(&self) -> &str {
		&self.name
	}
}

/// Returns the workgroup size the platform compiler preserves for compute, task, and mesh stages.
#[cfg(any(target_vendor = "apple", target_os = "windows"))]
fn workgroup_extent(settings: &ShaderGenerationSettings) -> Option<utils::Extent> {
	match settings.stage {
		crate::shader::generator::Stages::Compute { local_size }
		| crate::shader::generator::Stages::Task { local_size, .. }
		| crate::shader::generator::Stages::Mesh { local_size, .. } => Some(local_size),
		_ => None,
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
}

pub use Generator as PlatformShaderCompiler;
