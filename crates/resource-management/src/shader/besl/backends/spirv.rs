pub use crate::shader::generator::{CompiledShader as GeneratedShader, CompiledShaderBinding as Binding};

#[cfg(target_os = "linux")]
mod compilation {
	use std::cell::RefCell;

	use crate::shader::{
		besl::{backends::glsl::GLSLShaderGenerator, evaluation::ProgramEvaluation},
		generator::{CompiledShader, CompiledShaderBinding, ShaderGenerationSettings, ShaderGenerator},
		glsl_compile,
	};

	/// The `Generator` generates SPIR-V shaders from Byte Engine Shader Language program descriptions.
	/// > [!IMPORTANT]
	/// > Creating an instance of `Generator` is an expensive operation, and as such, it should be reused whenever possible.
	pub struct Generator {
		glsl_shader_generator: GLSLShaderGenerator,
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
				glsl_shader_generator: GLSLShaderGenerator::new(),
			}
		}

		pub fn generate(
			&mut self,
			shader_compilation_settings: &ShaderGenerationSettings,
			main_function_node: &besl::NodeReference,
		) -> Result<CompiledShader, String> {
			let glsl_shader = self
				.glsl_shader_generator
				.generate(shader_compilation_settings, main_function_node)
				.map_err(|_| "Failed to generate initial GLSL shader".to_string())?;

			let compiler = shaderc::Compiler::new().unwrap();
			let mut options = shaderc::CompileOptions::new().unwrap();

			options.set_optimization_level(shaderc::OptimizationLevel::Performance);
			options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_4 as u32);

			if cfg!(debug_assertions) {
				options.set_generate_debug_info();
			}

			options.set_target_spirv(shaderc::SpirvVersion::V1_6);
			options.set_invert_y(true);

			let binary = compiler.compile_into_spirv(
				&glsl_shader,
				shaderc::ShaderKind::InferFromSource,
				&shader_compilation_settings.name,
				"main",
				Some(&options),
			);

			let compilation_artifact = match binary {
				Ok(binary) => binary,
				Err(err) => {
					let error_string = err.to_string();
					dbg!(&error_string);
					println!("{}", glsl_shader);
					return Err(glsl_compile::pretty_format_glslang_error_string(
						&error_string,
						&shader_compilation_settings.name,
						&glsl_shader,
					));
				}
			};

			{
				let node_borrow = RefCell::borrow(main_function_node);
				let node_ref = node_borrow.node();

				match node_ref {
					besl::Nodes::Function { name, .. } => {
						assert_eq!(name, "main");
					}
					_ => panic!("Root node must be a function node."),
				}
			}

			let program_evaluation = ProgramEvaluation::from_main(main_function_node)?;

			let bindings = program_evaluation.bindings();

			Ok(CompiledShader::new(
				Box::from(compilation_artifact.as_binary_u8()),
				bindings
					.iter()
					.map(|b| CompiledShaderBinding::new(b.set, b.binding, b.read, b.write))
					.collect(),
				match shader_compilation_settings.stage {
					crate::shader::generator::Stages::Compute { local_size } => Some(local_size),
					_ => None,
				},
			))
		}
	}

	#[cfg(test)]
	mod tests {
		use super::Generator;
		use crate::shader::{generator, generator::ShaderGenerationSettings};

		#[test]
		fn bindings() {
			let main = generator::tests::bindings();

			let shader = Generator::new()
				.generate(&ShaderGenerationSettings::vertex(), &main)
				.expect("Failed to generate shader");

			let bindings = shader.bindings();

			assert_eq!(bindings.len(), 3);

			let buffer_binding = &bindings[0];
			assert_eq!(buffer_binding.binding, 0);
			assert_eq!(buffer_binding.set, 0);
			assert_eq!(buffer_binding.read, true);
			assert_eq!(buffer_binding.write, true);

			let image_binding = &bindings[1];
			assert_eq!(image_binding.binding, 1);
			assert_eq!(image_binding.set, 0);
			assert_eq!(image_binding.read, false);
			assert_eq!(image_binding.write, true);

			let texture_binding = &bindings[2];
			assert_eq!(texture_binding.binding, 0);
			assert_eq!(texture_binding.set, 1);
			assert_eq!(texture_binding.read, true);
			assert_eq!(texture_binding.write, false);
		}
	}
}

#[cfg(target_os = "linux")]
pub use compilation::Generator as SPIRVShaderGenerator;
