use std::hash::{Hash, Hasher};

use super::super::{local::Local, packets::ConnectionStatus, remote::Remote, ConnectionStates};

#[derive(Clone, Copy)]
pub struct Client {
	local: Local,
	remote: Remote,
	connection_state: ConnectionStates,
	address: std::net::SocketAddr,
	client_salt: u64,
	server_salt: u64,
	last_time: std::time::Instant,
}

impl Client {
	pub fn new(address: std::net::SocketAddr, client_salt: u64, server_salt: u64) -> Self {
		Self {
			local: Local::new(),
			remote: Remote::new(),
			connection_state: ConnectionStates::Negotiating,
			address,
			client_salt,
			server_salt,
			last_time: std::time::Instant::now(),
		}
	}

	pub(crate) fn send(&mut self,) -> ConnectionStatus {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();

		ConnectionStatus::new(sequence_number, ack, ack_bitfield,)
	}

	pub(crate) fn receive(&mut self, packet_header: ConnectionStatus) {
		let sequence = packet_header.sequence;
		let ack = packet_header.ack;
		let ack_bitfield = packet_header.ack_bitfield;

		self.local.acknowledge_packets(ack, ack_bitfield);
		self.remote.acknowledge_packet(sequence);

		self.last_time = std::time::Instant::now();
	}

	pub fn client_salt(&self) -> u64 {
		self.client_salt
	}

	pub fn server_salt(&self) -> u64 {
		self.server_salt
	}

	pub fn address(&self) -> std::net::SocketAddr {
		self.address
	}

	pub fn last_seen(&self) -> std::time::Instant {
		self.last_time
	}
}
#[cfg(test)]
mod tests {
	use std::io::{BufRead, Read};

	use super::*;

	#[test]
	fn test_client_send() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1);

		let header = client.send();

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		let remote = client.send();

		assert_eq!(remote.sequence, 1);
		assert_eq!(remote.ack, 0);
		assert_eq!(remote.ack_bitfield, 0);
	}

	#[test]
	fn test_client_receive() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1);

		client.receive(ConnectionStatus::new(0, 0, 0));

		client.receive(ConnectionStatus::new(1, 0, 0));
	}

	#[test]
	fn test_client_request_response() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1);

		let header = client.send(); // sequence: 0

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(ConnectionStatus::new(0, 0, 1 << 0));

		let header = client.send(); // sequence: 1

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(ConnectionStatus::new(1, 1, 1 << 0 | 1 << 1));
	}

	#[test]
	fn test_dropped_packet() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1);

		let header = client.send(); // sequence: 0

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(ConnectionStatus::new(0, 0, 1 << 0));

		let header = client.send(); // sequence: 1

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(ConnectionStatus::new(1, 1, 1 << 0 | 1 << 1));

		let header = client.send(); // sequence: 2

		assert_eq!(header.sequence, 2);
		assert_eq!(header.ack, 1);
		assert_eq!(header.ack_bitfield, 1 << 0 | 1 << 1);

		client.receive(ConnectionStatus::new(2, 1, 1 << 0 | 1 << 1));
	}
}
