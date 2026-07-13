use std::{fs, io, path::Path};

const CLIENT_CORPUS_DIRECTORY: &str = "corpus/client_session";
const SERVER_CORPUS_DIRECTORY: &str = "corpus/server_session";
const CLIENT_STATE_COUNT: u64 = 5;
const SERVER_STATE_COUNT: u64 = 3;
const OPERATION_COUNT: u64 = 11;

const OPERATION_SEND: u64 = 2;
const OPERATION_TICK: u64 = 3;
const OPERATION_ADVANCE_MILLISECONDS: u64 = 4;
const OPERATION_CURRENT_DATA: u64 = 6;
const OPERATION_OTHER_DATA: u64 = 7;
const OPERATION_DUPLICATE_CURRENT_DATA: u64 = 8;
const OPERATION_CURRENT_DISCONNECT: u64 = 9;
const OPERATION_OTHER_DISCONNECT: u64 = 10;

/// Selects the middle of an `arbitrary` derive enum bucket for a stable variant choice.
fn enum_choice(index: u64, count: u64) -> [u8; 4] {
	let numerator = u128::from(2 * index + 1) * (u128::from(u32::MAX) + 1);
	let value = (numerator / u128::from(2 * count)) as u32;
	assert_eq!((u64::from(value) * count) >> 32, index);
	value.to_le_bytes()
}

/// Encodes the fixed fields that precede a client's operation vector.
fn client_seed(state: u64) -> Vec<u8> {
	let mut seed = Vec::with_capacity(32);
	seed.extend_from_slice(&enum_choice(state, CLIENT_STATE_COUNT));
	seed.extend_from_slice(&0x1122_3344_5566_7788_u64.to_le_bytes());
	seed.extend_from_slice(&0x8877_6655_4433_2211_u64.to_le_bytes());
	seed
}

/// Encodes the fixed fields that precede a server's operation vector.
fn server_seed(state: u64) -> Vec<u8> {
	let mut seed = Vec::with_capacity(24);
	seed.extend_from_slice(&enum_choice(state, SERVER_STATE_COUNT));
	seed.extend_from_slice(&0x9955_3311_2244_6688_u64.to_le_bytes());
	seed
}

/// Appends one vector element and its operation discriminant in `arbitrary` derive format.
fn push_operation(seed: &mut Vec<u8>, operation: u64, fields: &[u8]) {
	seed.push(1);
	seed.extend_from_slice(&enum_choice(operation, OPERATION_COUNT));
	seed.extend_from_slice(fields);
}

/// Appends deterministic connection-status fields for a correlated data operation.
fn push_data_operation(seed: &mut Vec<u8>, operation: u64) {
	let mut fields = [0_u8; 9];
	fields[..2].copy_from_slice(&7_u16.to_le_bytes());
	fields[2..4].copy_from_slice(&3_u16.to_le_bytes());
	fields[4..8].copy_from_slice(&5_u32.to_le_bytes());
	fields[8] = 0xA5;
	push_operation(seed, operation, &fields);
}

/// Writes one stable seed without relying on shell-specific binary escaping.
fn write_seed(directory: &str, name: &str, bytes: &[u8]) -> io::Result<()> {
	fs::write(Path::new(directory).join(name), bytes)
}

/// Rebuilds curated seeds for every lifecycle and correlated identity branch.
fn main() -> io::Result<()> {
	fs::create_dir_all(CLIENT_CORPUS_DIRECTORY)?;
	fs::create_dir_all(SERVER_CORPUS_DIRECTORY)?;

	for (name, state) in [
		("initial-idle.betp", 0),
		("initiating-request.betp", 1),
		("connecting-transition.betp", 2),
		("disconnecting-notice.betp", 4),
	] {
		let mut seed = client_seed(state);
		push_operation(&mut seed, OPERATION_TICK, &1_u16.to_le_bytes());
		write_seed(CLIENT_CORPUS_DIRECTORY, name, &seed)?;
	}

	for (name, operation) in [
		("connected-current-data.betp", OPERATION_CURRENT_DATA),
		("connected-other-data.betp", OPERATION_OTHER_DATA),
		("connected-duplicate-data.betp", OPERATION_DUPLICATE_CURRENT_DATA),
	] {
		let mut seed = client_seed(3);
		push_data_operation(&mut seed, operation);
		write_seed(CLIENT_CORPUS_DIRECTORY, name, &seed)?;
	}

	for (name, operation) in [
		("connected-current-disconnect.betp", OPERATION_CURRENT_DISCONNECT),
		("connected-other-disconnect.betp", OPERATION_OTHER_DISCONNECT),
	] {
		let mut seed = client_seed(3);
		push_operation(&mut seed, operation, &[]);
		write_seed(CLIENT_CORPUS_DIRECTORY, name, &seed)?;
	}

	let mut client_retry = client_seed(3);
	push_operation(&mut client_retry, OPERATION_SEND, &[1, 0x5A]);
	write_seed(CLIENT_CORPUS_DIRECTORY, "connected-reliable-retry.betp", &client_retry)?;

	for (name, state) in [("initial-idle.betp", 0), ("disconnecting-notice.betp", 2)] {
		let mut seed = server_seed(state);
		push_operation(&mut seed, OPERATION_TICK, &1_u16.to_le_bytes());
		write_seed(SERVER_CORPUS_DIRECTORY, name, &seed)?;
	}

	for (name, operation) in [
		("accepted-current-data.betp", OPERATION_CURRENT_DATA),
		("accepted-other-data.betp", OPERATION_OTHER_DATA),
		("accepted-duplicate-data.betp", OPERATION_DUPLICATE_CURRENT_DATA),
	] {
		let mut seed = server_seed(1);
		push_data_operation(&mut seed, operation);
		write_seed(SERVER_CORPUS_DIRECTORY, name, &seed)?;
	}

	for (name, operation) in [
		("accepted-current-disconnect.betp", OPERATION_CURRENT_DISCONNECT),
		("accepted-other-disconnect.betp", OPERATION_OTHER_DISCONNECT),
	] {
		let mut seed = server_seed(1);
		push_operation(&mut seed, operation, &[]);
		write_seed(SERVER_CORPUS_DIRECTORY, name, &seed)?;
	}

	let mut server_retry = server_seed(1);
	push_operation(&mut server_retry, OPERATION_SEND, &[1, 0x5A]);
	write_seed(SERVER_CORPUS_DIRECTORY, "accepted-reliable-retry.betp", &server_retry)?;

	let mut server_timeout = server_seed(1);
	push_operation(&mut server_timeout, OPERATION_ADVANCE_MILLISECONDS, &6_001_u16.to_le_bytes());
	write_seed(SERVER_CORPUS_DIRECTORY, "accepted-timeout.betp", &server_timeout)?;

	Ok(())
}
