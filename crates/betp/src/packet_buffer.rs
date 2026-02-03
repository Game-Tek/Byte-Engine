use tinyvec::ArrayVec;

use crate::packets::DataPacket;

/// A buffered packet.
#[derive(Debug, Clone, Copy)]
struct BufferedPacket<const S: usize> {
	/// The packet.
	packet: DataPacket<S>,
	/// The connection the packet is intended for.
	connection_id: u64,
	/// The number of tries.
	try_count: u8,
	/// Is the packet reliable.
	reliable: bool,
}

impl <const S: usize> BufferedPacket<S> {
	/// Creates a new buffered packet.
	fn new(packet: DataPacket<S>, connection_id: u64, reliable: bool) -> Self {
		Self {
			packet,
			connection_id,
			try_count: 0,
			reliable,
		}
	}
}

/// A packet buffer that holds packets to be sent.
/// The buffer has a fixed size and will overwrite old packets when full.
#[derive(Debug, Clone, Copy)]
pub struct PacketBuffer<const N: usize, const S: usize> {
	/// The buffer.
	buffer: [Option<BufferedPacket<S>>; N],
}

impl <const N: usize, const S: usize> PacketBuffer<N, S> {
	/// Creates a new packet buffer.
	pub fn new() -> Self {
		Self {
			buffer: [None; N],
		}
	}

	/// Adds a packet to the buffer.
	/// Called when a packet is sent.
	/// Reliable packets have higher priority.
	pub fn add(&mut self, packet: DataPacket<S>, connection_id: u64, reliable: bool) {
		// Try to find an empty slot in the buffer.
		for i in 0..N {
			if self.buffer[i].is_none() {
				self.buffer[i] = Some(BufferedPacket::new(packet, connection_id, reliable));
				return;
			}
		}

		// If the buffer is full, replace the first unreliable packet.
		for i in 0..N {
			if let Some(p) = self.buffer[i] {
				if !p.reliable {
					self.buffer[i] = Some(BufferedPacket::new(packet, connection_id, reliable));
					return;
				}
			}
		}

		// If the buffer is full and the packet is reliable, replace the oldest packet with the most retries.
		if reliable {
			if let Some(p) = self.buffer.iter_mut().max_by_key(|packet| packet.as_ref().map_or((std::cmp::Reverse(0), 0), |p| (std::cmp::Reverse(p.packet.connection_status.sequence), p.try_count))) {
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			}
		} else {
			// If the buffer is full and the packet is unreliable, replace the first non-reliable packet with the most retries.
			if let Some(p) = self.buffer.iter_mut().filter(|packet| packet.as_ref().map_or(false, |p| !p.reliable)).max_by_key(|packet| packet.as_ref().map_or(0, |p| p.try_count)) {
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			}
		}
	}

	/// Removes a packet from the buffer.
	/// Usually called when a packet is acknowledged.
	pub fn remove(&mut self, sequence: u16) {
		for i in 0..N {
			if let Some(packet) = self.buffer[i] {
				if packet.packet.connection_status.sequence == sequence {
					self.buffer[i] = None;
					break;
				}
			}
		}
	}

	/// Gets the unsent packets in the buffer.
	/// This is not an idempotent operation as the retry count will be incremented.
	pub fn gather_unsent_packets_for_retry(&mut self) -> ArrayVec<[DataPacket<S>; N]> {
        self.buffer.iter_mut().filter_map(|packet| packet.as_mut()).map(|packet| {
			packet.try_count += 1;
			packet.packet
		}).collect()
    }
}

#[cfg(test)]
mod tests {
	use crate::packets::ConnectionStatus;

	use super::*;

	fn make_packet<const S: usize>(sequence: u16, fill: u8) -> DataPacket<S> {
		DataPacket::new(1, ConnectionStatus::new(sequence, 0, 0), [fill; S])
	}

	#[test]
	fn test_new_buffer_is_empty() {
		let buffer = PacketBuffer::<4, 8>::new();

		assert!(buffer.buffer.iter().all(|packet| packet.is_none()));
	}

