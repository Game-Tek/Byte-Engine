#![no_main]

mod support;

use std::time::{Duration, Instant};

use betp::server::Session;
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use support::{make_batch, Operation};

const MAX_OPERATIONS: usize = 64;
const MAX_BATCH_SIZE: usize = 8;
const MAX_UPDATES: usize = 512;

/// The server lifecycle state selected after transport-owned handshake validation.
#[derive(Arbitrary, Debug)]
enum StartingState {
	Initial,
	Accepted,
	Disconnecting,
}

/// A bounded post-handshake server scenario with arbitrary connection identity and hostile operations.
#[derive(Arbitrary, Debug)]
struct Input {
	state: StartingState,
	connection_id: u64,
	operations: Vec<Operation>,
}

/// Builds a real post-handshake server session without moving transport-owned handshake policy into BETP.
fn make_session(state: &StartingState, connection_id: u64, current_time: Instant) -> Session {
	let mut session = Session::new();
	if matches!(state, StartingState::Initial) {
		return session;
	}

	session.accept(connection_id, current_time);
	if matches!(state, StartingState::Disconnecting) {
		session.disconnect();
	}

	session
}

fuzz_target!(|input: Input| {
	let mut current_time = Instant::now();
	let mut session = make_session(&input.state, input.connection_id, current_time);
	let mut updates_left = MAX_UPDATES;

	for operation in input.operations.iter().take(MAX_OPERATIONS) {
		match operation {
			Operation::Packet(input) => {
				let packet = input.to_packet();
				let packets = packet.as_slice();
				let _ = session.update(packets, current_time);
			}
			Operation::Batch(inputs) => {
				let packets = make_batch(inputs, MAX_BATCH_SIZE);
				let _ = session.update(&packets, current_time);
			}
			Operation::Send { reliable, fill } => {
				if session.is_connected() {
					session.send(*reliable, [*fill; 1024]);
				}
			}
			Operation::Tick(count) => {
				let count = usize::from(*count).min(updates_left);
				for _ in 0..count {
					current_time += Duration::from_millis(1);
					let _ = session.update(&[], current_time);
				}
				updates_left -= count;
			}
			Operation::AdvanceMilliseconds(milliseconds) => {
				current_time += Duration::from_millis(u64::from(*milliseconds));
				if updates_left > 0 {
					let _ = session.update(&[], current_time);
					updates_left -= 1;
				}
			}
			Operation::Disconnect => session.disconnect(),
		}
	}
});
