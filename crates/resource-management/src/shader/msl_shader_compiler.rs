use std::{
	fs,
	path::{Path, PathBuf},
	sync::{
		Arc, LazyLock, OnceLock,
		atomic::{AtomicBool, Ordering},
	},
	time::{SystemTime, UNIX_EPOCH},
};

use announcement::Announcement;
use utils::{
	hash::{HashMap, HashSet},
	sync::Mutex,
};

use crate::shader::besl::backends::msl::MSL_ENTRY_POINT;

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

/// Whether compiled libraries record their source, which debug builds do so Xcode GPU captures can resolve generated
/// BESL back to MSL.
const RECORDS_SOURCES: bool = cfg!(debug_assertions);

/// Returns the build-dependent Metal compiler flags that [`compile_msl_source_to_metallib`] passes.
fn metal_build_arguments() -> &'static [&'static str] {
	// Every shader compiles as MSL 4.1, the version the GHI compiles source shaders at, whatever the SDK defaults to.
	// Preserve line tables and source when recording sources. Omit them in release builds to keep the compiled library
	// smaller.
	if RECORDS_SOURCES {
		&["-std=metal4.1", "-gline-tables-only", "-frecord-sources=yes"]
	} else {
		&["-std=metal4.1"]
	}
}

/// The toolchain query of [`metal_compiler_identity`], kept once it succeeds.
static COMPILER_IDENTITY: LazyLock<SharedRuns<(), Result<String, String>>> = LazyLock::new(Default::default);

/// Describes the installed Metal compiler and the flags [`compile_msl_source_to_metallib`] passes.
///
/// Baked shader reuse hashes this text, so a stored Metal library is only reused by the toolchain that produced it.
/// The toolchain is queried once per process: the first bakes of a run all ask at once, and each query starts a
/// compiler. A failed query is asked again by the next caller.
pub async fn metal_compiler_identity() -> Result<String, String> {
	run_shared(&COMPILER_IDENTITY, (), Result::is_ok, query_metal_compiler_identity).await
}

/// Asks the Metal compiler for its version and joins it with the build arguments.
async fn query_metal_compiler_identity() -> Result<String, String> {
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

/// The standard-library types the MSL backend emits, completed once so the Metal compiler can reuse them.
///
/// Completing a texture type makes the compiler instantiate its whole class template. That takes about 80 ms for each
/// type on an Apple M4 and dominates the compile time of every shader that reads a texture. The compiler also
/// completes the `memory_coherence_device` form of every sampled texture a shader names, so the header lists both. A
/// type this header leaves out still compiles; its shaders only pay for it again. Keep the list in step with the
/// texture types [`MSLTranspiler`](crate::shader::besl::backends::msl::MSLTranspiler) writes.
const PRELUDE_HEADER: &str = "\
#include <metal_stdlib>
using namespace metal;
struct _besl_prelude {
	texture2d<float> t0;
	texture2d<float, access::sample, memory_coherence_device> t1;
	texture2d<uint> t2;
	texture2d<uint, access::sample, memory_coherence_device> t3;
	texture2d_array<float> t4;
	texture2d_array<float, access::sample, memory_coherence_device> t5;
	texture3d<float> t6;
	texture3d<float, access::sample, memory_coherence_device> t7;
	texturecube<float> t8;
	texturecube<float, access::sample, memory_coherence_device> t9;
	texturecube_array<float> t10;
	texturecube_array<float, access::sample, memory_coherence_device> t11;
	texture2d<float, access::read> t12;
	texture2d<float, access::write> t13;
	texture2d<float, access::read_write> t14;
	texture2d<uint, access::read> t15;
	texture2d<uint, access::write> t16;
	texture2d<uint, access::read_write> t17;
	float4x4 m0;
	float4x3 m1;
};
";

/// Returns the precompiled form of [`PRELUDE_HEADER`], building it when this machine has none yet.
///
/// The file lives in the system temporary directory under a name derived from the compiler version, the build
/// arguments, the deployment target, and the header, so every process reuses it until one of them changes. Returns
/// `None` while another compile is building the header, or when it could not be built; shaders then compile without
/// it.
async fn precompiled_prelude() -> Option<&'static Path> {
	static PATH: OnceLock<PathBuf> = OnceLock::new();
	static BUILD_STARTED: AtomicBool = AtomicBool::new(false);

	let path = match PATH.get() {
		Some(path) => path,
		None => {
			// The compiler takes its target from this variable, which Cargo sets for tests, and a header built for
			// one target is rejected for another.
			let deployment_target = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_default();
			let identity = metal_compiler_identity().await.ok()?;
			let key = md5::compute(format!("{identity}\0{deployment_target}\0{PRELUDE_HEADER}"));
			PATH.get_or_init(|| std::env::temp_dir().join(format!("byte-engine-msl-prelude-{key:x}.pch")))
		}
	};

	if path.is_file() {
		return Some(path);
	}

	// A process builds the header once. Compiles that arrive meanwhile, or after a failed build, go without it.
	if BUILD_STARTED.swap(true, Ordering::Relaxed) {
		return None;
	}

	let built = build_precompiled_header(PRELUDE_HEADER, &["-x", "metal-header"], path).await;

	if let Err(error) = &built {
		log::warn!(
			"Failed to precompile the MSL prelude header, so shaders compile without it and take longer. The most likely cause is a Metal toolchain that does not accept the header.\n{error}"
		);
	}

	built.ok().map(|()| path.as_path())
}

