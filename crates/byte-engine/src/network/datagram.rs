//! Pure BETP datagram processing for engine-owned transports.
//!
//! Socket and channel adapters can pass raw datagrams through these pipelines
//! before exposing application payloads. BETP remains the sans-I/O protocol
//! layer and decides delivery; this module owns framing and endpoint routing.

use std::time::Instant;

use betp::{
	PacketReadError,
	client::{Errors as ClientSessionError, Session as ClientSession},
	packets::{ChallengePacket, Packets, Payload},
	server::{PacketHandlingResults as ServerSessionError, Session as ServerSession},
};

/// The largest canonical BETP wire packet, including its header.
pub const MAX_BETP_DATAGRAM_SIZE: usize = betp::MAX_PACKET_SIZE;

/// The `EncodedDatagram` struct provides transport adapters with one canonical
/// BETP packet and its exact wire length.
#[derive(Clone, PartialEq, Eq)]
pub struct EncodedDatagram {
	bytes: [u8; MAX_BETP_DATAGRAM_SIZE],
	len: usize,
}

impl EncodedDatagram {
	/// Encodes one typed packet without allocating storage for its bytes.
	pub fn encode(packet: Packets) -> Result<Self, PipelineError> {
		let len = packet.wire_size();
		let mut bytes = [0; MAX_BETP_DATAGRAM_SIZE];
		betp::write_packet(&mut bytes[..len], packet).ok_or(PipelineError::EncodingFailed)?;

		Ok(Self { bytes, len })
	}

	/// Returns the encoded packet without unused capacity.
	pub fn as_bytes(&self) -> &[u8] {
		&self.bytes[..self.len]
	}

	/// Returns the encoded packet length.
	pub fn len(&self) -> usize {
		self.len
	}

	/// Returns whether the encoded packet contains no bytes.
	pub fn is_empty(&self) -> bool {
		self.len == 0
	}
}

impl std::fmt::Debug for EncodedDatagram {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("EncodedDatagram")
			.field("len", &self.len)
			.finish_non_exhaustive()
	}
}

/// The `DatagramOutcome` enum gives transport adapters a bounded, typed result
/// for routing one peer datagram.
///
/// Payloads are not part of the outcome: one datagram can complete a run of reliable messages, so read them with
/// the pipeline's `drain_received` after each call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatagramOutcome {
	/// The server accepted a client's handshake and established a session with this connection identifier. A
	/// client learns of its own connection through [`ClientDatagramPipeline::is_connected`] instead.
	Connected {
		/// The established connection identifier.
		id: u64,
	},
	/// A validated disconnect or timeout ended this session.
	Disconnected {
		/// The ended connection identifier.
		id: u64,
	},
	/// The packet was valid protocol traffic but produced no application event.
	Handled,
	/// The packet was rejected before it could become application-visible.
	Dropped(DatagramDrop),
}

/// The `DatagramDrop` enum helps transport adapters measure and contain invalid
/// peer traffic without turning it into an application failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatagramDrop {
	/// The datagram exceeds every canonical BETP packet size.
	TooLarge {
		/// The received datagram length.
		actual: usize,
	},
	/// The datagram is not a canonical BETP packet.
	Decode(PacketReadError),
	/// The packet is not valid for this endpoint or lifecycle state.
	UnexpectedPacket,
	/// Session data arrived before a connection was established.
	SessionNotEstablished,
	/// The packet belongs to a different connection.
	ConnectionIdMismatch,
	/// The BETP client session rejected the packet.
	ClientSession(ClientSessionError),
	/// The BETP server session rejected the packet.
	ServerSession(ServerSessionError),
}

/// The `PipelineError` enum separates local encoding failures from peer-input
/// drops so adapters can fail explicitly at the responsible layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineError {
	/// A typed BETP packet could not fit its canonical wire representation.
	EncodingFailed,
}

impl std::fmt::Display for PipelineError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::EncodingFailed => write!(
				formatter,
				"BETP packet encoding failed. The most likely cause is an internal packet-size invariant violation."
			),
		}
	}
}

impl std::error::Error for PipelineError {}

/// The `ClientDatagramPipeline` struct isolates untrusted wire input from a
/// client application's accepted-payload boundary.
pub struct ClientDatagramPipeline {
	session: ClientSession,
	connection_id: Option<u64>,
}

impl ClientDatagramPipeline {
	/// Creates a client pipeline and begins a connection using the caller-owned salt.
	pub fn new(client_salt: u64) -> Self {
		let mut session = ClientSession::new();
		session.connect(client_salt);

		Self {
			session,
			connection_id: None,
		}
	}

	/// Returns whether the client handshake has established a session.
	pub fn is_connected(&self) -> bool {
		self.session.is_connected()
	}

