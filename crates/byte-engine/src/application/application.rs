//! The minimal process configuration shared by higher-level runtimes.
//!
//! Compose [`BaseApplication`] into a new top-level runtime to reuse parameter
//! precedence, logging setup, and frame-local allocation. `GraphicsApplication`
//! is the main headed example of that composition.

use std::sync::Arc;

use crate::metrics::{Metrics, MetricsLayer};

/// The [`BaseApplication`] struct provides shared process configuration and
/// frame-local storage for application implementations.
///
/// Embed it in a specialized application rather than using it as a complete game
/// loop. `GraphicsApplication` uses the established headed composition pattern.
pub struct BaseApplication {
	name: String,
	parameters: Vec<Parameter>,
	pub(crate) frame_allocator: bumpalo::Bump,
	metrics: Arc<Metrics>,
}

impl BaseApplication {
	/// Creates the process configuration with the specified name and configuration parameters.
	///
	/// Parameters may be overridden by the project's `config.json`, then by `BE_*` environment variables, and then by
	/// `--name=value` command-line arguments. See [`parameters_from_json`] for the file's format. Applications are
	/// singletons: this also installs the process logger and the `tracing` subscriber that feeds span times into
	/// [`Self::metrics`].
	///
	/// # Configuration
	/// - `log.level`: Sets the most verbose `log` level that is printed: `trace`, `debug`, `info`, `warn`, `error`, or `off`.
	/// - `trace`: Prints every `tracing` span and event at `DEBUG` or less verbose levels to standard output.
	///
	/// With the `tracy` Cargo feature, spans and logs also stream to a connected Tracy profiler.
	pub fn new(name: &str, parameters: &[Parameter]) -> BaseApplication {
		let mut parameters = parameters.to_vec();
		for parameter in read_configuration_file(&resolve_application_path(None, CONFIGURATION_FILE_NAME)) {
			upsert_parameter(&mut parameters, parameter);
		}
		for (key, value) in std::env::vars().filter(|(key, _)| key.as_str().starts_with("BE_")) {
			upsert_parameter(
				&mut parameters,
				Parameter::new_string(
					key.trim_start_matches("BE_").to_string().replace('_', "-").to_lowercase(),
					value,
				),
			);
		}

		// Take all arguments that have the form `--name=value` and convert them to parameters.
		for argument in std::env::args().filter(|argument| argument.starts_with("--")) {
			upsert_parameter(&mut parameters, parse_argument(&argument).unwrap());
		}

		let metrics = Arc::new(Metrics::new());
		let trace = parameters.iter().any(|parameter| parameter.name == "trace");
		install_subscriber(Arc::clone(&metrics), trace);

		let application = BaseApplication {
			name: String::from(name),
			parameters,
			frame_allocator: bumpalo::Bump::with_capacity(1024 * 1024 * 32), // TODO: take this from parameters
			metrics,
		};

		if let Some(e) = application.get_parameter("log.level") {
			let level = match e.value.as_str() {
				"trace" => log::LevelFilter::Trace,
				"debug" => log::LevelFilter::Debug,
				"info" => log::LevelFilter::Info,
				"warn" => log::LevelFilter::Warn,
				"error" => log::LevelFilter::Error,
				"off" => log::LevelFilter::Off,
				_ => log::LevelFilter::Off,
			};

			log::set_max_level(level);
		}

		info!("Byte-Engine");
		info!(
			"Initializing \x1b[4m{}\x1b[24m application with parameters: {}.",
			name,
			application
				.parameters
				.iter()
				.map(|p| format!("{}={}", p.name, p.value))
				.collect::<Vec<String>>()
				.join(", ")
		);

		trace!("Initialized base Byte-Engine application!");

		application
	}

	/// Returns the name of the application.
	pub fn get_name(&self) -> &str {
		&self.name
	}

	/// Returns the per-tick performance samples every `tracing` span of the process reports into.
	///
	/// Register GPU or custom metrics on it with [`Metrics::register`]; the runtime that owns the tick calls
	/// [`Metrics::end_tick`].
	pub fn metrics(&self) -> &Arc<Metrics> {
		&self.metrics
	}

