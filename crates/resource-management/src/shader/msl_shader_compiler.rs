use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fs,
	path::{Path, PathBuf},
	process::Command,
	time::{SystemTime, UNIX_EPOCH},
};

pub use crate::shader::generator::{CompiledShader as GeneratedShader, CompiledShaderBinding as Binding};
use crate::shader::{
	besl::{
		backends::msl::MSLShaderGenerator,
		evaluation::{collect_bindings, BindingKind, BindingRecord, IntrinsicBindingTraversalOrder},
	},
	generator::{CompiledShader, CompiledShaderBinding, ShaderGenerationSettings, ShaderGenerator},
};

/// The `Compiler` struct exists to compile Metal Shading Language shaders into binary libraries.
pub struct Compiler<A: Allocator + Clone = Global> {
	allocator: A,
	msl_shader_generator: MSLShaderGenerator<A>,
}

impl<A: Allocator + Clone> ShaderGenerator for Compiler<A> {}

impl BindingRecord for CompiledShaderBinding {
	fn from_usage(_name: &str, _kind: BindingKind, _count: u32, set: u32, binding: u32, read: bool, write: bool) -> Self {
		Self::new(set, binding, read, write)
	}

	fn usage(&self) -> (u32, u32, bool, bool) {
		(self.set, self.binding, self.read, self.write)
	}
}

impl Default for Compiler<Global> {
	fn default() -> Self {
		Self::new()
	}
}

