use super::{sequence_greater_than, PacketInfo};

/// Local is a state tracking structure to keep track of the state of the communication with a remote.
#[derive(Clone, Copy)]
pub struct Local {
	// The sequence is a 16-bit number that is incremented for each packet sent.
	sequence: u16,
	packet_data: BitArray<1024>,
	sequence_buffer: [u16; 1024],
}

impl Local {
	pub fn new() -> Self {
		Self {
			sequence: 0,
			sequence_buffer: [u16::MAX; 1024],
			packet_data: BitArray::new(),
		}
	}

	pub(crate) fn get_sequence_number(&mut self) -> u16 {
		let index = (self.sequence % 1024) as usize;
		self.sequence_buffer[index] = self.sequence;
		self.packet_data.set(index, false);
		let sequence = self.sequence;
		self.sequence = self.sequence.wrapping_add(1);
		sequence
	}

	pub(crate) fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
		let index = (sequence % 1024) as usize;
		if self.sequence_buffer[index] == sequence {
			Some(PacketInfo { acked: self.packet_data.get(index) })
		} else {
			None
		}
	}

	/// Acknowledges a packet with the given sequence number. This means that the remote has received the packet.
	pub fn acknowledge_packet(&mut self, sequence: u16) {
		let index = (sequence % 1024) as usize;
		if self.sequence_buffer[index] == sequence {
			self.packet_data.set(index, true);
		}
	}

	pub fn acknowledge_packets(&mut self, ack: u16, ack_bitfield: u32) {
		for i in 0..32 {
			if (ack_bitfield >> i) & 1 == 1 {
				let sequence = ack - i;
				self.acknowledge_packet(sequence);
			}
		}
	}

	/// Returns the unacknowledged packets of this [`Local`]. These are the packets that have been sent but have not been acknowledged by the remote.
	pub fn unacknowledged_packets(&self) -> Vec<u16> {
		self.sequence_buffer.iter().enumerate().filter(|(i, &sequence)| sequence != u16::MAX && !self.packet_data.get(*i)).map(|(_, &e)| e).collect()
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
	fn test_make_request() {
		let mut local = Local::new();
		let packet_header = local.get_sequence_number();
		assert_eq!(packet_header, 0);

		local.acknowledge_packet(0);

		let packet_info = local.get_packet_data(0);
		assert_eq!(packet_info, Some(PacketInfo { acked: true }));

		for i in 1..1024 {
			let packet_header = local.get_sequence_number();
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

		for i in 0..32 {
			local.get_sequence_number();
		}

		assert_eq!(local.unacknowledged_packets(), (0u16..32u16).collect::<Vec<_>>());

		for i in 0..32 {
			local.acknowledge_packet(i);
		}

		assert_eq!(local.unacknowledged_packets(), Vec::<u16>::new());

		for i in 0..32 {
			local.get_sequence_number();
		}

		assert_eq!(local.unacknowledged_packets(), (32u16..64u16).collect::<Vec<_>>());

		for i in 0..32 {
			local.acknowledge_packet(i);
		}

		assert_eq!(local.unacknowledged_packets(), (32u16..64u16).collect::<Vec<_>>());

		for i in 32..64 {
			local.acknowledge_packet(i);
		}

		assert_eq!(local.unacknowledged_packets(), Vec::<u16>::new());
	}

	#[test]
	fn test_sparse_packet_acknowledgement() {
		let mut local = Local::new();

		for i in 0..32 {
			local.get_sequence_number();
		}

		local.acknowledge_packet(0);

		assert_eq!(local.unacknowledged_packets(), (1u16..32u16).collect::<Vec<_>>());

		local.acknowledge_packet(2);

		assert_eq!(local.unacknowledged_packets(), (1u16..32u16).filter(|&i| i != 2).collect::<Vec<_>>());

		local.acknowledge_packet(4);

		assert_eq!(local.unacknowledged_packets(), (1u16..32u16).filter(|&i| i != 2 && i != 4).collect::<Vec<_>>());

		local.acknowledge_packet(1);

		assert_eq!(local.unacknowledged_packets(), (3u16..32u16).filter(|&i| i != 4).collect::<Vec<_>>());

		local.acknowledge_packet(3);

		assert_eq!(local.unacknowledged_packets(), (5u16..32u16).collect::<Vec<_>>());
	}

	#[test]
	fn test_acknowledge_packets() {
		let mut local = Local::new();

		for i in 0..32 {
			local.get_sequence_number();
		}

		local.acknowledge_packets(0, 0b0);

		assert_eq!(local.unacknowledged_packets(), (0u16..32u16).collect::<Vec<_>>());

		local.acknowledge_packets(0, 0b1);

		assert_eq!(local.unacknowledged_packets(), (1u16..32u16).collect::<Vec<_>>());

		local.acknowledge_packets(2, 0b101);

		assert_eq!(local.unacknowledged_packets(), (1u16..32u16).filter(|&i| i != 2).collect::<Vec<_>>());
	}

	#[test]
	fn test_acknowledge_unsent_packets() {
		let mut local = Local::new();

		local.acknowledge_packet(0);
	}
}