/// Precompiles `header` with `arguments` into `path`, returning the compiler's diagnostics when it fails.
///
/// It builds in a private directory and renames, so no compile ever reads a half-written header.
async fn build_precompiled_header(header: &str, arguments: &[&str], path: &Path) -> Result<(), String> {
	let temp_dir = TempShaderDir::new("header")?;
	let building = temp_dir.path().join("header.pch");
	let output = run_metal(header, arguments, Some(&building)).await?;

	if !output.status.success() {
		return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
	}

	fs::rename(&building, path).map_err(|error| error.to_string())
}

/// The shortest declarations, in bytes, that [`declarations_header`] precompiles.
///
/// A header saves its shaders the time to check the declarations and costs about as much as one such check to build,
/// so it pays once a few shaders reuse it. Material evaluation shaders share about 75 KiB, which takes 0.1 s of each
/// 0.3 s compile on an Apple M4; most standalone shaders share nothing.
const MIN_SHARED_DECLARATIONS_BYTES: usize = 16 * 1024;

/// Splits `source` into the declarations before its entry point and the entry point onward, when the declarations are
/// at least [`MIN_SHARED_DECLARATIONS_BYTES`] long.
///
/// The MSL backend declares the entry point [`MSL_ENTRY_POINT`] once, after every declaration it uses, and starts it
/// with only its stage attributes and return type, so the split lands where the declaration before it ends: after a
/// `}`, a `;`, or a line break. Returns `None` for any other shape, which compiles whole.
fn split_at_entry_point(source: &str) -> Option<(&str, &str)> {
	let call = format!("{MSL_ENTRY_POINT}(");
	let entry = source.find(&call)?;
	if source[entry + call.len()..].contains(&call) {
		return None;
	}

	let start = source[..entry].rfind(['}', ';', '\n'])? + 1;
	(start >= MIN_SHARED_DECLARATIONS_BYTES).then(|| source.split_at(start))
}

/// The declarations of this process that one shader declared, by the digest of the prelude path and the declarations.
static SEEN_DECLARATIONS: LazyLock<Mutex<HashSet<[u8; 16]>>> = LazyLock::new(Default::default);

/// Whether each set of seen declarations has a header, by the same digest. A failed or rejected header stays `false`
/// for the rest of the process, so its shaders compile whole.
static DECLARATIONS_HEADERS: LazyLock<SharedRuns<[u8; 16], bool>> = LazyLock::new(Default::default);

/// The `DeclarationsHeaderPath` struct names a built declarations header, so a compile that the header fails can
/// withdraw it.
struct DeclarationsHeaderPath {
	key: [u8; 16],
	path: PathBuf,
}

