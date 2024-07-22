pub mod remote;
pub mod local;

pub mod packets;

pub mod server;
pub mod client;

use remote::Remote;
use local::Local;

use std::{hash::{Hash, Hasher}, io::{Read, Write}, ops::Sub};

use self::packets::DataPacket;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PacketInfo {
	acked: bool,
}

#[derive(Clone, Copy)]
enum ConnectionStates {
	Negotiating,
	Connected,
}

pub(crate) fn sequence_greater_than(s1: u16, s2: u16) -> bool {
	((s1 > s2) && (s1 - s2 <= 32768u16)) || (( s1 < s2) && (s2 - s1 > 32768u16))
}

fn has_written_anything(s: usize) -> Option<()> {
	if s > 0 { Some(()) } else { None }
}

fn write_packet<const N: usize>(buffer: &mut [u8], packet_header: DataPacket<N>) -> Option<()> {
	let mut cursor = std::io::Cursor::new(buffer);

	{
		let protocol = &packet_header.header.protocol_id;
		let sequence = packet_header.connection_status.sequence.to_le_bytes();
		let ack = packet_header.connection_status.ack.to_le_bytes();
		let ack_bifield = packet_header.connection_status.ack_bitfield.to_le_bytes();

		cursor.write(protocol).ok().and_then(has_written_anything)?;
		cursor.write(&sequence).ok().and_then(has_written_anything)?;
		cursor.write(&ack).ok().and_then(has_written_anything)?;
		cursor.write(&ack_bifield).ok().and_then(has_written_anything)?;
	}

	Some(())
}

#[cfg(test)]
mod tests {
	use std::io::{BufRead, Read};

	use tests::packets::{ConnectionStatus, PacketHeader, PacketType};

	use super::*;

	#[test]
	fn test_write_packet() {
		let mut buffer = [0u8; 12];
		write_packet(&mut buffer, DataPacket { header: PacketHeader{ protocol_id: [b'B', b'E', b'T', b'P'], r#type: PacketType::Data }, connection_id: 0, connection_status: ConnectionStatus::new(0, 0, 0),  data: [] }).unwrap();

		let mut cursor = std::io::Cursor::new(&buffer);

		let mut protocol_id = [0u8; 4];
		assert_eq!(cursor.read(&mut protocol_id).unwrap(), 4);
		assert_eq!(&protocol_id, b"BETP");

		let mut sequence = [0u8; 2];
		assert_eq!(cursor.read(&mut sequence).unwrap(), 2);
		assert_eq!(u16::from_le_bytes([sequence[0], sequence[1]]), 0);

		let mut ack = [0u8; 2];
		assert_eq!(cursor.read(&mut ack).unwrap(), 2);
		assert_eq!(u16::from_le_bytes([ack[0], ack[1]]), 0);

		let mut ack_bitfield = [0u8; 4];
		assert_eq!(cursor.read(&mut ack_bitfield).unwrap(), 4);
		assert_eq!(u32::from_le_bytes([ack_bitfield[0], ack_bitfield[1], ack_bitfield[2], ack_bitfield[3]]), 0);

		assert!(!cursor.has_data_left().unwrap());
	}

	#[test]
	fn test_sequence_greater_than() {
		assert_eq!(sequence_greater_than(1, 0), true);
		assert_eq!(sequence_greater_than(0, 1), false);
		assert_eq!(sequence_greater_than(32768, 0), true);
		assert_eq!(sequence_greater_than(0, 32768), false);
		assert_eq!(sequence_greater_than(32767, 0), true);
		assert_eq!(sequence_greater_than(0, 32767), false);
		assert_eq!(sequence_greater_than(32767, 1), true);
		assert_eq!(sequence_greater_than(1, 32767), false);
	}
}