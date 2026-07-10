/// The Session holds the state for a connection to this server..
#[derive(Debug, Clone, Copy)]
pub struct Session {
	local: Local,
	remote: Remote,
	state: State,
	timeout: std::time::Duration,
}

impl Session {
	/// Creates a client<->server session that manages the connection state.
	/// The session is initiated is the `Initial` state.
	/// Must call `connect` to establish a connection.
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

				for packet in packets {
					match packet {
						Packets::Data(data_packet) => {
							if id == data_packet.get_connection_id() {
								// Validate connection ID
								self.remote.acknowledge_packet(data_packet.get_connection_status().sequence);
								packet_buffer.remove(data_packet.get_connection_status().sequence);
							} else {
								println!("This client received a data packet with an incorrect connection id");
							}
						}
						Packets::Disconnect(disconnect_packet) => {
							if id == disconnect_packet.get_connection_id() {
								// Validate connection ID
								self.state = State::Disconnecting { id };
								return Ok(Vec::new());
							} else {
								println!("This client received a disconnect packet with an incorrect connection id");
							}
						}
						_ => {}
					}
				}

				if !packets.is_empty() {
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

	/// Enqueuesdata packets to be sent.
	/// Messages can be flagged as realiable for them to be retried if sending them fails.
	/// Data packets sent whilw the session is not in the `Connected` state will be discarded.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		match self.state {
			State::Connected {
				id, mut packet_buffer, ..
			} => {
				let sequence_number = self.local.get_sequence_number();
				let ack = self.remote.get_ack();
				let ack_bitfield = self.remote.get_ack_bitfield();
				let packet = DataPacket::new(id, ConnectionStatus::new(sequence_number, ack, ack_bitfield), data);
				packet_buffer.add(packet, id, reliable);
			}
			_ => {
				println!("Discarding packet as connection is not yet established")
			}
		}
	}

	/// Returns a disconnect packet to send to the server.
	/// The client will no longer be able to handle server packets after this.
	/// The client will need to reconnect to the server to continue.
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
/// The different states a session can be in.
/// Used to manage the connection lifecycle.
#[derive(Debug, Clone, Copy)]
pub enum State {
	/// The initial state of the session.
	/// No connection has been initiated yet.
	Initial,
	/// The session is attempting to initiate a connection.
	InitiatingConnection {
		/// The client salt used to identify the connection attempt.
		salt: u64,
	},
	/// The session is in the process of connecting.
	Connecting {
		/// The connection ID assigned to this session.
		id: u64,
	},
	/// The session is fully connected.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The packet buffer that manages sent and acknowledged packets.
		packet_buffer: PacketBuffer<16, 1024>,
		/// The last time a packet was received from the client.
		last_seen: std::time::Instant,
	},
	/// The session is in the process of disconnecting.
	Disconnecting {
		/// The connection ID being disconnected.
		id: u64,
	},
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::packets::ChallengePacket;

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
}

use crate::{
	local::Local,
	packet_buffer::PacketBuffer,
	packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packets},
	remote::Remote,
	server::PacketHandlingResults,
};
