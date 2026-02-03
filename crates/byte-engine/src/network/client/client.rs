use std::net::SocketAddr;

use betp::{self, Client as _};

use crate::{core::{Entity}, network::{Replicable, replicable::Importance, client::udp}};

/// The `Client` entity represents a client connection for a replicated application setup.
/// This class handles replication of application entities.
pub struct Client {
	client: udp::Client,
}

impl Client {
	pub fn new(server_address: SocketAddr) -> Result<Client, String> {
		Ok(Client {
			client: udp::Client::new(server_address).map_err(|_| "Failed to initilize BETP client.".to_string())?,
		})
	}

	pub fn connect(&mut self) {
		self.client.connect(std::time::Instant::now());
	}

	pub fn update(&mut self) {
		let _ = self.client.update();
	}

	pub fn disconnect(&mut self) {
		let _ = self.client.disconnect();
	}
}
