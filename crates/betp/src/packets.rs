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
	/// Application data sent by either endpoint.
	Data,
	/// A request from either endpoint to end the connection.
	Disconnect,
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

#[repr(C)]
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
/// The `ConnectionStatus` struct carries packet ordering and acknowledgment state.
pub struct ConnectionStatus {
	/// The packet's wrapping sequence number.
	pub sequence: u16,
	/// The newest sequence number acknowledged by the sender.
	pub ack: u16,
	/// Acknowledgments relative to [`ConnectionStatus::ack`].
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
/// The `DataPacket` struct carries application data and connection status between endpoints.
///
/// A session can send the packet once or retry it until the peer acknowledges it.
pub struct DataPacket<const S: usize> {
	pub header: PacketHeader,
	pub connection_id: u64,
	pub connection_status: ConnectionStatus,
	pub data: [u8; S],
}

impl<const S: usize> DataPacket<S> {
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

impl<const S: usize> Packet for DataPacket<S> {
	fn packet_type(&self) -> PacketType {
		self.header.r#type
	}

	fn header(&self) -> PacketHeader {
		self.header
	}
}

impl<const S: usize> Default for DataPacket<S> {
	fn default() -> Self {
		Self {
			header: PacketHeader::default(),
			connection_id: 0,
			connection_status: ConnectionStatus::default(),
			data: [0; S],
		}
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
// Keep data packets inline so packet construction and retry paths do not allocate per packet.
#[allow(clippy::large_enum_variant)]
/// A typed BETP packet.
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
