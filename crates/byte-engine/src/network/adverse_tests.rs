use std::time::{Duration, Instant};

use super::datagram::{ClientDatagramPipeline, DatagramDrop, DatagramOutcome, EncodedDatagram, ServerDatagramPipeline};

/// The `Pair` struct keeps one isolated client/server route together during deterministic network simulation.
struct Pair {
	client: ClientDatagramPipeline,
	server: ServerDatagramPipeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
	Client,
	Server,
}

#[derive(Clone, Copy)]
enum DeliveryPlan {
	Drop,
	Once { delay_steps: u8 },
	Duplicate { first_delay_steps: u8, second_delay_steps: u8 },
}

/// The `ScheduledDatagram` struct represents one bounded delivery chosen by the deterministic fault script.
struct ScheduledDatagram {
	destination: Destination,
	due_step: u8,
	order: u16,
	datagram: EncodedDatagram,
}

/// The `DeterministicLink` struct provides reproducible loss, duplication, delay, and reordering without socket I/O.
struct DeterministicLink {
	step: u8,
	next_order: u16,
	dropped: usize,
	queue: Vec<ScheduledDatagram>,
}

impl DeterministicLink {
	fn new() -> Self {
		Self {
			step: 0,
			next_order: 0,
			dropped: 0,
			queue: Vec::new(),
		}
	}

	/// Applies one explicit fault plan while retaining a deterministic tie-break order for same-step deliveries.
	fn schedule(&mut self, destination: Destination, datagram: EncodedDatagram, plan: DeliveryPlan) {
		match plan {
			DeliveryPlan::Drop => self.dropped += 1,
			DeliveryPlan::Once { delay_steps } => self.push(destination, datagram, delay_steps),
			DeliveryPlan::Duplicate {
				first_delay_steps,
				second_delay_steps,
			} => {
				self.push(destination, datagram.clone(), first_delay_steps);
				self.push(destination, datagram, second_delay_steps);
			}
		}
	}

	fn push(&mut self, destination: Destination, datagram: EncodedDatagram, delay_steps: u8) {
		self.queue.push(ScheduledDatagram {
			destination,
			due_step: self.step.saturating_add(delay_steps),
			order: self.next_order,
			datagram,
		});
		self.next_order = self.next_order.wrapping_add(1);
	}

	/// Advances one logical step and delivers every due datagram, newest first when delays converge.
	fn advance(
		&mut self,
		pair: &mut Pair,
		current_time: Instant,
		scratch: &mut Vec<EncodedDatagram>,
		outcomes: &mut Vec<(Destination, DatagramOutcome)>,
	) {
		self.step = self.step.saturating_add(1);

		loop {
			let next = self
				.queue
				.iter()
				.enumerate()
				.filter(|(_, scheduled)| scheduled.due_step <= self.step)
				.min_by_key(|(_, scheduled)| (scheduled.due_step, std::cmp::Reverse(scheduled.order)))
				.map(|(index, _)| index);
			let Some(index) = next else {
				break;
			};

			let scheduled = self.queue.remove(index);
			let outcome = match scheduled.destination {
				Destination::Client => pair
					.client
					.process_datagram(scheduled.datagram.as_bytes(), current_time, scratch),
				Destination::Server => pair
					.server
					.process_datagram(scheduled.datagram.as_bytes(), current_time, scratch),
			}
			.expect("typed pipeline output must retain a canonical encoding");
			outcomes.push((scheduled.destination, outcome));
		}
	}
}

/// Runs a handshake through the production raw-datagram boundary up to the client's challenge response, which it
/// returns undelivered.
fn challenged_pair(client_salt: u64, server_salt: u64, current_time: Instant) -> (Pair, EncodedDatagram) {
	let mut pair = Pair {
		client: ClientDatagramPipeline::new(client_salt),
		server: ServerDatagramPipeline::new(server_salt),
	};
	let mut client_output = Vec::new();
	let mut server_output = Vec::new();

	pair.client.advance(current_time, &mut client_output).unwrap();
	pair.server
		.process_datagram(client_output[0].as_bytes(), current_time, &mut server_output)
		.unwrap();
	pair.client
		.process_datagram(server_output[0].as_bytes(), current_time, &mut client_output)
		.unwrap();

	(pair, client_output[0].clone())
}

/// Establishes an isolated pair entirely through the production raw-datagram boundary.
fn connected_pair(client_salt: u64, server_salt: u64, current_time: Instant) -> Pair {
	let (mut pair, response) = challenged_pair(client_salt, server_salt, current_time);
	let mut client_output = Vec::new();
	let mut server_output = Vec::new();

	assert_eq!(
		pair.server
			.process_datagram(response.as_bytes(), current_time, &mut server_output),
		Ok(DatagramOutcome::Connected {
			id: client_salt ^ server_salt,
		})
	);
	// The server's first keepalive confirms the connection to the client.
	pair.server.advance(current_time, &mut server_output).unwrap();
	assert_eq!(
		pair.client
			.process_datagram(server_output[0].as_bytes(), current_time, &mut client_output),
		Ok(DatagramOutcome::Handled)
	);
	assert!(pair.client.is_connected());

	pair
}

