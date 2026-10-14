//! Use the Byte Engine Transport Protocol (BETP) to exchange reliable, ordered
//! data between a multiplayer client and an authoritative server.
//!
//! BETP runs over UDP but does not perform network I/O. Your application supplies
//! the I/O layer, so you can use BETP with different networking runtimes.
//!
//! # Modules
//!
//! - [`server`] manages authoritative protocol sessions.
//! - [`client`] manages client protocol sessions.
//! - [`packets`] defines the protocol's wire types.
//!
//! # Connection sequence
//!
//! 1. The client requests a connection.
//! 2. The server sends a challenge.
//! 3. The client responds to the challenge.
//! 4. The client and server exchange data.
//! 5. Either endpoint can end the connection.
//!
//! Reliable payloads travel as numbered messages, which the receiver delivers
//! once and in order, and which the sender resends until acknowledged.
//! Unreliable payloads are sent once and delivered as they arrive.
//!
//! Start with [`client::Client`] or [`server::Server`]. Queue work through that
//! trait, then call its `update` method from the application's regular tick.

#![allow(clippy::items_after_test_module)]
// Protocol state transitions stay explicit so packet validation order remains auditable.
#![allow(clippy::cognitive_complexity, clippy::excessive_nesting, clippy::too_many_lines)]

pub mod client;
pub mod server;

mod stream;

pub mod packets;

pub use client::Client;
pub use server::Server;

/// An error that prevents BETP packet decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketReadError {
	/// The packet header bytes could not be read from the source buffer.
	ShortHeader,
	/// The header protocol id does not match BETP.
	WrongProtocol,
	/// The packet type byte does not map to a supported BETP packet type.
	UnknownPacketType,
	/// The reserved default packet type cannot appear as a complete wire packet.
	ReservedPacketType,
	/// The packet length does not match the fixed wire size for its packet type.
	InvalidPacketLength { expected: usize, actual: usize },
}

impl std::fmt::Display for PacketReadError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::ShortHeader => write!(
				formatter,
				"Packet header is incomplete. The most likely cause is that the input buffer is shorter than the BETP header."
			),
			Self::WrongProtocol => write!(
				formatter,
				"Packet protocol id is invalid. The most likely cause is that the input buffer does not contain a BETP packet."
			),
			Self::UnknownPacketType => write!(
				formatter,
				"Packet type is unknown. The most likely cause is that the input buffer contains malformed or unsupported BETP data."
			),
			Self::ReservedPacketType => write!(
				formatter,
				"Packet type is reserved. The most likely cause is that the default packet type was sent as a complete BETP packet."
			),
			Self::InvalidPacketLength { expected, actual } => write!(
				formatter,
				"Packet length is invalid: expected {expected} bytes but received {actual}. The most likely cause is that the packet body is truncated or contains trailing bytes."
			),
		}
	}
}

impl std::error::Error for PacketReadError {}

/// Returns whether `s1` follows `s2` in the wrapping sequence-number space.
pub(crate) fn sequence_greater_than(s1: u16, s2: u16) -> bool {
	((s1 > s2) && (s1 - s2 <= 32768u16)) || ((s1 < s2) && (s2 - s1 > 32768u16))
}

/// How long a connected session may send nothing before it sends a [`packets::KeepAlivePacket`].
pub const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
/// How long a connected session waits to hear from its peer before it ends the connection.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long a session waits for a reliable message's acknowledgement before it sends the message again.
pub const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// How many reliable messages a session may have unacknowledged at once. Later messages wait their turn.
///
/// It matches the 32 messages a [`packets::Acknowledgement`] covers, so every message in flight can be acknowledged.
pub const WINDOW: usize = 32;

const PACKET_HEADER_SIZE: usize = 5;
const ACKNOWLEDGEMENT_SIZE: usize = 6;
const CONNECTION_PACKET_SIZE: usize = PACKET_HEADER_SIZE + 8;
const CHALLENGE_PACKET_SIZE: usize = PACKET_HEADER_SIZE + 16;
const KEEP_ALIVE_PACKET_SIZE: usize = CONNECTION_PACKET_SIZE + ACKNOWLEDGEMENT_SIZE;
const DATA_PACKET_SIZE: usize = KEEP_ALIVE_PACKET_SIZE + PAYLOAD_SIZE;
const MESSAGE_PACKET_SIZE: usize = DATA_PACKET_SIZE + 2;

impl Packets {
	/// Returns the packet's exact length on the wire.
	pub fn wire_size(&self) -> usize {
		wire_size(self.packet_type())
	}
}

