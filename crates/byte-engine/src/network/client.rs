use std::net::SocketAddr;

use betp;

use crate::{core::{Entity, entity::EntityBuilder, listener::{CreateEvent, Listener}}, network::{Replicable, replicable::Importance}};

/// The `Client` entity represents a client connection for a replicated application setup.
/// This class handles replication of application entities.
pub struct Client {
	client: Box<dyn betp::Client>,
}

impl Client {
	pub fn new(server_address: SocketAddr) -> Result<Client, String> {
		Ok(Client {
			client: Box::new(betp::udp::Client::new(server_address).map_err(|_| "Failed to initilize BETP client.".to_string())?),
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

impl Entity for Client {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn Replicable>>()
	}
}

impl Listener<CreateEvent<dyn Replicable>> for Client {
	fn handle(&mut self, event: &CreateEvent<dyn Replicable>) {
		let handle = event.handle();
		let handle = handle.read();
		let reliable = match handle.importance() {
			Importance::Essential => true,
			Importance::Optional => false,
		};

		let _ = self.client.send(reliable, [0u8; 1024]);
	}
}
