//! BETP client/server integration and replication contracts.
//!
//! Use [`Client`] and [`Server`] for protocol sessions. Implement
//! [`Replicable`] on application messages that need retry and importance
//! semantics; transport-specific UDP implementations remain behind the public
//! client and server modules.

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
				client.update().unwrap();
				server.update(Instant::now()).unwrap();
			}
			client.update().unwrap();

			assert!(client.is_connected());
			assert!(server.is_connected());

			client.send(true, [7; 1024]).unwrap();
			client.update().unwrap();
			server.update(Instant::now()).unwrap();
			assert_eq!(server.drain_received().next(), Some([7; 1024]));

			server.send(true, [9; 1024]);
			server.update(Instant::now()).unwrap();
			client.update().unwrap();
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
				client_a.update().unwrap();
				client_b.update().unwrap();
				server.update(Instant::now()).unwrap();
			}
			client_a.update().unwrap();
			client_b.update().unwrap();

			assert!(client_a.is_connected());
			assert!(client_b.is_connected());
			assert_eq!(server.connected_clients(), 2);
		}
	}
}
#[doc(hidden)]
pub mod replicable;
#[doc(hidden)]
pub mod server;

pub use client::Client;
pub use replicable::Importance;
pub use replicable::Replicable;
pub use server::Server;