	/// Returns the active connection identifier after the handshake completes.
	pub fn connection_id(&self) -> Option<u64> {
		self.connection_id.filter(|_| self.session.is_connected())
	}

	/// Routes one raw datagram through decoding and session validation before
	/// returning any application-visible data.
	pub fn process_datagram(
		&mut self,
		datagram: &[u8],
		current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		outbound.clear();
		let packet = match decode_datagram(datagram) {
			Ok(packet) => packet,
			Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
		};

		match packet {
			Packets::Challenge(challenge) if self.connection_id.is_none() => {
				let id = challenge.get_client_salt() ^ challenge.get_server_salt();
				let packets = self
					.session
					.update(std::slice::from_ref(&Packets::Challenge(challenge)), current_time)
					.map_err(DatagramDrop::ClientSession);
				let packets = match packets {
					Ok(packets) => packets,
					Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
				};
				self.connection_id = Some(id);
				encode_packets(packets, outbound)?;
				Ok(DatagramOutcome::Handled)
			}
			packet @ (Packets::Data(_) | Packets::Message(_) | Packets::KeepAlive(_) | Packets::Disconnect(_)) => {
				// The first packet the server sends on this connection is what confirms it, so it must reach the
				// session before `is_connected` turns true.
				let connection_id = self.connection_id.filter(|_| !self.session.has_ended());
				process_session_packet(packet, connection_id, outbound, |packets| {
					self.session
						.update(packets, current_time)
						.map_err(DatagramDrop::ClientSession)
				})
			}
			_ => Ok(DatagramOutcome::Dropped(DatagramDrop::UnexpectedPacket)),
		}
	}

	/// Advances handshake, retry, keepalive, and timeout work when no datagram was received.
	///
	/// Returns [`DatagramOutcome::Disconnected`] once the server goes quiet for [`betp::TIMEOUT`], or never confirms
	/// the connection within it. Check [`Self::is_connected`] to learn when the server has confirmed it.
	pub fn advance(
		&mut self,
		current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		outbound.clear();
		let had_ended = self.session.has_ended();
		let packets = match self.session.update(&[], current_time) {
			Ok(packets) => packets,
			Err(error) => return Ok(DatagramOutcome::Dropped(DatagramDrop::ClientSession(error))),
		};
		encode_packets(packets, outbound)?;

		match self.connection_id {
			Some(id) if !had_ended && self.session.has_ended() => Ok(DatagramOutcome::Disconnected { id }),
			_ => Ok(DatagramOutcome::Handled),
		}
	}

	/// Queues one application payload for reliable or unreliable delivery.
	pub fn send(&mut self, reliable: bool, data: Payload) {
		self.session.send(reliable, data);
	}

	/// Removes and returns the payloads delivered so far, reliable ones in the order they were sent.
	pub fn drain_received(&mut self) -> impl Iterator<Item = Payload> + '_ {
		self.session.drain_received()
	}
}

/// The `ServerDatagramPipeline` struct gives one engine-owned peer route an
/// isolated handshake, session, and application delivery boundary.
pub struct ServerDatagramPipeline {
	session: ServerSession,
	server_salt: u64,
	pending_connection_id: Option<u64>,
}

impl ServerDatagramPipeline {
	/// Creates a server pipeline that challenges peers with the supplied salt.
	pub fn new(server_salt: u64) -> Self {
		Self {
			session: ServerSession::new(),
			server_salt,
			pending_connection_id: None,
		}
	}

	/// Returns whether this peer route has established a session.
	pub fn is_connected(&self) -> bool {
		self.session.is_connected()
	}

	/// Returns the active connection identifier after the handshake completes.
	pub fn connection_id(&self) -> Option<u64> {
		self.session.connection_id()
	}

	/// Routes one raw peer datagram through server handshake or connected-session
	/// validation before returning any application-visible data.
	pub fn process_datagram(
		&mut self,
		datagram: &[u8],
		current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		outbound.clear();
		let packet = match decode_datagram(datagram) {
			Ok(packet) => packet,
			Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
		};

		match packet {
			Packets::ConnectionRequest(request) if !self.session.is_connected() => {
				let id = request.get_client_salt() ^ self.server_salt;
				self.pending_connection_id = Some(id);
				outbound.push(EncodedDatagram::encode(
					ChallengePacket::new(request.get_client_salt(), self.server_salt).into(),
				)?);
				Ok(DatagramOutcome::Handled)
			}
			Packets::ChallengeResponse(response) if !self.session.is_connected() => {
				let id = response.get_connection_id();
				if self.pending_connection_id != Some(id) {
					return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
				}

				self.session.accept(id, current_time);
				self.pending_connection_id = None;
				Ok(DatagramOutcome::Connected { id })
			}
			packet @ (Packets::Data(_) | Packets::Message(_) | Packets::KeepAlive(_) | Packets::Disconnect(_)) => {
				process_session_packet(packet, self.session.connection_id(), outbound, |packets| {
					self.session
						.update(packets, current_time)
						.map_err(DatagramDrop::ServerSession)
				})
			}
			_ => Ok(DatagramOutcome::Dropped(DatagramDrop::UnexpectedPacket)),
		}
	}

