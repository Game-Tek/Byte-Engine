//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use std::{hash::{Hash, Hasher}, net::ToSocketAddrs};

use crate::{local::Local, packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packet, Packets}, remote::Remote};

/// The client is the entity that connects to a server and participates in the game.
pub struct Client {
	local: Local,
	remote: Remote,
	client_id: u64,
	/// The connection ID is a unique identifier for the client's connection/session to the server.
	connection_id: Option<u64>,
	salt: Option<u64>,
}

impl Client {
	/// Creates a client that will connect to the server at the specified address.
	/// Must call `connect` to establish a connection.
	pub fn new(address: std::net::SocketAddr) -> Result<Self, ()> {
		let client_id = machineid_rs::IdBuilder::new(machineid_rs::Encryption::MD5).add_component(machineid_rs::HWIDComponent::MacAddress).build("Byte-Engine").ok().ok_or(())?;

		let client_id = {
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			client_id.hash(&mut hasher);
			hasher.finish()
		};

		Ok(Self {
			local: Local::new(),
			remote: Remote::new(),
			client_id,
			connection_id: None,
			salt: None,
		})
	}

	/// Returns a packet to send to the server to establish a connection.
	pub fn connect(&mut self, current_time: std::time::Instant) -> ConnectionRequestPacket {
		let salt = current_time.elapsed().as_nanos() as u64;
		self.salt = Some(salt);
		ConnectionRequestPacket::new(salt)
	}

	/// Handles a packet received from the server.
	/// Returns a packet to send back to the server, if any.
	/// Returns `Err(())` if the packet is invalid.
	pub fn handle_packet(&mut self, packet: Packets) -> Result<Option<Packets>, ()> {
		match packet {
			Packets::Challenge(challenge_packet) => {
				if self.salt == Some(challenge_packet.get_client_salt()) {
					let connection_id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();
					self.connection_id = Some(connection_id);
					Ok(Some(Packets::ChallengeResponse(ChallengeResponsePacket::new(connection_id))))
				} else {
					Err(())
				}
			}
			Packets::Data(data_packet) => {
				if self.connection_id == Some(data_packet.get_connection_id()) { // Validate connection ID
					self.remote.acknowledge_packet(data_packet.get_connection_status().sequence);
					Ok(None)
				} else {
					Err(())
				}
			}
			Packets::Disconnect(disconnect_packet) => {
				if self.connection_id == Some(disconnect_packet.get_connection_id()) { // Validate connection ID
					self.connection_id = None;
					self.salt = None;
					Ok(None)
				} else {
					Err(())
				}
			}
			_ => Err(()),
		}
	}

	/// Returns a data packet to send to the server.
	pub fn send<const S: usize>(&mut self, data: [u8; S]) -> Result<DataPacket<S>, ()> {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();
		Ok(DataPacket::<S>::new(self.connection_id.unwrap_or(0), ConnectionStatus::new(sequence_number, ack, ack_bitfield), data))
	}

	/// Returns a disconnect packet to send to the server.
	/// The client will no longer be able to handle server packets after this.
	/// The client will need to reconnect to the server to continue.
	pub fn disconnect(&mut self) -> Result<DisconnectPacket, ()> {
		let packet = DisconnectPacket::new(self.connection_id.unwrap_or(0));
		self.connection_id = None;
		self.salt = None;
		Ok(packet)
	}
}

#[cfg(test)]
mod tests {
	use std::str::FromStr;

	use super::*;

	#[test]
	fn test_client_connect() {
		let client = Client::new(std::net::SocketAddr::from_str("localhost:6669").unwrap()).expect("Failed to connect to server.");
	}
}
