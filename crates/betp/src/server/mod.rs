//! The server module contains all code related to the server side of the implmentation of the BETP.
//! The server is the authoritative entity which manages connections to clients and maintains the state of the game.

pub mod server;
pub mod session;

pub use server::Server;
pub use session::Session;

/// Events that can occur on the server.
#[derive(Debug, Clone, Copy)]
pub enum Events {
	/// A client has connected to the server. Contains the client's ID.
	ClientConnected { id: u64 },
	/// A client has disconnected from the server. Contains the client's ID.
	ClientDisconnected { id: u64 },
}
