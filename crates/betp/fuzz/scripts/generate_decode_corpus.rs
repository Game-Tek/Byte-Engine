use std::{fs, io, path::Path};

const CORPUS_DIRECTORY: &str = "corpus/decode_packet";
const HEADER_SIZE: usize = 5;

/// Builds a canonical packet with a deterministic non-zero body.
fn canonical_packet(packet_type: u8, packet_size: usize) -> Vec<u8> {
	let mut packet = vec![0xA5; packet_size];
	packet[..4].copy_from_slice(b"BETP");
	packet[4] = packet_type;
	packet
}

/// Writes one stable seed without relying on shell-specific binary escaping.
fn write_seed(name: &str, bytes: &[u8]) -> io::Result<()> {
	fs::write(Path::new(CORPUS_DIRECTORY).join(name), bytes)
}

/// Rebuilds the curated decoder corpus from BETP's fixed wire sizes.
fn main() -> io::Result<()> {
	fs::create_dir_all(CORPUS_DIRECTORY)?;

	write_seed("reserved.betp", b"BETP\0")?;
	write_seed("unknown-type.betp", b"BETP\xFF")?;
	write_seed("truncated-header.betp", b"BETP")?;
	write_seed("truncated-connection-request.betp", b"BETP\x01")?;
	write_seed("connection-request.betp", &canonical_packet(1, HEADER_SIZE + 8))?;
	write_seed("challenge.betp", &canonical_packet(2, HEADER_SIZE + 16))?;
	write_seed("challenge-response.betp", &canonical_packet(3, HEADER_SIZE + 8))?;
	write_seed("data.betp", &canonical_packet(4, HEADER_SIZE + 8 + 8 + 1024))?;
	write_seed("disconnect.betp", &canonical_packet(5, HEADER_SIZE + 8))?;

	Ok(())
}
