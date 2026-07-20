//! Application lifecycle and composition.
//!
//! Use [`BaseApplication`] when building a custom runtime that only needs parameter
//! handling and frame-local storage. Headed applications should start with
//! `GraphicsApplication` and choose either `default_setup` or individual
//! graphics setup functions.
//!
//! The `triangle` example demonstrates the standard setup path, while the
//! `window` example demonstrates selecting only one setup component.

use crate::time::MediaTime;

pub mod application;
#[doc(hidden)]
pub mod parameters;
pub mod thread;
#[doc(hidden)]
pub mod tracy;
pub use application::{Application, BaseApplication};
pub use tracy::{setup_tracy, TracySetupError};
pub use trotcast::Channel as Sender;
pub use trotcast::Receiver;

#[cfg(feature = "headed")]
pub mod graphics;

/// The [`Time`] struct provides frame timing to application systems without exposing
/// the lifecycle clock owned by the application.
#[derive(Debug, Clone, Copy)]
pub struct Time {
	elapsed: MediaTime,
	delta: MediaTime,
}

impl Time {
	/// Creates frame timing data for systems that run inside an application tick.
	pub fn new(elapsed: MediaTime, delta: MediaTime) -> Self {
		Self { elapsed, delta }
	}

	/// Returns the total time since the application started.
	pub fn elapsed(&self) -> MediaTime {
		self.elapsed
	}

	/// Returns the time since the previous application tick.
	pub fn delta(&self) -> MediaTime {
		self.delta
	}
}

/// The [`Parameter`] struct represents one application configuration value from
/// code, the environment, or command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
	name: String,
	value: String,
}

impl Parameter {
	/// Creates a parameter from borrowed name and value strings.
	pub fn new(name: &str, value: &str) -> Self {
		Parameter {
			name: name.into(),
			value: value.into(),
		}
	}

	/// Creates a parameter from owned name and value strings.
	pub fn new_string(name: String, value: String) -> Self {
		Parameter { name, value }
	}

	/// Returns whether this parameter has the requested name.
	pub fn is(&self, name: &str) -> bool {
		self.name == name
	}

	/// Parses the parameter's value as a bool.
	/// Some(True) if param equals "true", "TRUE", "1"
	/// Some(False) if param equals "false", "FALSE", "0"
	/// Else None
	pub fn as_bool(&self) -> Option<bool> {
		match self.value.as_str() {
			"true" | "TRUE" | "1" => Some(true),
			"false" | "FALSE" | "0" => Some(false),
			_ => None,
		}
	}

	/// Parses the parameter's value as bool using `as_bool` but return false if it could not be parsed.
	/// This is provided as a convenience.
	pub fn as_bool_simple(&self) -> bool {
		self.as_bool().unwrap_or(false)
	}

	/// Returns the parameter name used by application configuration lookup.
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Returns the raw parameter value before caller-specific parsing.
	pub fn value(&self) -> &str {
		&self.value
	}
}

/// The [`Events`] enum defines messages shared with application-owned worker threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Events {
	/// Request the application to close.
	Close,
}
