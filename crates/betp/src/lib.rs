//! This crate contains the implementation of the Byte Engine Transport Protocol (BETP) protocol.
//! The implementation is designed _sans-io_ and can be used with any I/O implementation.
//!
//! # Module Structure
//!
//! The module is divided into the following submodules:
//!
//! - `server`: Contains the implementation of the server.
//! - `client`: Contains the implementation of the client.
//! - `remote`: Contains the implementation of the remote connection.
//! - `local`: Contains the implementation of the local connection.
//! - `packets`: Contains the implementation of the packets used in the protocol.
//!
//! # Protocol Overview
//!
//! The Byte Engine Transport Protocol (BETP) is a simple, reliable, and ordered protocol that is used to transfer data between a client and a server.
//! The protocol is designed to be used in a client-server architecture where the server is the authoritative entity that manages connections to clients and maintains the state of the game.
//! The protocol is built on top of the User Datagram Protocol (UDP) and provides reliable and ordered delivery of packets.
//!
//! The protocol consists of the following packets in the following order:
//!
//! - Connection Request Packet: Sent by the client to request a connection to the server.
//! - Challenge Packet: Sent by the server to challenge the client.
//! - Challenge Response Packet: Sent by the client to respond to the challenge.
//! - Data Packet: Sent by the client or server to send data.
//! - Disconnect Packet: Sent by the client or server to update the connection status.
//!
//! The protocol uses sequence numbers to ensure that packets are delivered in order and to detect lost packets.
//! The protocol also uses acknowledgments to ensure that packets are reliably delivered.
//!
//! The protocol is designed to be simple and easy to implement, making it suitable for use in real-time multiplayer games.

#![allow(incomplete_features)]
#![allow(clippy::items_after_test_module)]
#![feature(generic_const_exprs)] // https://github.com/rust-lang/rust/issues/133199

pub mod client;
pub mod server;

mod local;
mod remote;

mod packet_buffer;

pub mod packets;

pub use client::Client;
pub use local::Local;
pub use remote::Remote;
pub use server::Server;

/// Packet header parsing failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketReadError {
	/// The packet header bytes could not be read from the source buffer.
	ShortHeader,
	/// The header protocol id does not match BETP.
	WrongProtocol,
	/// The packet type byte does not map to a supported BETP packet type.
	UnknownPacketType,
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
		}
	}
}

impl std::error::Error for PacketReadError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// [`PacketInfo`] contains information about a packet.
/// - `acked`: A boolean that indicates if the packet has been acknowledged.
pub struct PacketInfo {
	pub acked: bool,
}

/// Compares two sequence numbers and returns true if the first sequence number is greater than the second.
/// The function takes into account the wrap-around of the sequence numbers.
pub(crate) fn sequence_greater_than(s1: u16, s2: u16) -> bool {
	((s1 > s2) && (s1 - s2 <= 32768u16)) || ((s1 < s2) && (s2 - s1 > 32768u16))
}

const PACKET_HEADER_SIZE: usize = 5;
const CONNECTION_STATUS_SIZE: usize = 8;

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

pub(crate) fn write_connection_status(buffer: &mut [u8], connection_status: ConnectionStatus) -> Option<()> {
	let mut offset = 0;
	write_bytes(buffer, &mut offset, &connection_status.sequence.to_le_bytes())?;
	write_bytes(buffer, &mut offset, &connection_status.ack.to_le_bytes())?;
	write_bytes(buffer, &mut offset, &connection_status.ack_bitfield.to_le_bytes())?;

	Some(())
}

