use std::{
	fs,
	path::{Path, PathBuf},
	time::{SystemTime, UNIX_EPOCH},
};

pub use crate::shader::generator::{CompiledShader as GeneratedShader, CompiledShaderBinding as Binding};
use crate::shader::{
	besl::{backends::msl::MSLTranspiler, evaluation::collect_bindings},
	generator::{CompiledShader, CompiledShaderBinding, ShaderGenerationSettings},
};

/// The `Compiler` struct exists to compile Metal Shading Language shaders into binary libraries.
pub struct Compiler {
	msl_transpiler: MSLTranspiler,
}

impl Default for Compiler {
	fn default() -> Self {
		Self::new()
	}
}

impl Compiler {
	pub fn new() -> Self {
		Self {
			msl_transpiler: MSLTranspiler::new(),
		}
	}

	pub async fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<GeneratedShader, String> {
		let msl_shader = self
			.msl_transpiler
			.generate_program(shader_compilation_settings, program)
			.map_err(|_| error("Failed to generate MSL shader source", "The MSL transpiler returned an error"))?;

		let binary = compile_msl_source_to_metallib(&msl_shader, &shader_compilation_settings.name).await?;

		Ok(CompiledShader {
			binary,
			bindings: collect_bindings(program)?
				.into_iter()
				.map(CompiledShaderBinding::from)
				.collect(),
			extent: shader_compilation_settings.stage.local_size(),
		})
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

/// Returns the build-dependent Metal compiler flags that [`compile_msl_source_to_metallib`] passes.
fn metal_build_arguments() -> &'static [&'static str] {
	// Preserve line tables and source in debug builds so Xcode GPU captures can resolve generated BESL back to MSL.
	// Omit in release builds to keep the compiled library smaller.
	if cfg!(debug_assertions) {
		&["-gline-tables-only", "-frecord-sources=yes"]
	} else {
		&[]
	}
}

/// Describes the installed Metal compiler and the flags [`compile_msl_source_to_metallib`] passes.
///
/// Baked shader reuse hashes this text, so a stored Metal library is only reused by the toolchain that produced it.
pub async fn metal_compiler_identity() -> Result<String, String> {
	let mut version_cmd = crate::r#async::Command::new("xcrun");
	version_cmd.args(["-sdk", "macosx", "metal", "--version"]);
	version_cmd
		.stdout(std::process::Stdio::piped())
		.map_err(|_| error("Failed to configure Metal compiler stdout", "Stdio pipe failed"))?;
	version_cmd
		.stderr(std::process::Stdio::piped())
		.map_err(|_| error("Failed to configure Metal compiler stderr", "Stdio pipe failed"))?;

	let output = version_cmd.output().await.map_err(invoke_error)?;

	if !output.status.success() {
		return Err(format_tool_failure(
			"Failed to query the Metal compiler version",
			"The Metal compiler could not report its version",
			&output,
		));
	}

	Ok(format!(
		"{}; arguments={:?}",
		String::from_utf8_lossy(&output.stdout).trim(),
		metal_build_arguments()
	))
}

/// Compiles Metal Shading Language source into a Metal library binary.
pub async fn compile_msl_source_to_metallib(msl_source: &str, name: &str) -> Result<Box<[u8]>, String> {
	if !cfg!(target_os = "macos") {
		return Err(error(
			"MSL compilation is only supported on macOS",
			"The Metal toolchain is not available on this platform",
		));
	}

	// The Metal driver compiles and links in one process when it writes a library, so no AIR intermediate is written
	// and `metallib` is not started separately. Source is piped through stdin to avoid writing it to disk.
	let safe_name = sanitize_shader_name(name);
	let temp_dir = TempShaderDir::new(&safe_name)?;
	let metallib_path = temp_dir.path().join(format!("{safe_name}.metallib"));
	let metallib_path_argument = metallib_path
		.to_str()
		.ok_or_else(|| error("Failed to compile MSL shader", "The temporary file path was not valid UTF-8"))?;

	let mut metal_cmd = crate::r#async::Command::new("xcrun");
	metal_cmd
		.args(["-sdk", "macosx", "metal", "-x", "metal"])
		.args(metal_build_arguments())
		.args(["-", "-o", metallib_path_argument]);
	metal_cmd
		.stdin(std::process::Stdio::piped())
		.map_err(|_| error("Failed to configure Metal compiler stdin", "Stdio pipe failed"))?;
	metal_cmd
		.stdout(std::process::Stdio::piped())
		.map_err(|_| error("Failed to configure Metal compiler stdout", "Stdio pipe failed"))?;
	metal_cmd
		.stderr(std::process::Stdio::piped())
		.map_err(|_| error("Failed to configure Metal compiler stderr", "Stdio pipe failed"))?;

	let mut metal_process = metal_cmd.spawn().map_err(invoke_error)?;

	if let Some(mut stdin) = metal_process.stdin.take() {
		use compio::io::AsyncWriteExt;
		stdin
			.write_all(msl_source.as_bytes().to_vec())
			.await
			.0
			.map_err(|_| error("Failed to write MSL source to Metal compiler", "Stdin write failed"))?;
	}

	let metal_output = metal_process.wait_with_output().await.map_err(invoke_error)?;

	if !metal_output.status.success() {
		return Err(format_tool_failure(
			"Failed to compile MSL shader",
			"The Metal compiler reported an error",
			&metal_output,
		));
	}

	let binary = crate::r#async::read(&metallib_path).await.map_err(|_| {
		error(
			"Failed to read compiled Metal library",
			"The Metal compiler did not create the library",
		)
	})?;

	Ok(binary.into_boxed_slice())
}

