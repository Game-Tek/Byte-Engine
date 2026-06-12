//! The application module contains the application trait and some alternative implementations.\
//! An application is the main entry point of the engine and is responsible for initializing and deinitializing the engine.
//! It also contains the main loop of the engine.
//! An application MUST be a singleton and created before any other engine functionality is used.\
//! All state associated with the application/process should be stored in an application.

//use utils::hash::HashSet; // Triggers address sanitation error

/// The application trait is the main entry point of the engine.
/// It is responsible for initializing and deinitializing the engine.
/// It also contains the main loop of the engine.
/// An application MUST be a singleton and created before any other engine functionality is used.\
/// All state associated with the application/process should be stored in an application.
///
/// ## Features
/// ### Arguments
/// The application can take arguments during startup.
/// The arguments can be passed as OS environment variables in the form of `BE_NAME=value`, as command line arguments in the form of `--name=value`, or as parameters in code during the creation of the application.
///
/// Parameters as command line arguments take precedence over environment variables which take precedence over parameters in code.
/// Parameters < Environment variables < Command line arguments
pub trait Application {
	/// Creates a new application with the given name.
	fn new(name: &str, parameters: &[Parameter]) -> Self;

	/// Returns the name of the application.
	fn get_name(&self) -> &str;

	/// Performs a tick of the application.
	fn tick(&mut self) -> bool;
}

/// The most basic implementation of the application trait.
/// It has no functionality and is only used as a base for other implementations.
/// It just stores the name of the application.
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