	/// Returns the resolved startup parameters after code, environment, and command-line precedence.
	pub(crate) fn parameters(&self) -> &[Parameter] {
		&self.parameters
	}
}

impl Parameters for BaseApplication {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.parameters.iter().find(|p| p.name == name)
	}
}

/// Installs the process logger and the global `tracing` subscriber, so span times reach `metrics`.
///
/// `log` and `tracing` allow one global collector each per process. When another one was installed first, the
/// engine keeps running without CPU metrics and says so.
fn install_subscriber(metrics: Arc<Metrics>, trace: bool) {
	use tracing_subscriber::{Layer as _, layer::SubscriberExt as _};

	let subscriber = tracing_subscriber::registry()
		.with(MetricsLayer::new(metrics))
		.with(trace.then(|| tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::LevelFilter::DEBUG)));

	#[cfg(feature = "tracy")]
	let subscriber = subscriber.with(tracing_tracy::TracyLayer::default());

	let logger_installed = {
		#[cfg(feature = "tracy")]
		{
			// Bridge `log` records into `tracing` so the Tracy layer receives engine logs too.
			tracing_log::LogTracer::init().is_ok()
		}
		#[cfg(not(feature = "tracy"))]
		{
			env_logger::try_init().is_ok()
		}
	};

	if tracing::subscriber::set_global_default(subscriber).is_err() {
		log::warn!(
			"Byte-Engine could not install its tracing subscriber, so the inspector reports no CPU metrics. The most likely cause is that the application installed another tracing subscriber before creating the application."
		);
	}
	if !logger_installed {
		log::warn!(
			"Byte-Engine could not install its logger. The most likely cause is that the application installed another logger before creating the application."
		);
	}
}

/// Reads the project's configuration file, or returns no parameters when the file does not exist.
///
/// # Panics
///
/// Panics when the file exists but cannot be read or parsed, so a typo does not silently drop settings.
fn read_configuration_file(path: &std::path::Path) -> Vec<Parameter> {
	let source = match std::fs::read_to_string(path) {
		Ok(source) => source,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
		Err(error) => panic!(
			"Configuration file '{}' could not be read. The most likely cause is missing read permission: {error}",
			path.display()
		),
	};
	parameters_from_json(&source).unwrap_or_else(|error| panic!("{error} File: '{}'.", path.display()))
}

/// Resolves an explicit path as supplied, or `default_path` inside the application directory.
///
/// The application directory is the Cargo package of a development binary, or the executable's directory otherwise.
pub(crate) fn resolve_application_path(parameter: Option<&Parameter>, default_path: &str) -> std::path::PathBuf {
	parameter.map(|parameter| parameter.value().into()).unwrap_or_else(|| {
		// Cargo provides the application manifest directory while running development binaries.
		#[cfg(debug_assertions)]
		if let Some(manifest_directory) = std::env::var_os("CARGO_MANIFEST_DIR") {
			return std::path::Path::new(&manifest_directory).join(default_path);
		}

		let executable = std::env::current_exe().unwrap_or_else(|error| {
			panic!(
				"Application directory could not be resolved. The most likely cause is that the current executable path is unavailable: {error}"
			)
		});
		executable
			.parent()
			.unwrap_or_else(|| {
				panic!(
					"Application directory could not be resolved. The most likely cause is that neither a Cargo manifest directory nor an executable parent is available."
				)
			})
			.join(default_path)
	})
}

/// Replaces a previous parameter with the same name so later sources have deterministic precedence.
fn upsert_parameter(parameters: &mut Vec<Parameter>, parameter: Parameter) {
	if let Some(existing) = parameters.iter_mut().find(|existing| existing.name == parameter.name) {
		*existing = parameter;
	} else {
		parameters.push(parameter);
	}
}

use log::{info, trace};

use super::Parameter;
use crate::application::parameters::{CONFIGURATION_FILE_NAME, Parameters, parameters_from_json, parse_argument};
