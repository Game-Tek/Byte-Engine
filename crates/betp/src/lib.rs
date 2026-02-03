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

#![feature(buf_read_has_data_left)]
#![feature(generic_const_exprs)] // https://github.com/rust-lang/rust/issues/133199

pub mod client;
pub mod server;

mod local;
mod remote;

mod packet_buffer;

pub mod packets;

pub use client::Client;
pub use server::Server;

use std::io::{Read as _, Write};

use crate::packets::{ConnectionStatus, Packet, PacketHeader, Packets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// [`PacketInfo`] contains information about a packet.
/// - `acked`: A boolean that indicates if the packet has been acknowledged.
pub(crate) struct PacketInfo {
    acked: bool,
}

/// Compares two sequence numbers and returns true if the first sequence number is greater than the second.
/// The function takes into account the wrap-around of the sequence numbers.
pub(crate) fn sequence_greater_than(s1: u16, s2: u16) -> bool {
	((s1 > s2) && (s1 - s2 <= 32768u16)) || ((s1 < s2) && (s2 - s1 > 32768u16))
}

fn has_written_anything(s: usize) -> Option<()> {
	if s > 0 {
		Some(())
	} else {
		None
	}
}

pub(crate) fn write_packet_header(buffer: &mut [u8], packet_header: PacketHeader) -> Option<()> {
	let mut cursor = std::io::Cursor::new(buffer);

	let protocol = &packet_header.protocol_id;
	let packet_type = [packet_header.r#type as u8];
	let packet_type = &packet_type;

	cursor.write(protocol).ok().and_then(has_written_anything)?;
	cursor.write(packet_type).ok().and_then(has_written_anything)?;

	Some(())
}

pub(crate) fn write_connection_status(buffer: &mut [u8], connection_status: ConnectionStatus) -> Option<()> {
	let mut cursor = std::io::Cursor::new(buffer);

	let sequence = connection_status.sequence.to_le_bytes();
	let ack = connection_status.ack.to_le_bytes();
	let ack_bifield = connection_status.ack_bitfield.to_le_bytes();

	cursor
		.write(&sequence)
		.ok()
		.and_then(has_written_anything)?;
	cursor.write(&ack).ok().and_then(has_written_anything)?;
	cursor
		.write(&ack_bifield)
		.ok()
		.and_then(has_written_anything)?;

	Some(())
}

pub fn write_packet(buffer: &mut [u8], packet: Packets) -> Option<()> {
	let header = packet.header();

	write_packet_header(buffer, header)?;

	match packet {
		Packets::Data(packet) => {
			write_connection_status(buffer, packet.connection_status)?;

			let mut cursor = std::io::Cursor::new(buffer);

			cursor.write(&packet.data).ok().and_then(has_written_anything)?;
		}
		_ => {}
	}

	Some(())
}

pub(crate) fn read_packet_header(buffer: &[u8]) -> Result<PacketHeader, ()> {
	let mut cursor = std::io::Cursor::new(buffer);

	let mut protocol_id = [0u8; 4];

	cursor.read(&mut protocol_id);

	if protocol_id != ['B' as u8, 'E' as u8, 'T' as u8, 'P' as u8] {
		return Err(());
	}

	let mut r#type = [0u8; 1];

	cursor.read(&mut r#type);

	let r#type = unsafe { std::mem::transmute(r#type[0]) };

	Ok(PacketHeader {
		protocol_id, r#type,
	})
}
