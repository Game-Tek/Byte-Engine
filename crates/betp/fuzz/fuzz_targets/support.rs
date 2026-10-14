use betp::packets::{
	Acknowledgement, ChallengePacket, ChallengeResponsePacket, ConnectionRequestPacket, DataPacket, DisconnectPacket,
	KeepAlivePacket, MAX_PAYLOAD_SIZE, MessagePacket, Packets, Payload,
};
use libfuzzer_sys::arbitrary::Arbitrary;

/// Every message the window allows in flight, plus one unreliable payload for each bounded operation.
pub const MAX_SESSION_OUTPUT_PACKETS: usize = betp::WINDOW + 64;
pub const RETRY_QUIESCENCE_UPDATES: usize = 9;

/// A packet source that combines malformed wire input with valid packets containing arbitrary attacker-controlled fields.
#[derive(Arbitrary, Debug)]
pub enum PacketInput {
	Raw(Vec<u8>),
	ConnectionRequest {
		client_salt: u64,
	},
	Challenge {
		client_salt: u64,
		server_salt: u64,
	},
	ChallengeResponse {
		connection_id: u64,
	},
	Data {
		connection_id: u64,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	Message {
		connection_id: u64,
		sequence: u16,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	Disconnect {
		connection_id: u64,
	},
	KeepAlive {
		connection_id: u64,
		ack: u16,
		ack_bitfield: u32,
	},
}

impl PacketInput {
	/// Converts semantic input directly and sends raw input through the production decoder.
	pub fn to_packet(&self) -> Option<Packets> {
		match self {
			Self::Raw(bytes) => betp::read_packet(bytes).ok(),
			Self::ConnectionRequest { client_salt } => Some(ConnectionRequestPacket::new(*client_salt).into()),
			Self::Challenge {
				client_salt,
				server_salt,
			} => Some(ChallengePacket::new(*client_salt, *server_salt).into()),
			Self::ChallengeResponse { connection_id } => Some(ChallengeResponsePacket::new(*connection_id).into()),
			Self::Data {
				connection_id,
				ack,
				ack_bitfield,
				fill,
			} => Some(DataPacket::new(*connection_id, Acknowledgement::new(*ack, *ack_bitfield), fuzz_payload(*fill)).into()),
			Self::Message {
				connection_id,
				sequence,
				ack,
				ack_bitfield,
				fill,
			} => Some(make_message(*connection_id, *sequence, *ack, *ack_bitfield, *fill)),
			Self::Disconnect { connection_id } => Some(DisconnectPacket::new(*connection_id).into()),
			Self::KeepAlive {
				connection_id,
				ack,
				ack_bitfield,
			} => Some(KeepAlivePacket::new(*connection_id, Acknowledgement::new(*ack, *ack_bitfield)).into()),
		}
	}
}

/// A bounded state-machine action used by both session fuzz targets.
#[derive(Arbitrary, Debug)]
pub enum Operation {
	Packet(PacketInput),
	Batch(Vec<PacketInput>),
	Send {
		reliable: bool,
		fill: u8,
	},
	Tick(u16),
	AdvanceMilliseconds(u16),
	Disconnect,
	CurrentData {
		sequence: u16,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	OtherData {
		sequence: u16,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	DuplicateCurrentData {
		sequence: u16,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	CurrentDisconnect,
	OtherDisconnect,
}

/// Decodes at most `limit` packet descriptions so a single batch has bounded work and storage.
pub fn make_batch(inputs: &[PacketInput], limit: usize) -> Vec<Packets> {
	inputs.iter().take(limit).filter_map(PacketInput::to_packet).collect()
}

/// Builds a reliable message whose identity is chosen relative to the session by the caller.
pub fn make_message(connection_id: u64, sequence: u16, ack: u16, ack_bitfield: u32, fill: u8) -> Packets {
	MessagePacket::new(
		connection_id,
		sequence,
		Acknowledgement::new(ack, ack_bitfield),
		fuzz_payload(fill),
	)
	.into()
}

/// Builds a payload of `fill` bytes whose length also follows `fill`, so fuzzing covers empty, short, and nearly
/// full payloads without changing the operation format the seed corpus encodes.
pub fn fuzz_payload(fill: u8) -> Payload {
	let bytes = [fill; MAX_PAYLOAD_SIZE];
	Payload::new(&bytes[..usize::from(fill) * 4]).expect("at most 1020 bytes fit a payload")
}

/// Derives an identity that is guaranteed not to equal the active session identity.
pub fn other_connection_id(connection_id: u64) -> u64 {
	connection_id ^ 1
}

/// Checks the bounded-output and session-identity contracts shared by both state machines.
pub fn observe_session_output(packets: &[Packets], connection_id: &mut Option<u64>) {
	assert!(packets.len() <= MAX_SESSION_OUTPUT_PACKETS);

	for packet in packets {
		match packet {
			Packets::ChallengeResponse(packet) => match connection_id {
				Some(expected) => assert_eq!(*expected, packet.get_connection_id()),
				None => *connection_id = Some(packet.get_connection_id()),
			},
			Packets::Data(packet) => assert_eq!(Some(packet.get_connection_id()), *connection_id),
			Packets::Message(packet) => assert_eq!(Some(packet.get_connection_id()), *connection_id),
			Packets::Disconnect(packet) => assert_eq!(Some(packet.get_connection_id()), *connection_id),
			Packets::KeepAlive(packet) => assert_eq!(Some(packet.get_connection_id()), *connection_id),
			Packets::ConnectionRequest(_) | Packets::Challenge(_) => {}
		}
	}
}
