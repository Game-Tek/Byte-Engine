//! In-memory transport endpoints for deterministic replication tests.
//!
//! They route every datagram through the same [`datagram`](super::datagram) pipelines as the UDP [`Client`] and
//! [`Server`], over in-process channels instead of a socket.
//!
//! [`Client`]: super::Client
//! [`Server`]: super::Server

use std::{
	sync::mpsc::{Receiver, Sender, channel},
	time::Instant,
};

use betp::{packets::Payload, server::Events};

use super::datagram::{ClientDatagramPipeline, DatagramOutcome, EncodedDatagram, ServerDatagramPipeline};

/// The `Endpoint` struct is one end of an in-process connection: its pipeline and the channels to its peer.
struct Endpoint<P> {
	pipeline: P,
	incoming: Receiver<EncodedDatagram>,
	outgoing: Sender<EncodedDatagram>,
	outbound: Vec<EncodedDatagram>,
}

impl<P> Endpoint<P> {
	/// Sends what the pipeline produced. A peer that was dropped loses it, as a UDP peer would.
	fn send_outbound(&mut self) {
		for datagram in self.outbound.drain(..) {
			let _ = self.outgoing.send(datagram);
		}
	}
}

/// The `ChannelServer` struct hosts BETP sessions for in-process [`ChannelClient`]s, so tests replicate without a
/// socket.
///
/// Create clients with [`Self::client`], then call [`Self::update`] and each client's [`ChannelClient::update`] once
/// per tick. Read what arrived with [`Self::drain_events`] and [`Self::drain_received`].
#[derive(Default)]
pub struct ChannelServer {
	peers: Vec<Endpoint<ServerDatagramPipeline>>,
	events: Vec<Events>,
	received: Vec<(u64, Payload)>,
}

impl ChannelServer {
	/// Creates a server without clients.
	pub fn new() -> Self {
		Self::default()
	}

	/// Creates a client that starts its handshake with this server on its first [`ChannelClient::update`].
	pub fn client(&mut self) -> ChannelClient {
		let (to_client, client_incoming) = channel();
		let (to_server, server_incoming) = channel();
		// Each client gets its own salt, so every connection id differs and tests stay deterministic.
		let salt = self.peers.len() as u64 + 1;
		self.peers.push(Endpoint {
			pipeline: ServerDatagramPipeline::default(),
			incoming: server_incoming,
			outgoing: to_client,
			outbound: Vec::new(),
		});
		ChannelClient(Endpoint {
			pipeline: ClientDatagramPipeline::new(salt),
			incoming: client_incoming,
			outgoing: to_server,
			outbound: Vec::new(),
		})
	}

	/// Returns whether any client finished the handshake.
	pub fn is_connected(&self) -> bool {
		self.connected_clients() > 0
	}

	/// Returns how many clients finished the handshake and have not disconnected.
	pub fn connected_clients(&self) -> usize {
		self.peers.iter().filter(|peer| peer.pipeline.is_connected()).count()
	}

	/// Reads every datagram the clients sent, then advances retries, keepalives, and timeouts for each of them.
	pub fn update(&mut self, current_time: Instant) {
		for peer in &mut self.peers {
			while let Ok(datagram) = peer.incoming.try_recv() {
				let outcome = peer
					.pipeline
					.process_datagram(datagram.as_bytes(), current_time, &mut peer.outbound);
				peer.send_outbound();
				match outcome {
					DatagramOutcome::Connected { id } => self.events.push(Events::ClientConnected { id }),
					DatagramOutcome::Disconnected { id } => self.events.push(Events::ClientDisconnected { id }),
					DatagramOutcome::Handled | DatagramOutcome::Dropped(_) => {}
				}
			}
			if let DatagramOutcome::Disconnected { id } = peer.pipeline.advance(current_time, &mut peer.outbound) {
				self.events.push(Events::ClientDisconnected { id });
			}
			peer.send_outbound();
			if let Some(id) = peer.pipeline.connection_id() {
				self.received.extend(peer.pipeline.drain_received().map(|data| (id, data)));
			}
		}
	}

	/// Queues a payload for every connected client.
	pub fn send(&mut self, reliable: bool, data: Payload) {
		for peer in self.peers.iter_mut().filter(|peer| peer.pipeline.is_connected()) {
			peer.pipeline.send(reliable, data);
		}
	}

	/// Removes the connection events gathered by [`Self::update`].
	pub fn drain_events(&mut self) -> impl Iterator<Item = Events> + '_ {
		self.events.drain(..)
	}

	/// Removes and returns every application payload received since the last drain, with its sender's connection id.
	pub fn drain_received(&mut self) -> impl Iterator<Item = (u64, Payload)> + '_ {
		self.received.drain(..)
	}
}

/// The `ChannelClient` struct joins a [`ChannelServer`] over in-process channels.
///
/// Get one from [`ChannelServer::client`], then call [`Self::update`] once per tick.
pub struct ChannelClient(Endpoint<ClientDatagramPipeline>);

impl ChannelClient {
	/// Returns whether the server confirmed the connection.
	pub fn is_connected(&self) -> bool {
		self.0.pipeline.is_connected()
	}

	/// Queues a payload for the server. The next [`Self::update`] sends it.
	pub fn send(&mut self, reliable: bool, data: Payload) {
		self.0.pipeline.send(reliable, data);
	}

	/// Reads every datagram the server sent, then sends handshake, payload, retry, and keepalive datagrams.
	pub fn update(&mut self, current_time: Instant) {
		let endpoint = &mut self.0;
		while let Ok(datagram) = endpoint.incoming.try_recv() {
			endpoint
				.pipeline
				.process_datagram(datagram.as_bytes(), current_time, &mut endpoint.outbound);
			endpoint.send_outbound();
		}
		endpoint.pipeline.advance(current_time, &mut endpoint.outbound);
		endpoint.send_outbound();
	}

	/// Removes and returns every application payload received since the last drain.
	pub fn drain_received(&mut self) -> impl Iterator<Item = Payload> + '_ {
		self.0.pipeline.drain_received()
	}
}

#[cfg(test)]
mod tests {
	use std::time::Instant;

	use super::*;
	use crate::network::payload;

	/// Runs enough updates for a handshake or a payload round trip to finish.
	fn exchange(server: &mut ChannelServer, clients: &mut [&mut ChannelClient]) {
		for _ in 0..4 {
			for client in clients.iter_mut() {
				client.update(Instant::now());
			}
			server.update(Instant::now());
		}
	}

	#[test]
	fn two_clients_connect_and_exchange_payloads_without_crossing_sessions() {
		let mut server = ChannelServer::new();
		let (mut client_a, mut client_b) = (server.client(), server.client());
		exchange(&mut server, &mut [&mut client_a, &mut client_b]);

		assert!(client_a.is_connected() && client_b.is_connected());
		assert_eq!(server.connected_clients(), 2);
		assert_eq!(server.drain_events().count(), 2);

		client_a.send(true, payload(7));
		server.send(true, payload(9));
		exchange(&mut server, &mut [&mut client_a, &mut client_b]);

		assert_eq!(
			server.drain_received().map(|(_, data)| data).collect::<Vec<_>>(),
			[payload(7)]
		);
		assert_eq!(client_a.drain_received().collect::<Vec<_>>(), [payload(9)]);
		assert_eq!(client_b.drain_received().collect::<Vec<_>>(), [payload(9)]);
	}
}
