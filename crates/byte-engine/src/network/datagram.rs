//! Pure BETP datagram processing for engine-owned transports.
//!
//! Socket and channel adapters can pass raw datagrams through these pipelines
//! before exposing application payloads. BETP remains the sans-I/O protocol
//! layer; this module owns framing, endpoint routing, and delivery policy.

use std::time::Instant;

use betp::{
	client::{Errors as ClientSessionError, Session as ClientSession},
	packets::{ChallengePacket, Packets},
	server::{PacketHandlingResults as ServerSessionError, Session as ServerSession},
	PacketReadError, Remote,
};

/// The largest canonical BETP wire packet, including its header.
pub const MAX_BETP_DATAGRAM_SIZE: usize = 1045;

const CONNECTION_DATAGRAM_SIZE: usize = 13;
const CHALLENGE_DATAGRAM_SIZE: usize = 21;

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
		let len = packet_wire_size(&packet);
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatagramOutcome {
	/// A payload passed framing, endpoint, session, and receive-window validation.
	Accepted([u8; 1024]),
	/// A handshake established a session with this connection identifier.
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
	receive_window: Remote,
}

impl ClientDatagramPipeline {
	/// Creates a client pipeline and begins a connection using the caller-owned salt.
	pub fn new(client_salt: u64) -> Self {
		let mut session = ClientSession::new();
		session.connect(client_salt);

		Self {
			session,
			connection_id: None,
			receive_window: Remote::new(),
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
		_current_time: Instant,
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
					.update(std::slice::from_ref(&Packets::Challenge(challenge)))
					.map_err(DatagramDrop::ClientSession);
				let packets = match packets {
					Ok(packets) => packets,
					Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
				};
				self.connection_id = Some(id);
				encode_packets(packets, outbound)?;
				Ok(DatagramOutcome::Handled)
			}
			Packets::Data(data) => self.process_data(data, outbound),
			Packets::Disconnect(disconnect) => {
				let Some(id) = self.connection_id() else {
					return Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished));
				};
				if disconnect.get_connection_id() != id {
					return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
				}

				let packets = self
					.session
					.update(std::slice::from_ref(&Packets::Disconnect(disconnect)))
					.map_err(DatagramDrop::ClientSession);
				let packets = match packets {
					Ok(packets) => packets,
					Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
				};
				encode_packets(packets, outbound)?;
				Ok(DatagramOutcome::Disconnected { id })
			}
			_ => Ok(DatagramOutcome::Dropped(DatagramDrop::UnexpectedPacket)),
		}
	}

	/// Advances handshake and retry work when no datagram was received.
	pub fn advance(
		&mut self,
		_current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		outbound.clear();
		let was_connected = self.session.is_connected();
		let packets = match self.session.update(&[]) {
			Ok(packets) => packets,
			Err(error) => return Ok(DatagramOutcome::Dropped(DatagramDrop::ClientSession(error))),
		};
		encode_packets(packets, outbound)?;

		if !was_connected && self.session.is_connected() {
			if let Some(id) = self.connection_id {
				return Ok(DatagramOutcome::Connected { id });
			}
		}

		Ok(DatagramOutcome::Handled)
	}

	/// Queues one application payload for reliable or unreliable delivery.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		self.session.send(reliable, data);
	}

	/// Validates a data packet against endpoint and receive-window state while
	/// still forwarding its acknowledgement fields to the BETP session.
	fn process_data(
		&mut self,
		data: betp::packets::DataPacket<1024>,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		let Some(id) = self.connection_id() else {
			return Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished));
		};
		if data.get_connection_id() != id {
			return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
		}

		let payload = data.data;
		let sequence = data.get_connection_status().sequence;
		let receive_result = self.receive_window.acknowledge_packet(sequence);
		let packets = match self.session.update(std::slice::from_ref(&Packets::Data(data))) {
			Ok(packets) => packets,
			Err(error) => return Ok(DatagramOutcome::Dropped(DatagramDrop::ClientSession(error))),
		};
		encode_packets(packets, outbound)?;

		classify_delivery(receive_result, payload)
	}
}

