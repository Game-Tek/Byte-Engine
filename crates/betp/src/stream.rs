//! The payload traffic of one established connection, shared by client and server sessions.

use std::{collections::VecDeque, time::Instant};

use crate::{
	KEEPALIVE_INTERVAL, RETRY_INTERVAL, WINDOW,
	packets::{Acknowledgement, DataPacket, KeepAlivePacket, MessagePacket, Packets, Payload},
	sequence_greater_than,
};

/// The `Stream` struct gives client and server sessions one set of delivery rules for their payloads.
///
/// Reliable messages are numbered, resent every [`RETRY_INTERVAL`] until acknowledged, and delivered once and in
/// order. Unreliable payloads are sent once and delivered as they arrive. Queue payloads with [`Self::send`], hand
/// every packet of the connection to [`Self::receive`], and send what [`Self::flush`] returns.
#[derive(Debug)]
pub(crate) struct Stream {
	/// Reliable messages waiting for room in the window.
	queued: VecDeque<Payload>,
	/// Unacknowledged reliable messages, each at `sequence % WINDOW`.
	in_flight: [Option<InFlight>; WINDOW],
	/// The sequence the next admitted message takes.
	next_sequence: u16,
	/// Unreliable payloads for the next flush.
	unreliable: Vec<Payload>,
	/// Reliable messages that arrived ahead of a missing one, each at `sequence % WINDOW`.
	held: [Option<Payload>; WINDOW],
	/// The sequence of the next reliable message to deliver.
	next_expected: u16,
	/// What the next packet acknowledges: the newest message received and which of the ones before it arrived.
	acknowledgement: Acknowledgement,
	/// Set when a message arrived, so the next flush acknowledges it even when nothing else is due.
	acknowledgement_due: bool,
	/// Payloads ready for the application, in delivery order.
	received: Vec<Payload>,
	/// When the stream last sent anything, which decides when a keepalive is due.
	last_sent: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct InFlight {
	sequence: u16,
	data: Payload,
	last_sent: Option<Instant>,
}

impl Stream {
	pub fn new() -> Self {
		Self {
			queued: VecDeque::new(),
			in_flight: [None; WINDOW],
			next_sequence: 0,
			unreliable: Vec::new(),
			held: [None; WINDOW],
			next_expected: 0,
			acknowledgement: Acknowledgement::default(),
			acknowledgement_due: false,
			received: Vec::new(),
			last_sent: None,
		}
	}

	/// Queues a payload for the next [`Self::flush`].
	pub fn send(&mut self, reliable: bool, data: Payload) {
		if reliable {
			self.queued.push_back(data);
		} else {
			self.unreliable.push(data);
		}
	}

	/// Takes in one packet that the session matched to this connection.
	pub fn receive(&mut self, packet: &Packets) {
		match packet {
			Packets::Data(packet) => {
				self.acknowledge(packet.get_acknowledgement());
				self.received.push(*packet.get_data());
			}
			Packets::Message(packet) => {
				self.acknowledge(packet.get_acknowledgement());
				self.hold(packet.get_sequence(), *packet.get_data());
			}
			Packets::KeepAlive(packet) => self.acknowledge(packet.get_acknowledgement()),
			_ => {}
		}
	}

	/// Removes and returns the payloads delivered so far.
	pub fn drain_received(&mut self) -> impl Iterator<Item = Payload> + '_ {
		self.received.drain(..)
	}

