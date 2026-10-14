//! Defines the typed packets used by BETP.

/// The `Packet` trait provides access to the header shared by all BETP packets.
pub trait Packet: Sized {
	/// Returns the type of the packet.
	fn packet_type(&self) -> PacketType;
	fn header(&self) -> PacketHeader;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
/// A packet type supported by BETP.
pub enum PacketType {
	/// A reserved value that must not appear in a complete wire packet.
	#[default]
	Default = 0,
	/// A client request to start a server connection.
	ConnectionRequest = 1,
	/// A server challenge sent during connection negotiation.
	Challenge,
	/// A client's response to a server challenge.
	ChallengeResponse,
	/// An unreliable application payload sent by either endpoint.
	Data,
	/// A request from either endpoint to end the connection.
	Disconnect,
	/// A sign of life from an endpoint that had nothing else to send.
	KeepAlive,
	/// A reliable application payload sent by either endpoint.
	Message,
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
/// The `PacketHeader` struct identifies the protocol and packet type on the wire.
pub struct PacketHeader {
	/// The four-byte `BETP` protocol identifier.
	pub protocol_id: [u8; 4],
	/// The type of the packet.
	pub r#type: PacketType,
}

impl PacketHeader {
	pub fn new(r#type: PacketType) -> Self {
		Self {
			protocol_id: *b"BETP",
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
#[derive(PartialEq, Eq, Debug)]
/// The `ConnectionRequestPacket` struct starts client connection negotiation.
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

impl From<ConnectionRequestPacket> for Packets {
	fn from(val: ConnectionRequestPacket) -> Self {
		Packets::ConnectionRequest(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
/// The `ChallengePacket` struct lets a server validate a connection request.
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

impl From<ChallengePacket> for Packets {
	fn from(val: ChallengePacket) -> Self {
		Packets::Challenge(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
/// The `ChallengeResponsePacket` struct lets a client answer a server challenge.
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

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
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

impl From<ChallengeResponsePacket> for Packets {
	fn from(val: ChallengeResponsePacket) -> Self {
		Packets::ChallengeResponse(val)
	}
}

/// The size, in bytes, of the application payload in [`DataPacket`] and [`MessagePacket`].
pub const PAYLOAD_SIZE: usize = 1024;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
/// The `Acknowledgement` struct tells a peer which of its reliable messages arrived, so it stops resending them.
///
/// Every packet sent on an established connection carries one.
pub struct Acknowledgement {
	/// The newest message sequence received.
	pub ack: u16,
	/// Bit `i` is set when message `ack - i` arrived. Bit 0 is `ack` itself, so a zero bitfield acknowledges nothing.
	pub ack_bitfield: u32,
}

impl Acknowledgement {
	pub fn new(ack: u16, ack_bitfield: u32) -> Self {
		Self { ack, ack_bitfield }
	}

	/// Returns the sequences of every message this acknowledges.
	pub fn sequences(&self) -> impl Iterator<Item = u16> + '_ {
		(0..u32::BITS)
			.filter(|bit| (self.ack_bitfield >> bit) & 1 == 1)
			.map(|bit| self.ack.wrapping_sub(bit as u16))
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// The `DataPacket` struct carries an unreliable payload, which the session sends once and the peer delivers as it
/// arrives.
pub struct DataPacket {
	header: PacketHeader,
	connection_id: u64,
	acknowledgement: Acknowledgement,
	data: [u8; PAYLOAD_SIZE],
}

impl DataPacket {
	pub fn new(connection_id: u64, acknowledgement: Acknowledgement, data: [u8; PAYLOAD_SIZE]) -> Self {
		Self {
			header: PacketHeader::new(PacketType::Data),
			connection_id,
			acknowledgement,
			data,
		}
	}

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
	}

	pub fn get_acknowledgement(&self) -> Acknowledgement {
		self.acknowledgement
	}

	pub fn get_data(&self) -> &[u8; PAYLOAD_SIZE] {
		&self.data
	}
}

impl Packet for DataPacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

impl From<DataPacket> for Packets {
	fn from(val: DataPacket) -> Self {
		Packets::Data(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
/// The `MessagePacket` struct carries a reliable payload, which the session resends until the peer acknowledges it.
///
/// Only messages take sequence numbers, so the receiver knows that a gap is a message still to come and delivers
/// messages in sequence order.
pub struct MessagePacket {
	header: PacketHeader,
	connection_id: u64,
	sequence: u16,
	acknowledgement: Acknowledgement,
	data: [u8; PAYLOAD_SIZE],
}

impl MessagePacket {
	pub fn new(connection_id: u64, sequence: u16, acknowledgement: Acknowledgement, data: [u8; PAYLOAD_SIZE]) -> Self {
		Self {
			header: PacketHeader::new(PacketType::Message),
			connection_id,
			sequence,
			acknowledgement,
			data,
		}
	}

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
	}

	pub fn get_sequence(&self) -> u16 {
		self.sequence
	}

	pub fn get_acknowledgement(&self) -> Acknowledgement {
		self.acknowledgement
	}

	pub fn get_data(&self) -> &[u8; PAYLOAD_SIZE] {
		&self.data
	}
}

impl Packet for MessagePacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

impl From<MessagePacket> for Packets {
	fn from(val: MessagePacket) -> Self {
		Packets::Message(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
/// The `DisconnectPacket` struct asks an endpoint to end a connection.
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

impl From<DisconnectPacket> for Packets {
	fn from(val: DisconnectPacket) -> Self {
		Packets::Disconnect(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
/// The `KeepAlivePacket` struct keeps a quiet connection from timing out.
///
/// A connected session sends one when it has sent nothing else for [`crate::KEEPALIVE_INTERVAL`], or when it owes
/// its peer an acknowledgement. It carries no application data, so applications never see it.
pub struct KeepAlivePacket {
	header: PacketHeader,
	connection_id: u64,
	acknowledgement: Acknowledgement,
}

impl KeepAlivePacket {
	pub fn new(connection_id: u64, acknowledgement: Acknowledgement) -> Self {
		Self {
			header: PacketHeader::new(PacketType::KeepAlive),
			connection_id,
			acknowledgement,
		}
	}

	pub fn get_connection_id(&self) -> u64 {
		self.connection_id
	}

	pub fn get_acknowledgement(&self) -> Acknowledgement {
		self.acknowledgement
	}
}

impl Packet for KeepAlivePacket {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

impl From<KeepAlivePacket> for Packets {
	fn from(val: KeepAlivePacket) -> Self {
		Packets::KeepAlive(val)
	}
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
// Keep data packets inline so packet construction and retry paths do not allocate per packet.
#[allow(clippy::large_enum_variant)]
/// A typed BETP packet.
pub enum Packets {
	ConnectionRequest(ConnectionRequestPacket),
	Challenge(ChallengePacket),
	ChallengeResponse(ChallengeResponsePacket),
	Data(DataPacket),
	Disconnect(DisconnectPacket),
	KeepAlive(KeepAlivePacket),
	Message(MessagePacket),
}

impl Packets {
	/// Returns the connection a session packet belongs to, so a session can reject another connection's traffic.
	///
	/// Handshake packets return `None`: they run before the connection exists and carry salts instead.
	pub fn connection_id(&self) -> Option<u64> {
		match self {
			Packets::Data(packet) => Some(packet.get_connection_id()),
			Packets::Disconnect(packet) => Some(packet.get_connection_id()),
			Packets::KeepAlive(packet) => Some(packet.get_connection_id()),
			Packets::Message(packet) => Some(packet.get_connection_id()),
			Packets::ConnectionRequest(_) | Packets::Challenge(_) | Packets::ChallengeResponse(_) => None,
		}
	}
}

impl Packet for Packets {
	fn packet_type(&self) -> PacketType {
		match self {
			Packets::ConnectionRequest(packet) => packet.packet_type(),
			Packets::Challenge(packet) => packet.packet_type(),
			Packets::ChallengeResponse(packet) => packet.packet_type(),
			Packets::Data(packet) => packet.packet_type(),
			Packets::Disconnect(packet) => packet.packet_type(),
			Packets::KeepAlive(packet) => packet.packet_type(),
			Packets::Message(packet) => packet.packet_type(),
		}
	}

	fn header(&self) -> PacketHeader {
		match self {
			Packets::ConnectionRequest(packet) => packet.header(),
			Packets::Challenge(packet) => packet.header(),
			Packets::ChallengeResponse(packet) => packet.header(),
			Packets::Data(packet) => packet.header(),
			Packets::Disconnect(packet) => packet.header(),
			Packets::KeepAlive(packet) => packet.header(),
			Packets::Message(packet) => packet.header(),
		}
	}
}