/// Returns the exact wire length of a packet of type `packet_type`, which every packet of that type has.
fn wire_size(packet_type: PacketType) -> usize {
	match packet_type {
		PacketType::Default => PACKET_HEADER_SIZE,
		PacketType::ConnectionRequest | PacketType::ChallengeResponse | PacketType::Disconnect => CONNECTION_PACKET_SIZE,
		PacketType::Challenge => CHALLENGE_PACKET_SIZE,
		PacketType::KeepAlive => KEEP_ALIVE_PACKET_SIZE,
		PacketType::Data => DATA_PACKET_SIZE,
		PacketType::Message => MESSAGE_PACKET_SIZE,
	}
}

fn write_bytes(buffer: &mut [u8], offset: &mut usize, bytes: &[u8]) -> Option<()> {
	let end = offset.checked_add(bytes.len())?;
	let destination = buffer.get_mut(*offset..end)?;
	destination.copy_from_slice(bytes);
	*offset = end;
	Some(())
}

pub(crate) fn write_packet_header(buffer: &mut [u8], packet_header: PacketHeader) -> Option<()> {
	let mut offset = 0;
	write_bytes(buffer, &mut offset, &packet_header.protocol_id)?;
	write_bytes(buffer, &mut offset, &[packet_header.r#type as u8])?;

	Some(())
}

fn write_acknowledgement(buffer: &mut [u8], offset: &mut usize, acknowledgement: Acknowledgement) -> Option<()> {
	write_bytes(buffer, offset, &acknowledgement.ack.to_le_bytes())?;
	write_bytes(buffer, offset, &acknowledgement.ack_bitfield.to_le_bytes())
}

pub fn write_packet(buffer: &mut [u8], packet: Packets) -> Option<()> {
	// Validate capacity first so a failed serialization never leaves a partial packet in the caller's buffer.
	if buffer.len() < packet.wire_size() {
		return None;
	}

	write_packet_header(buffer, packet.header())?;
	let mut offset = PACKET_HEADER_SIZE;

	match packet {
		Packets::ConnectionRequest(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_client_salt().to_le_bytes())?;
		}
		Packets::Challenge(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_client_salt().to_le_bytes())?;
			write_bytes(buffer, &mut offset, &packet.get_server_salt().to_le_bytes())?;
		}
		Packets::ChallengeResponse(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
		}
		Packets::Data(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
			write_acknowledgement(buffer, &mut offset, packet.get_acknowledgement())?;
			write_bytes(buffer, &mut offset, packet.get_data())?;
		}
		Packets::Message(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
			write_bytes(buffer, &mut offset, &packet.get_sequence().to_le_bytes())?;
			write_acknowledgement(buffer, &mut offset, packet.get_acknowledgement())?;
			write_bytes(buffer, &mut offset, packet.get_data())?;
		}
		Packets::Disconnect(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
		}
		Packets::KeepAlive(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
			write_acknowledgement(buffer, &mut offset, packet.get_acknowledgement())?;
		}
	}

	Some(())
}

