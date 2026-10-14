//! BETP client/server integration and replication contracts.
//!
//! Host a session with [`Server::bind`] and join it with [`Client::connect`].
//! Both run over UDP and route every datagram through the [`datagram`]
//! pipelines. Implement [`Replicable`] on application messages that need retry
//! and importance semantics.

#[doc(hidden)]
pub mod client;
/// In-memory transport endpoints for deterministic replication tests.
pub mod channel {
	pub use super::{client::ChannelClient, server::ChannelServer};

	#[cfg(test)]
	mod tests {
		use std::time::Instant;

		use crate::network::server::ChannelServer;

		#[test]
		fn connects_and_delivers_payload_in_both_directions() {
			let mut server = ChannelServer::new();
			let mut client = server.client();

			client.connect(Instant::now());

			for _ in 0..3 {
				client.update(Instant::now()).expect("expected test value");
				server.update(Instant::now()).expect("expected test value");
			}
			client.update(Instant::now()).expect("expected test value");

			assert!(client.is_connected());
			assert!(server.is_connected());

			client.send(true, [7; 1024]).expect("expected test value");
			client.update(Instant::now()).expect("expected test value");
			server.update(Instant::now()).expect("expected test value");

			assert_eq!(server.drain_received().next().map(|e| e.data), Some([7; 1024]));

			server.send(true, [9; 1024]);
			server.update(Instant::now()).expect("expected test value");
			client.update(Instant::now()).expect("expected test value");

			assert_eq!(client.drain_received().next(), Some([9; 1024]));
		}

		#[test]
		fn connects_two_clients_without_crossing_handshakes() {
			let mut server = ChannelServer::new();
			let mut client_a = server.client();
			let mut client_b = server.client();

			client_a.connect(Instant::now());
			client_b.connect(Instant::now());

			for _ in 0..3 {
				client_a.update(Instant::now()).expect("expected test value");
				client_b.update(Instant::now()).expect("expected test value");
				server.update(Instant::now()).expect("expected test value");
			}
			client_a.update(Instant::now()).expect("expected test value");
			client_b.update(Instant::now()).expect("expected test value");

			assert!(client_a.is_connected());
			assert!(client_b.is_connected());
			assert_eq!(server.connected_clients(), 2);
		}
	}
}
#[cfg(test)]
mod adverse_tests;
/// Raw BETP datagram processing for transport adapters and defensive tests.
pub mod datagram;
#[doc(hidden)]
pub mod replicable;
#[doc(hidden)]
pub mod server;

pub use client::{Client, ClientCreateError};
pub use replicable::Importance;
pub use replicable::Replicable;
pub use server::{Server, ServerCreateError};

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
