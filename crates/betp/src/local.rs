/// The number of recently sent packets retained for acknowledgment tracking.
const PACKET_HISTORY: usize = 1024;

/// The `Local` struct preserves the bounded send history used to interpret peer acknowledgements.
#[derive(Debug, Clone, Copy)]
pub struct Local {
	// The sequence is a 16-bit number that is incremented for each packet sent.
	sequence: u16,
	packet_data: BitArray<PACKET_HISTORY>,
	// Validity is tracked independently because every `u16`, including `u16::MAX`, is a valid sequence number.
	packet_valid: BitArray<PACKET_HISTORY>,
	sequence_buffer: [u16; PACKET_HISTORY],
}

impl Default for Local {
	fn default() -> Self {
		Self::new()
	}
}

impl Local {
	pub fn new() -> Self {
		Self {
			sequence: 0,
			sequence_buffer: [0; PACKET_HISTORY],
			packet_data: BitArray::new(),
			packet_valid: BitArray::new(),
		}
	}

	pub(crate) fn get_sequence_number(&mut self) -> u16 {
		let index = (self.sequence % PACKET_HISTORY as u16) as usize;
		self.sequence_buffer[index] = self.sequence;
		self.packet_data.set(index, false);
		self.packet_valid.set(index, true);
		let sequence = self.sequence;
		self.sequence = self.sequence.wrapping_add(1);
		sequence
	}

