use std::hash::{Hash, Hasher};

use super::{local::Local, remote::Remote, ConnectionStates, PacketHeader};

#[derive(Clone, Copy)]
pub struct Client {
	local: Local,
	remote: Remote,
	connection_state: ConnectionStates,
	address: std::net::SocketAddr,
	salt: u64,
}

impl Client {
	pub fn new(address: std::net::SocketAddr) -> Self {
		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		address.hash(&mut hasher);
		let salt = hasher.finish();
		
		Self {
			local: Local::new(),
			remote: Remote::new(),
			connection_state: ConnectionStates::Negotiating,
			address,
			salt,
		}
	}

	pub(crate) fn send(&mut self,) -> PacketHeader {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();

		PacketHeader {
			protocol_id: [b'B', b'E', b'T', b'P'],
			sequence: sequence_number,
			ack,
			ack_bitfield,
		}
	}

	pub(crate) fn receive(&mut self, packet_header: PacketHeader) {
		let protocol_id = packet_header.protocol_id;

		if protocol_id != [b'B', b'E', b'T', b'P'] {
			return;
		}

		let sequence = packet_header.sequence;
		let ack = packet_header.ack;
		let ack_bitfield = packet_header.ack_bitfield;

		self.local.acknowledge_packets(ack, ack_bitfield);
		self.remote.acknowledge_packet(sequence);
	}

	pub fn address(&self) -> std::net::SocketAddr {
		self.address
	}
}
#[cfg(test)]
mod tests {
	use std::io::{BufRead, Read};

	use super::*;

	#[test]
	fn test_client_send() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669));

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
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669));

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 0, ack: 0, ack_bitfield: 0 });

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 1, ack: 0, ack_bitfield: 0 });
	}

	#[test]
	fn test_client_request_response() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669));

		let header = client.send(); // sequence: 0

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 0, ack: 0, ack_bitfield: 1 << 0 });

		let header = client.send(); // sequence: 1

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 1, ack: 1, ack_bitfield: 1 << 0 | 1 << 1});	
	}

	#[test]
	fn test_dropped_packet() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669));

		let header = client.send(); // sequence: 0

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 0, ack: 0, ack_bitfield: 1 << 0 });

		let header = client.send(); // sequence: 1

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 1, ack: 1, ack_bitfield: 1 << 0 | 1 << 1});

		let header = client.send(); // sequence: 2

		assert_eq!(header.sequence, 2);
		assert_eq!(header.ack, 1);
		assert_eq!(header.ack_bitfield, 1 << 0 | 1 << 1);

		client.receive(PacketHeader { protocol_id: [b'B', b'E', b'T', b'P'], sequence: 2, ack: 1, ack_bitfield: 1 << 0 | 1 << 1});
	}
}