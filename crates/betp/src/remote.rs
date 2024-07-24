//! Remote is a state tracking structure to keep track of the state of the communication with a remote.

use std::ops::Shl;

use utils::bit_array::BitArray;

use super::{sequence_greater_than, PacketInfo};

/// The packet history is the number of (last) packets that we keep track of.
const PACKET_HISTORY: usize = 1024;

/// Remote is used to keep track of the state of the communication with a remote.
/// If we are the server, the remote is a client. If we are the client, the remote is the server.
#[derive(Clone, Copy)]
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
    pub(crate) fn get_packet_data(&self, sequence: u16) -> Option<PacketInfo> {
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
        let window_shift = sequence.max(self.ack) - self.ack;

        // If the packet sequence is more recent, we update the remote sequence number.
        if sequence_greater_than(sequence, self.ack) {
            // Under ridiculously high packet loss (99%) old sequence buffer entries might stick around from before the previous sequence number wrap at 65535 and break the ack logic.
            // The solution to this problem is to walk between the previous highest insert sequence and the new insert sequence (if it is more recent) and clear those entries in the sequence buffer to 0xFFFF.
            for i in self.ack..sequence {
                let index = (i % PACKET_HISTORY as u16) as usize;
                self.receive_sequence_buffer[index] = u16::MAX;
            }

            self.receive_sequence_buffer[index] = sequence;

            self.ack = sequence;
        }

        self.packet_data.set(index, true);

        self.ack_bitfield = self.ack_bitfield.checked_shl(window_shift as u32).unwrap_or(0) | (1 << ((self.ack - sequence) % u32::BITS as u16));
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
        assert_eq!(
            remote.get_ack_bitfield(),
            1 << 4 | 1 << 3 | 1 << 2 | 1 << 1 | 1 << 0
        );
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
}
