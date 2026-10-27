//! Bake source assets and inspect Byte-Engine resource stores.
//!
//! Call one of the command functions, such as [`bake`] or [`list`], from an
//! [`Executor`] runtime. To build your own version of BELD, for example one that
//! rejects resources over a size limit, compose the steps in [`pipeline`].

// Command integration tests keep complete CLI workflows together for readability.
#![allow(clippy::too_many_lines)]

mod commands;
pub mod pipeline;

pub use commands::{bake, bake_dry_run, clear, delete, inspect, list, query, wipe};
pub use resource_management::r#async::Executor;

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
