#![no_main]

mod support;

use betp::{client::Session, packets::ChallengePacket};
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use support::{make_batch, Operation};

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

/// A bounded client scenario with arbitrary salts, lifecycle state, and hostile operations.
#[derive(Arbitrary, Debug)]
struct Input {
	state: StartingState,
	client_salt: u64,
	server_salt: u64,
	operations: Vec<Operation>,
}

/// Builds a real session in the selected state without duplicating any client transition logic in the harness.
fn make_session(state: &StartingState, client_salt: u64, server_salt: u64) -> Session {
	let mut session = Session::new();
	if matches!(state, StartingState::Initial) {
		return session;
	}

	session.connect(client_salt);
	if matches!(state, StartingState::Initiating) {
		return session;
	}

	let challenge = ChallengePacket::new(client_salt, server_salt).into();
	assert!(session.update(&[challenge]).is_ok());
	if matches!(state, StartingState::Connecting) {
		return session;
	}

	assert!(session.update(&[]).is_ok());
	assert!(session.is_connected());
	if matches!(state, StartingState::Disconnecting) {
		session.disconnect();
	}

	session
}

fuzz_target!(|input: Input| {
	let mut session = make_session(&input.state, input.client_salt, input.server_salt);
	let mut updates_left = MAX_UPDATES;

	for operation in input.operations.iter().take(MAX_OPERATIONS) {
		match operation {
			Operation::Packet(input) => {
				let packet = input.to_packet();
				let packets = packet.as_slice();
				let _ = session.update(packets);
			}
			Operation::Batch(inputs) => {
				let packets = make_batch(inputs, MAX_BATCH_SIZE);
				let _ = session.update(&packets);
			}
			Operation::Send { reliable, fill } => {
				if session.is_connected() {
					session.send(*reliable, [*fill; 1024]);
				}
			}
			Operation::Tick(count) => {
				let count = usize::from(*count).min(updates_left);
				for _ in 0..count {
					let _ = session.update(&[]);
				}
				updates_left -= count;
			}
			Operation::AdvanceMilliseconds(milliseconds) => {
				// Client sessions have no wall-clock input, but consuming the payload keeps the shared operation format stable.
				let _ = milliseconds;
				if updates_left > 0 {
					let _ = session.update(&[]);
					updates_left -= 1;
				}
			}
			Operation::Disconnect => session.disconnect(),
		}
	}
});