	/// Returns the packets to send now: new and overdue messages, unreliable payloads, and a keepalive when nothing
	/// else carries the acknowledgement or the peer has heard nothing for [`KEEPALIVE_INTERVAL`].
	pub fn flush(&mut self, connection_id: u64, current_time: Instant) -> Vec<Packets> {
		// Admit queued messages while the receiver has room for them: a message may only be sent once the one a
		// window before it was acknowledged.
		while self.in_flight[slot(self.next_sequence)].is_none()
			&& let Some(data) = self.queued.pop_front()
		{
			self.in_flight[slot(self.next_sequence)] = Some(InFlight {
				sequence: self.next_sequence,
				data,
				last_sent: None,
			});
			self.next_sequence = self.next_sequence.wrapping_add(1);
		}

		let acknowledgement = self.acknowledgement;
		let mut packets = Vec::new();
		for message in self.in_flight.iter_mut().flatten() {
			if message
				.last_sent
				.is_none_or(|last_sent| current_time.duration_since(last_sent) >= RETRY_INTERVAL)
			{
				packets.push(MessagePacket::new(connection_id, message.sequence, acknowledgement, message.data).into());
				message.last_sent = Some(current_time);
			}
		}
		packets.extend(
			self.unreliable
				.drain(..)
				.map(|data| DataPacket::new(connection_id, acknowledgement, data).into()),
		);

		let quiet = self
			.last_sent
			.is_none_or(|last_sent| current_time.duration_since(last_sent) >= KEEPALIVE_INTERVAL);
		if packets.is_empty() && (self.acknowledgement_due || quiet) {
			packets.push(KeepAlivePacket::new(connection_id, acknowledgement).into());
		}
		if !packets.is_empty() {
			self.acknowledgement_due = false;
			self.last_sent = Some(current_time);
		}
		packets
	}

	/// Stops resending the messages the peer acknowledged.
	fn acknowledge(&mut self, acknowledgement: Acknowledgement) {
		for sequence in acknowledgement.sequences() {
			let message = &mut self.in_flight[slot(sequence)];
			if message.is_some_and(|message| message.sequence == sequence) {
				*message = None;
			}
		}
	}

	/// Holds a reliable message until every earlier one has arrived, then delivers the run of messages it completes.
	fn hold(&mut self, sequence: u16, data: Payload) {
		// Even a duplicate needs acknowledging: the peer resent it because the last acknowledgement was lost.
		self.acknowledgement_due = true;

		// Earlier sequences were delivered already, and a peer that respects the window never sends further ahead.
		if usize::from(sequence.wrapping_sub(self.next_expected)) >= WINDOW {
			return;
		}

		// The window keeps every held message within 32 of the newest one. Before any message, the empty
		// acknowledgement at 0 marks message 0 as the newest when it arrives first.
		let Acknowledgement { ack, ack_bitfield } = self.acknowledgement;
		self.acknowledgement = if sequence_greater_than(sequence, ack) {
			let advance = u32::from(sequence.wrapping_sub(ack));
			Acknowledgement::new(sequence, ack_bitfield.checked_shl(advance).unwrap_or(0) | 1)
		} else {
			Acknowledgement::new(ack, ack_bitfield | 1 << ack.wrapping_sub(sequence))
		};
		self.held[slot(sequence)] = Some(data);
		while let Some(data) = self.held[slot(self.next_expected)].take() {
			self.received.push(data);
			self.next_expected = self.next_expected.wrapping_add(1);
		}
	}
}

