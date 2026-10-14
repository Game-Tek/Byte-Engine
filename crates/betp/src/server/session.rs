/// The `Session` struct preserves the protocol state for one server-side connection.
///
/// The server transport runs the handshake, then calls [`Self::accept`] and [`Self::update`] once per tick.
#[derive(Debug)]
pub struct Session {
	state: State,
	stream: Stream,
}

impl Session {
	/// Creates a session with no connection. Call [`Self::accept`] once the client answers its challenge.
	pub fn new() -> Self {
		Self {
			state: State::Initial,
			stream: Stream::new(),
		}
	}

	/// Accepts a connection after its challenge response has been validated by
	/// the server transport.
	///
	/// The next [`Self::update`] sends a keepalive unless a payload goes out first. Either one tells the client
	/// that the server accepted it.
	pub fn accept(&mut self, connection_id: u64, current_time: Instant) {
		self.state = State::Connected {
			id: connection_id,
			last_seen: current_time,
		};
		self.stream = Stream::new();
	}

	/// Handles the packets received since the last call and returns the packets to send now.
	///
	/// Call it on every application tick, with or without packets, so the session can resend unacknowledged
	/// messages, send keepalives, and end a connection whose client has gone quiet for [`crate::TIMEOUT`]. Read the
	/// payloads it delivered with [`Self::drain_received`].
	pub fn update(&mut self, packets: &[Packets], current_time: Instant) -> Result<Vec<Packets>, PacketHandlingResults> {
		match &mut self.state {
			State::Initial => Ok(Vec::new()),
			State::Connected { id, last_seen } => {
				let id = *id;
				for packet in packets {
					// Only packets for this session may mutate delivery, acknowledgement state, or liveness.
					if packet.connection_id() != Some(id) {
						continue;
					}
					if let Packets::Disconnect(_) = packet {
						self.state = State::Disconnecting { id };
						return Ok(Vec::new());
					}

					self.stream.receive(packet);
					*last_seen = current_time;
				}

				if current_time.duration_since(*last_seen) > TIMEOUT {
					self.state = State::Disconnecting { id };
					return Ok(Vec::new());
				}

				Ok(self.stream.flush(id, current_time))
			}
			State::Disconnecting { id } => {
				let id = *id;

				Ok(vec![DisconnectPacket::new(id).into()])
			}
		}
	}

	/// Queues a payload for the client.
	///
	/// Reliable payloads are resent until the client acknowledges them and arrive in the order they were queued.
	/// Unreliable payloads are sent once.
	pub fn send(&mut self, reliable: bool, data: Payload) {
		self.stream.send(reliable, data);
	}

	/// Removes and returns the payloads the client sent, reliable ones in order.
	pub fn drain_received(&mut self) -> impl Iterator<Item = Payload> + '_ {
		self.stream.drain_received()
	}

	/// Starts a voluntary disconnect from the client.
	///
	/// Reconnect the session before you send or receive more data.
	pub fn disconnect(&mut self) {
		if let State::Connected { id, .. } = self.state {
			self.state = State::Disconnecting { id }
		}
	}

	pub fn is_connected(&self) -> bool {
		matches!(self.state, State::Connected { .. })
	}

	pub fn connection_id(&self) -> Option<u64> {
		match self.state {
			State::Connected { id, .. } => Some(id),
			_ => None,
		}
	}
}

impl Default for Session {
	fn default() -> Self {
		Self::new()
	}
}

/// The `State` enum identifies the current phase of a server-side session.
#[derive(Debug, Clone, Copy)]
pub enum State {
	/// The session has no connection yet.
	Initial,
	/// The session can send and receive payloads.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The last time a packet was received from the client.
		last_seen: Instant,
	},
	/// The session is ending the connection.
	Disconnecting {
		/// The connection ID being disconnected.
		id: u64,
	},
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;
	use crate::{
		KEEPALIVE_INTERVAL,
		packets::{Acknowledgement, ChallengePacket, DataPacket, KeepAlivePacket, MessagePacket},
	};

	fn keep_alive(connection_id: u64) -> Packets {
		KeepAlivePacket::new(connection_id, Acknowledgement::default()).into()
	}

	fn data(connection_id: u64) -> Packets {
		DataPacket::new(connection_id, Acknowledgement::default(), Payload::filled(0)).into()
	}

	#[test]
	fn an_accepted_connection_is_confirmed_at_once_then_kept_alive_once_per_interval() {
		let start = Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		assert_eq!(session.update(&[], start), Ok(vec![keep_alive(7)]));
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL / 2), Ok(Vec::new()));
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL), Ok(vec![keep_alive(7)]));
	}

	#[test]
	fn a_payload_queued_at_acceptance_confirms_the_connection_without_a_keepalive() {
		let start = Instant::now();
		let mut session = Session::new();
		session.accept(7, start);
		session.send(false, Payload::filled(1));

		assert!(matches!(session.update(&[], start).unwrap()[..], [Packets::Data(_)]));
	}

	#[test]
	fn a_clients_message_is_delivered_and_acknowledged() {
		let start = Instant::now();
		let mut session = Session::new();
		session.accept(7, start);
		session.update(&[], start).unwrap();
		let message = MessagePacket::new(7, 0, Acknowledgement::default(), Payload::filled(3)).into();

		let sent = session.update(&[message], start).unwrap();

		assert_eq!(session.drain_received().collect::<Vec<_>>(), [Payload::filled(3)]);
		assert_eq!(sent, [KeepAlivePacket::new(7, Acknowledgement::new(0, 1)).into()]);
	}

	#[test]
	fn wrong_disconnect_is_ignored_and_matching_disconnect_transitions() {
		let now = Instant::now();
		let mut session = Session::new();
		session.accept(7, now);

		assert_eq!(
			session.update(&[DisconnectPacket::new(8).into()], now),
			Ok(vec![keep_alive(7)])
		);
		assert!(session.is_connected());
		assert_eq!(session.update(&[DisconnectPacket::new(7).into()], now), Ok(Vec::new()));
		assert!(!session.is_connected());
		assert_eq!(session.update(&[], now), Ok(vec![DisconnectPacket::new(7).into()]));
	}

	#[test]
	fn timeout_boundary_disconnects_only_after_the_configured_duration() {
		let start = Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		session.update(&[], start + TIMEOUT).unwrap();
		assert!(session.is_connected());
		assert_eq!(session.update(&[], start + TIMEOUT + Duration::from_nanos(1)), Ok(Vec::new()));
		assert!(!session.is_connected());
	}

	#[test]
	fn unauthenticated_packets_do_not_refresh_connection_timeout() {
		let start = Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		let packets = [
			data(8),
			DisconnectPacket::new(8).into(),
			keep_alive(8),
			ChallengePacket::new(1, 2).into(),
		];
		session.update(&packets, start + Duration::from_secs(4)).unwrap();
		session.update(&[], start + Duration::from_secs(6)).unwrap();

		assert!(!session.is_connected());
		assert_eq!(session.drain_received().count(), 0);
	}

	#[test]
	fn matching_data_and_keepalives_refresh_connection_timeout() {
		let start = Instant::now();
		for packet in [data(7), keep_alive(7)] {
			let mut session = Session::new();
			session.accept(7, start);

			session.update(&[packet], start + Duration::from_secs(4)).unwrap();
			session.update(&[], start + Duration::from_secs(6)).unwrap();

			assert!(session.is_connected());
		}
	}
}

use std::time::Instant;

use crate::{
	TIMEOUT,
	packets::{DisconnectPacket, Packets, Payload},
	server::PacketHandlingResults,
	stream::Stream,
};