fn sanitize_shader_name(name: &str) -> String {
	let sanitized: String = name
		.chars()
		.map(|ch| {
			if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
				ch
			} else {
				'_'
			}
		})
		.collect();
	let trimmed = sanitized.trim_matches('_');
	if trimmed.is_empty() { "shader" } else { trimmed }.to_string()
}

fn error(message: &str, cause: &str) -> String {
	format!("{message}. {cause}.")
}

/// Reports that the Metal compiler process could not be started or awaited.
fn invoke_error<E>(_: E) -> String {
	error(
		"Failed to invoke the Metal compiler",
		"The Xcode command line tools may be missing",
	)
}

/// Formats a failed Metal tool run with its exit status and output.
///
/// `cause` is replaced when the `xcrun` diagnostics report that the optional Metal Toolchain component is missing.
fn format_tool_failure(message: &str, cause: &str, output: &std::process::Output) -> String {
	let exit_status = output
		.status
		.code()
		.map_or_else(|| output.status.to_string(), |code| code.to_string());
	let stderr = String::from_utf8_lossy(&output.stderr);
	let cause = if stderr.contains("missing Metal Toolchain") || stderr.contains("cannot execute tool 'metal'") {
		"The Metal Toolchain is missing; install it with `xcodebuild -downloadComponent MetalToolchain`"
	} else {
		cause
	};
	let stdout = String::from_utf8_lossy(&output.stdout);
	let stdout = stdout.trim();
	let stdout = if stdout.is_empty() { "<empty>" } else { stdout };
	let stderr = stderr.trim();
	let stderr = if stderr.is_empty() { "<empty>" } else { stderr };

	format!("{message}. {cause}.\nExit status: {exit_status}\nstderr:\n{stderr}\nstdout:\n{stdout}")
}

pub use Compiler as MSLShaderCompiler;

#[cfg(test)]
mod tests {
	use crate::shader::{
		besl::evaluation::{BindingUsage, collect_bindings},
		generator::tests::sampled_binding as binding,
	};

	fn usage(bindings: &[BindingUsage]) -> Vec<(u32, bool, bool)> {
		bindings
			.iter()
			.map(|binding| (binding.slot, binding.read, binding.write))
			.collect()
	}

	#[test]
	fn binding_collector_uses_only_instantiated_intrinsic_elements() {
		let root = besl::Node::root();
		let void_type = root.get_child("void").expect("Expected the built-in void type");
		let intrinsic: besl::NodeReference = besl::Node::intrinsic(
			"binding_order_fixture",
			vec![
				binding("definition_first", 0, true, false),
				binding("definition_only", 2, true, true),
			],
			void_type.clone(),
		)
		.into();
		// The intrinsic definition is only a template; emitted bindings come from the instantiated elements.
		let call = besl::Node::expression(besl::Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: vec![binding("instantiated", 100, true, false)],
		})
		.into();
		let main: besl::NodeReference = besl::Node::function("main", Vec::new(), void_type, vec![call]).into();

		let bindings = collect_bindings(&main).expect("Expected instantiated flat resource metadata");

		assert_eq!(usage(&bindings), vec![(100, true, false)]);
	}

	#[test]
	fn binding_collector_deduplicates_shared_binding_references() {
		let root = besl::Node::root();
		let void_type = root.get_child("void").expect("Expected the built-in void type");
		let shared = binding("shared", 3, true, false);
		let main: besl::NodeReference =
			besl::Node::function("main", Vec::new(), void_type, vec![shared.clone(), shared]).into();

		let bindings = collect_bindings(&main).expect("Expected one shared flat resource declaration");

		assert_eq!(usage(&bindings), vec![(3, true, false)]);
	}

	#[test]
	fn binding_collector_rejects_distinct_same_slot_declarations() {
		let root = besl::Node::root();
		let void_type = root.get_child("void").expect("Expected the built-in void type");
		let main: besl::NodeReference = besl::Node::function(
			"main",
			Vec::new(),
			void_type,
			vec![binding("first", 3, true, false), binding("second", 3, false, true)],
		)
		.into();

		let error = collect_bindings(&main).expect_err("Expected distinct same-slot declarations to fail");

		assert!(error.contains("Duplicate resource declaration at slot 3"));
	}
}
