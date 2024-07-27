//! The application module contains the application trait and some alternative implementations.\
//! An application is the main entry point of the engine and is responsible for initializing and deinitializing the engine.
//! It also contains the main loop of the engine.
//! An application MUST be a singleton and created before any other engine functionality is used.\
//! All state associated with the application/process should be stored in an application.

use log::{info, trace};
use utils::hash::HashSet;

use super::Parameter;

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
	fn new(name: &str, parameters: &[Parameter],) -> Self;

	/// Returns the name of the application.
	fn get_name(&self) -> String;

	fn get_parameter(&self, name: &str) -> Option<&Parameter>;

	/// Performs a tick of the application.
	fn tick(&mut self);
}

/// The most basic implementation of the application trait.
/// It has no functionality and is only used as a base for other implementations.
/// It just stores the name of the application.
pub struct BaseApplication {
	name: String,
	parameters: HashSet<Parameter>,
}

impl Application for BaseApplication {
	fn new(name: &str, parameters: &[Parameter],) -> BaseApplication {
		let _ = simple_logger::SimpleLogger::new().env().init();

		let parameters = parameters.to_vec();
	
		let environment_variables = std::env::vars().filter(|(k, v)| k.as_str().starts_with("BE_")).map(|(k, v)| Parameter::new_string(k.trim_start_matches("BE_").to_string().replace('_', "-").to_lowercase(), v.into())).collect::<Vec<Parameter>>();
		// Take all arguments that have the form `--name=value` and convert them to parameters.
		let arguments = std::env::args().filter(|a| a.starts_with("--")).map(|a| {
			let mut split = a.split('=');
			let name = split.next().unwrap().trim_start_matches("--");
			let value = split.next().unwrap_or("");
			Parameter::new(name, value)
		}).collect::<Vec<Parameter>>();

		let mut parameter_set: HashSet<Parameter> = parameters.into_iter().collect();
		parameter_set.extend(environment_variables);
		parameter_set.extend(arguments);

		info!("Byte-Engine");
		info!("Initializing \x1b[4m{}\x1b[24m application with parameters: {}.", name, parameter_set.iter().map(|p| format!("{}={}", p.name, p.value)).collect::<Vec<String>>().join(", "));
	
		trace!("Initialized base Byte-Engine application!");

		BaseApplication { name: String::from(name), parameters: parameter_set }
	}

	fn tick(&mut self) {}

	fn get_name(&self) -> String { self.name.clone() }

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
