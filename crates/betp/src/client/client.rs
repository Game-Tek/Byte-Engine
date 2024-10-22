//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use std::{hash::{Hash, Hasher}, net::ToSocketAddrs};

use crate::{local::Local, packet_buffer::PacketBuffer, packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packet, Packets}, remote::Remote};

/// The client is the entity that connects to a server and participates in the game.
pub struct Client {
	local: Local,
	remote: Remote,
	/// The connection ID is a unique identifier for the client's connection/session to the server.
	connection_id: Option<u64>,
	salt: Option<u64>,
	packet_buffer: PacketBuffer<16, 1024>,
}

impl Client {
	/// Creates a client that will connect to the server at the specified address.
	/// Must call `connect` to establish a connection.
	pub fn new(address: std::net::SocketAddr) -> Result<Self, ()> {
		let _ = machineid_rs::IdBuilder::new(machineid_rs::Encryption::MD5).add_component(machineid_rs::HWIDComponent::MacAddress).build("Byte-Engine").ok().ok_or(())?;

		Ok(Self {
			local: Local::new(),
			remote: Remote::new(),
			connection_id: None,
			salt: None,
			packet_buffer: PacketBuffer::new(),
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
					self.packet_buffer.remove(data_packet.get_connection_status().sequence);
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

	/// Updates the client.
	/// Returns a list of packets to send to the server.
	pub fn update(&mut self, current_time: std::time::Instant) -> Result<Vec<Packets>, ()> {
		Ok(self.packet_buffer.gather_unsent_packets().into_iter().map(|p| Packets::Data(p)).collect())
	}

	/// Returns a data packet to send to the server.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<DataPacket<1024>, ()> {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();
		let packet = DataPacket::new(self.connection_id.unwrap_or(0), ConnectionStatus::new(sequence_number, ack, ack_bitfield), data);
		self.packet_buffer.add(packet.clone(), self.connection_id.unwrap_or(0), reliable);
		Ok(packet)
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
		let client = Client::new(std::net::SocketAddr::from_str("127.0.0.1:6669").unwrap()).expect("Failed to connect to server.");
	}
}