impl Compiler<Global> {
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> Compiler<A> {
	pub fn new_in(allocator: A) -> Self {
		Self {
			allocator: allocator.clone(),
			msl_shader_generator: MSLShaderGenerator::new_in(allocator),
		}
	}

	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<GeneratedShader, String> {
		self.generate_in(shader_compilation_settings, main_function_node, self.allocator.clone())
	}

	/// Generates a compiled Metal shader using `allocator` for one-call source-generation scratch.
	pub fn generate_in(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
		allocator: A,
	) -> Result<GeneratedShader, String> {
		let msl_shader = self
			.msl_shader_generator
			.generate_in(shader_compilation_settings, main_function_node, allocator)
			.map_err(|_| {
				error(
					"Failed to generate MSL shader source",
					"The MSL shader generator returned an error",
				)
			})?;

		let binary = compile_msl_source_to_metallib(&msl_shader, &shader_compilation_settings.name)?;

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

		let bindings = collect_bindings::<CompiledShaderBinding>(
			main_function_node,
			IntrinsicBindingTraversalOrder::ElementsBeforeDefinition,
		);

		Ok(CompiledShader::new(
			binary,
			bindings,
			match shader_compilation_settings.stage {
				crate::shader::generator::Stages::Compute { local_size } => Some(local_size),
				_ => None,
			},
		))
	}
}

struct TempShaderDir {
	path: PathBuf,
}

impl TempShaderDir {
	fn new(prefix: &str) -> Result<Self, String> {
		let unique_id = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|_| {
				error(
					"Failed to generate a temporary directory name",
					"The system clock reported an invalid time",
				)
			})?
			.as_nanos();
		let dir_name = format!("byte-engine-msl-{}-{}", prefix, unique_id);
		let path = std::env::temp_dir().join(dir_name);
		fs::create_dir_all(&path).map_err(|_| {
			error(
				"Failed to create a temporary directory",
				"The system temporary directory is not writable",
			)
		})?;
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

/// Compiles Metal Shading Language source into a Metal library binary.
pub fn compile_msl_source_to_metallib(msl_source: &str, name: &str) -> Result<Box<[u8]>, String> {
	if !cfg!(target_os = "macos") {
		return Err(error(
			"MSL compilation is only supported on macOS",
			"The Metal toolchain is not available on this platform",
		));
	}

	let safe_name = sanitize_shader_name(name);
	let temp_dir = TempShaderDir::new(&safe_name)?;

	let source_path = temp_dir.path().join(format!("{safe_name}.metal"));
	let air_path = temp_dir.path().join(format!("{safe_name}.air"));
	let metallib_path = temp_dir.path().join(format!("{safe_name}.metallib"));

	fs::write(&source_path, msl_source).map_err(|_| {
		error(
			"Failed to write MSL shader source to disk",
			"The temporary directory could not be written",
		)
	})?;

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
		.map_err(|_| {
			error(
				"Failed to invoke the Metal compiler",
				"The Xcode command line tools may be missing",
			)
		})?;

	if !metal_output.status.success() {
		let exit_status = metal_output
			.status
			.code()
			.map_or_else(|| metal_output.status.to_string(), |code| code.to_string());
		return Err(format_tool_failure(
			"Failed to compile MSL shader",
			"The Metal compiler reported an error",
			&exit_status,
			&metal_output.stdout,
			&metal_output.stderr,
		));
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
		let exit_status = metallib_output
			.status
			.code()
			.map_or_else(|| metallib_output.status.to_string(), |code| code.to_string());
		return Err(format_tool_failure(
			"Failed to link Metal library",
			"The metallib tool reported an error",
			&exit_status,
			&metallib_output.stdout,
			&metallib_output.stderr,
		));
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

fn format_tool_failure(message: &str, cause: &str, exit_status: &str, stdout: &[u8], stderr: &[u8]) -> String {
	let stdout = String::from_utf8_lossy(stdout);
	let stdout = stdout.trim();
	let stdout = if stdout.is_empty() { "<empty>" } else { stdout };
	let stderr = String::from_utf8_lossy(stderr);
	let stderr = stderr.trim();
	let stderr = if stderr.is_empty() { "<empty>" } else { stderr };

	format!("{message}. {cause}.\nExit status: {exit_status}\nstderr:\n{stderr}\nstdout:\n{stdout}")
}

pub use Compiler as MSLShaderCompiler;

#[cfg(test)]
mod tests {
	use super::{format_tool_failure, CompiledShaderBinding};
	use crate::shader::besl::evaluation::{collect_bindings, BindingRecord, BindingUsage, IntrinsicBindingTraversalOrder};

	fn binding(name: &str, set: u32, binding: u32, read: bool, write: bool) -> besl::NodeReference {
		besl::Node::binding(
			name,
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			set,
			binding,
			read,
			write,
		)
		.into()
	}

	fn usage<T: BindingRecord>(bindings: &[T]) -> Vec<(u32, u32, bool, bool)> {
		bindings.iter().map(BindingRecord::usage).collect()
	}

	#[test]
	fn binding_collector_preserves_msl_intrinsic_order_and_first_wins_access() {
		let root = besl::Node::root();
		let void_type = root.get_child("void").expect("Expected the built-in void type");
		let intrinsic: besl::NodeReference = besl::Node::intrinsic(
			"binding_order_fixture",
			vec![
				binding("definition_first", 0, 0, true, false),
				binding("definition_only", 1, 2, true, true),
			],
			void_type.clone(),
		)
		.into();
		// Conflicting duplicate slots make traversal order and first-wins behavior observable without compiling MSL.
		let call = besl::Node::expression(besl::Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: vec![
				binding("element_first", 0, 0, false, true),
				binding("element_duplicate", 0, 0, true, true),
			],
		})
		.into();
		let main: besl::NodeReference = besl::Node::function("main", Vec::new(), void_type, vec![call]).into();

		let compiled =
			collect_bindings::<CompiledShaderBinding>(&main, IntrinsicBindingTraversalOrder::ElementsBeforeDefinition);
		assert_eq!(usage(&compiled), vec![(0, 0, false, true), (1, 2, true, true)]);

		let evaluated = collect_bindings::<BindingUsage>(&main, IntrinsicBindingTraversalOrder::DefinitionBeforeElements);
		assert_eq!(usage(&evaluated), vec![(0, 0, true, false), (1, 2, true, true)]);
	}

	#[test]
	fn tool_failure_includes_exit_status_and_stderr() {
		let failure = format_tool_failure(
			"Failed to compile MSL shader",
			"The Metal compiler reported an error",
			"1",
			b"",
			b"shader.metal:7:3: error: unknown identifier\n",
		);

		assert_eq!(
			failure,
			"Failed to compile MSL shader. The Metal compiler reported an error.\n\
Exit status: 1\n\
stderr:\n\
shader.metal:7:3: error: unknown identifier\n\
stdout:\n\
<empty>"
		);
	}

	#[test]
	fn tool_failure_includes_stdout_when_stderr_is_empty() {
		let failure = format_tool_failure(
			"Failed to link Metal library",
			"The metallib tool reported an error",
			"2",
			b"metallib: malformed AIR input\n",
			b"",
		);

		assert_eq!(
			failure,
			"Failed to link Metal library. The metallib tool reported an error.\n\
Exit status: 2\n\
stderr:\n\
<empty>\n\
stdout:\n\
metallib: malformed AIR input"
		);
	}
}
