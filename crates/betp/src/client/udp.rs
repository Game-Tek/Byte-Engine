//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use crate::{client::Session, write_packet};

/// The client is the entity that connects to a server and participates in the game.
pub struct Client {
	session: Session,
	socket: std::net::UdpSocket,
}

impl Client {
	/// Creates a client that will connect to the server at the specified address.
	/// Must call `connect` to establish a connection.
	pub fn new(server_address: std::net::SocketAddr) -> Result<Self, ()> {
		let _ = mid::get("Byte-Engine").ok().ok_or(())?; // TODO: should be better key

		let socket = std::net::UdpSocket::bind(server_address).map_err(|_| ())?;

		let session = Session::new()?;

		Ok(Self {
			socket,
			session,
		})
	}
}

impl super::Client for Client {
	fn connect(&mut self, current_time: std::time::Instant) -> () {
		let salt = current_time.elapsed().as_nanos() as u64;

		self.session.connect(salt);
	}

	fn update(&mut self) -> Result<(), ()> {
		let socket = &mut self.socket;

		let mut buffer = [0u8; 1024];

		let bytes_read = socket.recv(&mut buffer).map_err(|_| ())?;

		let session = &mut self.session;

		let packets = [];

		let packets_to_send = session.update(&packets)?;

		for packet in packets_to_send {
			let mut buffer = [0u8; 1024];

			write_packet(&mut buffer, packet);

			let bytes_sent = socket.send(&buffer).map_err(|_| ());
		}

		Ok(())
	}

	fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<(), ()> {
		let sesion = &mut self.session;

		sesion.send(reliable, data);

		Ok(())
	}

	fn disconnect(&mut self) -> Result<(), ()> {
		self.session.disconnect();

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use std::str::FromStr;

	use super::*;

	#[test]
	fn test_client_connect() {
		let _ = Client::new(std::net::SocketAddr::from_str("127.0.0.1:6669").unwrap()).expect("Failed to connect to server.");
	}
}