	#[test]
	fn test_add_packets() {
		let mut buffer = PacketBuffer::<4, 16>::new();

		assert_eq!(buffer.gather_unsent_packets_for_retry().len(), 0);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(0, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(1, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(2, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(3, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer.iter().filter(|p| p.is_some()).count(), 4);
	}

	#[test]
	fn test_add_fills_first_empty_slot() {
		let mut buffer = PacketBuffer::<4, 8>::new();

		buffer.add(make_packet::<8>(10, 1), 7, true);

		let entry = buffer.buffer[0].expect("expected a packet in the first slot");
		assert_eq!(entry.packet.connection_status.sequence, 10);
		assert_eq!(entry.connection_id, 7);
		assert!(entry.reliable);
		assert_eq!(entry.try_count, 0);
	}

	#[test]
	fn test_add_replaces_first_unreliable_even_for_reliable_packet() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, true);
		buffer.add(make_packet::<8>(2, 2), 1, false);
		buffer.add(make_packet::<8>(3, 3), 1, true);

		buffer.add(make_packet::<8>(9, 9), 1, true);

		assert_eq!(buffer.buffer[1].unwrap().packet.connection_status.sequence, 9);
	}

	#[test]
	fn test_add_unreliable_dropped_when_buffer_full_of_reliable() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, true);
		buffer.add(make_packet::<8>(2, 2), 1, true);
		buffer.add(make_packet::<8>(3, 3), 1, true);

		let sequences_before: [u16; 3] = [
			buffer.buffer[0].unwrap().packet.connection_status.sequence,
			buffer.buffer[1].unwrap().packet.connection_status.sequence,
			buffer.buffer[2].unwrap().packet.connection_status.sequence,
		];

		buffer.add(make_packet::<8>(9, 9), 1, false);

		let sequences_after: [u16; 3] = [
			buffer.buffer[0].unwrap().packet.connection_status.sequence,
			buffer.buffer[1].unwrap().packet.connection_status.sequence,
			buffer.buffer[2].unwrap().packet.connection_status.sequence,
		];

		assert_eq!(sequences_after, sequences_before);
	}

	#[test]
	fn test_add_reliable_replaces_packet_with_most_retries() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, true);
		buffer.add(make_packet::<8>(2, 2), 1, true);
		buffer.add(make_packet::<8>(3, 3), 1, true);

		buffer.gather_unsent_packets_for_retry();

		buffer.remove(2);
		buffer.add(make_packet::<8>(4, 4), 1, true);

		buffer.gather_unsent_packets_for_retry();

		assert_eq!(buffer.buffer[0].unwrap().try_count, 2);
		assert_eq!(buffer.buffer[1].unwrap().try_count, 1);
		assert_eq!(buffer.buffer[2].unwrap().try_count, 2);

		buffer.add(make_packet::<8>(9, 9), 1, true);

		assert_eq!(buffer.buffer[0].unwrap().packet.connection_status.sequence, 9);
	}

	#[test]
	fn test_replace_packets() {
		let mut buffer = PacketBuffer::<4, 16>::new();

		assert_eq!(buffer.gather_unsent_packets_for_retry().len(), 0);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(0, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(1, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(2, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(3, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer.iter().filter(|p| p.is_some()).count(), 4);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(4, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer[0].unwrap().packet.connection_status.sequence, 4); // The first packet should be replaced.
	}

	#[test]
	fn test_remove_existing_and_missing_sequence() {
		let mut buffer = PacketBuffer::<4, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, false);
		buffer.add(make_packet::<8>(2, 2), 1, false);

		buffer.remove(2);

		assert!(buffer.buffer.iter().any(|packet| {
			packet
				.map(|packet| packet.packet.connection_status.sequence == 1)
				.unwrap_or(false)
		}));
		assert!(buffer.buffer.iter().all(|packet| {
			packet
				.map(|packet| packet.packet.connection_status.sequence != 2)
				.unwrap_or(true)
		}));

		let count_before = buffer.buffer.iter().filter(|packet| packet.is_some()).count();
		buffer.remove(99);
		let count_after = buffer.buffer.iter().filter(|packet| packet.is_some()).count();

		assert_eq!(count_after, count_before);
	}

	#[test]
	fn test_gather_unsent_packets_increments_try_count() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, false);
		buffer.add(make_packet::<8>(2, 2), 1, true);

		let first = buffer.gather_unsent_packets_for_retry();
		let second = buffer.gather_unsent_packets_for_retry();

		assert_eq!(first.len(), 2);
		assert_eq!(second.len(), 2);
		assert_eq!(buffer.buffer[0].unwrap().try_count, 2);
		assert_eq!(buffer.buffer[1].unwrap().try_count, 2);

		let sequences: ArrayVec<[u16; 3]> = buffer
			.buffer
			.iter()
			.filter_map(|packet| packet.map(|packet| packet.packet.connection_status.sequence))
			.collect();
		assert!(sequences.contains(&1));
		assert!(sequences.contains(&2));
	}
}
