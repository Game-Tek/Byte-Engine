/// The `Session` struct preserves the protocol state for one server-side connection.
#[derive(Debug, Clone, Copy)]
pub struct Session {
	local: Local,
	remote: Remote,
	state: State,
	timeout: std::time::Duration,
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
			timeout: std::time::Duration::from_secs(5),
		}
	}

	pub fn connect(&mut self, salt: u64) {
		if let State::Initial = self.state {
			self.state = State::InitiatingConnection { salt }
		}
	}

	/// Accepts a connection after its challenge response has been validated by
	/// the server transport.
	pub fn accept(&mut self, connection_id: u64, current_time: std::time::Instant) {
		self.state = State::Connected {
			id: connection_id,
			packet_buffer: PacketBuffer::new(),
			last_seen: current_time,
		};
	}

	pub fn update(
		&mut self,
		packets: &[Packets],
		current_time: std::time::Instant,
	) -> Result<Vec<Packets>, PacketHandlingResults> {
		match &mut self.state {
			State::Initial => Ok(Vec::new()),
			State::InitiatingConnection { salt } => {
				let salt = *salt;

				for packet in packets {
					if let Packets::Challenge(challenge_packet) = packet {
						if salt == challenge_packet.get_client_salt() {
							let connection_id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();

							let id = connection_id;

							self.state = State::Connecting { id };

							return Ok(vec![ChallengeResponsePacket::new(id).into()]);
						} else {
							return Err(PacketHandlingResults::UnhandleablePacket);
						}
					}
				}

				Ok(vec![ConnectionRequestPacket::new(salt).into()])
			}
			State::Connecting { id } => {
				let id = *id;

				self.state = State::Connected {
					id,
					packet_buffer: PacketBuffer::new(),
					last_seen: current_time,
				};

				Ok(Vec::new())
			}
			State::Connected {
				id,
				packet_buffer,
				last_seen,
			} => {
				let id = *id;
				let mut received_authenticated_data = false;

				for packet in packets {
					match packet {
						Packets::Data(data_packet) if id == data_packet.get_connection_id() => {
							let status = data_packet.get_connection_status();
							// Only packets for this session may mutate receive ordering, acknowledgement state, or liveness.
							self.remote.acknowledge_packet(status.sequence);
							self.local.acknowledge_packets(status.ack, status.ack_bitfield);
							packet_buffer.acknowledge_packets(status.ack, status.ack_bitfield);
							received_authenticated_data = true;
						}
						Packets::Disconnect(disconnect_packet) if id == disconnect_packet.get_connection_id() => {
							*last_seen = current_time;
							self.state = State::Disconnecting { id };
							return Ok(Vec::new());
						}
						_ => {}
					}
				}

				if received_authenticated_data {
					*last_seen = current_time;
				}

				if current_time.duration_since(*last_seen) > self.timeout {
					self.state = State::Disconnecting { id };
					return Ok(Vec::new());
				}

				Ok(packet_buffer
					.gather_unsent_packets_for_retry()
					.into_iter()
					.map(Packets::Data)
					.collect())
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

// Keep the packet buffer inline: this state is copied as protocol state, and boxing it would add an allocation to every connection.
#[allow(clippy::large_enum_variant)]
/// The `State` enum identifies the current phase of a server-side session.
#[derive(Debug, Clone, Copy)]
pub enum State {
	/// The session is idle and has not started a connection.
	Initial,
	/// The session is waiting for a challenge.
	InitiatingConnection {
		/// The client salt used to identify the connection attempt.
		salt: u64,
	},
	/// The session is waiting for connection confirmation.
	Connecting {
		/// The connection ID assigned to this session.
		id: u64,
	},
	/// The session can send and receive data packets.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The packet buffer that manages sent and acknowledged packets.
		packet_buffer: PacketBuffer<16, 1024>,
		/// The last time a packet was received from the client.
		last_seen: std::time::Instant,
	},
	/// The session is ending the connection.
	Disconnecting {
		/// The connection ID being disconnected.
		id: u64,
	},
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::packets::{ChallengePacket, DisconnectPacket};

	#[test]
	fn test_session_start() {
		let mut session = Session::new();

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(Vec::new()));
	}

	#[test]
	fn test_establish_connection() {
		let mut session = Session::new();

		session.connect(0);

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[ChallengePacket::new(0, 0).into()], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ChallengeResponsePacket::new(0).into()]));

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![]));
	}

	#[test]
	fn test_connect_with_unresponsive_server() {
		let mut session = Session::new();

		session.connect(0);

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[], std::time::Instant::now());

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));
	}

	#[test]
	fn initiating_connection_uses_xor_when_salts_have_overlapping_bits() {
		let now = std::time::Instant::now();
		let mut session = Session::new();
		session.connect(7);

		assert_eq!(
			session.update(&[ChallengePacket::new(7, 3).into()], now),
			Ok(vec![ChallengeResponsePacket::new(4).into()])
		);
	}

	#[test]
	fn wrong_disconnect_is_ignored_and_matching_disconnect_transitions() {
		let now = std::time::Instant::now();
		let mut session = Session::new();
		session.accept(7, now);

		assert_eq!(session.update(&[DisconnectPacket::new(8).into()], now), Ok(Vec::new()));
		assert!(session.is_connected());
		assert_eq!(session.update(&[DisconnectPacket::new(7).into()], now), Ok(Vec::new()));
		assert!(!session.is_connected());
		assert_eq!(session.update(&[], now), Ok(vec![DisconnectPacket::new(7).into()]));
	}

	#[test]
	fn timeout_boundary_disconnects_only_after_the_configured_duration() {
		let start = std::time::Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		assert_eq!(session.update(&[], start + std::time::Duration::from_secs(5)), Ok(Vec::new()));
		assert!(session.is_connected());
		assert_eq!(
			session.update(
				&[],
				start + std::time::Duration::from_secs(5) + std::time::Duration::from_nanos(1),
			),
			Ok(Vec::new())
		);
		assert!(!session.is_connected());
	}

	#[test]
	fn unauthenticated_packets_do_not_refresh_connection_timeout() {
		let start = std::time::Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		let wrong_data = Packets::Data(DataPacket::new(8, ConnectionStatus::new(u16::MAX, 0, 0), [0; 1024]));
		let wrong_disconnect = DisconnectPacket::new(8).into();
		let irrelevant_challenge = ChallengePacket::new(1, 2).into();
		session
			.update(
				&[wrong_data, wrong_disconnect, irrelevant_challenge],
				start + std::time::Duration::from_secs(4),
			)
			.unwrap();

		session.update(&[], start + std::time::Duration::from_secs(6)).unwrap();

		assert!(!session.is_connected());
	}

	#[test]
	fn matching_data_refreshes_connection_timeout() {
		let start = std::time::Instant::now();
		let mut session = Session::new();
		session.accept(7, start);

		let data = Packets::Data(DataPacket::new(7, ConnectionStatus::new(u16::MAX, 0, 0), [0; 1024]));
		session.update(&[data], start + std::time::Duration::from_secs(4)).unwrap();
		session.update(&[], start + std::time::Duration::from_secs(6)).unwrap();

		assert!(session.is_connected());
	}

	#[test]
	fn receive_sequence_cannot_acknowledge_an_unrelated_local_send() {
		let start = std::time::Instant::now();
		let mut session = Session::new();
		session.accept(7, start);
		session.send(true, [1; 1024]);
		let first_send = session.update(&[], start).unwrap();
		assert_eq!(first_send.len(), 1);

		let unrelated_receive = Packets::Data(DataPacket::new(7, ConnectionStatus::new(0, 400, 0), [2; 1024]));
		let retry = session.update(&[unrelated_receive], start).unwrap();
		assert_eq!(retry.len(), 1);

		let explicit_ack = Packets::Data(DataPacket::new(7, ConnectionStatus::new(1, 0, 1), [3; 1024]));
		assert!(session.update(&[explicit_ack], start).unwrap().is_empty());
	}
}

use crate::{
	local::Local,
	packet_buffer::PacketBuffer,
	packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packets},
	remote::Remote,
	server::PacketHandlingResults,
};
