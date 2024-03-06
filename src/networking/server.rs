use super::{client::Client, ConnectionResults, PacketHeader};

struct Server {
	max_clients: usize,
	clients: [Option<Client>; 32],
}

impl Server {
	fn new() -> Self {
		Self {
			max_clients: 32,
			clients: [None; 32],
		}
	}

	fn connect(&mut self, address: std::net::SocketAddr) -> Result<usize, ConnectionResults> {
		if let Some(i) = self.clients.iter().enumerate().find(|(i, client)| if let Some(client) = client { client.address() == address } else { false }).map(|(i, _)| i) {
			return Ok(i);
		}

		for (i, client) in self.clients.iter_mut().enumerate() {
			if client.is_none() {
				*client = Some(Client::new(address));
				return Ok(i);
			}
		}

		Err(ConnectionResults::ServerFull)
	}

	fn disconnect(&mut self, client_index: usize) {
		self.clients[client_index] = None;
	}

	fn send(&mut self, client_index: usize,) {
		if let Some(client) = self.clients[client_index].as_mut() {
			client.send();
		}
	}

	fn receive(&mut self, client_index: usize) {
		if let Some(client) = self.clients[client_index].as_mut() {
			client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 0, ack: 0, ack_bitfield: 0 });
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_server_connect() {
		let mut server = Server::new();

		let client_index = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)).unwrap();

		server.send(client_index);
		server.receive(client_index);
	}

	#[test]
	fn test_server_reconnect() {
		let mut server = Server::new();

		let client_index_0 = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)).unwrap();
		let client_index_1 = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)).unwrap();

		assert_eq!(client_index_0, client_index_1);
	}

	#[test]
	fn test_server_disconnect() {
		let mut server = Server::new();

		let client_index = server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)).unwrap();

		server.disconnect(client_index);
	}

	#[test]
	fn test_exhaust_connections() {
		let mut server = Server::new();

		for i in 0..32 {
			server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)).unwrap();
		}

		assert_eq!(server.connect(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669)), Err(ConnectionResults::ServerFull));
	}
}