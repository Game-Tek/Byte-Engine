/// A reliable packet is retired after this many sends so an unresponsive peer cannot retain work indefinitely.
pub(crate) const MAX_RELIABLE_SEND_ATTEMPTS: u8 = 8;

/// The `BufferedPacket` struct retains the metadata needed to enforce send policy without allocating.
#[derive(Debug, Clone, Copy)]
struct BufferedPacket<const S: usize> {
	/// The packet.
	packet: DataPacket<S>,
	/// The connection the packet is intended for.
	connection_id: u64,
	/// The number of completed send attempts.
	try_count: u8,
	/// Whether the session retries the packet.
	reliable: bool,
}

impl<const S: usize> BufferedPacket<S> {
	/// Stores a packet with no completed send attempts.
	fn new(packet: DataPacket<S>, connection_id: u64, reliable: bool) -> Self {
		Self {
			packet,
			connection_id,
			try_count: 0,
			reliable,
		}
	}
}

/// The `PacketBuffer` struct bounds queued sends and retry work for one protocol session.
#[derive(Debug, Clone, Copy)]
pub struct PacketBuffer<const N: usize, const S: usize> {
	/// Fixed storage for pending packets.
	buffer: [Option<BufferedPacket<S>>; N],
}

impl<const N: usize, const S: usize> PacketBuffer<N, S> {
	/// Creates an empty packet buffer.
	pub fn new() -> Self {
		Self { buffer: [None; N] }
	}

	/// Adds a packet and gives reliable packets replacement priority.
	pub fn add(&mut self, packet: DataPacket<S>, connection_id: u64, reliable: bool) {
		// Try to find an empty slot in the buffer.
		if let Some(slot) = self.buffer.iter_mut().find(|slot| slot.is_none()) {
			*slot = Some(BufferedPacket::new(packet, connection_id, reliable));
			return;
		}

		// If the buffer is full, replace the first unreliable packet.
		if let Some(slot) = self.buffer.iter_mut().find(|slot| slot.as_ref().is_some_and(|p| !p.reliable)) {
			*slot = Some(BufferedPacket::new(packet, connection_id, reliable));
			return;
		}

		// If the buffer is full and the packet is reliable, replace the oldest packet with the most retries.
		if reliable {
			if let Some(p) = self.buffer.iter_mut().max_by_key(|packet| {
				packet.as_ref().map_or((std::cmp::Reverse(0), 0), |p| {
					(std::cmp::Reverse(p.packet.connection_status.sequence), p.try_count)
				})
			}) {
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			}
		} else {
			// If the buffer is full and the packet is unreliable, replace the first non-reliable packet with the most retries.
			if let Some(p) = self
				.buffer
				.iter_mut()
				.filter(|packet| packet.as_ref().is_some_and(|p| !p.reliable))
				.max_by_key(|packet| packet.as_ref().map_or(0, |p| p.try_count))
			{
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			}
		}
	}

	/// Removes the packet with the given sequence number.
	pub fn remove(&mut self, sequence: u16) {
		if let Some(slot) = self
			.buffer
			.iter_mut()
			.find(|slot| slot.as_ref().is_some_and(|p| p.packet.connection_status.sequence == sequence))
		{
			*slot = None;
		}
	}

	/// Removes the packets covered by a peer acknowledgement window.
	pub fn acknowledge_packets(&mut self, ack: u16, ack_bitfield: u32) {
		for bit in 0..u32::BITS {
			if (ack_bitfield >> bit) & 1 == 1 {
				self.remove(ack.wrapping_sub(bit as u16));
			}
		}
	}