/// Returns the precompiled form of `declarations`, chained on `prelude`, from the second shader of the process that
/// declares them on; the first returns `None` and compiles whole.
///
/// Every process builds its own headers and never reads one an earlier process left behind, so a header can't outlive a
/// change it should follow. The file lives in the system temporary directory under a name derived from the prelude
/// path, which follows the toolchain, the arguments, and the deployment target, and from the declarations, so a later
/// bake replaces it instead of adding another. Shaders that arrive while the header builds wait for it.
async fn declarations_header(declarations: &str, prelude: &Path) -> Option<DeclarationsHeaderPath> {
	let prelude_path = prelude.to_str()?;
	let digest = md5::compute(format!("{prelude_path}\0{declarations}"));
	let key = digest.0;
	if SEEN_DECLARATIONS.lock().insert(key) {
		return None;
	}

	let path = std::env::temp_dir().join(format!("byte-engine-msl-declarations-{digest:x}.pch"));
	let build = || async {
		let arguments = ["-x", "metal-header", "-include-pch", prelude_path];
		let built = build_precompiled_header(declarations, &arguments, &path).await;
		if let Err(error) = &built {
			log::debug!(
				"Failed to precompile the declarations a shader shares with others, so they compile whole. The most likely cause is a Metal toolchain that does not accept them as a header.\n{error}"
			);
		}
		built.is_ok()
	};
	let built = run_shared(&DECLARATIONS_HEADERS, key, |_| true, build).await;

	built.then_some(DeclarationsHeaderPath { key, path })
}

/// Stops every later compile of this process from using `header`, and deletes it so a later bake rebuilds it.
fn withdraw_declarations_header(header: &DeclarationsHeaderPath) {
	let (announcer, withdrawn) = Announcement::new();
	let _ = announcer.announce(false);
	DECLARATIONS_HEADERS.lock().insert(header.key, withdrawn);
	let _ = fs::remove_file(&header.path);
}

/// The `MetalDriver` struct locates the Metal compiler driver and the SDK it compiles against, so compiles start the
/// driver themselves instead of through `xcrun`.
struct MetalDriver {
	path: PathBuf,
	sdk: PathBuf,
}

/// Returns the Metal driver `xcrun -sdk macosx metal` runs, resolved once per process, or `None` when `xcrun` can't
/// locate it and compiles go through `xcrun`.
///
/// `xcrun` only finds the driver and sets `SDKROOT` before starting it, and a driver started with the same `SDKROOT`
/// receives the same compiler and linker commands. Each `xcrun` costs a process start, a tenth of a small shader's
/// compile. The first compiles of a process wait on the two queries, which take about 20 ms together.
fn metal_driver() -> Option<&'static MetalDriver> {
	static DRIVER: OnceLock<Option<MetalDriver>> = OnceLock::new();

	DRIVER
		.get_or_init(|| {
			let query = |arguments: &[&str]| {
				let output = std::process::Command::new("xcrun").args(arguments).output().ok()?;
				let path = String::from_utf8(output.stdout).ok()?;
				output.status.success().then(|| PathBuf::from(path.trim()))
			};
			Some(MetalDriver {
				path: query(&["-sdk", "macosx", "-f", "metal"])?,
				sdk: query(&["-sdk", "macosx", "--show-sdk-path"])?,
			})
		})
		.as_ref()
}

/// Runs the Metal compiler on `source`, with the build arguments every compile shares, and writes `output_path`, or
/// standard output when there is none, which the returned output then holds.
async fn run_metal(source: &str, arguments: &[&str], output_path: Option<&Path>) -> Result<std::process::Output, String> {
	let output_path = match output_path {
		Some(path) => path
			.to_str()
			.ok_or_else(|| error("Failed to compile MSL shader", "The temporary file path was not valid UTF-8"))?,
		None => "-",
	};

	// The Metal driver compiles and links when it writes a library, so `metallib` is not started separately. Source is
	// piped through stdin, and a library comes back through stdout, to avoid writing either to disk.
	let mut metal_cmd = match metal_driver() {
		Some(driver) => {
			let mut command = crate::r#async::Command::new(&driver.path);
			command.env("SDKROOT", &driver.sdk);
			command
		}
		None => {
			let mut command = crate::r#async::Command::new("xcrun");
			command.args(["-sdk", "macosx", "metal"]);
			command
		}
	};
	metal_cmd
		.args(arguments)
		.args(metal_build_arguments())
		.args(["-", "-o", output_path]);
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
			.write_all(source.as_bytes().to_vec())
			.await
			.0
			.map_err(|_| error("Failed to write MSL source to Metal compiler", "Stdin write failed"))?;
	}

	metal_process.wait_with_output().await.map_err(invoke_error)
}

