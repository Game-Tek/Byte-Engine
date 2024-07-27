use std::time::Duration;

pub mod application;
pub use application::{Application, BaseApplication};

#[cfg(not(feature = "headless"))]
pub mod graphics_application;

#[cfg(not(feature = "headless"))]
pub use graphics_application::GraphicsApplication;

/// [`Time`] is used to query information about time from an application.
/// Is contains the elapsed time since the application started and the time since the last tick.
#[derive(Debug, Clone, Copy)]
pub struct Time {
	elapsed: Duration,
	delta: Duration,
}

impl Time {
	pub fn elapsed(&self) -> Duration {
		self.elapsed
	}

	pub fn delta(&self) -> Duration {
		self.delta
	}
}

/// A parameter for applications.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
	name: String, value: String,
}

impl Parameter {
	pub fn new(name: &str, value: &str) -> Self {
		Parameter { name: name.into(), value: value.into() }
	}

	pub fn new_string(name: String, value: String) -> Self {
		Parameter { name, value }
	}

	pub fn is(&self, name: &str) -> bool {
		self.name == name
	}
}