	/// Gathers the next bounded batch and retires one-shot or exhausted entries.
	pub fn gather_unsent_packets_for_retry(&mut self) -> ArrayVec<[DataPacket<S>; N]> {
		let mut packets = ArrayVec::new();

		for slot in &mut self.buffer {
			let Some(mut buffered) = *slot else {
				continue;
			};

			let _ = buffered.connection_id;
			buffered.try_count += 1;
			packets.push(buffered.packet);

			// Unreliable packets are one-shot. Reliable packets remain only while another bounded attempt is allowed.
			*slot = if buffered.reliable && buffered.try_count < MAX_RELIABLE_SEND_ATTEMPTS {
				Some(buffered)
			} else {
				None
			};
		}

		packets
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::packets::ConnectionStatus;

	fn make_packet<const S: usize>(sequence: u16, fill: u8) -> DataPacket<S> {
		DataPacket::new(1, ConnectionStatus::new(sequence, 0, 0), [fill; S])
	}

	fn gathered_sequences<const N: usize, const S: usize>(buffer: &mut PacketBuffer<N, S>) -> Vec<u16> {
		buffer
			.gather_unsent_packets_for_retry()
			.into_iter()
			.map(|packet| packet.connection_status.sequence)
			.collect()
	}

	#[test]
	fn full_buffer_admission_prioritizes_reliable_packets() {
		let mut replaces_unreliable = PacketBuffer::<3, 8>::new();

		replaces_unreliable.add(make_packet::<8>(1, 1), 1, true);
		replaces_unreliable.add(make_packet::<8>(2, 2), 1, false);
		replaces_unreliable.add(make_packet::<8>(3, 3), 1, true);
		replaces_unreliable.add(make_packet::<8>(9, 9), 1, true);

		assert_eq!(gathered_sequences(&mut replaces_unreliable), [1, 9, 3]);

		let mut rejects_unreliable = PacketBuffer::<3, 8>::new();

		rejects_unreliable.add(make_packet::<8>(1, 1), 1, true);
		rejects_unreliable.add(make_packet::<8>(2, 2), 1, true);
		rejects_unreliable.add(make_packet::<8>(3, 3), 1, true);
		rejects_unreliable.add(make_packet::<8>(9, 9), 1, false);

		assert_eq!(gathered_sequences(&mut rejects_unreliable), [1, 2, 3]);
	}

	#[test]
	fn reliable_admission_replaces_the_most_retried_packet() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, true);
		buffer.add(make_packet::<8>(2, 2), 1, true);
		buffer.add(make_packet::<8>(3, 3), 1, true);

		let _ = buffer.gather_unsent_packets_for_retry();

		buffer.remove(2);
		buffer.add(make_packet::<8>(4, 4), 1, true);

		let _ = buffer.gather_unsent_packets_for_retry();

		buffer.add(make_packet::<8>(9, 9), 1, true);

		assert_eq!(gathered_sequences(&mut buffer), [9, 4, 3]);
	}

	#[test]
	fn test_remove_existing_and_missing_sequence() {
		let mut buffer = PacketBuffer::<4, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, true);
		buffer.add(make_packet::<8>(2, 2), 1, true);

		buffer.remove(2);
		assert_eq!(gathered_sequences(&mut buffer), [1]);
		buffer.remove(99);
		assert_eq!(gathered_sequences(&mut buffer), [1]);
	}

	#[test]
	fn unreliable_packets_are_sent_once_while_reliable_packets_remain_for_retry() {
		let mut buffer = PacketBuffer::<3, 8>::new();

		buffer.add(make_packet::<8>(1, 1), 1, false);
		buffer.add(make_packet::<8>(2, 2), 1, true);

		let first = buffer.gather_unsent_packets_for_retry();
		let second = buffer.gather_unsent_packets_for_retry();

		assert_eq!(
			first
				.iter()
				.map(|packet| packet.connection_status.sequence)
				.collect::<Vec<_>>(),
			[1, 2]
		);
		assert_eq!(
			second
				.iter()
				.map(|packet| packet.connection_status.sequence)
				.collect::<Vec<_>>(),
			[2]
		);
	}

	#[test]
	fn reliable_retry_eventually_quiesces_when_acknowledgements_never_arrive() {
		let mut buffer = PacketBuffer::<1, 8>::new();
		buffer.add(make_packet::<8>(1, 1), 1, true);

		for _ in 0..MAX_RELIABLE_SEND_ATTEMPTS {
			assert_eq!(buffer.gather_unsent_packets_for_retry().len(), 1);
		}

		assert_eq!(buffer.gather_unsent_packets_for_retry().len(), 0);
		assert!(buffer.buffer[0].is_none());
	}

	#[test]
	fn acknowledgement_window_removes_exactly_the_modeled_sequences() {
		let mut buffer = PacketBuffer::<8, 8>::new();
		for sequence in [u16::MAX - 1, u16::MAX, 0, 1, 2, 3, 4, 5] {
			buffer.add(make_packet::<8>(sequence, 1), 1, true);
		}

		// Relative to ack 2, bits 0, 2, and 4 cover sequences 2, 0, and u16::MAX - 1.
		buffer.acknowledge_packets(2, 0b1_0101);

		for sequence in [2, 0, u16::MAX - 1] {
			assert!(
				buffer
					.buffer
					.iter()
					.all(|entry| { entry.is_none_or(|entry| entry.packet.connection_status.sequence != sequence) })
			);
		}
		for sequence in [u16::MAX, 1, 3, 4, 5] {
			assert!(
				buffer
					.buffer
					.iter()
					.any(|entry| { entry.is_some_and(|entry| entry.packet.connection_status.sequence == sequence) })
			);
		}
	}
}

use tinyvec::ArrayVec;

use crate::packets::DataPacket;
