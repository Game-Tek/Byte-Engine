use std::{
	cell::RefCell,
	fs,
	path::{Path, PathBuf},
	process::Command,
	time::{SystemTime, UNIX_EPOCH},
};

use utils::Extent;

use crate::{msl_shader_generator::MSLShaderGenerator, shader_generator::{ShaderGenerationSettings, ShaderGenerator}};

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

/// The `MSLShaderCompiler` struct compiles Metal Shading Language shaders into binary libraries.
pub struct MSLShaderCompiler {
	msl_shader_generator: MSLShaderGenerator,
}

impl ShaderGenerator for MSLShaderCompiler {}

impl MSLShaderCompiler {
	pub fn new() -> Self {
		Self {
			msl_shader_generator: MSLShaderGenerator::new(),
		}
	}

	pub fn generate(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &besl::NodeReference) -> Result<GeneratedShader, String> {
		let msl_shader = self
			.msl_shader_generator
			.generate(shader_compilation_settings, main_function_node)
			.map_err(|_| error("Failed to generate MSL shader source", "The MSL shader generator returned an error"))?;

		let binary = compile_msl_to_metallib(&msl_shader, &shader_compilation_settings.name)?;

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

		Ok(GeneratedShader {
			binary,
			bindings,
			extent: match shader_compilation_settings.stage {
				crate::shader_generator::Stages::Compute { local_size } => Some(local_size),
				_ => None,
			},
		})
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
			besl::Nodes::Raw { input, output, .. } => {
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
			besl::Nodes::Input { format, .. } | besl::Nodes::Output { format, .. } => {
				self.build_graph(bindings, format);
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

struct TempShaderDir {
	path: PathBuf,
}

impl TempShaderDir {
	fn new(prefix: &str) -> Result<Self, String> {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|_| error("Failed to generate a temporary directory name", "The system clock reported an invalid time"))?
			.as_nanos();
		let dir_name = format!("byte-engine-msl-{}-{}", prefix, unique_id);
		let path = std::env::temp_dir().join(dir_name);
		fs::create_dir_all(&path)
			.map_err(|_| error("Failed to create a temporary directory", "The system temporary directory is not writable"))?;
		Ok(Self { path })
	}

	fn path(&self) -> &Path {
		&self.path
	}
}

impl Drop for TempShaderDir {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.path);
	}
}

fn compile_msl_to_metallib(msl_source: &str, name: &str) -> Result<Box<[u8]>, String> {
	if !cfg!(target_os = "macos") {
		return Err(error("MSL compilation is only supported on macOS", "The Metal toolchain is not available on this platform"));
	}

	let safe_name = sanitize_shader_name(name);
	let temp_dir = TempShaderDir::new(&safe_name)?;

	let source_path = temp_dir.path().join(format!("{safe_name}.metal"));
	let air_path = temp_dir.path().join(format!("{safe_name}.air"));
	let metallib_path = temp_dir.path().join(format!("{safe_name}.metallib"));

	fs::write(&source_path, msl_source)
		.map_err(|_| error("Failed to write MSL shader source to disk", "The temporary directory could not be written"))?;

	let metal_output = Command::new("xcrun")
		.args([
			"-sdk",
			"macosx",
			"metal",
			"-c",
			source_path
				.to_str()
				.ok_or_else(|| error("Failed to compile MSL shader", "The temporary file path was not valid UTF-8"))?,
			"-o",
			air_path
				.to_str()
				.ok_or_else(|| error("Failed to compile MSL shader", "The temporary file path was not valid UTF-8"))?,
		])
		.output()
		.map_err(|_| error("Failed to invoke the Metal compiler", "The Xcode command line tools may be missing"))?;

	if !metal_output.status.success() {
		let stderr = String::from_utf8_lossy(&metal_output.stderr);
		return Err(error_with_details("Failed to compile MSL shader", "The Metal compiler reported an error", &stderr));
	}

	let metallib_output = Command::new("xcrun")
		.args([
			"-sdk",
			"macosx",
			"metallib",
			air_path
				.to_str()
				.ok_or_else(|| error("Failed to link Metal library", "The temporary file path was not valid UTF-8"))?,
			"-o",
			metallib_path
				.to_str()
				.ok_or_else(|| error("Failed to link Metal library", "The temporary file path was not valid UTF-8"))?,
		])
		.output()
		.map_err(|_| error("Failed to invoke metallib", "The Xcode command line tools may be missing"))?;

	if !metallib_output.status.success() {
		let stderr = String::from_utf8_lossy(&metallib_output.stderr);
		return Err(error_with_details("Failed to link Metal library", "The metallib tool reported an error", &stderr));
	}

	let binary = fs::read(&metallib_path)
		.map_err(|_| error("Failed to read compiled Metal library", "The metallib output was not created"))?;

	Ok(binary.into_boxed_slice())
}

fn sanitize_shader_name(name: &str) -> String {
	let mut sanitized = String::with_capacity(name.len());

	for ch in name.chars() {
		if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
			sanitized.push(ch);
		} else {
			sanitized.push('_');
		}
	}

	let trimmed = sanitized.trim_matches('_');
	if trimmed.is_empty() {
		"shader".to_string()
	} else {
		trimmed.to_string()
	}
}

fn error(message: &str, cause: &str) -> String {
	format!("{message}. {cause}.")
}

fn error_with_details(message: &str, cause: &str, details: &str) -> String {
	let details = details.trim();
	if details.is_empty() {
		return error(message, cause);
	}

	format!("{message}. {cause}.\n{details}")
}