#[test]
fn reliable_exchange_survives_loss_duplication_delay_and_reordering_without_duplicate_delivery() {
	let start = Instant::now();
	let at = |milliseconds| start + Duration::from_millis(milliseconds);
	let mut pair = connected_pair(0x1010, 0x2020, start);
	let mut link = DeterministicLink::new();
	let mut scratch = Vec::new();
	let mut outcomes = Vec::new();
	let request = [0x31; 1024];
	let response = [0x42; 1024];

	pair.client.send(true, request);
	pair.client.advance(start, &mut scratch).unwrap();

	assert_eq!(scratch.len(), 1);
	link.schedule(Destination::Server, scratch[0].clone(), DeliveryPlan::Drop);

	// The retry is duplicated with inverted delays so the later scheduled copy arrives first.
	pair.client.advance(start + betp::RETRY_INTERVAL, &mut scratch).unwrap();
	assert_eq!(scratch.len(), 1);
	link.schedule(
		Destination::Server,
		scratch[0].clone(),
		DeliveryPlan::Duplicate {
			first_delay_steps: 2,
			second_delay_steps: 1,
		},
	);
	link.advance(&mut pair, at(101), &mut scratch, &mut outcomes);
	link.advance(&mut pair, at(102), &mut scratch, &mut outcomes);

	assert_eq!(link.dropped, 1);
	assert_eq!(pair.server.drain_received().collect::<Vec<_>>(), [request]);

	// The server's one-shot response carries the acknowledgement that retires the client's reliable send.
	pair.server.send(false, response);
	pair.server.advance(at(103), &mut scratch).unwrap();
	link.schedule(Destination::Client, scratch[0].clone(), DeliveryPlan::Once { delay_steps: 2 });
	link.advance(&mut pair, at(104), &mut scratch, &mut outcomes);
	link.advance(&mut pair, at(105), &mut scratch, &mut outcomes);

	assert_eq!(pair.client.drain_received().collect::<Vec<_>>(), [response]);
	pair.client.advance(at(100) + betp::RETRY_INTERVAL, &mut scratch).unwrap();

	assert!(scratch.is_empty(), "the peer acknowledgement must retire the reliable send");
	assert!(link.queue.is_empty());
	assert!(outcomes.iter().all(|(_, outcome)| *outcome == DatagramOutcome::Handled));
}

#[test]
fn misrouted_peer_traffic_cannot_deliver_or_refresh_another_session() {
	let start = Instant::now();
	let mut pair_a = connected_pair(0x1111, 0x2222, start);
	let mut pair_b = connected_pair(0xAAAA, 0x5555, start);

	assert_ne!(pair_a.client.connection_id(), pair_b.client.connection_id());
	let mut datagrams = Vec::new();
	let mut scratch = Vec::new();

	pair_a.client.send(false, [0xA1; 1024]);
	pair_a.client.advance(start + Duration::from_secs(4), &mut datagrams).unwrap();
	let from_a = datagrams[0].clone();

	assert_eq!(
		pair_b
			.server
			.process_datagram(from_a.as_bytes(), start + Duration::from_secs(4), &mut scratch,),
		Ok(DatagramOutcome::Dropped(DatagramDrop::ConnectionIdMismatch))
	);
	assert_eq!(
		pair_a
			.server
			.process_datagram(from_a.as_bytes(), start + Duration::from_secs(4), &mut scratch,),
		Ok(DatagramOutcome::Handled)
	);
	assert_eq!(pair_a.server.drain_received().collect::<Vec<_>>(), [[0xA1; 1024]]);
	assert_eq!(pair_b.server.drain_received().count(), 0);
	assert_eq!(
		pair_b.server.advance(start + Duration::from_secs(6), &mut scratch),
		Ok(DatagramOutcome::Disconnected {
			id: pair_b.client.connection_id().unwrap(),
		})
	);
	assert_eq!(
		pair_a.server.advance(start + Duration::from_secs(6), &mut scratch),
		Ok(DatagramOutcome::Handled)
	);
	assert!(pair_a.server.is_connected());
	assert!(!pair_b.server.is_connected());
}

#[test]
fn a_lost_challenge_response_is_resent_and_a_confirming_message_is_delivered_once() {
	let start = Instant::now();
	// The challenge response the handshake produces never reaches the server.
	let (mut pair, _) = challenged_pair(0x1010, 0x2020, start);
	let mut client_output = Vec::new();
	let mut server_output = Vec::new();

	pair.client
		.advance(start + Duration::from_millis(1), &mut client_output)
		.unwrap();
	assert!(!pair.client.is_connected());
	assert_eq!(
		pair.server
			.process_datagram(client_output[0].as_bytes(), start, &mut server_output),
		Ok(DatagramOutcome::Connected { id: 0x1010 ^ 0x2020 })
	);

	pair.server.send(true, [9; 1024]);
	pair.server
		.advance(start + Duration::from_millis(2), &mut server_output)
		.unwrap();
	assert_eq!(server_output.len(), 1);

	for _ in 0..2 {
		assert_eq!(
			pair.client
				.process_datagram(server_output[0].as_bytes(), start, &mut client_output),
			Ok(DatagramOutcome::Handled)
		);
	}
	assert!(pair.client.is_connected());
	assert_eq!(pair.client.drain_received().collect::<Vec<_>>(), [[9; 1024]]);
}

#[test]
fn a_client_the_server_never_confirms_reports_the_attempt_ended_after_the_timeout() {
	let start = Instant::now();
	let (mut pair, _) = challenged_pair(0x1010, 0x2020, start);
	let mut client_output = Vec::new();

	assert_eq!(
		pair.client.advance(start + Duration::from_secs(6), &mut client_output),
		Ok(DatagramOutcome::Disconnected { id: 0x1010 ^ 0x2020 })
	);
	assert!(!pair.client.is_connected());
	assert_eq!(
		pair.client.advance(start + Duration::from_secs(7), &mut client_output),
		Ok(DatagramOutcome::Handled)
	);
}
