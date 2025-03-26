use crate::{glsl_shader_generator::GLSLShaderGenerator, shader_generator::{ShaderGenerationSettings, ShaderGenerator}};

pub struct SPIRVShaderGenerator {}

impl ShaderGenerator for SPIRVShaderGenerator {}

impl SPIRVShaderGenerator {
    pub fn new() -> Self {
        Self {}
    }

	pub fn generate(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &besl::NodeReference) -> Result<Box<[u8]>, String> {
		let glsl_shader = GLSLShaderGenerator::new().minified(true).generate(shader_compilation_settings, main_function_node).map_err(|_| "Failed to generate initial GLSL shader".to_string())?;

		let compiler = shaderc::Compiler::new().unwrap();
		let mut options = shaderc::CompileOptions::new().unwrap();

		options.set_optimization_level(shaderc::OptimizationLevel::Performance);
		options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_4 as u32);

		if cfg!(debug_assertions) {
			options.set_generate_debug_info();
		}

		options.set_target_spirv(shaderc::SpirvVersion::V1_6);
		options.set_invert_y(true);

		let binary = compiler.compile_into_spirv(&glsl_shader, shaderc::ShaderKind::InferFromSource, &shader_compilation_settings.name, "main", Some(&options));

		// TODO: if shader fails to compile try to generate a failsafe shader

		let compilation_artifact = match binary {
			Ok(binary) => { binary }
			Err(err) => {
				let error_string = err.to_string();
				return Err(besl::glsl::format_glslang_error(&shader_compilation_settings.name, &error_string, &glsl_shader).unwrap_or(error_string));
			}
		};

		return Ok(Box::from(compilation_artifact.as_binary_u8()));
	}
}
