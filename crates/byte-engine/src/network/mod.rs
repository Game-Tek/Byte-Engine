//! BETP client/server integration and replication contracts.
//!
//! Use [`Client`] and [`Server`] for protocol sessions. Implement
//! [`Replicable`] on application messages that need retry and importance
//! semantics; transport-specific UDP implementations remain behind the public
//! client and server modules.

pub mod client;
pub mod replicable;
pub mod server;

pub use client::Client;
pub use replicable::Replicable;
pub use server::Server;
