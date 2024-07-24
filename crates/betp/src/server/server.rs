use crate::packets::{ChallengePacket, DataPacket, Packets};

use super::{super::packets::ConnectionStatus, client::Client};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionResults {
	ServerFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketHandlingResults {
	Undefined,
	ClientNotFound,
	BadConnectionId,
	UnhandleablePacket,
}

pub struct Settings {
	max_clients: usize,
	timeout: std::time::Duration,
}

/// A BETP authoritative server.
pub struct Server {
	settings: Settings,
	clients: [Option<Client>; 128],
}

impl Server {
	/// Creates a new server.
	///
	/// The server is created with the following settings:
	/// - `max_clients`: 32
	/// - `timeout`: 5 seconds
	pub fn new() -> Self {
		Self {
			settings: Settings {
				max_clients: 32,
				timeout: std::time::Duration::from_secs(5),
			},
			clients: [None; 128],
		}
	}

	/// Handles an incoming packet.
	/// - `origin`: The origin of the packet.
	/// - `packet`: The packet to handle.
	///
	/// If the return is Some, then the server produced a response packet.
	/// If the return is None, then no response packet was produced.
	/// If the return is an error, then the server encountered an error.
	pub fn handle_packet(&mut self, (origin, packet): (std::net::SocketAddr, Packets), current_time: std::time::Instant) -> Result<Option<Packets>, PacketHandlingResults> {
		match packet {
			Packets::ConnectionRequest(connection_request_packet) => {
				let (client_index, client_salt, server_salt) = self.connect(origin, connection_request_packet.get_client_salt(), current_time).map_err(|_| PacketHandlingResults::Undefined)?;
				Ok(Some(Packets::Challenge(ChallengePacket::new(client_salt, server_salt))))
			}
			Packets::Data(data_packet) => {
				let client_index = data_packet.get_connection_id();
				match self.clients.iter_mut().filter_map(|c| c.as_mut()).find(|c| c.address() == origin) { // Match the client by address
					Some(client) => {
						if client.connection_id() == client_index { // Validate the connection id
							client.receive(data_packet.get_connection_status(), current_time);
							Ok(None)
						} else {
							Err(PacketHandlingResults::BadConnectionId)
						}
					}
					None => {
						Err(PacketHandlingResults::ClientNotFound)
					}
				}
			}
			Packets::Disconnect(disconnect_packet) => {
				let _ = self.disconnect(origin, disconnect_packet.get_connection_id()).map_err(|_| PacketHandlingResults::Undefined)?;
				Ok(None)
			}
			_ => Err(PacketHandlingResults::UnhandleablePacket),
		}
	}

	/// Runs periodic updates on the server.
	/// Performs the following tasks:
	/// - Disconnects clients timed out clients.
	/// - `current_time`: The current time.
	/// Returns a list of packets to send to the clients.
	/// Returns an error if the server encountered an error.
	///
	/// This function should be called periodically.
	pub fn update(&mut self, current_time: std::time::Instant) -> Result<Vec<Packets>, ConnectionResults> {
		for client in self.clients.iter_mut() {
			if let Some(c) = client {
				if current_time.duration_since(c.last_seen()).as_secs() > 5 {
					// TODO: Disconnect the client.
					*client = None;
				}
			}
		}

		Ok(Vec::new())
	}

	/// Tries to connect a client to the server.
	/// - `address`: The address of the client.
	/// - `salt`: The connection salt.
	/// Returns the index of the client if a connection was successful or the client is already connected.
	/// Returns an error if the server is full.
	fn connect(&mut self, address: std::net::SocketAddr, salt: u64, current_time: std::time::Instant) -> Result<(u64, u64, u64), ConnectionResults> {
		// Check if the client is already connected.
		if let Some((i, Some(client))) = self.clients.iter().enumerate().find(|(i, client)| if let Some(client) = client { client.address() == address } else { false }) {
			return Ok((i as u64, client.client_salt(), client.server_salt()));
		}

		// Try to find an empty slot for the client.
		for (i, client) in self.clients.iter_mut().enumerate() {
			if client.is_none() {
				let server_salt = current_time.elapsed().as_nanos() as u64;
				*client = Some(Client::new(address, salt, server_salt, current_time));
				return Ok((i as u64, salt, server_salt));
			}
		}

		// We failed, the server is full.
		Err(ConnectionResults::ServerFull)
	}

	fn disconnect(&mut self, address: std::net::SocketAddr, connection_id: u64) -> Result<(), ()> {
		if let Some(client) = self.clients.iter_mut().find(|c| c.map_or(false, |c| c.address() == address && c.connection_id() == connection_id)) {
			*client = None;
			Ok(())
		} else {
			Err(())
		}
	}

	pub fn send<const S: usize>(&mut self, client_address: std::net::SocketAddr, data: [u8; S]) -> Result<DataPacket<S>, ConnectionResults> {
		if let Some(client) = self.clients.iter_mut().filter_map(|c| c.as_mut()).find(|c| c.address() == client_address) {
			Ok(DataPacket::new(client.connection_id(), client.send(), data))
		} else {
			Err(ConnectionResults::ServerFull)
		}
	}

	fn receive(&mut self, client_index: u64, current_time: std::time::Instant) {
		if let Some(client) = self.clients[client_index as usize].as_mut() {
			client.receive(ConnectionStatus::new(0, 0, 0), current_time);
		}
	}

	fn get_client(&self, address: std::net::SocketAddr) -> Option<&Client> {
		self.clients.iter().filter_map(|c| c.as_ref()).find(|c| c.address() == address)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_server_connect() {
		let mut server = Server::new();

		let client_address = std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669);

		let (client_index, _, _) = server.connect(client_address, 1, std::time::Instant::now()).unwrap();

		let _ = server.send(client_address, [0; 1024]);
		server.receive(client_index, std::time::Instant::now());
	}

	#[test]
	fn test_server_reconnect() {
		let mut server = Server::new();

		let (client_index_0, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, std::time::Instant::now()).unwrap();
		let (client_index_1, _, _) = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 2, std::time::Instant::now()).unwrap();

		assert_eq!(client_index_0, client_index_1);
	}

	#[test]
	fn test_server_disconnect() {
		let mut server = Server::new();

		let client_address = std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669);

		let (client_index, _, _) = server.connect(client_address, 1, std::time::Instant::now()).unwrap();

		let client = server.get_client(client_address).unwrap();

		let _ = server.disconnect(client_address, client.connection_id());
	}

	#[test]
	fn test_exhaust_connections() {
		let mut server = Server::new();

		for i in 0..32 {
			server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), i), 1, std::time::Instant::now()).unwrap();
		}

		assert_eq!(server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 32), 1, std::time::Instant::now()), Err(ConnectionResults::ServerFull));
	}
}
