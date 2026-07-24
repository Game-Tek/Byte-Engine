//! In-memory server transport for deterministic replication tests.

/// The `ChannelServer` struct provides an engine server endpoint backed by
/// isolated in-process channels for each client.
pub struct ChannelServer {
	clients: Vec<ChannelConnection>,
	received: Vec<DataPacket<1024>>,
}

/// Each channel client needs independent handshake and BETP sequence state.
struct ChannelConnection {
	outgoing: Sender<Packets>,
	incoming: Receiver<Packets>,
	pending_connection_id: Option<u64>,
	session: Option<Session>,
}

impl ChannelServer {
	pub fn new() -> Self {
		Self {
			clients: Vec::new(),
			received: Vec::new(),
		}
	}

	/// Creates a client whose channel endpoints are owned by this server.
	pub fn client(&mut self) -> ChannelClient {
		let (server_to_client, client_incoming) = std::sync::mpsc::channel();
		let (client_to_server, server_incoming) = std::sync::mpsc::channel();

		self.clients.push(ChannelConnection {
			outgoing: server_to_client,
			incoming: server_incoming,
			pending_connection_id: None,
			session: None,
		});

		ChannelClient::new(client_incoming, client_to_server)
	}

	pub fn is_connected(&self) -> bool {
		self.clients
			.iter()
			.any(|client| client.session.as_ref().is_some_and(Session::is_connected))
	}

	pub fn connected_clients(&self) -> usize {
		self.clients
			.iter()
			.filter(|client| client.session.as_ref().is_some_and(Session::is_connected))
			.count()
	}

	/// Advances every client protocol session without blocking on its channel.
	pub fn update(&mut self, current_time: Instant) -> Result<Vec<Events>, ConnectionResults> {
		let mut events = Vec::new();

		for client in &mut self.clients {
			// Drain only this client's endpoint so packets cannot cross handshakes.
			let mut packets = Vec::new();

			while let Ok(packet) = client.incoming.try_recv() {
				packets.push(packet);
			}

			for packet in &packets {
				match packet {
					Packets::ConnectionRequest(request) => {
						let server_salt = 0x4254_4550;
						let connection_id = request.get_client_salt() ^ server_salt;
						client.pending_connection_id = Some(connection_id);
						client
							.outgoing
							.send(ChallengePacket::new(request.get_client_salt(), server_salt).into())
							.map_err(|_| ConnectionResults::ServerFull)?;
					}
					Packets::ChallengeResponse(response)
						if client.pending_connection_id == Some(response.get_connection_id()) =>
					{
						let id = response.get_connection_id();
						let mut session = Session::new();
						session.accept(id, current_time);
						client.session = Some(session);
						client.pending_connection_id = None;
						events.push(Events::ClientConnected { id });
					}
					Packets::Data(packet) => self.received.push(*packet),
					_ => {}
				}
			}

			if let Some(session) = &mut client.session {
				for packet in session
					.update(&packets, current_time)
					.map_err(|_| ConnectionResults::ServerFull)?
				{
					client.outgoing.send(packet).map_err(|_| ConnectionResults::ServerFull)?;
				}
			}
		}

		Ok(events)
	}

	/// Enqueues a payload for every connected client.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		for client in &mut self.clients {
			if let Some(session) = &mut client.session {
				session.send(reliable, data);
			}
		}
	}

	/// Removes and returns every application payload received since the last drain.
	pub fn drain_received(&mut self) -> impl Iterator<Item = DataPacket<1024>> + '_ {
		self.received.drain(..)
	}

	pub fn disconnect(&mut self) {
		for client in &mut self.clients {
			if let Some(session) = &mut client.session {
				session.disconnect();
			}
		}
	}
}

impl Default for ChannelServer {
	fn default() -> Self {
		Self::new()
	}
}

use std::{
	sync::mpsc::{Receiver, Sender, TryRecvError},
	time::Instant,
};

use betp::{
	packets::{ChallengePacket, DataPacket, Packets},
	server::{ConnectionResults, Events, Session},
};

use crate::network::client::ChannelClient;
