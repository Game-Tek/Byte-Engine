//! Manages the protocol state for one BETP client connection.

/// The `Session` struct preserves the protocol state for one client connection.
pub struct Session {
	local: Local,
	remote: Remote,
	state: State,
}

impl Session {
	/// Creates an idle session in the [`State::Initial`] state.
	///
	/// Call [`Session::connect`] to start a connection.
	pub fn new() -> Self {
		Self {
			local: Local::new(),
			remote: Remote::new(),
			state: State::Initial,
		}
	}

	pub fn connect(&mut self, salt: u64) {
		if let State::Initial = self.state {
			self.state = State::InitiatingConnection { salt }
		}
	}

	/// Handles the packets received since the last call and returns the packets to send now.
	///
	/// Call it on every application tick, with or without packets, so the session can retry reliable payloads,
	/// send keepalives, and end a connection whose server has gone quiet for [`crate::TIMEOUT`].
	pub fn update(&mut self, packets: &[Packets], current_time: Instant) -> Result<Vec<Packets>, Errors> {
		match &mut self.state {
			State::Initial => Ok(Vec::new()),
			State::InitiatingConnection { salt } => {
				let salt = *salt;

				for packet in packets {
					if let Packets::Challenge(challenge_packet) = packet {
						if salt == challenge_packet.get_client_salt() {
							let connection_id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();

							let id = connection_id;

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
						packet_buffer: PacketBuffer::new(),
						last_seen: current_time,
						// The last challenge response counts as recent traffic, so no keepalive is due yet.
						last_sent: Some(current_time),
					};
					// The confirming packets may carry data or acknowledgements, so handle them as a connected session.
					return self.update(packets, current_time);
				}

				if current_time.duration_since(*since) > TIMEOUT {
					// The server may have accepted without any confirmation arriving, so tell it the attempt ended.
					self.state = State::Disconnecting { id };
					return Ok(Vec::new());
				}

				Ok(vec![ChallengeResponsePacket::new(id).into()])
			}
			State::Connected {
				id,
				packet_buffer,
				last_seen,
				last_sent,
			} => {
				let id = *id;
				for packet in packets {
					// Handshake packets carry no connection id, and a connected session has nothing left to do with them.
					let Some(packet_id) = packet.connection_id() else {
						continue;
					};
					if packet_id != id {
						return Err(Errors::BadConnectionId);
					}

					match packet {
						Packets::Data(data_packet) => {
							let status = data_packet.get_connection_status();
							// Receive ordering and peer acknowledgement are independent fields with independent state.
							self.remote.acknowledge_packet(status.sequence);
							self.local.acknowledge_packets(status.ack, status.ack_bitfield);
							packet_buffer.acknowledge_packets(status.ack, status.ack_bitfield);
						}
						Packets::Disconnect(_) => {
							self.state = State::Disconnecting { id };
							return Ok(Vec::new());
						}
						_ => {}
					}
					*last_seen = current_time;
				}

				if current_time.duration_since(*last_seen) > TIMEOUT {
					self.state = State::Disconnecting { id };
					return Ok(Vec::new());
				}

				let packets = packet_buffer
					.gather_unsent_packets_for_retry()
					.into_iter()
					.map(Packets::Data)
					.collect();
				Ok(with_keep_alive(packets, id, last_sent, current_time))
			}
			State::Disconnecting { id } => {
				let id = *id;

				Ok(vec![DisconnectPacket::new(id).into()])
			}
		}
	}

	/// Queues a data packet for transmission.
	///
	/// Reliable packets remain queued for retry. The session discards packets that
	/// are queued before it reaches [`State::Connected`].
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		match &mut self.state {
			State::Connected { id, packet_buffer, .. } => {
				let sequence_number = self.local.get_sequence_number();
				let ack = self.remote.get_ack();
				let ack_bitfield = self.remote.get_ack_bitfield();
				let packet = DataPacket::new(*id, ConnectionStatus::new(sequence_number, ack, ack_bitfield), data);
				packet_buffer.add(packet, *id, reliable);
			}
			_ => {
				println!("Discarding packet as connection is not yet established")
			}
		}
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

// Keep the packet buffer inline: this state is hot-path protocol storage, and boxing it would add an allocation to every connection.
#[allow(clippy::large_enum_variant)]
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
	/// The client can send and receive data packets.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The packet buffer that manages sent and acknowledged packets.
		packet_buffer: PacketBuffer<16, 1024>,
		/// The last time a packet was received from the server.
		last_seen: Instant,
		/// The last time the session sent a packet, which decides when a keepalive is due.
		last_sent: Option<Instant>,
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
		packets::{ChallengePacket, KeepAlivePacket},
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

	fn connected_session(connection_id: u64, start: Instant) -> Session {
		let mut session = challenged_session(connection_id, start);
		session.update(&[KeepAlivePacket::new(connection_id).into()], start).unwrap();
		assert!(session.is_connected());
		session
	}

	#[test]
	fn test_establish_connection() {
		let start = Instant::now();
		let mut session = Session::new();

		session.connect(0);

		let res = session.update(&[], start);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[ChallengePacket::new(0, 0).into()], start);

		assert_eq!(res, Ok(vec![ChallengeResponsePacket::new(0).into()]));
		assert!(!session.is_connected());

		let res = session.update(&[KeepAlivePacket::new(0).into()], start);

		assert_eq!(res, Ok(vec![]));
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
	fn wrong_disconnect_is_recoverable_and_matching_disconnect_transitions() {
		let start = Instant::now();
		let mut session = connected_session(7, start);

		assert_eq!(
			session.update(&[DisconnectPacket::new(8).into()], start),
			Err(Errors::BadConnectionId)
		);
		assert!(session.is_connected());
		assert_eq!(session.update(&[DisconnectPacket::new(7).into()], start), Ok(Vec::new()));
		assert!(!session.is_connected());
		assert_eq!(session.update(&[], start), Ok(vec![DisconnectPacket::new(7).into()]));
	}

	#[test]
	fn receive_sequence_cannot_acknowledge_an_unrelated_local_send() {
		let start = Instant::now();
		let connection_id = 7;
		let mut session = connected_session(connection_id, start);
		session.send(true, [1; 1024]);
		let first_send = session.update(&[], start).unwrap();

		assert_eq!(first_send.len(), 1);

		let unrelated_receive = Packets::Data(DataPacket::new(connection_id, ConnectionStatus::new(0, 400, 0), [2; 1024]));
		let retry = session.update(&[unrelated_receive], start).unwrap();

		assert_eq!(retry.len(), 1);

		let explicit_ack = Packets::Data(DataPacket::new(connection_id, ConnectionStatus::new(1, 0, 1), [3; 1024]));

		assert!(session.update(&[explicit_ack], start).unwrap().is_empty());
	}

	#[test]
	fn an_idle_connection_sends_a_keepalive_once_per_interval() {
		let start = Instant::now();
		let mut session = connected_session(7, start);

		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL / 2), Ok(Vec::new()));
		assert_eq!(
			session.update(&[], start + KEEPALIVE_INTERVAL),
			Ok(vec![KeepAlivePacket::new(7).into()])
		);
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL * 3 / 2), Ok(Vec::new()));
	}

	#[test]
	fn sending_data_postpones_the_keepalive() {
		let start = Instant::now();
		let mut session = connected_session(7, start);
		let sent = start + KEEPALIVE_INTERVAL / 2;
		session.send(false, [1; 1024]);

		assert!(matches!(session.update(&[], sent).unwrap()[..], [Packets::Data(_)]));
		assert_eq!(session.update(&[], start + KEEPALIVE_INTERVAL), Ok(Vec::new()));
		assert_eq!(
			session.update(&[], sent + KEEPALIVE_INTERVAL),
			Ok(vec![KeepAlivePacket::new(7).into()])
		);
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
	fn only_a_matching_keepalive_keeps_the_connection_alive() {
		let start = Instant::now();
		let mut session = connected_session(7, start);
		let heard = start + Duration::from_secs(4);

		assert_eq!(
			session.update(&[KeepAlivePacket::new(8).into()], heard),
			Err(Errors::BadConnectionId)
		);
		session.update(&[KeepAlivePacket::new(7).into()], heard).unwrap();
		session.update(&[], start + Duration::from_secs(6)).unwrap();

		assert!(session.is_connected());
	}

	#[test]
	fn the_challenge_response_is_resent_until_the_server_confirms_the_connection() {
		let start = Instant::now();
		let mut session = challenged_session(7, start);

		// Another connection's traffic does not confirm this one.
		assert_eq!(
			session.update(&[KeepAlivePacket::new(8).into()], start),
			Ok(vec![ChallengeResponsePacket::new(7).into()])
		);
		assert_eq!(session.update(&[], start), Ok(vec![ChallengeResponsePacket::new(7).into()]));
		assert!(!session.is_connected());

		session.update(&[KeepAlivePacket::new(7).into()], start).unwrap();

		assert!(session.is_connected());
	}

	#[test]
	fn confirming_data_also_acknowledges_the_clients_sends() {
		let start = Instant::now();
		let mut session = challenged_session(7, start);
		let confirmation = Packets::Data(DataPacket::new(7, ConnectionStatus::new(0, 0, 0), [1; 1024]));
		session.update(&[confirmation], start).unwrap();
		session.send(true, [2; 1024]);
		session.update(&[], start).unwrap();

		let acknowledgement = Packets::Data(DataPacket::new(7, ConnectionStatus::new(1, 0, 1), [3; 1024]));

		assert_eq!(session.update(&[acknowledgement], start), Ok(Vec::new()));
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
}

use std::time::Instant;

use crate::{
	TIMEOUT,
	client::Errors,
	local::Local,
	packet_buffer::PacketBuffer,
	packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packets},
	remote::Remote,
	with_keep_alive,
};
