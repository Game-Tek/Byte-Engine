//! Bake source assets and inspect Byte-Engine resource stores.
//!
//! Call one of the command functions, such as [`bake`] or [`list`], from an
//! [`Executor`] runtime.

// Command integration tests keep complete CLI workflows together for readability.
#![allow(clippy::too_many_lines)]

mod commands;
mod utils;

pub use commands::{bake, clear, delete, inspect, list, query, wipe};
pub use resource_management::r#async::Executor;

/// Selects the representation written by commands that return structured output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum OutputFormat {
	/// Writes output for a person reading a terminal.
	Human,
	/// Writes stable JSON for scripts and editor integrations.
	JSON,
}