pub fn read_packet_header(buffer: &[u8]) -> Result<PacketHeader, PacketReadError> {
	let mut cursor = std::io::Cursor::new(buffer);

	let mut protocol_id = [0u8; 4];

	cursor
		.read_exact(&mut protocol_id)
		.map_err(|_| PacketReadError::ShortHeader)?;

	if protocol_id != *b"BETP" {
		return Err(PacketReadError::WrongProtocol);
	}

	let mut r#type = [0u8; 1];

	cursor.read_exact(&mut r#type).map_err(|_| PacketReadError::ShortHeader)?;

	let r#type = match r#type[0] {
		0 => PacketType::Default,
		1 => PacketType::ConnectionRequest,
		2 => PacketType::Challenge,
		3 => PacketType::ChallengeResponse,
		4 => PacketType::Data,
		5 => PacketType::Disconnect,
		6 => PacketType::KeepAlive,
		7 => PacketType::Message,
		_ => return Err(PacketReadError::UnknownPacketType),
	};

	Ok(PacketHeader { protocol_id, r#type })
}

/// Reads one fixed-width field without indexing outside the source packet.
fn read_field<const N: usize>(buffer: &[u8], offset: &mut usize) -> Option<[u8; N]> {
	let end = offset.checked_add(N)?;
	let field = buffer.get(*offset..end)?.try_into().ok()?;
	*offset = end;
	Some(field)
}

fn read_acknowledgement(buffer: &[u8], offset: &mut usize) -> Option<Acknowledgement> {
	let ack = u16::from_le_bytes(read_field(buffer, offset)?);
	let ack_bitfield = u32::from_le_bytes(read_field(buffer, offset)?);
	Some(Acknowledgement::new(ack, ack_bitfield))
}

/// Decodes one complete BETP datagram into its typed packet representation.
///
/// BETP packets use exact, type-specific wire lengths. Truncated packets and
/// packets with trailing bytes are rejected before any body field is read.
pub fn read_packet(buffer: &[u8]) -> Result<Packets, PacketReadError> {
	let header = read_packet_header(buffer)?;
	if header.r#type == PacketType::Default {
		return Err(PacketReadError::ReservedPacketType);
	}
	let expected = wire_size(header.r#type);

	if buffer.len() != expected {
		return Err(PacketReadError::InvalidPacketLength {
			expected,
			actual: buffer.len(),
		});
	}

	let invalid_length = || PacketReadError::InvalidPacketLength {
		expected,
		actual: buffer.len(),
	};
	let mut offset = PACKET_HEADER_SIZE;

	match header.r#type {
		PacketType::Default => Err(PacketReadError::ReservedPacketType),
		PacketType::ConnectionRequest => {
			let client_salt = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			Ok(ConnectionRequestPacket::new(client_salt).into())
		}
		PacketType::Challenge => {
			let client_salt = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			let server_salt = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			Ok(ChallengePacket::new(client_salt, server_salt).into())
		}
		PacketType::ChallengeResponse => {
			let connection_id = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			Ok(ChallengeResponsePacket::new(connection_id).into())
		}
		PacketType::Data => {
			let connection_id = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			let acknowledgement = read_acknowledgement(buffer, &mut offset).ok_or_else(invalid_length)?;
			let data = read_field(buffer, &mut offset).ok_or_else(invalid_length)?;
			Ok(DataPacket::new(connection_id, acknowledgement, data).into())
		}
		PacketType::Message => {
			let connection_id = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			let sequence = u16::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			let acknowledgement = read_acknowledgement(buffer, &mut offset).ok_or_else(invalid_length)?;
			let data = read_field(buffer, &mut offset).ok_or_else(invalid_length)?;
			Ok(MessagePacket::new(connection_id, sequence, acknowledgement, data).into())
		}
		PacketType::Disconnect => {
			let connection_id = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			Ok(DisconnectPacket::new(connection_id).into())
		}
		PacketType::KeepAlive => {
			let connection_id = u64::from_le_bytes(read_field(buffer, &mut offset).ok_or_else(invalid_length)?);
			let acknowledgement = read_acknowledgement(buffer, &mut offset).ok_or_else(invalid_length)?;
			Ok(KeepAlivePacket::new(connection_id, acknowledgement).into())
		}
	}
}

use std::io::Read as _;

use crate::packets::{
	Acknowledgement, ChallengePacket, ChallengeResponsePacket, ConnectionRequestPacket, DataPacket, DisconnectPacket,
	KeepAlivePacket, MessagePacket, PAYLOAD_SIZE, Packet, PacketHeader, PacketType, Packets,
};

#[cfg(test)]
mod tests {
	use super::{
		CHALLENGE_PACKET_SIZE, CONNECTION_PACKET_SIZE, DATA_PACKET_SIZE, KEEP_ALIVE_PACKET_SIZE, MESSAGE_PACKET_SIZE,
		PACKET_HEADER_SIZE, PacketReadError, read_packet, sequence_greater_than, write_packet, write_packet_header,
	};
	use crate::packets::{
		Acknowledgement, ChallengePacket, ChallengeResponsePacket, ConnectionRequestPacket, DataPacket, DisconnectPacket,
		KeepAlivePacket, MessagePacket, PacketHeader, PacketType, Packets,
	};

	fn assert_packet_round_trip(packet: Packets, expected: Packets, packet_size: usize) {
		let mut bytes = [0u8; MESSAGE_PACKET_SIZE];
		assert_eq!(packet.wire_size(), packet_size);
		write_packet(&mut bytes[..packet_size], packet).expect("the exact packet capacity must serialize");

		assert_eq!(read_packet(&bytes[..packet_size]), Ok(expected));
	}

	#[test]
	fn every_packet_variant_round_trips_through_the_canonical_wire_format() {
		assert_eq!(CONNECTION_PACKET_SIZE, 13);
		assert_eq!(CHALLENGE_PACKET_SIZE, 21);
		assert_eq!(KEEP_ALIVE_PACKET_SIZE, 19);
		assert_eq!(DATA_PACKET_SIZE, 1043);
		assert_eq!(MESSAGE_PACKET_SIZE, 1045);

		assert_packet_round_trip(
			ConnectionRequestPacket::new(0x0102_0304_0506_0708).into(),
			ConnectionRequestPacket::new(0x0102_0304_0506_0708).into(),
			CONNECTION_PACKET_SIZE,
		);
		assert_packet_round_trip(
			ChallengePacket::new(0x1112_1314_1516_1718, 0x2122_2324_2526_2728).into(),
			ChallengePacket::new(0x1112_1314_1516_1718, 0x2122_2324_2526_2728).into(),
			CHALLENGE_PACKET_SIZE,
		);
		assert_packet_round_trip(
			ChallengeResponsePacket::new(0x3132_3334_3536_3738).into(),
			ChallengeResponsePacket::new(0x3132_3334_3536_3738).into(),
			CONNECTION_PACKET_SIZE,
		);

		let acknowledgement = Acknowledgement::new(0x4344, 0x4546_4748);
		assert_packet_round_trip(
			DataPacket::new(0x5152_5354_5556_5758, acknowledgement, [0xA5; 1024]).into(),
			DataPacket::new(0x5152_5354_5556_5758, acknowledgement, [0xA5; 1024]).into(),
			DATA_PACKET_SIZE,
		);
		assert_packet_round_trip(
			MessagePacket::new(0x5152_5354_5556_5758, 0x4142, acknowledgement, [0x5A; 1024]).into(),
			MessagePacket::new(0x5152_5354_5556_5758, 0x4142, acknowledgement, [0x5A; 1024]).into(),
			MESSAGE_PACKET_SIZE,
		);
		assert_packet_round_trip(
			DisconnectPacket::new(0x6162_6364_6566_6768).into(),
			DisconnectPacket::new(0x6162_6364_6566_6768).into(),
			CONNECTION_PACKET_SIZE,
		);
		assert_packet_round_trip(
			KeepAlivePacket::new(0x7172_7374_7576_7778, acknowledgement).into(),
			KeepAlivePacket::new(0x7172_7374_7576_7778, acknowledgement).into(),
			KEEP_ALIVE_PACKET_SIZE,
		);
	}

	fn assert_noncanonical_lengths_are_rejected(packet: Packets, packet_size: usize) {
		let mut bytes = [0u8; MESSAGE_PACKET_SIZE + 1];
		write_packet(&mut bytes[..packet_size], packet).expect("the exact packet capacity must serialize");

		assert_eq!(
			read_packet(&bytes[..packet_size - 1]),
			Err(PacketReadError::InvalidPacketLength {
				expected: packet_size,
				actual: packet_size - 1,
			})
		);
		assert_eq!(
			read_packet(&bytes[..packet_size + 1]),
			Err(PacketReadError::InvalidPacketLength {
				expected: packet_size,
				actual: packet_size + 1,
			})
		);
	}

	#[test]
	fn packet_bodies_require_the_exact_length_for_their_type() {
		assert_noncanonical_lengths_are_rejected(ConnectionRequestPacket::new(1).into(), CONNECTION_PACKET_SIZE);
		assert_noncanonical_lengths_are_rejected(ChallengePacket::new(2, 3).into(), CHALLENGE_PACKET_SIZE);
		assert_noncanonical_lengths_are_rejected(ChallengeResponsePacket::new(4).into(), CONNECTION_PACKET_SIZE);
		let acknowledgement = Acknowledgement::new(7, 8);
		assert_noncanonical_lengths_are_rejected(DataPacket::new(5, acknowledgement, [9; 1024]).into(), DATA_PACKET_SIZE);
		assert_noncanonical_lengths_are_rejected(
			MessagePacket::new(5, 6, acknowledgement, [9; 1024]).into(),
			MESSAGE_PACKET_SIZE,
		);
		assert_noncanonical_lengths_are_rejected(DisconnectPacket::new(10).into(), CONNECTION_PACKET_SIZE);
		assert_noncanonical_lengths_are_rejected(KeepAlivePacket::new(11, acknowledgement).into(), KEEP_ALIVE_PACKET_SIZE);
	}

	#[test]
	fn complete_packet_decoding_rejects_malformed_headers_and_the_reserved_type() {
		assert_eq!(read_packet(b"BET"), Err(PacketReadError::ShortHeader));
		assert_eq!(read_packet(b"NOPE\x01\0\0\0\0\0\0\0\0"), Err(PacketReadError::WrongProtocol));
		assert_eq!(read_packet(b"BETP\xff"), Err(PacketReadError::UnknownPacketType));
		assert_eq!(read_packet(b"BETP\x00"), Err(PacketReadError::ReservedPacketType));
	}

	#[test]
	fn every_packet_variant_serializes_header_and_complete_payload() {
		let mut request = [0u8; 13];
		write_packet(&mut request, Packets::from(ConnectionRequestPacket::new(0x0102030405060708))).unwrap();

		assert_eq!(&request[..5], b"BETP\x01");
		assert_eq!(&request[5..], &0x0102030405060708u64.to_le_bytes());

		let mut challenge = [0u8; 21];
		write_packet(&mut challenge, Packets::from(ChallengePacket::new(11, 22))).unwrap();

		assert_eq!(&challenge[..5], b"BETP\x02");
		assert_eq!(&challenge[5..13], &11u64.to_le_bytes());
		assert_eq!(&challenge[13..21], &22u64.to_le_bytes());

		let mut response = [0u8; 13];
		write_packet(&mut response, Packets::from(ChallengeResponsePacket::new(33))).unwrap();

		assert_eq!(&response[..5], b"BETP\x03");
		assert_eq!(&response[5..], &33u64.to_le_bytes());

		let acknowledgement = Acknowledgement::new(0x3344, 0x55667788);
		let mut data = [0u8; 5 + 8 + 6 + 1024];
		write_packet(&mut data, DataPacket::new(44, acknowledgement, [0xAB; 1024]).into()).unwrap();

		assert_eq!(&data[..5], b"BETP\x04");
		assert_eq!(&data[5..13], &44u64.to_le_bytes());
		assert_eq!(&data[13..19], &[0x44, 0x33, 0x88, 0x77, 0x66, 0x55]);
		assert!(data[19..].iter().all(|byte| *byte == 0xAB));

		let mut message = [0u8; 5 + 8 + 2 + 6 + 1024];
		write_packet(
			&mut message,
			MessagePacket::new(44, 0x1122, acknowledgement, [0xBA; 1024]).into(),
		)
		.unwrap();

		assert_eq!(&message[..5], b"BETP\x07");
		assert_eq!(&message[5..13], &44u64.to_le_bytes());
		assert_eq!(&message[13..21], &[0x22, 0x11, 0x44, 0x33, 0x88, 0x77, 0x66, 0x55]);
		assert!(message[21..].iter().all(|byte| *byte == 0xBA));

		let mut disconnect = [0u8; 13];
		write_packet(&mut disconnect, Packets::from(DisconnectPacket::new(55))).unwrap();

		assert_eq!(&disconnect[..5], b"BETP\x05");
		assert_eq!(&disconnect[5..], &55u64.to_le_bytes());

		let mut keep_alive = [0u8; 19];
		write_packet(&mut keep_alive, KeepAlivePacket::new(66, acknowledgement).into()).unwrap();

		assert_eq!(&keep_alive[..5], b"BETP\x06");
		assert_eq!(&keep_alive[5..13], &66u64.to_le_bytes());
		assert_eq!(&keep_alive[13..], &[0x44, 0x33, 0x88, 0x77, 0x66, 0x55]);
	}

	#[test]
	fn insufficient_packet_capacity_fails_without_partial_writes() {
		let mut bytes = [0xCC; 12];

		assert_eq!(write_packet(&mut bytes, Packets::from(ConnectionRequestPacket::new(1))), None);
		assert_eq!(bytes, [0xCC; 12]);

		let mut header = [0xCC; PACKET_HEADER_SIZE - 1];

		assert_eq!(write_packet_header(&mut header, PacketHeader::new(PacketType::Data)), None);
	}

	#[test]
	fn sequence_order_is_antisymmetric_across_wraparound() {
		assert!(!sequence_greater_than(7, 7));
		assert!(sequence_greater_than(1, 0));
		assert!(sequence_greater_than(0, u16::MAX));
		assert!(!sequence_greater_than(u16::MAX, 0));
		// The ambiguous half-range is resolved toward the numerically larger value so ordering remains antisymmetric.

		assert!(sequence_greater_than(32_768, 0));
		assert!(!sequence_greater_than(0, 32_768));

		for base in [0u16, 1, 1024, 32767, 65534, 65535] {
			for delta in [1u16, 2, 127, 32767] {
				let newer = base.wrapping_add(delta);

				assert!(sequence_greater_than(newer, base), "base={base}, delta={delta}");
				assert!(!sequence_greater_than(base, newer), "base={base}, delta={delta}");
			}
		}
	}
}
