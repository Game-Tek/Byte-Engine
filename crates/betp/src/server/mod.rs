//! The server module contains all code related to the server side of the implmentation of the BETP.
//! The server is the authoritative entity which manages connections to clients and maintains the state of the game.

pub mod server;
pub mod session;
pub mod udp;

pub use server::Server;
pub use session::Session;

#[derive(Debug, Clone, Copy)]
pub enum Events {
	ClientConnected {
		id: u64,
	},
	ClientDisconnected {
		id: u64,
	},
}
