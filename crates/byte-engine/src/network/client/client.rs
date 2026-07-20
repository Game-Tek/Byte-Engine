use std::net::SocketAddr;

use betp::{self, Client as _};

use crate::{
	core::Entity,
	network::{client::udp, replicable::Importance, Replicable},
};

/// The `Client` struct provides the application-facing connection for a
/// replicated client.
pub struct Client {
	client: udp::Client,
}

impl Client {
	pub fn new(server_address: SocketAddr) -> Result<Client, String> {
		Ok(Client {
			client: udp::Client::new(server_address).map_err(|error| {
				format!("Failed to initialize BETP client. The most likely cause is a UDP setup error: {error}")
			})?,
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
