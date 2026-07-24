//! Tracks packets received from a remote BETP endpoint.

/// The number of recently received packets retained for acknowledgment tracking.
const PACKET_HISTORY: usize = 1024;

/// The `Remote` struct preserves the bounded receive history used to construct wire acknowledgements.
#[derive(Debug, Clone, Copy)]
pub struct Remote {
	/// The newest sequence number received from the remote.
	ack: u16,
	/// The 32-packet acknowledgment window relative to `ack`.
	ack_bitfield: u32,
	/// The receive status of packets in retained history.
	packet_data: BitArray<PACKET_HISTORY>,
	// Validity is tracked independently because every `u16`, including `u16::MAX`, is a valid sequence number.
	packet_valid: BitArray<PACKET_HISTORY>,
	/// The sequence numbers stored in retained history.
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
			receive_sequence_buffer: [0; PACKET_HISTORY],
			packet_data: BitArray::new(),
			packet_valid: BitArray::new(),
		}
	}

	/// Returns tracking information when the sequence number remains in history.
	pub fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		if self.packet_valid.get(index) && self.receive_sequence_buffer[index] == sequence {
			Some(PacketInfo {
				acked: self.packet_data.get(index),
			})
		} else {
			None
		}
	}

	/// Records a packet and returns whether it is newly accepted within the retained receive window.
	pub fn acknowledge_packet(&mut self, sequence: u16) -> bool {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		let is_newer = sequence_greater_than(sequence, self.ack);
		let was_received = self.packet_valid.get(index) && self.receive_sequence_buffer[index] == sequence;

		if !is_newer {
			let history_distance = self.ack.wrapping_sub(sequence);
			// Duplicates and packets outside retained history must not produce another application-visible delivery.
			if was_received || usize::from(history_distance) >= PACKET_HISTORY {
				return false;
			}
		}

		if is_newer {
			let previous_ack = self.ack;
			let advance = sequence.wrapping_sub(previous_ack);

			// Only retained history can affect future lookups. Limiting cleanup to that window prevents a large sequence jump from causing work proportional to the 16-bit sequence space.
			let entries_to_clear = usize::from(advance).min(PACKET_HISTORY);
			for offset in 1..=entries_to_clear {
				let cleared_sequence = previous_ack.wrapping_add(offset as u16);
				let cleared_index = (cleared_sequence % PACKET_HISTORY as u16) as usize;
				self.packet_valid.set(cleared_index, false);
			}

			self.ack = sequence;
			let shifted_acknowledgements = self.ack_bitfield.checked_shl(u32::from(advance)).unwrap_or(0);
			// A positive shift clears bit zero, so adding one records the newest packet without changing older acknowledgement bits.
			self.ack_bitfield = shifted_acknowledgements + 1;
		} else {
			let distance = self.ack.wrapping_sub(sequence);

			// Packets older than the acknowledgement bitfield cannot contribute to the wire acknowledgement state.
			if distance < u32::BITS as u16 {
				self.ack_bitfield |= 1 << distance;
			}
		}

		// The early stale/duplicate return leaves only newer or retained-window packets to record here.
		self.receive_sequence_buffer[index] = sequence;
		self.packet_data.set(index, true);
		self.packet_valid.set(index, true);

		true
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

	#[test]
	fn empty_history_does_not_treat_max_sequence_as_received() {
		let remote = Remote::new();

		assert_eq!(remote.get_packet_data(u16::MAX), None);
	}

	#[test]
	fn full_sequence_cycle_matches_a_sliding_receive_window() {
		let mut remote = Remote::new();

		for sequence in 0..=u16::MAX {
			remote.acknowledge_packet(sequence);
			assert_eq!(remote.get_ack(), sequence);
		}

		assert_eq!(remote.get_ack_bitfield(), u32::MAX);
		assert_eq!(remote.get_packet_data(u16::MAX), Some(PacketInfo { acked: true }));
		assert_eq!(remote.get_packet_data(u16::MAX - 1_023), Some(PacketInfo { acked: true }));
		assert_eq!(remote.get_packet_data(u16::MAX - 1_024), None);
	}

	#[test]
	fn duplicate_and_stale_sequences_do_not_change_acknowledgement_state() {
		let mut remote = Remote::new();
		for sequence in 0..=2_000 {
			remote.acknowledge_packet(sequence);
		}
		let ack = remote.get_ack();
		let ack_bitfield = remote.get_ack_bitfield();

		assert!(!remote.acknowledge_packet(2_000));
		assert!(!remote.acknowledge_packet(0));
		assert!(!remote.acknowledge_packet(976));

		assert_eq!(remote.get_ack(), ack);
		assert_eq!(remote.get_ack_bitfield(), ack_bitfield);
		assert_eq!(remote.get_packet_data(0), None);
	}

	#[test]
	fn large_jumps_clear_history_and_accept_missing_packets_only_within_the_new_window() {
		let mut remote = Remote::new();
		for sequence in 0..=2 {
			assert!(remote.acknowledge_packet(sequence));
		}

		assert!(remote.acknowledge_packet(2_000));
		assert_eq!(remote.get_ack(), 2_000);
		assert_eq!(remote.get_packet_data(2), None);
		assert!(remote.acknowledge_packet(1_500));
		assert!(!remote.acknowledge_packet(1_500));
		assert!(!remote.acknowledge_packet(900));
		assert_eq!(remote.get_packet_data(1_500), Some(PacketInfo { acked: true }));
		assert_eq!(remote.get_packet_data(900), None);
	}
}

use utils::bit_array::BitArray;

use super::{sequence_greater_than, PacketInfo};