/// The `SharedRuns` type holds the runs of [`run_shared`] by key: the ones in flight, and the finished ones it keeps.
type SharedRuns<K, T> = Mutex<HashMap<K, Announcement<T>>>;

/// Runs `run` once for every concurrent caller with `key`: the first runs it, and the others wait for its result.
///
/// `keep` decides whether the result also serves callers that arrive after it finished. A caller whose run stops
/// before it finishes, such as a cancelled bake, leaves no entry, and its waiters run on their own.
async fn run_shared<K, T, F>(runs: &SharedRuns<K, T>, key: K, keep: impl FnOnce(&T) -> bool, run: impl Fn() -> F) -> T
where
	K: std::hash::Hash + Eq + Clone,
	T: Clone + Send + Sync + 'static,
	F: Future<Output = T>,
{
	/// Removes the entry of a run that ends without being kept, however it ends.
	struct Entry<'a, K: std::hash::Hash + Eq, T> {
		runs: &'a SharedRuns<K, T>,
		key: Option<K>,
	}

	impl<K: std::hash::Hash + Eq, T> Drop for Entry<'_, K, T> {
		fn drop(&mut self) {
			if let Some(key) = self.key.take() {
				self.runs.lock().remove(&key);
			}
		}
	}

	let leader = {
		let mut runs = runs.lock();
		match runs.get(&key) {
			Some(announcement) => Err(announcement.listener()),
			None => {
				let (announcer, announcement) = Announcement::new();
				runs.insert(key.clone(), announcement);
				Ok(announcer)
			}
		}
	};

	match leader {
		Ok(announcer) => {
			let mut entry = Entry { runs, key: Some(key) };
			let result = run().await;
			if keep(&result) {
				entry.key = None;
			}
			let _ = announcer.announce(result.clone());
			result
		}
		Err(listener) => match listener.listen().await {
			Ok(result) => result,
			Err(_) => run().await,
		},
	}
}

/// The compiles running in this process, by the digest of their source.
static SHARED_COMPILES: LazyLock<SharedRuns<[u8; 16], Result<Arc<[u8]>, String>>> = LazyLock::new(Default::default);

/// Compiles Metal Shading Language source into a Metal library binary.
///
/// Requests for a source that is already compiling wait for that compile, because `name` only names temporary files and
/// two compiles of one source produce equivalent libraries. Materials of different containers often lower to the same
/// shader, such as the hit shaders of two FBX files that share a character.
///
/// The compile reuses the precompiled [`PRELUDE_HEADER`] when the toolchain accepts it, and in release builds the
/// precompiled declarations other shaders share; see [`declarations_header`]. Headers only save compiler work, so the
/// library is the same with or without them.
pub async fn compile_msl_source_to_metallib(msl_source: &str, name: &str) -> Result<Box<[u8]>, String> {
	if !cfg!(target_os = "macos") {
		return Err(error(
			"MSL compilation is only supported on macOS",
			"The Metal toolchain is not available on this platform",
		));
	}

	let run = || async { compile_unshared(msl_source, name).await.map(Arc::<[u8]>::from) };
	let library = run_shared(&SHARED_COMPILES, md5::compute(msl_source).0, |_| false, run).await;
	library.map(|library| Box::from(&*library))
}

/// Compiles `msl_source` with the precompiled headers that apply, falling back to compiling it whole so a shader's own
/// error is reported against its whole source. Failures name the shader `name`.
async fn compile_unshared(msl_source: &str, name: &str) -> Result<Box<[u8]>, String> {
	let prelude = precompiled_prelude().await;

	// Release builds compile the entry point alone against a header of the declarations before it, which other shaders,
	// such as every material evaluation shader of one feature set, repeat word for word. Debug builds compile whole,
	// because a library compiled against a header records only the entry point's source, which Xcode GPU captures read.
	let split = split_at_entry_point(msl_source).filter(|_| !RECORDS_SOURCES || cfg!(test));
	let shared_header = match (prelude, split) {
		(Some(prelude), Some((declarations, _))) => declarations_header(declarations, prelude).await,
		_ => None,
	};
	if let (Some(header), Some((_, entry_point))) = (&shared_header, split)
		&& let Some(header_path) = header.path.to_str()
	{
		let output = run_metal(entry_point, &["-x", "metal", "-include-pch", header_path], None).await?;
		if output.status.success() {
			return Ok(output.stdout.into_boxed_slice());
		}
	}

	let library = compile_with_prelude(msl_source, prelude.and_then(Path::to_str), name).await;

	// A failure against the declarations header is either a shader error or a header this toolchain no longer accepts.
	// The whole compile tells them apart, and only a rejected header is withdrawn.
	if let Some(header) = &shared_header
		&& library.is_ok()
	{
		withdraw_declarations_header(header);
	}

	library
}

