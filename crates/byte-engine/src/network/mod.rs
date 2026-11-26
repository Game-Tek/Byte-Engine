//! BETP client/server integration and replication contracts.
//!
//! Host a session with [`Server::bind`] and join it with [`Client::connect`].
//! Both run over UDP and route every datagram through the [`datagram`]
//! pipelines. Implement [`Replicable`] on application messages that need retry
//! and importance semantics.

#[cfg(test)]
mod adverse_tests;
pub mod channel;
pub mod client;
/// Raw BETP datagram processing for transport adapters and defensive tests.
pub mod datagram;
pub mod replicable;
pub mod server;

/// Application data for one send, up to [`betp::packets::MAX_PAYLOAD_SIZE`] bytes.
pub use betp::packets::{MAX_PAYLOAD_SIZE, Payload, PayloadTooLarge};
/// The connection events [`Server::drain_events`] reports.
pub use betp::server::Events as ServerEvents;
pub use client::{Client, ClientCreateError};
pub use replicable::Importance;
pub use replicable::Replicable;
pub use server::{Server, ServerCreateError};

/// A one-byte payload of `fill`, for tests.
#[cfg(test)]
fn payload(fill: u8) -> Payload {
	Payload::new(&[fill]).expect("one byte fits a payload")
}

#[cfg(test)]
mod udp_tests;

/// Returns a fresh handshake salt. Both peers' salts make up the connection id, so each session needs its own.
fn random_salt() -> u64 {
	use std::hash::BuildHasher;

	// Every `RandomState` carries new keys, which is enough to keep concurrent sessions apart. BETP salts are not a
	// security boundary.
	std::hash::RandomState::new().hash_one(std::time::SystemTime::now())
}

/// Returns whether a socket error reports a past datagram rather than a broken socket. Datagrams to a closed port
/// come back as these on the next call, and they must not end the session.
fn is_transient(error: &std::io::Error) -> bool {
	matches!(
		error.kind(),
		std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::Interrupted
	)
}
