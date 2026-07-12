//! Remote is a state tracking structure to keep track of the state of the communication with a remote.

/// The packet history is the number of (last) packets that we keep track of.
const PACKET_HISTORY: usize = 1024;

/// Remote is used to keep track of the state of the communication with a remote.
/// If we are the server, the remote is a client. If we are the client, the remote is the server.
#[derive(Debug, Clone, Copy)]
pub struct Remote {
	/// The ack is the most recent acknoledged sequence number by the remote.
	ack: u16,
	/// The ack bitfield is a 32-bit number that represents the last 32 sequence numbers acknowledged by the remote.
	ack_bitfield: u32,
	/// The packet data is a bit array tracks the ack status of the last 1024 packets.
	packet_data: BitArray<PACKET_HISTORY>,
	/// The receive sequence buffer is a buffer that stores the last 1024 sequence numbers received by the remote.
	receive_sequence_buffer: [u16; PACKET_HISTORY],
}

impl Default for Remote {
	fn default() -> Self {
		Self::new()
	}
}

impl Remote {
	pub fn new() -> Self {
		Self {
			ack: 0,
			ack_bitfield: 0,
			receive_sequence_buffer: [u16::MAX; PACKET_HISTORY],
			packet_data: BitArray::new(),
		}
	}

	/// Returns information about the packet with the given sequence number.
	/// If the packet is in the history, it returns the information about the packet.
	/// If the packet is not in the history, it returns None.
	pub fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		if self.receive_sequence_buffer[index] == sequence {
			Some(PacketInfo {
				acked: self.packet_data.get(index),
			})
		} else {
			None
		}
	}

	/// Acknowledges a packet with the given sequence number. This means that the remote has received the packet.
	pub fn acknowledge_packet(&mut self, sequence: u16) {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		let is_newer = sequence_greater_than(sequence, self.ack);

		if is_newer {
			let previous_ack = self.ack;
			let advance = sequence.wrapping_sub(previous_ack);

			// Only retained history can affect future lookups. Limiting cleanup to that window prevents a large sequence jump from causing work proportional to the 16-bit sequence space.
			let entries_to_clear = usize::from(advance).min(PACKET_HISTORY);
			for offset in 1..=entries_to_clear {
				let cleared_sequence = previous_ack.wrapping_add(offset as u16);
				let cleared_index = (cleared_sequence % PACKET_HISTORY as u16) as usize;
				self.receive_sequence_buffer[cleared_index] = u16::MAX;
			}

			self.ack = sequence;
			self.ack_bitfield = self.ack_bitfield.checked_shl(u32::from(advance)).unwrap_or(0) | 1;
		} else {
			let distance = self.ack.wrapping_sub(sequence);

			// Packets older than the acknowledgement bitfield cannot contribute to the wire acknowledgement state.
			if distance < u32::BITS as u16 {
				self.ack_bitfield |= 1 << distance;
			}
		}

		let history_distance = self.ack.wrapping_sub(sequence);
		if is_newer || usize::from(history_distance) < PACKET_HISTORY {
			self.receive_sequence_buffer[index] = sequence;
			self.packet_data.set(index, true);
		}
	}

	pub fn get_ack(&self) -> u16 {
		self.ack
	}

	pub fn get_ack_bitfield(&self) -> u32 {
		self.ack_bitfield
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_packet_acknowledgement() {
		let mut remote = Remote::new();

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 0);

		for i in 0..32 {
			remote.acknowledge_packet(i);
		}

		assert_eq!(remote.get_ack(), 31);
		assert_eq!(remote.get_ack_bitfield(), 0xFFFF_FFFF);

		for i in 0..32 {
			remote.acknowledge_packet(i);
		}

		assert_eq!(remote.get_ack(), 31);
		assert_eq!(remote.get_ack_bitfield(), 0xFFFF_FFFF);

		for i in 32..48 {
			remote.acknowledge_packet(i);
		}

		assert_eq!(remote.get_ack(), 47);
		assert_eq!(remote.get_ack_bitfield(), 0xFFFF_FFFF);

		for i in 48..64 {
			remote.acknowledge_packet(i);
		}

		assert_eq!(remote.get_ack(), 63);
		assert_eq!(remote.get_ack_bitfield(), 0xFFFF_FFFF);
	}

	#[test]
	fn test_sparse_packet_acknowledgement() {
		let mut remote = Remote::new();

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 0);

		remote.acknowledge_packet(0);

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 1 << 0);

		remote.acknowledge_packet(2);

		assert_eq!(remote.get_ack(), 2);
		assert_eq!(remote.get_ack_bitfield(), 1 << 2 | 1 << 0);

		remote.acknowledge_packet(4);

		assert_eq!(remote.get_ack(), 4);
		assert_eq!(remote.get_ack_bitfield(), 1 << 4 | 1 << 2 | 1 << 0);

		remote.acknowledge_packet(1);

		assert_eq!(remote.get_ack(), 4);
		assert_eq!(remote.get_ack_bitfield(), 1 << 4 | 1 << 3 | 1 << 2 | 1 << 0);

		remote.acknowledge_packet(3);

		assert_eq!(remote.get_ack(), 4);
		assert_eq!(remote.get_ack_bitfield(), 1 << 4 | 1 << 3 | 1 << 2 | 1 << 1 | 1 << 0);
	}

	#[test]
	fn test_out_of_range_packet_acknowledgement() {
		let mut remote = Remote::new();

		remote.acknowledge_packet(0);

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 1 << 0);

		remote.acknowledge_packet(64);

		assert_eq!(remote.get_ack(), 64);
		assert_eq!(remote.get_ack_bitfield(), 1 << 0);

		remote.acknowledge_packet(0);

		assert_eq!(remote.get_ack(), 64);
		assert_eq!(remote.get_ack_bitfield(), 1 << 0);

		remote.acknowledge_packet(32);

		assert_eq!(remote.get_ack(), 64);
		assert_eq!(remote.get_ack_bitfield(), 1 << 0);
	}

	#[test]
	fn acknowledgement_wraps_without_overflow() {
		let mut remote = Remote::new();

		remote.acknowledge_packet(u16::MAX);

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 0b10);
		assert_eq!(remote.get_packet_data(u16::MAX), Some(PacketInfo { acked: true }));
	}

	#[test]
	fn advancing_across_wrap_preserves_recent_history() {
		let mut remote = Remote::new();

		remote.acknowledge_packet(32_767);
		remote.acknowledge_packet(u16::MAX);
		remote.acknowledge_packet(0);

		assert_eq!(remote.get_ack(), 0);
		assert_eq!(remote.get_ack_bitfield(), 0b11);
		assert_eq!(remote.get_packet_data(u16::MAX), Some(PacketInfo { acked: true }));
		assert_eq!(remote.get_packet_data(0), Some(PacketInfo { acked: true }));
	}
}

use utils::bit_array::BitArray;

use super::{sequence_greater_than, PacketInfo};
