//! The client module contains all code related to the client side of the implmentation of the BETP.

pub mod client;
pub mod session;

pub use client::Client;
pub use session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errors {
	BadSalt,
	BadConnectionId,
	IoError,
}
