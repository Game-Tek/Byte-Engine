#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_PACKET_SIZE: usize = 1045;

fuzz_target!(|data: &[u8]| {
	let Ok(packet) = betp::read_packet(data) else {
		return;
	};

	// Successful decoding implies an exact known wire size, so canonical re-encoding must reproduce every accepted byte.
	assert!(data.len() <= MAX_PACKET_SIZE);
	let mut encoded = [0u8; MAX_PACKET_SIZE];
	assert_eq!(betp::write_packet(&mut encoded[..data.len()], packet), Some(()));
	assert_eq!(&encoded[..data.len()], data);
	assert!(betp::read_packet(&encoded[..data.len()]).is_ok());
});
