use betp::Server as _;

use crate::{core::Entity, network::server::udp};

pub struct Server {
	server: udp::Server,
}

impl Server {
	pub fn new() -> Result<Server, String> {
		Ok(Server {
			server: udp::Server::new("0.0.0.0:6669").map_err(|_| "Failed to create BETP server".to_string())?,
		})
	}

	pub fn update(&mut self) {
		let server = &mut self.server;

		let current_time = std::time::Instant::now();

		let Ok(events) = server.update(current_time) else {
			return;
		};

		for event in events {
			log::debug!("Server event: {:#?}", event);

			match event {
				betp::server::Events::ClientConnected { id } => {
					// TODO: spawn client
				}
				betp::server::Events::ClientDisconnected { id } => {
					// TODO: kill client
				}
			}
		}
	}
}

impl Entity for Server {}

struct Client {}

impl Entity for Client {}