	pub fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		if self.packet_valid.get(index) && self.sequence_buffer[index] == sequence {
			Some(PacketInfo {
				acked: self.packet_data.get(index),
			})
		} else {
			None
		}
	}

	/// Records that the remote received the given sequence number.
	pub fn acknowledge_packet(&mut self, sequence: u16) {
		let index = (sequence % PACKET_HISTORY as u16) as usize;
		if self.packet_valid.get(index) && self.sequence_buffer[index] == sequence {
			self.packet_data.set(index, true);
		}
	}

	pub fn acknowledge_packets(&mut self, ack: u16, ack_bitfield: u32) {
		for i in 0..u32::BITS {
			if (ack_bitfield >> i) & 1 == 1 {
				let sequence = ack.wrapping_sub(i as u16);
				self.acknowledge_packet(sequence);
			}
		}
	}

	/// Returns sent sequence numbers that the remote has not acknowledged.
	pub fn unacknowledged_packets(&self) -> impl Iterator<Item = u16> + '_ {
		self.sequence_buffer
			.iter()
			.enumerate()
			.filter(|(i, _)| self.packet_valid.get(*i) && !self.packet_data.get(*i))
			.map(|(_, &e)| e)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_make_request() {
		let mut local = Local::new();
		let packet_header = local.get_sequence_number();
		assert_eq!(packet_header, 0);

		local.acknowledge_packet(0);

		let packet_info = local.get_packet_data(0);
		assert_eq!(packet_info, Some(PacketInfo { acked: true }));

		for _i in 1..1024 {
			let _packet_header = local.get_sequence_number();
		}

		local.get_sequence_number();

		let packet_info = local.get_packet_data(0); // Although indices wrap around, the packet with sequence 0 must not be valid anymore.
		assert_eq!(packet_info, None);
		let packet_info = local.get_packet_data(1024);
		assert_eq!(packet_info, Some(PacketInfo { acked: false }));
	}

	#[test]
	fn test_get_packet_data() {
		let mut local = Local::new();
		let packet_header = local.get_packet_data(0);
		assert_eq!(packet_header, None);
		let packet_header = local.get_packet_data(1023);
		assert_eq!(packet_header, None);

		local.get_sequence_number();
		let packet_header = local.get_packet_data(0);
		assert_eq!(packet_header, Some(PacketInfo { acked: false }));
		let packet_header = local.get_packet_data(1023);
		assert_eq!(packet_header, None);

		local.get_sequence_number();
		let packet_header = local.get_packet_data(0);
		assert_eq!(packet_header, Some(PacketInfo { acked: false }));
		let packet_header = local.get_packet_data(1023);
		assert_eq!(packet_header, None);
		let packet_header = local.get_packet_data(1);
		assert_eq!(packet_header, Some(PacketInfo { acked: false }));

		local.acknowledge_packet(0);
		let packet_header = local.get_packet_data(0);
		assert_eq!(packet_header, Some(PacketInfo { acked: true }));
		let packet_header = local.get_packet_data(1023);
		assert_eq!(packet_header, None);
		let packet_header = local.get_packet_data(1);
		assert_eq!(packet_header, Some(PacketInfo { acked: false }));
	}

	#[test]
	fn test_packet_acknowledgement() {
		let mut local = Local::new();

		for _i in 0..32 {
			local.get_sequence_number();
		}

		assert!(local.unacknowledged_packets().eq(0u16..32u16));

		for i in 0..32 {
			local.acknowledge_packet(i);
		}

		assert!(local.unacknowledged_packets().next().is_none());

		for _i in 0..32 {
			local.get_sequence_number();
		}

		assert!(local.unacknowledged_packets().eq(32u16..64u16));

		for i in 0..32 {
			local.acknowledge_packet(i);
		}

		assert!(local.unacknowledged_packets().eq(32u16..64u16));

		for i in 32..64 {
			local.acknowledge_packet(i);
		}

		assert!(local.unacknowledged_packets().next().is_none());
	}

	#[test]
	fn test_sparse_packet_acknowledgement() {
		let mut local = Local::new();

		for _i in 0..32 {
			local.get_sequence_number();
		}

		local.acknowledge_packet(0);

		assert!(local.unacknowledged_packets().eq(1u16..32u16));

		local.acknowledge_packet(2);

		assert!(local.unacknowledged_packets().eq((1u16..32u16).filter(|&i| i != 2)));

		local.acknowledge_packet(4);

		assert!(local.unacknowledged_packets().eq((1u16..32u16).filter(|&i| i != 2 && i != 4)));

		local.acknowledge_packet(1);

		assert!(local.unacknowledged_packets().eq((3u16..32u16).filter(|&i| i != 4)));

		local.acknowledge_packet(3);

		assert!(local.unacknowledged_packets().eq(5u16..32u16));
	}

	#[test]
	fn test_acknowledge_packets() {
		let mut local = Local::new();

		for _i in 0..32 {
			local.get_sequence_number();
		}

		local.acknowledge_packets(0, 0b0);

		assert!(local.unacknowledged_packets().eq(0u16..32u16));

		local.acknowledge_packets(0, 0b1);

		assert!(local.unacknowledged_packets().eq(1u16..32u16));

		local.acknowledge_packets(2, 0b101);

		assert!(local.unacknowledged_packets().eq((1u16..32u16).filter(|&i| i != 2)));
	}

	#[test]
	fn test_acknowledge_unsent_packets() {
		let mut local = Local::new();

		local.acknowledge_packet(0);
	}

	#[test]
	fn acknowledgement_bitfield_wraps_before_sequence_zero() {
		let mut local = Local::new();
		local.get_sequence_number();

		local.acknowledge_packets(0, 1 << 31);
		assert_eq!(local.get_packet_data(0), Some(PacketInfo { acked: false }));

		local.acknowledge_packets(0, (1 << 31) | 1);
		assert_eq!(local.get_packet_data(0), Some(PacketInfo { acked: true }));
	}

	#[test]
	fn same_slot_sequence_from_another_generation_cannot_acknowledge_a_send() {
		let mut local = Local::new();
		assert_eq!(local.get_sequence_number(), 0);

		local.acknowledge_packet(PACKET_HISTORY as u16);

		assert_eq!(local.get_packet_data(0), Some(PacketInfo { acked: false }));
	}

	#[test]
	fn full_sequence_cycle_retains_max_without_confusing_it_for_an_empty_slot() {
		let mut local = Local::new();

		for expected in 0..=u16::MAX {
			assert_eq!(local.get_sequence_number(), expected);
		}

		assert_eq!(local.get_packet_data(u16::MAX), Some(PacketInfo { acked: false }));
		assert_eq!(local.get_packet_data(0), None);
		assert_eq!(local.unacknowledged_packets().count(), PACKET_HISTORY);
		assert!(local.unacknowledged_packets().any(|sequence| sequence == u16::MAX));

		assert_eq!(local.get_sequence_number(), 0);
		assert_eq!(local.get_packet_data(0), Some(PacketInfo { acked: false }));
		assert_eq!(local.get_packet_data(64_512), None);
	}

	#[test]
	fn acknowledgement_window_matches_reference_model_after_sequence_wrap() {
		let mut local = Local::new();
		for _ in 0..=u16::MAX {
			local.get_sequence_number();
		}
		for _ in 0..8 {
			local.get_sequence_number();
		}

		let ack = 3;
		let ack_bitfield = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 31);
		local.acknowledge_packets(ack, ack_bitfield);

		for sequence in (64_520..=u16::MAX).chain(0..8) {
			let distance = ack.wrapping_sub(sequence);
			let expected_acknowledged = distance < u32::BITS as u16 && (ack_bitfield >> distance) & 1 == 1;
			assert_eq!(
				local.get_packet_data(sequence),
				Some(PacketInfo {
					acked: expected_acknowledged,
				}),
				"sequence={sequence}, distance={distance}",
			);
		}
	}
}

use utils::bit_array::BitArray;

use crate::PacketInfo;
