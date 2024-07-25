use std::hash::{Hash, Hasher};

use crate::{packet_buffer::PacketBuffer, packets::{DataPacket, DisconnectPacket}};

use super::super::{local::Local, packets::ConnectionStatus, remote::Remote, ConnectionStates};

#[derive(Clone, Copy)]
pub struct Client {
	/// The local state of the client.
	local: Local,
	/// The state of the server.
	remote: Remote,
	connection_state: ConnectionStates,
	address: std::net::SocketAddr,
	client_salt: u64,
	server_salt: u64,
	last_time: std::time::Instant,
	buffer: PacketBuffer<8, 1024>,
}

impl Client {
	pub fn new(address: std::net::SocketAddr, client_salt: u64, server_salt: u64, current_time: std::time::Instant) -> Self {
		Self {
			local: Local::new(),
			remote: Remote::new(),
			connection_state: ConnectionStates::Negotiating,
			address,
			client_salt,
			server_salt,
			last_time: current_time,
			buffer: PacketBuffer::new(),
		}
	}

	/// To be called to make a packet.
	pub fn send(&mut self, data: [u8; 1024], reliable: bool) -> DataPacket<1024> {
		let sequence_number = self.local.get_sequence_number();
		let ack = self.remote.get_ack();
		let ack_bitfield = self.remote.get_ack_bitfield();

		let packet = DataPacket::new(self.connection_id(), ConnectionStatus::new(sequence_number, ack, ack_bitfield,), data);

		self.buffer.add(packet.clone(), self.connection_id(), reliable);

		packet
	}

	/// To be called when a packet is received from the server.
	/// This will acknowledge the packet and update the state of the client.
	pub fn receive(&mut self, packet_header: ConnectionStatus, current_time: std::time::Instant) {
		let sequence = packet_header.sequence;
		let ack = packet_header.ack;
		let ack_bitfield = packet_header.ack_bitfield;

		self.local.acknowledge_packets(ack, ack_bitfield);
		self.remote.acknowledge_packet(sequence);
		self.buffer.remove(sequence);

		self.last_time = current_time;
	}

	/// Gather all unsent packets.
	/// This is not an idempotent operation as their retry count will be incremented.
	pub fn gather_unsent_packets(&mut self) -> Vec<DataPacket<1024>> {
		self.buffer.gather_unsent_packets()
	}

	pub fn disconnect(&mut self) -> Result<DisconnectPacket, ()> {
		self.client_salt = 0;
		self.server_salt = 0;
		Ok(DisconnectPacket::new(self.connection_id()))
	}

	pub fn client_salt(&self) -> u64 {
		self.client_salt
	}

	pub fn server_salt(&self) -> u64 {
		self.server_salt
	}

	pub fn connection_id(&self) -> u64 {
		self.client_salt ^ self.server_salt
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

	use crate::packets::Packet;

	use super::*;

	#[test]
	fn test_client_send() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1, std::time::Instant::now());

		let packet = client.send([0; 1024], false);
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		let remote = client.send([0; 1024], false);
		let header = remote.get_connection_status();

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);
	}

	#[test]
	fn test_client_receive() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1, std::time::Instant::now());

		client.receive(ConnectionStatus::new(0, 0, 0), std::time::Instant::now());

		client.receive(ConnectionStatus::new(1, 0, 0), std::time::Instant::now());
	}

	#[test]
	fn test_client_request_response() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1, std::time::Instant::now());

		let packet = client.send([0; 1024], false); // sequence: 0
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(ConnectionStatus::new(0, 0, 1 << 0), std::time::Instant::now());

		let packet = client.send([0; 1024], false); // sequence: 1
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(ConnectionStatus::new(1, 1, 1 << 0 | 1 << 1), std::time::Instant::now());
	}

	#[test]
	fn test_dropped_packet() {
		let mut client = Client::new(std::net::SocketAddr::new(std::net::Ipv4Addr::new(127, 0, 0, 1).into(), 6669), 1, 1, std::time::Instant::now());

		let packet = client.send([0; 1024], true); // sequence: 0
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 0);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 0);

		client.receive(ConnectionStatus::new(0, 0, 1 << 0), std::time::Instant::now());

		let packet = client.send([0; 1024], false); // sequence: 1
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 1);
		assert_eq!(header.ack, 0);
		assert_eq!(header.ack_bitfield, 1 << 0);

		client.receive(ConnectionStatus::new(1, 1, 1 << 0 | 1 << 1), std::time::Instant::now());

		let packet = client.send([0; 1024], false); // sequence: 2
		let header = packet.get_connection_status();

		assert_eq!(header.sequence, 2);
		assert_eq!(header.ack, 1);
		assert_eq!(header.ack_bitfield, 1 << 0 | 1 << 1);

		client.receive(ConnectionStatus::new(2, 1, 1 << 0 | 1 << 1), std::time::Instant::now());
	}
}