/// Returns where a message with `sequence` lives in a window-sized buffer.
fn slot(sequence: u16) -> usize {
	usize::from(sequence) % WINDOW
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;

	const ID: u64 = 7;

	/// Delivers every packet `from` flushes to `to`, as a lossless link would.
	fn deliver(from: &mut Stream, to: &mut Stream, current_time: Instant) -> usize {
		let packets = from.flush(ID, current_time);
		for packet in &packets {
			to.receive(packet);
		}
		packets.len()
	}

	fn message(packet: &Packets) -> u16 {
		match packet {
			Packets::Message(message) => message.get_sequence(),
			_ => panic!("expected a message, got {packet:?}"),
		}
	}

	#[test]
	fn reliable_messages_are_delivered_in_order_when_an_earlier_one_is_lost() {
		let start = Instant::now();
		let (mut sender, mut receiver) = (Stream::new(), Stream::new());
		for fill in 0..3 {
			sender.send(true, Payload::filled(fill));
		}

		let first = sender.flush(ID, start);
		assert_eq!(first.iter().map(message).collect::<Vec<_>>(), [0, 1, 2]);
		receiver.receive(&first[2]);
		receiver.receive(&first[1]);
		assert_eq!(receiver.drain_received().count(), 0);

		// The receiver acknowledges what it holds, so only the lost message is sent again.
		deliver(&mut receiver, &mut sender, start);
		let retry = sender.flush(ID, start + RETRY_INTERVAL);
		assert_eq!(retry.iter().map(message).collect::<Vec<_>>(), [0]);
		receiver.receive(&retry[0]);

		assert_eq!(
			receiver.drain_received().collect::<Vec<_>>(),
			[Payload::filled(0), Payload::filled(1), Payload::filled(2)]
		);
	}

	#[test]
	fn a_message_is_resent_each_retry_interval_until_acknowledged_and_delivered_once() {
		let start = Instant::now();
		let (mut sender, mut receiver) = (Stream::new(), Stream::new());
		sender.send(true, Payload::filled(1));

		let first = sender.flush(ID, start);
		receiver.receive(&first[0]);
		assert_eq!(sender.flush(ID, start + RETRY_INTERVAL / 2), []);

		// The acknowledgement was lost, so the message comes again and is not delivered twice.
		let retry = sender.flush(ID, start + RETRY_INTERVAL);
		assert_eq!(retry, first);
		receiver.receive(&retry[0]);
		assert_eq!(receiver.drain_received().count(), 1);

		deliver(&mut receiver, &mut sender, start + RETRY_INTERVAL);
		assert_eq!(sender.flush(ID, start + RETRY_INTERVAL * 2), []);
	}

	#[test]
	fn messages_beyond_the_window_wait_until_the_oldest_is_acknowledged() {
		let start = Instant::now();
		let (mut sender, mut receiver) = (Stream::new(), Stream::new());
		for _ in 0..=WINDOW {
			sender.send(true, Payload::filled(0));
		}

		let first = sender.flush(ID, start);
		assert_eq!(first.len(), WINDOW);
		// Every message but the oldest arrives, which leaves no room for another.
		for packet in &first[1..] {
			receiver.receive(packet);
		}
		deliver(&mut receiver, &mut sender, start);
		assert_eq!(
			sender
				.flush(ID, start + RETRY_INTERVAL)
				.iter()
				.map(message)
				.collect::<Vec<_>>(),
			[0]
		);

		receiver.receive(&first[0]);
		deliver(&mut receiver, &mut sender, start + RETRY_INTERVAL);
		assert_eq!(
			sender
				.flush(ID, start + RETRY_INTERVAL)
				.iter()
				.map(message)
				.collect::<Vec<_>>(),
			[WINDOW as u16]
		);
		assert_eq!(receiver.drain_received().count(), WINDOW);
	}

	#[test]
	fn unreliable_payloads_are_sent_once_and_delivered_as_they_arrive() {
		let start = Instant::now();
		let (mut sender, mut receiver) = (Stream::new(), Stream::new());
		sender.send(false, Payload::filled(1));
		sender.send(false, Payload::filled(2));

		let packets = sender.flush(ID, start);
		receiver.receive(&packets[1]);
		receiver.receive(&packets[0]);

		assert_eq!(
			receiver.drain_received().collect::<Vec<_>>(),
			[Payload::filled(2), Payload::filled(1)]
		);
		assert_eq!(sender.flush(ID, start + RETRY_INTERVAL), []);
	}

	#[test]
	fn a_received_message_is_acknowledged_at_once_and_otherwise_a_keepalive_goes_out_per_interval() {
		let start = Instant::now();
		let (mut sender, mut receiver) = (Stream::new(), Stream::new());
		assert!(matches!(receiver.flush(ID, start)[..], [Packets::KeepAlive(_)]));
		assert_eq!(receiver.flush(ID, start + KEEPALIVE_INTERVAL / 2), []);

		sender.send(true, Payload::filled(1));
		deliver(&mut sender, &mut receiver, start);
		let acknowledgement = receiver.flush(ID, start + Duration::from_millis(1));

		assert!(
			matches!(&acknowledgement[..], [Packets::KeepAlive(packet)] if packet.get_acknowledgement() == Acknowledgement::new(0, 1))
		);
		assert_eq!(receiver.flush(ID, start + Duration::from_millis(2)), []);
	}
}