	/// Advances timeout and retry work when no datagram was received.
	pub fn advance(
		&mut self,
		current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		outbound.clear();
		let connection_id = self.session.connection_id();
		let packets = match self.session.update(&[], current_time) {
			Ok(packets) => packets,
			Err(error) => return Ok(DatagramOutcome::Dropped(DatagramDrop::ServerSession(error))),
		};
		encode_packets(packets, outbound)?;

		if let Some(id) = connection_id
			&& !self.session.is_connected()
		{
			return Ok(DatagramOutcome::Disconnected { id });
		}

		Ok(DatagramOutcome::Handled)
	}

	/// Queues one application payload for reliable or unreliable delivery.
	pub fn send(&mut self, reliable: bool, data: Payload) {
		self.session.send(reliable, data);
	}

	/// Removes and returns the payloads delivered so far, reliable ones in the order they were sent.
	pub fn drain_received(&mut self) -> impl Iterator<Item = Payload> + '_ {
		self.session.drain_received()
	}
}

impl Default for ServerDatagramPipeline {
	fn default() -> Self {
		Self::new(0x4254_4550)
	}
}

/// Rejects oversized input early, then delegates canonical framing to BETP.
fn decode_datagram(datagram: &[u8]) -> Result<Packets, DatagramDrop> {
	if datagram.len() > MAX_BETP_DATAGRAM_SIZE {
		return Err(DatagramDrop::TooLarge { actual: datagram.len() });
	}

	betp::read_packet(datagram).map_err(DatagramDrop::Decode)
}

/// Serializes every session output into reusable, exact-length datagrams.
fn encode_packets(packets: Vec<Packets>, outbound: &mut Vec<EncodedDatagram>) -> Result<(), PipelineError> {
	outbound.reserve(packets.len());
	for packet in packets {
		match EncodedDatagram::encode(packet) {
			Ok(datagram) => outbound.push(datagram),
			Err(error) => {
				outbound.clear();
				return Err(error);
			}
		}
	}
	Ok(())
}

/// Accepts a payload, keepalive, or disconnect packet only for the established connection, lets the session handle
/// it through `update`, and reports whether it ended the session.
fn process_session_packet(
	packet: Packets,
	connection_id: Option<u64>,
	outbound: &mut Vec<EncodedDatagram>,
	update: impl FnOnce(&[Packets]) -> Result<Vec<Packets>, DatagramDrop>,
) -> Result<DatagramOutcome, PipelineError> {
	let Some(id) = connection_id else {
		return Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished));
	};
	if packet.connection_id() != Some(id) {
		return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
	}

	let disconnect = matches!(packet, Packets::Disconnect(_));
	let packets = match update(std::slice::from_ref(&packet)) {
		Ok(packets) => packets,
		Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
	};
	encode_packets(packets, outbound)?;

	Ok(if disconnect {
		DatagramOutcome::Disconnected { id }
	} else {
		DatagramOutcome::Handled
	})
}

#[cfg(test)]
mod tests {
	use betp::packets::{Acknowledgement, DataPacket, MessagePacket};

	use super::*;
	use crate::network::payload;

	const CLIENT_SALT: u64 = 0x1122_3344_5566_7788;
	const SERVER_SALT: u64 = 0x8877_6655_4433_2211;
	const CONNECTION_ID: u64 = CLIENT_SALT ^ SERVER_SALT;

	fn wire(packet: Packets) -> EncodedDatagram {
		EncodedDatagram::encode(packet).expect("typed BETP packets have a canonical encoding")
	}

	/// Establishes both pipelines entirely through their raw datagram boundary.
	fn connected_pair(now: Instant) -> (ClientDatagramPipeline, ServerDatagramPipeline) {
		let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut client_output = Vec::new();
		let mut server_output = Vec::new();

		assert_eq!(client.advance(now, &mut client_output), Ok(DatagramOutcome::Handled));
		assert_eq!(client_output.len(), 1);
		assert_eq!(
			server.process_datagram(client_output[0].as_bytes(), now, &mut server_output),
			Ok(DatagramOutcome::Handled)
		);
		assert_eq!(server_output.len(), 1);
		assert_eq!(
			client.process_datagram(server_output[0].as_bytes(), now, &mut client_output),
			Ok(DatagramOutcome::Handled)
		);
		assert_eq!(client_output.len(), 1);
		assert_eq!(
			server.process_datagram(client_output[0].as_bytes(), now, &mut server_output),
			Ok(DatagramOutcome::Connected { id: CONNECTION_ID })
		);
		// The server's first keepalive confirms the connection to the client.
		server.advance(now, &mut server_output).unwrap();
		assert_eq!(
			client.process_datagram(server_output[0].as_bytes(), now, &mut client_output),
			Ok(DatagramOutcome::Handled)
		);
		assert!(client.is_connected());

		(client, server)
	}

