#![no_main]

use std::time::Instant;

use betp::packets::{Acknowledgement, DataPacket, MAX_PAYLOAD_SIZE, MessagePacket, Packets, Payload};
use byte_engine::network::datagram::{
	ClientDatagramPipeline, DatagramOutcome, EncodedDatagram, MAX_BETP_DATAGRAM_SIZE, ServerDatagramPipeline,
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
	// The server's first keepalive confirms the connection to the client.
	server.advance(now, &mut server_output).unwrap();
	assert_eq!(
		client.process_datagram(server_output[0].as_bytes(), now, &mut client_output),
		Ok(DatagramOutcome::Handled)
	);
	assert!(client.is_connected());

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
	// Small sequences land inside the receive window, larger ones fall outside it.
	let sequence = body
		.get(..2)
		.and_then(|bytes| bytes.try_into().ok())
		.map(u16::from_le_bytes)
		.unwrap_or_default()
		% 64;
	// The rest of the input becomes the payload, so its length varies like real traffic.
	let payload = Payload::new(&body[..body.len().min(MAX_PAYLOAD_SIZE)]).ok()?;

	EncodedDatagram::encode(
		MessagePacket::new(connection_id, sequence, Acknowledgement::new(sequence, u32::MAX), payload).into(),
	)
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

/// Checks that an application payload can only originate from a matching-session payload packet.
fn assert_delivery_is_authenticated(candidate: &[u8], delivered: &[Payload]) {
	let Some(payload) = delivered.first() else {
		return;
	};

	let packet = betp::read_packet(candidate).expect("delivered data must have decoded");
	let data = match &packet {
		Packets::Data(packet) => packet.get_data(),
		Packets::Message(packet) => packet.get_data(),
		_ => panic!("a packet without a payload became application-visible"),
	};

	assert_eq!(packet.connection_id(), Some(CONNECTION_ID));
	assert_eq!(delivered, [*data]);
	assert_eq!(payload, data);
}

/// A delivered reliable message must not be delivered again when replayed immediately.
fn assert_duplicate_message_is_suppressed(
	client: &mut ClientDatagramPipeline,
	server: &mut ServerDatagramPipeline,
	candidate: &[u8],
	now: Instant,
	outbound: &mut Vec<EncodedDatagram>,
) {
	if !matches!(betp::read_packet(candidate), Ok(Packets::Message(_))) {
		return;
	}

	let _ = client.process_datagram(candidate, now, outbound);
	let _ = server.process_datagram(candidate, now, outbound);
	assert_eq!(client.drain_received().count(), 0);
	assert_eq!(server.drain_received().count(), 0);
}

fuzz_target!(|data: &[u8]| {
	let now = Instant::now();
	let structured = candidate_datagram(data);
	let candidate = structured.as_ref().map_or(data, EncodedDatagram::as_bytes);
	let mut outbound = Vec::new();

	// No raw packet may bypass the handshake boundary on either endpoint.
	let mut fresh_client = ClientDatagramPipeline::new(CLIENT_SALT);
	let mut fresh_server = ServerDatagramPipeline::new(SERVER_SALT);
	let _ = fresh_client.process_datagram(candidate, now, &mut outbound);
	let _ = fresh_server.process_datagram(candidate, now, &mut outbound);

	assert_eq!(fresh_client.drain_received().count(), 0);
	assert_eq!(fresh_server.drain_received().count(), 0);

	let (mut client, mut server) = connected_pair(now);
	client.send(false, Payload::new(&[0xC1]).unwrap());
	server.send(false, Payload::new(&[0x5E]).unwrap());

	client
		.process_datagram(candidate, now, &mut outbound)
		.expect("typed client output must always encode");
	assert_canonical(&outbound);
	assert_delivery_is_authenticated(candidate, &client.drain_received().collect::<Vec<_>>());

	server
		.process_datagram(candidate, now, &mut outbound)
		.expect("typed server output must always encode");
	assert_canonical(&outbound);
	assert_delivery_is_authenticated(candidate, &server.drain_received().collect::<Vec<_>>());

	assert_duplicate_message_is_suppressed(&mut client, &mut server, candidate, now, &mut outbound);

	// Any still-connected endpoint must deliver later valid traffic after arbitrary peer input.
	let recovery_payload = Payload::new(&[0xA5; 16]).unwrap();
	let recovery = EncodedDatagram::encode(DataPacket::new(CONNECTION_ID, Acknowledgement::default(), recovery_payload).into())
		.expect("typed recovery data must encode");

	if client.is_connected() {
		client.process_datagram(recovery.as_bytes(), now, &mut outbound).unwrap();
		assert_canonical(&outbound);
		assert_eq!(client.drain_received().collect::<Vec<_>>(), [recovery_payload]);
	}
	if server.is_connected() {
		server.process_datagram(recovery.as_bytes(), now, &mut outbound).unwrap();
		assert_canonical(&outbound);
		assert_eq!(server.drain_received().collect::<Vec<_>>(), [recovery_payload]);
	}
});
