use betp::packets::{
	ChallengePacket, ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packets,
};
use libfuzzer_sys::arbitrary::Arbitrary;

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
		sequence: u16,
		ack: u16,
		ack_bitfield: u32,
		fill: u8,
	},
	Disconnect {
		connection_id: u64,
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
				sequence,
				ack,
				ack_bitfield,
				fill,
			} => Some(Packets::Data(DataPacket::new(
				*connection_id,
				ConnectionStatus::new(*sequence, *ack, *ack_bitfield),
				[*fill; 1024],
			))),
			Self::Disconnect { connection_id } => Some(DisconnectPacket::new(*connection_id).into()),
		}
	}
}

/// A bounded state-machine action used by both session fuzz targets.
#[derive(Arbitrary, Debug)]
pub enum Operation {
	Packet(PacketInput),
	Batch(Vec<PacketInput>),
	Send { reliable: bool, fill: u8 },
	Tick(u16),
	AdvanceMilliseconds(u16),
	Disconnect,
}

/// Decodes at most `limit` packet descriptions so a single batch has bounded work and storage.
pub fn make_batch(inputs: &[PacketInput], limit: usize) -> Vec<Packets> {
	inputs.iter().take(limit).filter_map(PacketInput::to_packet).collect()
}
