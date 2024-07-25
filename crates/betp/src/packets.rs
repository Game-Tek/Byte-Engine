//! This module contains the multiple representations of the packets used by the BETP.

/// The trait that all `BETP` packets must implement.
/// Provides methods to access common packet information.
pub trait Packet: Sized {
	/// Returns the type of the packet.
	fn packet_type(&self) -> PacketType;
	fn header(&self) -> PacketHeader;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// The types of packet supported by the protocol.
pub enum PacketType {
	/// A connection request packet. Sent by the client to request a connection to the server.
	ConnectionRequest = 1,
	/// A challenge packet. Sent by the server to challenge the client.
	Challenge,
	/// A challenge response packet. Sent by the client to respond to the challenge.
	ChallengeResponse,
	/// A data packet. Sent by the client or server to send data.
	Data,
	/// A connection status packet. Sent by the client or server to update the connection status.
	Disconnect,
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// The header of a BETP packet.
/// The header contains the protocol id and the type of the packet.
pub struct PacketHeader {
	/// The protocol id is a 32-bit number (or 4 chars) that is used to identify the protocol.
	/// The value of the protocol id is "BETP".
	pub protocol_id: [u8; 4],
	/// The type of the packet.
	pub r#type: PacketType,
}

impl PacketHeader {
	pub fn new(r#type: PacketType) -> Self {
		Self {
			protocol_id: [b'B', b'E', b'T', b'P'],
			r#type,
		}
	}

	pub fn get_protocol_id(&self) -> [u8; 4] {
		self.protocol_id
	}

	pub fn get_type(&self) -> PacketType {
		self.r#type
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
/// A connection request packet.
pub struct ConnectionRequestPacket {
	header: PacketHeader,
	client_salt: u64,
}

impl ConnectionRequestPacket {
	pub fn new(client_salt: u64) -> Self {
		Self {
			header: PacketHeader::new(PacketType::ConnectionRequest),
			client_salt,
		}
	}

	pub fn get_client_salt(&self) -> u64 {
		self.client_salt
	}
}

impl Packet for ConnectionRequestPacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
/// A challenge packet.
pub struct ChallengePacket {
	header: PacketHeader,
	client_salt: u64,
	server_salt: u64,
}

impl ChallengePacket {
	pub fn new(client_salt: u64, server_salt: u64) -> Self {
		Self {
			header: PacketHeader::new(PacketType::Challenge),
			client_salt,
			server_salt,
		}
	}

	pub fn get_client_salt(&self) -> u64 {
		self.client_salt
	}

	pub fn get_server_salt(&self) -> u64 {
		self.server_salt
	}
}

impl Packet for ChallengePacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
/// A challenge response packet.
pub struct ChallengeResponsePacket {
	header: PacketHeader,
	connection_id: u64,
}

impl ChallengeResponsePacket {
	pub fn new(connection_id: u64) -> Self {
		Self {
			header: PacketHeader::new(PacketType::ChallengeResponse),
			connection_id,
		}
	}
}

impl Packet for ChallengeResponsePacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// The status of a connection.
pub struct ConnectionStatus {
	/// The sequence number of the packet. An incrementing number that is used to order the packets.
	pub sequence: u16,
	/// The last acknowledged sequence number by the sender.
	pub ack: u16,
	/// A bitfield of the last acknowledged packets by the sender, relative to the ack number.
	pub ack_bitfield: u32,
}

impl ConnectionStatus {
	pub fn new(sequence: u16, ack: u16, ack_bitfield: u32) -> Self {
		Self {
			sequence,
			ack,
			ack_bitfield,
		}
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// A data packet.
/// The data packet is used to send application data between the client and the server.
/// The data packet can be reliable or unreliable. If the packet is reliable, the sender will retry to send the packet until it is acknowledged by the receiver. (Up to a certain number of retries.)
pub struct DataPacket<const S: usize> {
	pub header: PacketHeader,
	pub connection_id: u64,
	pub connection_status: ConnectionStatus,
	pub data: [u8; S],
}

impl <const S: usize> DataPacket<S> {
	pub fn new(connection_id: u64, connection_status: ConnectionStatus, data: [u8; S]) -> Self {
		Self {
			header: PacketHeader::new(PacketType::Data),
			connection_id,
			connection_status,
			data,
		}
	}

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
	}

	pub fn get_connection_status(&self) -> ConnectionStatus {
		self.connection_status
	}
}

impl <const S: usize> Packet for DataPacket<S> {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
/// A disconnect packet.
pub struct DisconnectPacket {
	header: PacketHeader,
	connection_id: u64,
}

impl DisconnectPacket {
	pub fn new(connection_id: u64) -> Self {
		Self {
			header: PacketHeader::new(PacketType::Disconnect),
			connection_id,
		}
	}

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
	}
}

impl Packet for DisconnectPacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
/// Represents all the possible BETP packets.
pub enum Packets {
	ConnectionRequest(ConnectionRequestPacket),
	Challenge(ChallengePacket),
	ChallengeResponse(ChallengeResponsePacket),
	Data(DataPacket<1024>),
	Disconnect(DisconnectPacket),
}

impl Packet for Packets {
	fn packet_type(&self) -> PacketType {
		match self {
			Packets::ConnectionRequest(packet) => packet.packet_type(),
			Packets::Challenge(packet) => packet.packet_type(),
			Packets::ChallengeResponse(packet) => packet.packet_type(),
			Packets::Data(packet) => packet.packet_type(),
			Packets::Disconnect(packet) => packet.packet_type(),
		}
	}

	fn header(&self) -> PacketHeader {
		match self {
			Packets::ConnectionRequest(packet) => packet.header(),
			Packets::Challenge(packet) => packet.header(),
			Packets::ChallengeResponse(packet) => packet.header(),
			Packets::Data(packet) => packet.header(),
			Packets::Disconnect(packet) => packet.header(),
		}
	}
}
