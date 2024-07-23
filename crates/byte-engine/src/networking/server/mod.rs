//! This module contains the server implementation.
//! The server is the authoritative entity which manages connections to clients and maintains the state of the game.
//!
//! # Module Structure
//! - `server`: Contains the implementation of the server.
//! - `client`: Contains the implementation of the client as seen by the server.

pub mod server;
mod client;

pub use server::Server;
