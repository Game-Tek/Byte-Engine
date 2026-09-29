//! UDP implementation of a BETP server.

use std::net::ToSocketAddrs;

use betp::{
	packets::Packets,
	server::{Events, interface::ConnectionResults, session::Session},
};

/// The `Server` struct provides an authoritative BETP endpoint over UDP.
pub struct Server {
	clients: [Option<Session>; 64],
	socket: std::net::UdpSocket,
}

impl Server {
	/// Creates a server bound to the specified address.
	pub fn new<A: ToSocketAddrs>(address: A) -> Result<Self, ()> {
		Ok(Self {
			clients: [None; 64],
			socket: std::net::UdpSocket::bind(address).map_err(|_| ())?,
		})
	}
}

impl betp::Server for Server {
	fn update(&mut self, current_time: std::time::Instant) -> Result<Vec<Events>, ConnectionResults> {
		let socket = &mut self.socket;
		let events = Vec::with_capacity(256);

		let mut buffer = [0u8; 1024];

		let _ = socket.recv(&mut buffer);

		let packets: [Packets; _] = [];

		for packet in &packets {
			match packet {
				Packets::ConnectionRequest(_) => {}
				Packets::Challenge(_) => {}
				Packets::ChallengeResponse(_) => {}
				Packets::Data(_) => {}
				Packets::Disconnect(_) => {}
			}
		}

		for client in self.clients.iter_mut().filter_map(Option::as_mut) {
			client
				.update(&packets, current_time)
				.map_err(|_| ConnectionResults::ServerFull)?;
		}

		Ok(events)
	}

	fn disconnect(&mut self) {
		for client in self.clients.iter_mut().filter_map(Option::as_mut) {
			client.disconnect();
		}
	}

	fn disconnect_client(&mut self, connection_id: u64) {
		if let Some(client) = self
			.clients
			.iter_mut()
			.filter_map(Option::as_mut)
			.find(|c| c.connection_id() == Some(connection_id))
		{
			client.disconnect();
		}
	}

	fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		for client in self.clients.iter_mut().filter_map(Option::as_mut) {
			client.send(reliable, data);
		}
	}

	fn send_to_client(&mut self, connection_id: u64, reliable: bool, data: [u8; 1024]) {
		if let Some(client) = self
			.clients
			.iter_mut()
			.filter_map(Option::as_mut)
			.find(|c| c.connection_id() == Some(connection_id))
		{
			client.send(reliable, data);
		}
	}
}
