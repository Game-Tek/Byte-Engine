//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use crate::{
	client::Errors,
	local::Local,
	packet_buffer::PacketBuffer,
	packets::{ChallengeResponsePacket, ConnectionRequestPacket, ConnectionStatus, DataPacket, DisconnectPacket, Packets},
	remote::Remote,
};

/// The client is the entity that connects to a server and participates in the game.
pub struct Session {
	local: Local,
	remote: Remote,
	state: State,
}

impl Session {
	/// Creates a client<->server session that manages the connection state.
	/// The session is initiated is the `Initial` state.
	/// Must call `connect` to establish a connection.
	pub fn new() -> Result<Self, ()> {
		Ok(Self {
			local: Local::new(),
			remote: Remote::new(),
			state: State::Initial,
		})
	}

	pub fn connect(&mut self, salt: u64) {
		match self.state {
			State::Initial => self.state = State::InitiatingConnection { salt },
			_ => {}
		}
	}

	pub fn update(&mut self, packets: &[Packets]) -> Result<Vec<Packets>, Errors> {
		match &mut self.state {
			State::Initial => Ok(Vec::new()),
			State::InitiatingConnection { salt } => {
				let salt = *salt;

				for packet in packets {
					match packet {
						Packets::Challenge(challenge_packet) => {
							if salt == challenge_packet.get_client_salt() {
								let connection_id = challenge_packet.get_client_salt() ^ challenge_packet.get_server_salt();

								let id = connection_id;

								self.state = State::Connecting { id };

								return Ok(vec![ChallengeResponsePacket::new(id).into()]);
							} else {
								return Err(Errors::BadSalt);
							}
						}
						_ => {}
					}
				}

				Ok(vec![ConnectionRequestPacket::new(salt).into()])
			}
			State::Connecting { id } => {
				let id = *id;

				self.state = State::Connected {
					id,
					packet_buffer: PacketBuffer::new(),
				};

				Ok(Vec::new())
			}
			State::Connected { id, packet_buffer } => {
				let id = *id;

				for packet in packets {
					match packet {
						Packets::Data(data_packet) => {
							if id == data_packet.get_connection_id() {
								// Validate connection ID
								self.remote.acknowledge_packet(data_packet.get_connection_status().sequence);
								packet_buffer.remove(data_packet.get_connection_status().sequence);
							} else {
								return Err(Errors::BadConnectionId);
							}
						}
						Packets::Disconnect(disconnect_packet) => {
							if id == disconnect_packet.get_connection_id() {
								// Validate connection ID
								self.state = State::Disconnecting { id };
								return Ok(Vec::new());
							} else {
								return Err(Errors::BadConnectionId);
							}
						}
						_ => {}
					}
				}

				Ok(packet_buffer
					.gather_unsent_packets_for_retry()
					.into_iter()
					.map(|p| Packets::Data(p))
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
			State::Connected { id, mut packet_buffer } => {
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
		match self.state {
			State::Connected { id, .. } => self.state = State::Disconnecting { id },
			_ => {}
		}
	}

	pub fn is_connected(&self) -> bool {
		match self.state {
			State::Connected { .. } => true,
			_ => false,
		}
	}
}

/// `State` represents the current state of the client session. It is a state machine that transitions through various stages of the connection lifecycle.
pub enum State {
	/// The initial state of the session before any connection attempts have been made. The client could idle in this state or attempt to initiate a connection.
	Initial,
	/// The state where the client has initiated a connection request to the server and is awaiting a challenge response.
	InitiatingConnection {
		/// A generated salt value used for the connection request.
		salt: u64,
	},
	/// The state where the client has sent a challenge response and is waiting for the server to confirm the connection.
	Connecting {
		/// The connection ID generated from the challenge response.
		id: u64,
	},
	/// The state where the client is fully connected to the server and can send and receive data packets.
	Connected {
		/// The established connection ID.
		id: u64,
		/// The packet buffer that manages sent and acknowledged packets.
		packet_buffer: PacketBuffer<16, 1024>,
	},
	/// The state where the client is in the process of disconnecting from the server.
	Disconnecting {
		/// The connection ID being disconnected.
		id: u64,
	},
}

#[cfg(test)]
mod tests {
	use crate::packets::ChallengePacket;

	use super::*;

	#[test]
	fn test_session_start() {
		let mut session = Session::new().expect("Failed to connect to server.");

		let res = session.update(&[]);

		assert_eq!(res, Ok(Vec::new()));
	}

	#[test]
	fn test_establish_connection() {
		let mut session = Session::new().expect("Failed to connect to server.");

		session.connect(0);

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[ChallengePacket::new(0, 0).into()]);

		assert_eq!(res, Ok(vec![ChallengeResponsePacket::new(0).into()]));

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![]));
	}

	#[test]
	fn test_connect_with_unresponsive_server() {
		let mut session = Session::new().expect("Failed to connect to server.");

		session.connect(0);

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));

		let res = session.update(&[]);

		assert_eq!(res, Ok(vec![ConnectionRequestPacket::new(0).into()]));
	}
}
