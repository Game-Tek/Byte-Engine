use std::cell::RefCell;

use utils::Extent;

use crate::{glsl_shader_generator::GLSLShaderGenerator, shader_generator::{ShaderGenerationSettings, ShaderGenerator}};

pub struct Binding {
	pub binding: u32,
	pub set: u32,
	pub read: bool,
	pub write: bool,
}

pub struct GeneratedShader {
	binary: Box<[u8]>,
	bindings: Vec<Binding>,
	extent: Option<Extent>,
}

impl GeneratedShader {
	pub fn extent(&self) -> Option<Extent> {
		self.extent
	}

	pub fn binary(&self) -> &[u8] {
		&self.binary
	}

	pub fn into_binary(self) -> Box<[u8]> {
		self.binary
	}

	pub fn bindings(&self) -> &[Binding] {
		&self.bindings
	}
}

pub struct SPIRVShaderGenerator {}

impl ShaderGenerator for SPIRVShaderGenerator {}

impl SPIRVShaderGenerator {
    pub fn new() -> Self {
        Self {}
    }

	pub fn generate(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &besl::NodeReference) -> Result<GeneratedShader, String> {
		let glsl_shader = GLSLShaderGenerator::new().generate(shader_compilation_settings, main_function_node).map_err(|_| "Failed to generate initial GLSL shader".to_string())?;

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

		let compilation_artifact = match binary {
			Ok(binary) => { binary }
			Err(err) => {
				let error_string = err.to_string();
				return Err(besl::glsl::format_glslang_error(&shader_compilation_settings.name, &error_string, &glsl_shader).unwrap_or(error_string));
			}
		};

		let mut bindings = Vec::with_capacity(16);

		{
			let node_borrow = RefCell::borrow(&main_function_node);
			let node_ref = node_borrow.node();

			match node_ref {
				besl::Nodes::Function { name, .. } => {
					assert_eq!(name, "main");
				}
				_ => panic!("Root node must be a function node."),
			}
		}
		
		self.build_graph(&mut bindings, main_function_node);

		bindings.sort_by(|a, b| {
			if a.set == b.set {
				a.binding.cmp(&b.binding)
			} else {
				a.set.cmp(&b.set)
			}
		});

		return Ok(GeneratedShader {
			binary: Box::from(compilation_artifact.as_binary_u8()),
			bindings,
			extent: match shader_compilation_settings.stage { crate::shader_generator::Stages::Compute { local_size } => Some(local_size), _ => None },
		});
	}

	fn build_graph(&mut self, bindings: &mut Vec<Binding>, node: &besl::NodeReference) {
		let node_borrow = RefCell::borrow(&node);
		let node_ref = node_borrow.node();

		match node_ref {
			besl::Nodes::Function { statements, .. } => {
				for statement in statements {
					self.build_graph(bindings, statement);
				}
			}
			besl::Nodes::Expression(expresions) => {
				match expresions {
					besl::Expressions::FunctionCall { parameters, function } => {
						self.build_graph(bindings, function);
						for parameter in parameters {
							self.build_graph(bindings, parameter);
						}
					}
					besl::Expressions::Accessor { left, right } => {
						self.build_graph(bindings, left);
						self.build_graph(bindings, right);
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							self.build_graph(bindings, element);
						}
					}
					besl::Expressions::IntrinsicCall { intrinsic, elements } => {
						for element in elements {
							self.build_graph(bindings, element);
						}
						self.build_graph(bindings, intrinsic);
					}
					besl::Expressions::Return | besl::Expressions::Literal { .. } => {
						// Do nothing
					}
					besl::Expressions::Macro { body, .. } => {
						self.build_graph(bindings, body);
					}
					besl::Expressions::Member { source, .. } => {
						self.build_graph(bindings, source);
					}
					besl::Expressions::Operator { left, right, .. } => {
						self.build_graph(bindings, left);
						self.build_graph(bindings, right);
					}
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						self.build_graph(bindings, r#type);
					}
				}
			}
			besl::Nodes::Binding { set, binding, read, write, .. } => {
				if let None = bindings.iter().find(|b| b.binding == *binding && b.set == *set) {
					bindings.push(Binding { binding: *binding, set: *set, read: *read, write: *write });
				}
			}
			besl::Nodes::GLSL { input, output, .. } => {
				for input in input {
					self.build_graph(bindings, input);
				}
				for output in output {
					self.build_graph(bindings, output);
				}
			}
			besl::Nodes::Struct { fields, .. } => {
				for member in fields {
					self.build_graph(bindings, member);
				}
			}
			besl::Nodes::Intrinsic { elements, r#return, .. } => {
				for element in elements {
					self.build_graph(bindings, element);
				}
				self.build_graph(bindings, r#return);
			}
			besl::Nodes::Literal { value, .. } => {
				self.build_graph(bindings, value);
			}
			besl::Nodes::Member { r#type, .. } => {
				self.build_graph(bindings, r#type);
			}
			besl::Nodes::Null { .. } => {
				// Do nothing
			}
			besl::Nodes::Parameter { r#type, .. } => {
				self.build_graph(bindings, r#type);
			}
			besl::Nodes::PushConstant { members } => {
				for member in members {
					self.build_graph(bindings, member);
				}
			}
			besl::Nodes::Scope { children, .. } => {
				for child in children {
					self.build_graph(bindings, child);
				}
			}
			besl::Nodes::Specialization { r#type, .. } => {
				self.build_graph(bindings, r#type);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

    use crate::{shader_generator, spirv_shader_generator::SPIRVShaderGenerator};

	#[test]
	fn bindings() {
		let main = shader_generator::tests::bindings();

		let shader = SPIRVShaderGenerator::new().generate(&ShaderGenerationSettings::vertex(), &main).expect("Failed to generate shader");

		let bindings = shader.bindings;

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