/// The `ServerDatagramPipeline` struct gives one engine-owned peer route an
/// isolated handshake, session, and application delivery boundary.
pub struct ServerDatagramPipeline {
	session: ServerSession,
	server_salt: u64,
	pending_connection_id: Option<u64>,
	receive_window: Remote,
}

impl ServerDatagramPipeline {
	/// Creates a server pipeline that challenges peers with the supplied salt.
	pub fn new(server_salt: u64) -> Self {
		Self {
			session: ServerSession::new(),
			server_salt,
			pending_connection_id: None,
			receive_window: Remote::new(),
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
			Packets::Data(data) => self.process_data(data, current_time, outbound),
			Packets::Disconnect(disconnect) => {
				let Some(id) = self.session.connection_id() else {
					return Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished));
				};
				if disconnect.get_connection_id() != id {
					return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
				}

				let packets = self
					.session
					.update(std::slice::from_ref(&Packets::Disconnect(disconnect)), current_time)
					.map_err(DatagramDrop::ServerSession);
				let packets = match packets {
					Ok(packets) => packets,
					Err(reason) => return Ok(DatagramOutcome::Dropped(reason)),
				};
				encode_packets(packets, outbound)?;
				Ok(DatagramOutcome::Disconnected { id })
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

		if let Some(id) = connection_id {
			if !self.session.is_connected() {
				return Ok(DatagramOutcome::Disconnected { id });
			}
		}

		Ok(DatagramOutcome::Handled)
	}

	/// Queues one application payload for reliable or unreliable delivery.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		self.session.send(reliable, data);
	}

	/// Validates a data packet against endpoint and receive-window state while
	/// still forwarding its acknowledgement fields to the BETP session.
	fn process_data(
		&mut self,
		data: betp::packets::DataPacket<1024>,
		current_time: Instant,
		outbound: &mut Vec<EncodedDatagram>,
	) -> Result<DatagramOutcome, PipelineError> {
		let Some(id) = self.session.connection_id() else {
			return Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished));
		};
		if data.get_connection_id() != id {
			return Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch));
		}

		let payload = data.data;
		let sequence = data.get_connection_status().sequence;
		let receive_result = self.receive_window.acknowledge_packet(sequence);
		let packets = match self.session.update(std::slice::from_ref(&Packets::Data(data)), current_time) {
			Ok(packets) => packets,
			Err(error) => return Ok(DatagramOutcome::Dropped(DatagramDrop::ServerSession(error))),
		};
		encode_packets(packets, outbound)?;

		classify_delivery(receive_result, payload)
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

/// Maps receive-window state to the only application-visible payload outcome.
fn classify_delivery(receive_result: bool, payload: [u8; 1024]) -> Result<DatagramOutcome, PipelineError> {
	if receive_result {
		Ok(DatagramOutcome::Accepted(payload))
	} else {
		Ok(DatagramOutcome::Handled)
	}
}

fn packet_wire_size(packet: &Packets) -> usize {
	match packet {
		Packets::ConnectionRequest(_) | Packets::ChallengeResponse(_) | Packets::Disconnect(_) => CONNECTION_DATAGRAM_SIZE,
		Packets::Challenge(_) => CHALLENGE_DATAGRAM_SIZE,
		Packets::Data(_) => MAX_BETP_DATAGRAM_SIZE,
	}
}

#[cfg(test)]
mod tests {
	use betp::packets::{ConnectionStatus, DataPacket};

	use super::*;

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
		assert_eq!(
			client.advance(now, &mut client_output),
			Ok(DatagramOutcome::Connected { id: CONNECTION_ID })
		);

