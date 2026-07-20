//! Provides the API and protocol state for BETP clients.

pub mod interface;
pub mod session;

pub use interface::Client;
pub use session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errors {
	BadSalt,
	BadConnectionId,
	IoError,
}
