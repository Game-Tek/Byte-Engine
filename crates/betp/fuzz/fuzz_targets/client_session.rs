#![no_main]

mod support;

use betp::{
	client::{Errors, Session},
	packets::{ChallengePacket, DisconnectPacket, Packets},
};
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use support::{make_batch, make_data_packet, observe_session_output, other_connection_id, Operation, RETRY_QUIESCENCE_UPDATES};

const MAX_OPERATIONS: usize = 64;
const MAX_BATCH_SIZE: usize = 8;
const MAX_UPDATES: usize = 512;

/// The reachable client lifecycle state selected before hostile operations are applied.
#[derive(Arbitrary, Debug)]
enum StartingState {
	Initial,
	Initiating,
	Connecting,
	Connected,
	Disconnecting,
}

/// The `Input` struct exists to describe a bounded client lifecycle under hostile traffic.
#[derive(Arbitrary, Debug)]
struct Input {
	state: StartingState,
	client_salt: u64,
	server_salt: u64,
	operations: Vec<Operation>,
}

/// Builds a real session in the selected state without duplicating any client transition logic in the harness.
fn make_session(state: &StartingState, client_salt: u64, server_salt: u64) -> (Session, Option<u64>) {
	let mut session = Session::new();
	if matches!(state, StartingState::Initial) {
		return (session, None);
	}

	session.connect(client_salt);
	if matches!(state, StartingState::Initiating) {
		return (session, None);
	}

	let connection_id = client_salt ^ server_salt;
	let challenge = ChallengePacket::new(client_salt, server_salt).into();
	assert!(session.update(&[challenge]).is_ok());
	if matches!(state, StartingState::Connecting) {
		return (session, Some(connection_id));
	}

	assert!(session.update(&[]).is_ok());
	assert!(session.is_connected());
	if matches!(state, StartingState::Disconnecting) {
		session.disconnect();
	}

	(session, Some(connection_id))
}

/// Applies one bounded update and validates every packet emitted by the client session.
fn update_session(
	session: &mut Session,
	packets: &[Packets],
	connection_id: &mut Option<u64>,
	updates_left: &mut usize,
) -> Option<Result<Vec<Packets>, Errors>> {
	if *updates_left == 0 {
		return None;
	}

	*updates_left -= 1;
	let result = session.update(packets);
	if let Ok(output) = &result {
		observe_session_output(output, connection_id);
	}
	Some(result)
}

/// Proves buffered retries stop producing work within the protocol's bounded attempt budget.
fn assert_retry_quiescence(session: &mut Session, connection_id: &mut Option<u64>) {
	if !session.is_connected() {
		return;
	}

	for _ in 0..RETRY_QUIESCENCE_UPDATES {
		let output = session.update(&[]).expect("connected client retry drain must remain usable");
		observe_session_output(&output, connection_id);
		if output.is_empty() {
			return;
		}
	}

	panic!("client retries did not become quiescent within the bounded attempt budget");
}

