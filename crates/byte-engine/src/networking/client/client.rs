//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use std::{hash::{Hash, Hasher}, net::ToSocketAddrs};

use crate::networking::{local::Local, packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, Packets}, remote::Remote};

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
	/// Connects to a server at the specified address.
	pub fn connect(address: std::net::SocketAddr) -> Result<Self, ()> {
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

	pub fn request(&mut self) -> ConnectionRequestPacket {
		let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
		self.salt = Some(salt);
		ConnectionRequestPacket::new(salt)
	}

	pub fn handle_packet(&mut self, packet: Packets) -> Result<Packets, ()> {
		match packet {
			Packets::Challenge(challenge_packet) => {
				if self.salt == Some(challenge_packet.get_client_salt()) {
					let connection_id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();
					self.connection_id = Some(connection_id);
					Ok(Packets::ChallengeResponse(ChallengeResponsePacket::new(connection_id)))
				} else {
					Err(())
				}
			}
			_ => Err(()),
		}
	}

	pub fn send(&mut self, data: &[u8]) {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();
		DataPacket::<512>::new(self.connection_id.unwrap_or(0), ConnectionStatus::new(sequence_number, ack, ack_bitfield), [0; 512]);
	}
}

#[cfg(test)]
mod tests {
	use std::str::FromStr;

	use super::*;

	#[test]
	fn test_client_connect() {
		let client = Client::connect(std::net::SocketAddr::from_str("localhost:6669").unwrap()).expect("Failed to connect to server.");
	}
}
