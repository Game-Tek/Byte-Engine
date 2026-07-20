//! Provides the API and protocol state for an authoritative BETP server.

pub mod interface;
pub mod session;

pub use interface::{ConnectionResults, PacketHandlingResults, Server, Settings};
pub use session::Session;

/// An event produced by a BETP server.
#[derive(Debug, Clone, Copy)]
pub enum Events {
	/// A client connected with the given ID.
	ClientConnected { id: u64 },
	/// A client with the given ID disconnected.
	ClientDisconnected { id: u64 },
}