		(client, server)
	}

	#[test]
	fn handshake_uses_exact_canonical_datagram_lengths() {
		let now = Instant::now();
		let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut client_output = Vec::new();
		let mut server_output = Vec::new();

		client.advance(now, &mut client_output).unwrap();
		assert_eq!(client_output[0].len(), CONNECTION_DATAGRAM_SIZE);
		server
			.process_datagram(client_output[0].as_bytes(), now, &mut server_output)
			.unwrap();
		assert_eq!(server_output[0].len(), CHALLENGE_DATAGRAM_SIZE);
		client
			.process_datagram(server_output[0].as_bytes(), now, &mut client_output)
			.unwrap();
		assert_eq!(client_output[0].len(), CONNECTION_DATAGRAM_SIZE);
	}

	#[test]
	fn pre_handshake_data_never_becomes_application_visible() {
		let now = Instant::now();
		let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
		let mut server = ServerDatagramPipeline::new(SERVER_SALT);
		let mut outbound = Vec::new();
		let datagram = wire(Packets::Data(DataPacket::new(
			CONNECTION_ID,
			ConnectionStatus::new(0, 0, 0),
			[7; 1024],
		)));

		assert_eq!(
			client.process_datagram(datagram.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished))
		);
		assert_eq!(
			server.process_datagram(datagram.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Dropped(DatagramDrop::SessionNotEstablished))
		);
		assert!(outbound.is_empty());
	}

	#[test]
	fn wrong_session_data_is_dropped_and_later_matching_data_is_accepted() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();
		let wrong = wire(Packets::Data(DataPacket::new(
			!CONNECTION_ID,
			ConnectionStatus::new(1, 0, 0),
			[1; 1024],
		)));
		let matching = wire(Packets::Data(DataPacket::new(
			CONNECTION_ID,
			ConnectionStatus::new(1, 0, 0),
			[2; 1024],
		)));

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
			Ok(DatagramOutcome::Accepted([2; 1024]))
		);
		assert_eq!(
			server.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted([2; 1024]))
		);
	}

	#[test]
	fn duplicate_matching_data_is_delivered_at_most_once() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();
		let matching = wire(Packets::Data(DataPacket::new(
			CONNECTION_ID,
			ConnectionStatus::new(9, 0, 0),
			[5; 1024],
		)));

		assert_eq!(
			client.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted([5; 1024]))
		);
		assert_eq!(
			client.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
		assert_eq!(
			server.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted([5; 1024]))
		);
		assert_eq!(
			server.process_datagram(matching.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
	}

	#[test]
	fn data_older_than_the_receive_window_is_not_delivered() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();
		let newest = wire(Packets::Data(DataPacket::new(
			CONNECTION_ID,
			ConnectionStatus::new(2_000, 0, 0),
			[6; 1024],
		)));
		let stale = wire(Packets::Data(DataPacket::new(
			CONNECTION_ID,
			ConnectionStatus::new(0, 0, 0),
			[7; 1024],
		)));

		assert!(matches!(
			client.process_datagram(newest.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted(_))
		));
		assert_eq!(
			client.process_datagram(stale.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
		assert!(matches!(
			server.process_datagram(newest.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted(_))
		));
		assert_eq!(
			server.process_datagram(stale.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Handled)
		);
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

	#[test]
	fn queued_data_is_encoded_at_the_exact_maximum_wire_size() {
		let now = Instant::now();
		let (mut client, mut server) = connected_pair(now);
		let mut outbound = Vec::new();

		client.send(false, [3; 1024]);
		client.advance(now, &mut outbound).unwrap();
		assert_eq!(outbound.len(), 1);
		assert_eq!(outbound[0].len(), MAX_BETP_DATAGRAM_SIZE);
		assert!(betp::read_packet(outbound[0].as_bytes()).is_ok());

		server.send(false, [4; 1024]);
		server.advance(now, &mut outbound).unwrap();
		assert_eq!(outbound.len(), 1);
		assert_eq!(outbound[0].len(), MAX_BETP_DATAGRAM_SIZE);
		assert!(betp::read_packet(outbound[0].as_bytes()).is_ok());
	}
}
