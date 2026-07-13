#![no_main]

use std::time::Instant;

use betp::packets::{ConnectionStatus, DataPacket, Packets};
use byte_engine::network::datagram::{
	ClientDatagramPipeline, DatagramOutcome, EncodedDatagram, ServerDatagramPipeline, MAX_BETP_DATAGRAM_SIZE,
};
use libfuzzer_sys::fuzz_target;

const CLIENT_SALT: u64 = 0x1122_3344_5566_7788;
const SERVER_SALT: u64 = 0x8877_6655_4433_2211;
const CONNECTION_ID: u64 = CLIENT_SALT ^ SERVER_SALT;

/// Establishes both endpoints through the same raw framing boundary used for hostile input.
fn connected_pair(now: Instant) -> (ClientDatagramPipeline, ServerDatagramPipeline) {
	let mut client = ClientDatagramPipeline::new(CLIENT_SALT);
	let mut server = ServerDatagramPipeline::new(SERVER_SALT);
	let mut client_output = Vec::new();
	let mut server_output = Vec::new();

	assert_eq!(client.advance(now, &mut client_output), Ok(DatagramOutcome::Handled));
	assert_eq!(
		server.process_datagram(client_output[0].as_bytes(), now, &mut server_output),
		Ok(DatagramOutcome::Handled)
	);
	assert_eq!(
		client.process_datagram(server_output[0].as_bytes(), now, &mut client_output),
		Ok(DatagramOutcome::Handled)
	);
	assert_eq!(
		server.process_datagram(client_output[0].as_bytes(), now, &mut server_output),
		Ok(DatagramOutcome::Connected { id: CONNECTION_ID })
	);
	assert_eq!(
		client.advance(now, &mut client_output),
		Ok(DatagramOutcome::Connected { id: CONNECTION_ID })
	);

	assert_canonical(&client_output);
	assert_canonical(&server_output);
	(client, server)
}

/// Produces correlated session data often enough to exercise authenticated delivery while retaining a raw-input branch.
fn candidate_datagram(data: &[u8]) -> Option<EncodedDatagram> {
	let (&mode, body) = data.split_first()?;
	if mode % 3 == 0 {
		return None;
	}

	let connection_id = if mode % 3 == 1 { CONNECTION_ID } else { !CONNECTION_ID };
	let sequence = body
		.get(..2)
		.and_then(|bytes| bytes.try_into().ok())
		.map(u16::from_le_bytes)
		.unwrap_or_default()
		% 1024;
	let mut payload = [0; 1024];
	let copied = body.len().min(payload.len());
	payload[..copied].copy_from_slice(&body[..copied]);

	EncodedDatagram::encode(Packets::Data(DataPacket::new(
		connection_id,
		ConnectionStatus::new(sequence, sequence, u32::MAX),
		payload,
	)))
	.ok()
}

/// Confirms every emitted packet uses its exact canonical wire representation.
fn assert_canonical(datagrams: &[EncodedDatagram]) {
	for datagram in datagrams {
		assert!(!datagram.is_empty());
		assert!(datagram.len() <= MAX_BETP_DATAGRAM_SIZE);
		let packet = betp::read_packet(datagram.as_bytes()).expect("pipeline output must be canonical BETP");
		let mut reencoded = [0; MAX_BETP_DATAGRAM_SIZE];
		assert_eq!(betp::write_packet(&mut reencoded[..datagram.len()], packet), Some(()));
		assert_eq!(&reencoded[..datagram.len()], datagram.as_bytes());
	}
}

/// Checks that an application payload can only originate from matching-session data.
fn assert_accepted_is_authenticated(candidate: &[u8], outcome: DatagramOutcome) {
	let DatagramOutcome::Accepted(payload) = outcome else {
		return;
	};

	let Packets::Data(packet) = betp::read_packet(candidate).expect("accepted data must have decoded") else {
		panic!("a non-data packet became application-visible");
	};
	assert_eq!(packet.get_connection_id(), CONNECTION_ID);
	assert_eq!(payload, packet.data);
}

/// A newly accepted packet must become a no-delivery outcome when replayed immediately.
fn assert_duplicate_is_suppressed(
	client: &mut ClientDatagramPipeline,
	server: &mut ServerDatagramPipeline,
	candidate: &[u8],
	now: Instant,
	client_outcome: DatagramOutcome,
	server_outcome: DatagramOutcome,
	outbound: &mut Vec<EncodedDatagram>,
) {
	if matches!(client_outcome, DatagramOutcome::Accepted(_)) {
		assert!(!matches!(
			client.process_datagram(candidate, now, outbound),
			Ok(DatagramOutcome::Accepted(_))
		));
	}
	if matches!(server_outcome, DatagramOutcome::Accepted(_)) {
		assert!(!matches!(
			server.process_datagram(candidate, now, outbound),
			Ok(DatagramOutcome::Accepted(_))
		));
	}
}

fuzz_target!(|data: &[u8]| {
	let now = Instant::now();
	let structured = candidate_datagram(data);
	let candidate = structured.as_ref().map_or(data, EncodedDatagram::as_bytes);
	let mut outbound = Vec::new();

	// No raw packet may bypass the handshake boundary on either endpoint.
	let mut fresh_client = ClientDatagramPipeline::new(CLIENT_SALT);
	let mut fresh_server = ServerDatagramPipeline::new(SERVER_SALT);
	assert!(!matches!(
		fresh_client.process_datagram(candidate, now, &mut outbound),
		Ok(DatagramOutcome::Accepted(_))
	));
	assert!(!matches!(
		fresh_server.process_datagram(candidate, now, &mut outbound),
		Ok(DatagramOutcome::Accepted(_))
	));

	let (mut client, mut server) = connected_pair(now);
	client.send(false, [0xC1; 1024]);
	server.send(false, [0x5E; 1024]);

	let client_outcome = client
		.process_datagram(candidate, now, &mut outbound)
		.expect("typed client output must always encode");
	assert_canonical(&outbound);
	assert_accepted_is_authenticated(candidate, client_outcome);

	let server_outcome = server
		.process_datagram(candidate, now, &mut outbound)
		.expect("typed server output must always encode");
	assert_canonical(&outbound);
	assert_accepted_is_authenticated(candidate, server_outcome);

	assert_duplicate_is_suppressed(
		&mut client,
		&mut server,
		candidate,
		now,
		client_outcome,
		server_outcome,
		&mut outbound,
	);

	// Any still-connected endpoint must accept later valid traffic after arbitrary peer input.
	let candidate_was_accepted =
		matches!(client_outcome, DatagramOutcome::Accepted(_)) || matches!(server_outcome, DatagramOutcome::Accepted(_));
	let recovery_sequence = match betp::read_packet(candidate) {
		Ok(Packets::Data(packet)) if candidate_was_accepted => packet.get_connection_status().sequence.wrapping_add(1),
		_ => 1,
	};
	let recovery = EncodedDatagram::encode(Packets::Data(DataPacket::new(
		CONNECTION_ID,
		ConnectionStatus::new(recovery_sequence, 0, 0),
		[0xA5; 1024],
	)))
	.expect("typed recovery data must encode");

	if client.is_connected() {
		assert_eq!(
			client.process_datagram(recovery.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted([0xA5; 1024]))
		);
		assert_canonical(&outbound);
	}
	if server.is_connected() {
		assert_eq!(
			server.process_datagram(recovery.as_bytes(), now, &mut outbound),
			Ok(DatagramOutcome::Accepted([0xA5; 1024]))
		);
		assert_canonical(&outbound);
	}
});