pub fn write_packet(buffer: &mut [u8], packet: Packets) -> Option<()> {
	let payload_size = match &packet {
		Packets::ConnectionRequest(_) | Packets::ChallengeResponse(_) | Packets::Disconnect(_) => 8,
		Packets::Challenge(_) => 16,
		Packets::Data(packet) => 8 + CONNECTION_STATUS_SIZE + packet.data.len(),
	};
	let required_size = PACKET_HEADER_SIZE.checked_add(payload_size)?;

	// Validate capacity first so a failed serialization never leaves a partial packet in the caller's buffer.
	if buffer.len() < required_size {
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
			write_bytes(buffer, &mut offset, &packet.connection_id.to_le_bytes())?;
			write_connection_status(&mut buffer[offset..], packet.connection_status)?;
			offset += CONNECTION_STATUS_SIZE;
			write_bytes(buffer, &mut offset, &packet.data)?;
		}
		Packets::Disconnect(packet) => {
			write_bytes(buffer, &mut offset, &packet.get_connection_id().to_le_bytes())?;
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
		_ => return Err(PacketReadError::UnknownPacketType),
	};

	Ok(PacketHeader { protocol_id, r#type })
}

use std::io::Read as _;

use crate::packets::{ConnectionStatus, Packet, PacketHeader, PacketType, Packets};

#[cfg(test)]
mod tests {
	use super::{
		read_packet_header, sequence_greater_than, write_connection_status, write_packet, write_packet_header, PacketReadError,
		CONNECTION_STATUS_SIZE, PACKET_HEADER_SIZE,
	};
	use crate::packets::{
		ChallengePacket, ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket,
		PacketHeader, PacketType, Packets,
	};

	#[test]
	fn packet_header_round_trips_every_supported_discriminant() {
		for packet_type in [
			PacketType::Default,
			PacketType::ConnectionRequest,
			PacketType::Challenge,
			PacketType::ChallengeResponse,
			PacketType::Data,
			PacketType::Disconnect,
		] {
			let mut bytes = [0u8; PACKET_HEADER_SIZE];
			write_packet_header(&mut bytes, PacketHeader::new(packet_type)).expect("header capacity is exact");
			assert_eq!(&bytes[..4], b"BETP");
			assert_eq!(bytes[4], packet_type as u8);
			assert_eq!(read_packet_header(&bytes), Ok(PacketHeader::new(packet_type)));
		}
	}

	#[test]
	fn malformed_headers_report_distinct_causes() {
		assert_eq!(read_packet_header(b"BET"), Err(PacketReadError::ShortHeader));
		assert_eq!(read_packet_header(b"NOPE\x04"), Err(PacketReadError::WrongProtocol));
		assert_eq!(read_packet_header(b"BETP\xff"), Err(PacketReadError::UnknownPacketType));

		for error in [
			PacketReadError::ShortHeader,
			PacketReadError::WrongProtocol,
			PacketReadError::UnknownPacketType,
		] {
			let message = error.to_string();
			assert!(message.contains("most likely cause"));
			assert!(message.ends_with('.'));
		}
	}

	#[test]
	fn connection_status_uses_stable_little_endian_layout() {
		let mut bytes = [0u8; CONNECTION_STATUS_SIZE];
		write_connection_status(&mut bytes, ConnectionStatus::new(0x1122, 0x3344, 0x55667788))
			.expect("status capacity is exact");
		assert_eq!(bytes, [0x22, 0x11, 0x44, 0x33, 0x88, 0x77, 0x66, 0x55]);
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

		let status = ConnectionStatus::new(0x1122, 0x3344, 0x55667788);
		let mut data = [0u8; 5 + 8 + 8 + 1024];
		write_packet(&mut data, Packets::Data(DataPacket::new(44, status, [0xAB; 1024]))).unwrap();
		assert_eq!(&data[..5], b"BETP\x04");
		assert_eq!(&data[5..13], &44u64.to_le_bytes());
		assert_eq!(&data[13..21], &[0x22, 0x11, 0x44, 0x33, 0x88, 0x77, 0x66, 0x55]);
		assert!(data[21..].iter().all(|byte| *byte == 0xAB));

		let mut disconnect = [0u8; 13];
		write_packet(&mut disconnect, Packets::from(DisconnectPacket::new(55))).unwrap();
		assert_eq!(&disconnect[..5], b"BETP\x05");
		assert_eq!(&disconnect[5..], &55u64.to_le_bytes());
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

		for base in [0u16, 1, 1024, 32767, 65534, 65535] {
			for delta in [1u16, 2, 127, 32767] {
				let newer = base.wrapping_add(delta);
				assert!(sequence_greater_than(newer, base), "base={base}, delta={delta}");
				assert!(!sequence_greater_than(base, newer), "base={base}, delta={delta}");
			}
		}
	}
}
