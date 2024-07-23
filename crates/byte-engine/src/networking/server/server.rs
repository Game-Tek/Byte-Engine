use crate::networking::packets::{ChallengePacket, Packets};

use super::{super::packets::ConnectionStatus, client::Client};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionResults {
	ServerFull,
}

pub struct Server {
	max_clients: usize,
	clients: [Option<Client>; 32],
}

impl Server {
	pub fn new() -> Self {
		Self {
			max_clients: 32,
			clients: [None; 32],
		}
	}

	/// Handles an incoming packet.
	/// - `origin`: The origin of the packet.
	/// - `packet`: The packet to handle.
	///
	/// If the return is Some, the server should send the packet to the client.
	/// If the return is None, the server should not send the packet to the client.
	pub fn handle_packet(&mut self, (origin, packet): (std::net::SocketAddr, Packets)) -> Result<Option<Packets>, ConnectionResults> {
		match packet {
			Packets::ConnectionRequest(connection_request_packet) => {
				let (client_index, client_salt, server_salt) = self.connect(origin, connection_request_packet.get_client_salt())?;
				Ok(Some(Packets::Challenge(ChallengePacket::new(client_salt, server_salt))))
			}
			Packets::Disconnect(disconnect_packet) => {
				self.disconnect(disconnect_packet.get_connection_id());
				Ok(None)
			}
			_ => Err(ConnectionResults::ServerFull),
		}
	}

	pub fn update(&mut self) {
		// TODO: if a client has not sent a packet in a certain amount of time, disconnect them.
	}

	/// Tries to connect a client to the server.
	/// - `address`: The address of the client.
	/// - `salt`: The connection salt.
	/// Returns the index of the client if a connection was successful or the client is already connected.
	/// Returns an error if the server is full.
	fn connect(&mut self, address: std::net::SocketAddr, salt: u64) -> Result<(u64, u64, u64), ConnectionResults> {
		// Check if the client is already connected.
		if let Some((i, Some(client))) = self.clients.iter().enumerate().find(|(i, client)| if let Some(client) = client { client.address() == address } else { false }) {
			return Ok((i as u64, client.client_salt(), client.server_salt()));
		}

		// Try to find an empty slot for the client.
		for (i, client) in self.clients.iter_mut().enumerate() {
			if client.is_none() {
				let server_salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
				*client = Some(Client::new(address, salt, server_salt));
				return Ok((i as u64, salt, server_salt));
			}
		}

		// We failed, the server is full.
		Err(ConnectionResults::ServerFull)
	}

	fn disconnect(&mut self, client_index: u64) {
		self.clients[client_index as usize] = None;
	}

	fn send(&mut self, client_index: u64,) {
		if let Some(client) = self.clients[client_index as usize].as_mut() {
			client.send();
		}
	}

	fn receive(&mut self, client_index: u64) {
		if let Some(client) = self.clients[client_index as usize].as_mut() {
			client.receive(ConnectionStatus::new(0, 0, 0));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_server_connect() {
		let mut server = Server::new();

		let (client_index, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1).unwrap();

		server.send(client_index);
		server.receive(client_index);
	}

	#[test]
	fn test_server_reconnect() {
		let mut server = Server::new();

		let (client_index_0, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1).unwrap();
		let (client_index_1, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 2).unwrap();

		assert_eq!(client_index_0, client_index_1);
	}

	#[test]
	fn test_server_disconnect() {
		let mut server = Server::new();

		let (client_index, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1).unwrap();

		server.disconnect(client_index);
	}

	#[test]
	fn test_exhaust_connections() {
		let mut server = Server::new();

		for i in 0..32 {
			server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), i), 1).unwrap();
		}

		assert_eq!(server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 32), 1), Err(ConnectionResults::ServerFull));
	}
}
