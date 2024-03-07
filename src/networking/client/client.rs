use std::{hash::{Hash, Hasher}, net::ToSocketAddrs};

use crate::networking::{local::Local, packets::{ConnectionStatus, DataPacket}, remote::Remote};

use super::server::Server;

pub struct Client {
	local: Local,
	remote: Remote,
	server: Server,
	client_id: u64,
	connection_id: u64,
}

impl Client {
	pub fn connect(str: &str) -> Option<Self> {
		let client_id = machineid_rs::IdBuilder::new(machineid_rs::Encryption::MD5).add_component(machineid_rs::HWIDComponent::MacAddress).build("Byte-Engine").ok()?;

		let client_id = {
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			client_id.hash(&mut hasher);
			hasher.finish()
		};

		let socket_address = str.to_socket_addrs().ok()?;

		Some(Self {
			local: Local::new(),
			remote: Remote::new(),
			server: Server::new(str),
			client_id,
			connection_id: client_id,
		})
	}

	pub fn send(&mut self, data: &[u8]) {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();
		DataPacket::<512>::new(self.connection_id, ConnectionStatus::new(sequence_number, ack, ack_bitfield), [0; 512]);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_client_connect() {
		let client = Client::connect("localhost:6669").expect("Failed to connect to server.");
	}
}