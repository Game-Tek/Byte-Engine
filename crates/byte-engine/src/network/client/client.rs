use std::net::SocketAddr;

use betp::{self, Client as _};

use crate::{
	core::Entity,
	network::{client::udp, replicable::Importance, Replicable},
};

/// The `Client` struct provides the application-facing connection for a
/// replicated client.
///
/// Create the client with [`Self::new`], call [`Self::connect`], then call
/// [`Self::update`] during every application tick. Finish with
/// [`Self::disconnect`] when leaving the session.
pub struct Client {
	client: udp::Client,
}

impl Client {
	/// Creates a UDP-backed client for one BETP server address.
	///
	/// Next, call [`Self::connect`] and start calling [`Self::update`] once per
	/// application tick.
	pub fn new(server_address: SocketAddr) -> Result<Client, String> {
		Ok(Client {
			client: udp::Client::new(server_address).map_err(|error| {
				format!("Failed to initialize BETP client. The most likely cause is a UDP setup error: {error}")
			})?,
		})
	}

	/// Starts the BETP handshake.
	///
	/// Next, call [`Self::update`] on every application tick to send and receive
	/// handshake and session packets.
	pub fn connect(&mut self) {
		self.client.connect(std::time::Instant::now());
	}

	/// Advances the client connection and performs pending network I/O.
	///
	/// Keep calling this while the session is active. Call [`Self::disconnect`]
	/// before intentionally ending the session.
	pub fn update(&mut self) {
		let _ = self.client.update();
	}

	pub fn disconnect(&mut self) {
		let _ = self.client.disconnect();
	}
}
