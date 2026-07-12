//! In-memory client transport for deterministic replication tests.

use std::{
	sync::mpsc::{Receiver, Sender, TryRecvError},
	time::Instant,
};

use betp::{
	client::{Errors, Session},
	packets::Packets,
};

/// The `ChannelClient` struct provides an engine client endpoint backed by an
/// in-process channel.
pub struct ChannelClient {
	session: Session,
	incoming: Receiver<Packets>,
	outgoing: Sender<Packets>,
	received: Vec<[u8; 1024]>,
}

impl ChannelClient {
	pub(crate) fn new(incoming: Receiver<Packets>, outgoing: Sender<Packets>) -> Self {
		Self {
			session: Session::new(),
			incoming,
			outgoing,
			received: Vec::new(),
		}
	}

	pub fn connect(&mut self, current_time: Instant) {
		// A stable non-zero salt keeps in-process tests deterministic.
		self.session.connect(current_time.elapsed().as_nanos() as u64 + 1);
	}

	pub fn is_connected(&self) -> bool {
		self.session.is_connected()
	}

	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<(), Errors> {
		self.session.send(reliable, data);
		Ok(())
	}

	/// Advances the protocol without blocking on the channel.
	pub fn update(&mut self) -> Result<(), Errors> {
		let mut packets = Vec::new();
		while let Ok(packet) = self.incoming.try_recv() {
			packets.push(packet);
		}

		for packet in &packets {
			if let Packets::Data(packet) = packet {
				self.received.push(packet.data);
			}
		}

		for packet in self.session.update(&packets)? {
			self.outgoing.send(packet).map_err(|_| Errors::IoError)?;
		}
		Ok(())
	}

	/// Removes and returns every application payload received since the last drain.
	pub fn drain_received(&mut self) -> impl Iterator<Item = [u8; 1024]> + '_ {
		self.received.drain(..)
	}

	pub fn disconnect(&mut self) {
		self.session.disconnect();
	}
}