	#[test]
	fn handshake_uses_exact_canonical_datagram_lengths() {
		let now = Instant::now();
		let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut client_output = Vec::new();
		let mut server_output = Vec::new();

		client.advance(now, &mut client_output).expect("expected test value");

		assert_eq!(client_output[0].len(), 13);
		server
			.process_datagram(client_output[0].as_bytes(), now, &mut server_output)
			.expect("expected test value");

		assert_eq!(server_output[0].len(), 21);
		client
			.process_datagram(server_output[0].as_bytes(), now, &mut client_output)
			.expect("expected test value");

		assert_eq!(client_output[0].len(), 13);
	}

	fn data(connection_id: u64, fill: u8) -> EncodedDatagram {
		wire(DataPacket::new(connection_id, Acknowledgement::default(), payload(fill)).into())
	}

	fn message(sequence: u16, fill: u8) -> EncodedDatagram {
		wire(MessagePacket::new(CONNECTION_ID, sequence, Acknowledgement::default(), payload(fill)).into())
	}

	#[test]
	fn payload_datagrams_are_as_long_as_their_payload() {
		let short = Payload::new(&[1, 2, 3]).unwrap();

		assert_eq!(
			wire(DataPacket::new(CONNECTION_ID, Acknowledgement::default(), short).into()).len(),
			19 + 3
		);
		assert_eq!(
			wire(MessagePacket::new(CONNECTION_ID, 0, Acknowledgement::default(), short).into()).len(),
			21 + 3
		);
	}

	#[test]
	fn pre_handshake_data_never_becomes_application_visible() {
		let now = Instant::now();
		let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut outbound = Vec::new();
		let datagram = data(CONNECTION_ID, 7);

		assert_eq!(
			client.process_datagram(datagram.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished))
		);
		assert_eq!(
			server.process_datagram(datagram.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished))
		);
		assert!(outbound.is_empty());
		assert_eq!(client.drain_received().count() + server.drain_received().count(), 0);
	}

	#[test]
	fn wrong_session_data_is_dropped_and_later_matching_data_is_delivered() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();
		let wrong = data(!CONNECTION_ID, 1);
		let matching = data(CONNECTION_ID, 2);

		assert_eq!(
			client.process_datagram(wrong.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch))
		);
		assert_eq!(
			server.process_datagram(wrong.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch))
		);
		assert_eq!(
			client.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
		assert_eq!(
			server.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
		assert_eq!(client.drain_received().collect::<Vec<_>>(), [payload(2)]);
		assert_eq!(server.drain_received().collect::<Vec<_>>(), [payload(2)]);
	}

	#[test]
	fn messages_are_delivered_once_and_in_sequence_order_across_datagrams() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();

		for datagram in [message(1, 1), message(1, 1), message(0, 0)] {
			client.process_datagram(datagram.as_bytes(), now, &mut outbound).unwrap();
			server.process_datagram(datagram.as_bytes(), now, &mut outbound).unwrap();
		}

		for received in [client.drain_received().collect::<Vec<_>>(), server.drain_received().collect()] {
			assert_eq!(received, [payload(0), payload(1)]);
		}
	}

	#[test]
	fn a_message_beyond_the_receive_window_is_not_delivered() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();
		let ahead = message(2_000, 6);

		client.process_datagram(ahead.as_bytes(), now, &mut outbound).unwrap();
		server.process_datagram(ahead.as_bytes(), now, &mut outbound).unwrap();

		assert_eq!(client.drain_received().count() + server.drain_received().count(), 0);
	}

	#[test]
	fn malformed_and_oversized_datagrams_have_typed_peer_local_outcomes() {
		let now = Instant::now();
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut outbound = vec![wire(betp::packets::ConnectionRequestPacket::new(CLIENT_SALT).into())];

		assert!(matches!(
			server.process_datagram(b"not BETP", now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::Decode(_)))
		));
		assert!(outbound.is_empty());
		assert_eq!(
			server.process_datagram(&[0; MAX_BETP_DATAGRAM_SIZE + 1], now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::TooLarge {
				actual: MAX_BETP_DATAGRAM_SIZE + 1,
			}))
		);
	}
}