fuzz_target!(|input: Input| {
	let (mut session, mut connection_id) = make_session(&input.state, input.client_salt, input.server_salt);
	// Reserve the final update calls for the retry-quiescence oracle so every generated scenario remains globally bounded.
	let mut updates_left = MAX_UPDATES - RETRY_QUIESCENCE_UPDATES;

	for operation in input.operations.iter().take(MAX_OPERATIONS) {
		match operation {
			Operation::Packet(input) => {
				let packet = input.to_packet();
				let _ = update_session(&mut session, packet.as_slice(), &mut connection_id, &mut updates_left);
			}
			Operation::Batch(inputs) => {
				let packets = make_batch(inputs, MAX_BATCH_SIZE);
				let _ = update_session(&mut session, &packets, &mut connection_id, &mut updates_left);
			}
			Operation::Send { reliable, fill } => {
				if session.is_connected() {
					session.send(*reliable, [*fill; 1024]);
					assert!(session.is_connected());
				}
			}
			Operation::Tick(count) => {
				for _ in 0..usize::from(*count).min(updates_left) {
					let _ = update_session(&mut session, &[], &mut connection_id, &mut updates_left);
				}
			}
			Operation::AdvanceMilliseconds(milliseconds) => {
				// Client sessions have no wall-clock input, but consuming the payload keeps the shared operation format stable.
				let _ = milliseconds;
				let _ = update_session(&mut session, &[], &mut connection_id, &mut updates_left);
			}
			Operation::Disconnect => {
				let was_connected = session.is_connected();
				session.disconnect();
				if was_connected {
					assert!(!session.is_connected());
				}
			}
			Operation::CurrentData {
				sequence,
				ack,
				ack_bitfield,
				fill,
			} => {
				let was_connected = session.is_connected();
				let packet = make_data_packet(connection_id.unwrap_or_default(), *sequence, *ack, *ack_bitfield, *fill);
				let result = update_session(&mut session, &[packet], &mut connection_id, &mut updates_left);
				if was_connected && result.is_some() {
					assert!(result.is_some_and(|result| result.is_ok()));
					assert!(session.is_connected());
				}
			}
			Operation::OtherData {
				sequence,
				ack,
				ack_bitfield,
				fill,
			} => {
				if session.is_connected() && updates_left >= 2 {
					let id = connection_id.expect("connected client must have a negotiated identity");
					let invalid = make_data_packet(other_connection_id(id), *sequence, *ack, *ack_bitfield, *fill);
					let result = update_session(&mut session, &[invalid], &mut connection_id, &mut updates_left);
					assert!(matches!(result, Some(Err(Errors::BadConnectionId))));
					assert!(session.is_connected());

					// Valid traffic immediately after rejected traffic proves the error stays peer-local and recoverable.
					let valid = make_data_packet(id, *sequence, *ack, *ack_bitfield, *fill);
					let result = update_session(&mut session, &[valid], &mut connection_id, &mut updates_left);
					assert!(result.is_some_and(|result| result.is_ok()));
					assert!(session.is_connected());
				} else {
					let id = other_connection_id(connection_id.unwrap_or_default());
					let packet = make_data_packet(id, *sequence, *ack, *ack_bitfield, *fill);
					let _ = update_session(&mut session, &[packet], &mut connection_id, &mut updates_left);
				}
			}
			Operation::DuplicateCurrentData {
				sequence,
				ack,
				ack_bitfield,
				fill,
			} => {
				let was_connected = session.is_connected();
				let id = connection_id.unwrap_or_default();
				let packets = [
					make_data_packet(id, *sequence, *ack, *ack_bitfield, *fill),
					make_data_packet(id, *sequence, *ack, *ack_bitfield, *fill),
				];
				let result = update_session(&mut session, &packets, &mut connection_id, &mut updates_left);
				if was_connected && result.is_some() {
					assert!(result.is_some_and(|result| result.is_ok()));
					assert!(session.is_connected());
				}
			}
			Operation::CurrentDisconnect => {
				if session.is_connected() && updates_left > 0 {
					let id = connection_id.expect("connected client must have a negotiated identity");
					let packet = DisconnectPacket::new(id).into();
					let result = update_session(&mut session, &[packet], &mut connection_id, &mut updates_left);
					assert!(matches!(result, Some(Ok(output)) if output.is_empty()));
					assert!(!session.is_connected());

					if updates_left > 0 {
						let result = update_session(&mut session, &[], &mut connection_id, &mut updates_left);
						assert!(
							matches!(result, Some(Ok(output)) if matches!(output.as_slice(), [Packets::Disconnect(packet)] if packet.get_connection_id() == id))
						);
					}
				} else {
					let packet = DisconnectPacket::new(connection_id.unwrap_or_default()).into();
					let _ = update_session(&mut session, &[packet], &mut connection_id, &mut updates_left);
				}
			}
			Operation::OtherDisconnect => {
				if session.is_connected() && updates_left >= 2 {
					let id = connection_id.expect("connected client must have a negotiated identity");
					let invalid = DisconnectPacket::new(other_connection_id(id)).into();
					let result = update_session(&mut session, &[invalid], &mut connection_id, &mut updates_left);
					assert!(matches!(result, Some(Err(Errors::BadConnectionId))));
					assert!(session.is_connected());

					let valid = make_data_packet(id, 0, 0, 0, 0);
					let result = update_session(&mut session, &[valid], &mut connection_id, &mut updates_left);
					assert!(result.is_some_and(|result| result.is_ok()));
					assert!(session.is_connected());
				} else {
					let id = other_connection_id(connection_id.unwrap_or_default());
					let packet = DisconnectPacket::new(id).into();
					let _ = update_session(&mut session, &[packet], &mut connection_id, &mut updates_left);
				}
			}
		}
	}

	assert_retry_quiescence(&mut session, &mut connection_id);
});
