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

		// If the buffer is full and the packet is reliable, replace the packet with the most retries.
		if reliable {
			self.buffer.iter_mut().max_by_key(|packet| packet.as_ref().map_or(0, |p| p.try_count)).map(|p| {
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			});
		} else {
			// If the buffer is full and the packet is unreliable, replace the first non-reliable packet with the most retries.
			self.buffer.iter_mut().filter(|packet| packet.as_ref().map_or(false, |p| !p.reliable)).max_by_key(|packet| packet.as_ref().map_or(0, |p| p.try_count)).map(|p| {
				*p = Some(BufferedPacket::new(packet, connection_id, reliable));
			});
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
	pub fn gather_unsent_packets(&mut self) -> Vec<DataPacket<S>> {
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

	#[test]
	fn test_add_packets() {
		let mut buffer = PacketBuffer::<4, 16>::new();

		assert_eq!(buffer.gather_unsent_packets().len(), 0);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(0, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(1, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(2, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(3, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer.iter().filter(|p| p.is_some()).count(), 4);
	}

	#[test]
	fn test_replace_packets() {
		let mut buffer = PacketBuffer::<4, 16>::new();

		assert_eq!(buffer.gather_unsent_packets().len(), 0);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(0, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(1, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(2, 0, 0), [0; 16]), 1, false);
		buffer.add(DataPacket::new(1, ConnectionStatus::new(3, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer.iter().filter(|p| p.is_some()).count(), 4);

		buffer.add(DataPacket::new(1, ConnectionStatus::new(4, 0, 0), [0; 16]), 1, false);

		assert_eq!(buffer.buffer[0].unwrap().packet.connection_status.sequence, 4); // The first packet should be replaced.
	}
}
