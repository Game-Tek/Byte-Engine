//! Core application contract and the minimal implementation used by higher-level runtimes.
//!
//! Implement [`Application`] for a new top-level runtime, or compose
//! [`BaseApplication`] into it to reuse parameter precedence, logging setup, and
//! frame-local allocation. `GraphicsApplication` is the main headed example of
//! that composition.

//use utils::hash::HashSet; // Triggers address sanitation error

/// The [`Application`] trait defines the lifecycle contract for a process-level
/// Byte-Engine runtime.
///
/// Applications are intended to be singletons that own engine-wide state. Most
/// headed programs should use `GraphicsApplication` instead of implementing this
/// trait directly.
///
/// Parameters passed to [`Application::new`] may be overridden by `BE_*`
/// environment variables and then by `--name=value` command-line arguments.
pub trait Application {
	/// Creates an application with the specified name and configuration parameters.
	fn new(name: &str, parameters: &[Parameter]) -> Self;

	/// Returns the name of the application.
	fn get_name(&self) -> &str;

	/// Advances the application by one tick.
	fn tick(&mut self) -> bool;
}

/// The [`BaseApplication`] struct provides shared process configuration and
/// frame-local storage for application implementations.
///
/// Embed it in a specialized application rather than using it as a complete game
/// loop. `GraphicsApplication` uses the established headed composition pattern.
pub struct BaseApplication {
	name: String,
	parameters: Vec<Parameter>,
	pub(crate) frame_allocator: bumpalo::Bump,
}

impl Application for BaseApplication {
	fn new(name: &str, parameters: &[Parameter]) -> BaseApplication {
		env_logger::init();

		let mut parameters = parameters.to_vec();
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

		let application = BaseApplication {
			name: String::from(name),
			parameters,
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

impl BaseApplication {
	/// Returns the resolved startup parameters after code, environment, and command-line precedence.
	pub(crate) fn parameters(&self) -> &[Parameter] {
		&self.parameters
	}
}

/// Replaces a previous parameter with the same name so later sources have deterministic precedence.
fn upsert_parameter(parameters: &mut Vec<Parameter>, parameter: Parameter) {
	if let Some(existing) = parameters.iter_mut().find(|existing| existing.name == parameter.name) {
		*existing = parameter;
	} else {
		parameters.push(parameter);
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

	#[test]
	fn upsert_parameter_replaces_value_with_same_name() {
		let mut parameters = vec![Parameter::new("render.debug.extended", "true")];

		upsert_parameter(&mut parameters, Parameter::new("render.debug.extended", "false"));

		assert_eq!(parameters.len(), 1);
		assert_eq!(parameters[0].value(), "false");
	}
}

use log::{info, trace};

use super::Parameter;
use crate::application::parameters::{parse_argument, Parameters};
