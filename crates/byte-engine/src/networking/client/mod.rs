//! This module contains the client implementation.
//! The client is the entity that connects to a server and participates in the game.
//!
//! # Module Structure
//! - `client`: Contains the implementation of the client.
//! - `server`: Contains the implementation of the server as seen by the client.

pub mod client;
pub mod server;

pub use client::Client;
