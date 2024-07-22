#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PacketType {
	ConnectionRequest,
	Challenge,
	ChallengeResponse,
	Data,
	Disconnect,
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub(crate) struct PacketHeader {
	// The protocol id is a 32-bit number that is used to identify the protocol.
	pub protocol_id: [u8; 4],
	pub r#type: PacketType,
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub(crate) struct ConnectionRequestPacket {
	header: PacketHeader,
	client_salt: u64,
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub(crate) struct ChallengePacket {
	header: PacketHeader,
	client_salt: u64,
	server_salt: u64,
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub(crate) struct ChallengeResponsePacket {
	header: PacketHeader,
	connection_id: u64,
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub struct ConnectionStatus {
	pub sequence: u16,
	pub ack: u16,
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
#[derive(PartialEq, Eq)]
pub(crate) struct DataPacket<const S: usize> {
	pub header: PacketHeader,
	pub connection_id: u64,
	pub connection_status: ConnectionStatus,
	pub data: [u8; S],
}

impl <const S: usize> DataPacket<S> {
	pub fn new(connection_id: u64, connection_status: ConnectionStatus, data: [u8; S]) -> Self {
		Self {
			header: PacketHeader {
				protocol_id: [b'B', b'E', b'T', b'P'],
				r#type: PacketType::Data,
			},
			connection_id,
			connection_status,
			data,
		}
	}
}

#[repr(C)]
#[derive(PartialEq, Eq)]
pub(crate) struct DisconnectPacket {
	header: PacketHeader,
	connection_id: u64,
}

#[repr(C)]
enum Packet {
	ConnectionRequest(ConnectionRequestPacket),
	Challenge(ChallengePacket),
	ChallengeResponse(ChallengeResponsePacket),
	Disconnect(DisconnectPacket),
}