/// Compiles `msl_source` whole, against the precompiled `prelude` when there is one, and returns the library.
async fn compile_with_prelude(msl_source: &str, prelude: Option<&str>, name: &str) -> Result<Box<[u8]>, String> {
	let mut metal_output = match prelude {
		Some(prelude) => run_metal(msl_source, &["-x", "metal", "-include-pch", prelude], None).await?,
		None => run_metal(msl_source, &["-x", "metal"], None).await?,
	};

	// A failure with the prelude is either a shader error or a prelude this toolchain no longer accepts, such as one
	// built before an SDK update. Compiling without it tells them apart and reports the shader's own error.
	if let Some(prelude) = prelude.filter(|_| !metal_output.status.success()) {
		let rejection = std::mem::replace(&mut metal_output, run_metal(msl_source, &["-x", "metal"], None).await?);

		// Removing the rejected prelude lets a later compile or run rebuild it. Of the concurrent compiles that
		// meet the same rejection, only the one that removes the file reports it.
		if metal_output.status.success() && fs::remove_file(prelude).is_ok() {
			log::warn!(
				"The Metal compiler rejected the precompiled MSL prelude header, so it was removed. The most likely cause is a toolchain or SDK update.\n{}",
				String::from_utf8_lossy(&rejection.stderr).trim()
			);
		}
	}

	if !metal_output.status.success() {
		return Err(format_tool_failure(
			&format!("Failed to compile MSL shader '{name}'"),
			"The Metal compiler reported an error",
			&metal_output,
		));
	}

	Ok(metal_output.stdout.into_boxed_slice())
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

		assert!(error.contains("Duplicate resource declaration at binding 3"), "{error}");
	}

	/// Returns an MSL source whose declarations, named after `tag`, are long enough to precompile, and whose entry point
	/// writes `body`.
	fn shader_with_shared_declarations(tag: &str, body: &str) -> String {
		let mut source = String::from("#include <metal_stdlib>\nusing namespace metal;\n");
		for index in 0..400 {
			source.push_str(&format!(
				"inline float {tag}_{index}(float x) {{ return x * {index}.0 + 1.0; }}\n"
			));
		}
		source.push_str(&format!(
			"kernel void {}(device float* out [[buffer(0)]], uint id [[thread_position_in_grid]]) {{ {body} }}\n",
			super::MSL_ENTRY_POINT
		));
		source
	}

	/// Verifies shaders that repeat the same declarations each compile to a library, as the second one builds the
	/// declarations header and the third compiles against it.
	#[compio::test]
	async fn shaders_sharing_declarations_compile() {
		for value in 1..=3 {
			let body = format!("out[id] = shared_ok_7(float(id)) + {value}.0;");
			let source = shader_with_shared_declarations("shared_ok", &body);
			let library = super::compile_msl_source_to_metallib(&source, "shared_ok")
				.await
				.expect("Expected a shader with shared declarations to compile");

			assert!(library.starts_with(b"MTLB"), "Expected a Metal library for shader {value}");
		}
	}

	/// Verifies an error in an entry point whose declarations are precompiled still fails the compile and names the
	/// shader.
	#[compio::test]
	async fn entry_point_errors_are_reported_with_shared_declarations() {
		for value in 1..=2 {
			let body = format!("out[id] = shared_error_3(float(id)) + {value}.0;");
			let source = shader_with_shared_declarations("shared_error", &body);
			super::compile_msl_source_to_metallib(&source, "shared_error")
				.await
				.expect("Expected a shader with shared declarations to compile");
		}

		let source = shader_with_shared_declarations("shared_error", "out[id] = undeclared_value;");
		let error = super::compile_msl_source_to_metallib(&source, "broken_entry")
			.await
			.expect_err("Expected an undeclared name in the entry point to fail");

		assert!(error.contains("'broken_entry'"), "{error}");
		assert!(error.contains("undeclared_value"), "{error}");
	}
}
