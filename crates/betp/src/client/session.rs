//! Manages the protocol state for one BETP client connection.

/// The `Session` struct preserves the protocol state for one client connection.
pub struct Session {
	state: State,
	stream: Stream,
}

impl Session {
	/// Creates an idle session in the [`State::Initial`] state.
	///
	/// Call [`Session::connect`] to start a connection.
	pub fn new() -> Self {
		Self {
			state: State::Initial,
			stream: Stream::new(),
		}
	}

	pub fn connect(&mut self, salt: u64) {
		if let State::Initial = self.state {
			self.state = State::InitiatingConnection { salt }
		}
	}

	/// Handles the packets received since the last call and returns the packets to send now.
	///
	/// Call it on every application tick, with or without packets, so the session can resend unacknowledged
	/// messages, send keepalives, and end a connection whose server has gone quiet for [`crate::TIMEOUT`]. Read the
	/// payloads it delivered with [`Self::drain_received`].
	pub fn update(&mut self, packets: &[Packets], current_time: Instant) -> Result<Vec<Packets>, Errors> {
		match &mut self.state {
			State::Initial => Ok(Vec::new()),
			State::InitiatingConnection { salt } => {
				let salt = *salt;

				for packet in packets {
					if let Packets::Challenge(challenge_packet) = packet {
						if salt == challenge_packet.get_client_salt() {
							let id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();
							self.state = State::Connecting { id, since: current_time };

							return Ok(vec![ChallengeResponsePacket::new(id).into()]);
						} else {
							return Err(Errors::BadSalt);
						}
					}
				}

				Ok(vec![ConnectionRequestPacket::new(salt).into()])
			}
			State::Connecting { id, since } => {
				let id = *id;

				// The server only sends on this connection once it has accepted a challenge response. Until then any
				// response may have been lost, so keep sending it.
				if packets.iter().any(|packet| packet.connection_id() == Some(id)) {
					self.state = State::Connected {
						id,
						last_seen: current_time,
					};
					// The confirming packets may carry payloads or acknowledgements, so handle them as a connected session.
					return self.update(packets, current_time);
				}

				if current_time.duration_since(*since) > TIMEOUT {
					// The server may have accepted without any confirmation arriving, so tell it the attempt ended.
					self.state = State::Disconnecting { id };
					return Ok(Vec::new());
				}

				Ok(vec![ChallengeResponsePacket::new(id).into()])
			}
			State::Connected { id, last_seen } => {
				let id = *id;
				for packet in packets {
					// Handshake packets carry no connection id, and a connected session has nothing left to do with them.
					let Some(packet_id) = packet.connection_id() else {
						continue;
					};
					if packet_id != id {
						return Err(Errors::BadConnectionId);
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

	/// Queues a payload for the server.
	///
	/// Reliable payloads are resent until the server acknowledges them and arrive in the order they were queued.
	/// Unreliable payloads are sent once. Payloads queued before the connection is established go out once it is.
	pub fn send(&mut self, reliable: bool, data: [u8; PAYLOAD_SIZE]) {
		self.stream.send(reliable, data);
	}

	/// Removes and returns the payloads the server sent, reliable ones in order.
	pub fn drain_received(&mut self) -> impl Iterator<Item = [u8; PAYLOAD_SIZE]> + '_ {
		self.stream.drain_received()
	}

	/// Starts a voluntary disconnect from the server.
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

	/// Returns whether the connection or the attempt to make it has ended, by either side or by a timeout.
	///
	/// Reconnect with a new session.
	pub fn has_ended(&self) -> bool {
		matches!(self.state, State::Disconnecting { .. })
	}
}

impl Default for Session {
	fn default() -> Self {
		Self::new()
	}
}

/// The `State` enum identifies the current phase of a client session.
pub enum State {
	/// The client is idle and can start a connection.
	Initial,
	/// The client is waiting for the server's challenge.
	InitiatingConnection {
		/// A generated salt value used for the connection request.
		salt: u64,
	},
	/// The client sent its challenge response and is waiting for confirmation.
	Connecting {
		/// The connection ID generated from the challenge response.
		id: u64,
		/// When the challenge arrived, which bounds how long the client waits for confirmation.
		since: Instant,
	},
	/// The client can send and receive payloads.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The last time a packet was received from the server.
		last_seen: Instant,
	},
	/// The client is ending the connection.
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

	/// Returns a session that answered the server's challenge for `connection_id` and awaits confirmation.
	fn challenged_session(connection_id: u64, start: Instant) -> Session {
		let client_salt = 5;
		let mut session = Session::new();
		session.connect(client_salt);
		session
			.update(
				&[ChallengePacket::new(client_salt, client_salt ^ connection_id).into()],
				start,
			)
			.unwrap();
		session
	}

	fn keep_alive(connection_id: u64) -> Packets {
		KeepAlivePacket::new(connection_id, Acknowledgement::default()).into()
	}

	fn connected_session(connection_id: u64, start: Instant) -> Session {
		let mut session = challenged_session(connection_id, start);
		session.update(&[keep_alive(connection_id)], start).unwrap();
		assert!(session.is_connected());
		session
	}

	#[test]
	fn test_establish_connection() {
		let start = Instant::now();
		let mut session = Session::new();

		session.connect(0);

		assert_eq!(session.update(&[], start), Ok(vec![ConnectionRequestPacket::new(0).into()]));
		assert_eq!(
			session.update(&[ChallengePacket::new(0, 0).into()], start),
			Ok(vec![ChallengeResponsePacket::new(0).into()])
		);
		assert!(!session.is_connected());

		// Confirmation answers with a keepalive, which tells the server the client is listening.
		assert_eq!(session.update(&[keep_alive(0)], start), Ok(vec![keep_alive(0)]));
		assert!(session.is_connected());
	}

	#[test]
	fn connection_identity_uses_xor_when_salts_have_overlapping_bits() {
		let start = Instant::now();
		let mut session = Session::new();
		session.connect(7);

		assert_eq!(
			session.update(&[ChallengePacket::new(7, 3).into()], start),
			Ok(vec![ChallengeResponsePacket::new(4).into()])
		);
	}

	#[test]
	fn the_challenge_response_is_resent_until_the_server_confirms_the_connection() {
		let start = Instant::now();
		let mut session = challenged_session(7, start);

		// Another connection's traffic does not confirm this one.
		assert_eq!(
			session.update(&[keep_alive(8)], start),
			Ok(vec![ChallengeResponsePacket::new(7).into()])
		);
		assert_eq!(session.update(&[], start), Ok(vec![ChallengeResponsePacket::new(7).into()]));
		assert!(!session.is_connected());

		session.update(&[keep_alive(7)], start).unwrap();

		assert!(session.is_connected());
	}

	#[test]
	fn a_confirming_message_is_delivered_and_payloads_queued_while_connecting_go_out() {
		let start = Instant::now();
		let mut session = challenged_session(7, start);
		session.send(true, [2; PAYLOAD_SIZE]);
		let confirmation = MessagePacket::new(7, 0, Acknowledgement::default(), [1; PAYLOAD_SIZE]).into();

		let sent = session.update(&[confirmation], start).unwrap();

		assert!(
			matches!(&sent[..], [Packets::Message(message)] if message.get_acknowledgement() == Acknowledgement::new(0, 1))
		);
		assert_eq!(session.drain_received().collect::<Vec<_>>(), [[1; PAYLOAD_SIZE]]);
	}

	#[test]
	fn an_unconfirmed_connection_ends_after_the_timeout() {
		let start = Instant::now();
		let mut session = challenged_session(7, start);

		session.update(&[], start + TIMEOUT).unwrap();
		assert!(!session.has_ended());
		session.update(&[], start + TIMEOUT + Duration::from_nanos(1)).unwrap();

		assert!(session.has_ended());
		assert!(!session.is_connected());
		assert_eq!(
			session.update(&[], start + TIMEOUT * 2),
			Ok(vec![DisconnectPacket::new(7).into()])
		);
	}

	#[test]
	fn wrong_disconnect_is_recoverable_and_matching_disconnect_transitions() {
		let start = Instant::now();
		let mut session = connected_session(7, start);

		assert_eq!(
			session.update(&[DisconnectPacket::new(8).into()], start),
			Err(Errors::BadConnectionId)
		);
		assert!(session.is_connected());
		assert_eq!(session.update(&[DisconnectPacket::new(7).into()], start), Ok(Vec::new()));
		assert!(session.has_ended());
		assert_eq!(session.update(&[], start), Ok(vec![DisconnectPacket::new(7).into()]));
	}

	#[test]
	fn an_idle_connection_sends_a_keepalive_once_per_interval() {
		let start = Instant::now();
		let mut session = connected_session(7, start);

		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL / 2), Ok(Vec::new()));
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL), Ok(vec![keep_alive(7)]));
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL * 3 / 2), Ok(Vec::new()));
	}

	#[test]
	fn a_silent_server_ends_the_connection_after_the_timeout() {
		let start = Instant::now();
		let mut session = connected_session(7, start);

		session.update(&[], start + TIMEOUT).unwrap();
		assert!(session.is_connected());
		assert_eq!(session.update(&[], start + TIMEOUT + Duration::from_nanos(1)), Ok(Vec::new()));
		assert!(!session.is_connected());
	}

	#[test]
	fn only_matching_traffic_keeps_the_connection_alive() {
		let start = Instant::now();
		let mut session = connected_session(7, start);
		let heard = start + Duration::from_secs(4);
		let data = DataPacket::new(8, Acknowledgement::default(), [0; PAYLOAD_SIZE]).into();

		assert_eq!(session.update(&[data], heard), Err(Errors::BadConnectionId));
		assert_eq!(session.update(&[keep_alive(8)], heard), Err(Errors::BadConnectionId));
		session.update(&[keep_alive(7)], heard).unwrap();
		session.update(&[], start + Duration::from_secs(6)).unwrap();

		assert!(session.is_connected());
	}
}

use std::time::Instant;

use crate::{
	TIMEOUT,
	client::Errors,
	packets::{ChallengeResponsePacket, ConnectionRequestPacket, DisconnectPacket, PAYLOAD_SIZE, Packets},
	stream::Stream,
};
