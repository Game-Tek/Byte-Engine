use super::{sequence_greater_than, PacketHeader, PacketInfo};

/// Remote is a state tracking structure to keep track of the state of the communication with a remote.
#[derive(Clone, Copy)]
pub struct Remote {
	// The ack is the most recent sequence number received by the server.
	ack: u16,
	// The ack bitfield is a 32-bit number that represents the last 32 sequence numbers received by the server.
	ack_bitfield: u32,
	packet_data: BitArray<1024>,

	receive_sequence_buffer: [u16; 1024],
}

impl Remote {
	pub fn new() -> Self {
		Self {
			ack: 0,
			ack_bitfield: 0,
			receive_sequence_buffer: [u16::MAX; 1024],
			packet_data: BitArray::new(),
		}
	}

	pub(crate) fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
		let index = (sequence % 1024) as usize;
		if self.receive_sequence_buffer[index] == sequence {
			Some(PacketInfo { acked: self.packet_data.get(index) })
		} else {
			None
		}
	}

	/// Acknowledges a packet with the given sequence number. This means that the remote has received the packet.
	pub fn acknowledge_packet(&mut self, sequence: u16) {
		let index = (sequence % 1024) as usize;
		let window_shift = sequence.max(self.ack) - self.ack;

		// If the packet sequence is more recent, we update the remote sequence number.
		if sequence_greater_than(sequence, self.ack) {
			// Under ridiculously high packet loss (99%) old sequence buffer entries might stick around from before the previous sequence number wrap at 65535 and break the ack logic.
			// The solution to this problem is to walk between the previous highest insert sequence and the new insert sequence (if it is more recent) and clear those entries in the sequence buffer to 0xFFFF.
			for i in self.ack..sequence {
				let index = (i % 1024) as usize;
				self.receive_sequence_buffer[index] = u16::MAX;
			}

			self.receive_sequence_buffer[index] = sequence;

			self.ack = sequence;
		}

		self.packet_data.set(index, true);

		self.ack_bitfield = (self.ack_bitfield << window_shift) | 1 << ((self.ack - sequence) % 32);
	}

	pub fn get_ack(&self) -> u16 {
		self.ack
	}

	pub fn get_ack_bitfield(&self) -> u32 {
		self.ack_bitfield
	}
}

#[derive(Clone, Copy)]
struct BitArray<const N: usize> where [u8; N / 8]: {
	data: [u8; N / 8],
}

impl<const N: usize> BitArray<N> where [u8; N / 8]: {
	fn new() -> Self {
		Self {
			data: [0; N / 8],
		}
	}

	fn set(&mut self, index: usize, value: bool) {
		let byte_index = index / 8;
		let bit_index = index % 8;
		let mask = 1 << bit_index;
		if value {
			self.data[byte_index] |= mask;
		} else {
			self.data[byte_index] &= !mask;
		}
	}

	fn get(&self, index: usize) -> bool {
		let byte_index = index / 8;
		let bit_index = index % 8;
		let mask = 1 << bit_index;
		(self.data[byte_index] & mask) != 0
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

		// TODO: Test when ack and sequence are more than 32 apart.
	}
}