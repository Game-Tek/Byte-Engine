use std::fmt;

/// The `BeldError` struct reports why a BELD command or pipeline step failed, so the tool that called it decides how
/// to show the message and which exit code to return.
///
/// The `beld` binary logs [`Self::message`] and exits with [`Self::exit_code`]. A custom tool built from
/// [`crate::pipeline`] can do the same, or handle the failure its own way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeldError {
	message: String,
	exit_code: u8,
}

impl BeldError {
	/// Creates a failure that exits with code 1.
	pub fn new(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			exit_code: 1,
		}
	}

	/// Creates a failure caused by an invalid combination of arguments, which exits with code 2 like a parser error.
	pub fn invalid_arguments(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			exit_code: 2,
		}
	}

	/// Returns what failed, its most likely cause, and how to fix it.
	pub fn message(&self) -> &str {
		&self.message
	}

	/// Returns the process exit code for this failure: 1 for a failed operation, 2 for invalid arguments.
	pub fn exit_code(&self) -> u8 {
		self.exit_code
	}
}

impl fmt::Display for BeldError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.message)
	}
}

impl std::error::Error for BeldError {}
