//! Provides the API and protocol state for BETP clients.

pub mod session;

pub use session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errors {
	BadSalt,
	BadConnectionId,
}
