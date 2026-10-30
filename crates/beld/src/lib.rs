//! Bake source assets and inspect Byte-Engine resource stores.
//!
//! Give each application its own BELD binary, whose `main` calls [`run`] with the parameters and asset handlers the
//! application runs with, so its baked store matches what a debug run bakes. To run one command from code instead,
//! call a command function, such as [`bake`] or [`list`], from an [`Executor`] runtime.

// Command integration tests keep complete CLI workflows together for readability.
#![allow(clippy::too_many_lines)]

mod cli;
mod commands;
mod terminal;
mod utils;

pub use cli::run;
pub use commands::{bake, bake_dry_run, browse, clear, delete, inspect, list, query, wipe};
pub use resource_management::r#async::Executor;
pub use terminal::init_logging;

/// Selects the representation written by commands that return structured output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum OutputFormat {
	/// Writes output for a person reading a terminal.
	Human,
	/// Writes stable JSON for scripts and editor integrations.
	JSON,
}

/// Selects whether commands that print resources add each payload stream's bytes, and in which form.
///
/// Anything other than [`Payload::None`] needs [`OutputFormat::JSON`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum Payload {
	/// Prints stream descriptions only.
	#[default]
	None,
	/// Decodes streams with a known layout into values and prints the rest as base64.
	Typed,
	/// Prints every stream as base64.
	Base64,
}
