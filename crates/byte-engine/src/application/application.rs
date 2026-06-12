//! Core application contract and the minimal implementation used by higher-level runtimes.
//!
//! Implement [`Application`] for a new top-level runtime, or compose
//! [`BaseApplication`] into it to reuse parameter precedence, logging setup, and
//! frame-local allocation. [`crate::application::graphics::GraphicsApplication`]
//! is the main example of that composition.

//use utils::hash::HashSet; // Triggers address sanitation error

/// The [`Application`] trait defines the lifecycle contract for a process-level
/// Byte-Engine runtime.
///
/// Applications are intended to be singletons that own engine-wide state. Most
/// headed programs should use
/// [`crate::application::graphics::GraphicsApplication`] instead of implementing
/// this trait directly.
///
/// Parameters passed to [`Application::new`] may be overridden by `BE_*`
/// environment variables and then by `--name=value` command-line arguments.
pub trait Application {
	/// Creates a new application with the given name.
	fn new(name: &str, parameters: &[Parameter]) -> Self;

	/// Returns the name of the application.
	fn get_name(&self) -> &str;

	/// Performs a tick of the application.
	fn tick(&mut self) -> bool;
}

/// The [`BaseApplication`] struct provides shared process configuration and
/// frame-local storage for application implementations.
///
/// Embed it in a specialized application rather than using it as a complete game
/// loop. See [`crate::application::graphics::GraphicsApplication`] for the
/// established composition pattern.
pub struct BaseApplication {
	name: String,
	parameters: HashSet<Parameter>,
	pub(crate) frame_allocator: bumpalo::Bump,
}

impl Application for BaseApplication {
	fn new(name: &str, parameters: &[Parameter]) -> BaseApplication {
		env_logger::init();

		let parameters = parameters.to_vec();

		let environment_variables = std::env::vars()
			.filter(|(k, v)| k.as_str().starts_with("BE_"))
			.map(|(k, v)| Parameter::new_string(k.trim_start_matches("BE_").to_string().replace('_', "-").to_lowercase(), v))
			.collect::<Vec<Parameter>>();
		// Take all arguments that have the form `--name=value` and convert them to parameters.
		let arguments = std::env::args()
			.filter(|a| a.starts_with("--"))
			.map(|a| parse_argument(&a))
			.try_collect::<Vec<Parameter>>()
			.unwrap();

		let mut parameter_set: HashSet<Parameter> = parameters.into_iter().collect();
		parameter_set.extend(environment_variables);
		parameter_set.extend(arguments);

		let application = BaseApplication {
			name: String::from(name),
			parameters: parameter_set,
			frame_allocator: bumpalo::Bump::with_capacity(1024 * 1024 * 32), // TODO: take this from parameters
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

		if application.get_parameter("trace").is_some() {
			let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).try_init();
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

	fn tick(&mut self) -> bool {
		true
	}

	fn get_name(&self) -> &str {
		&self.name
	}
}

impl Parameters for BaseApplication {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.parameters.iter().find(|p| p.name == name)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn create_base_application() {
		let app = BaseApplication::new("Test", &[]);

		assert!(app.get_name() == "Test");
	}
}

use std::collections::HashSet;

use log::{info, trace};

use super::Parameter;
use crate::application::parameters::{parse_argument, Parameters};
