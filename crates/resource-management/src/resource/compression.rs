//! Compress complete CPU-readable resource payloads without changing their client-facing bytes.

/// Payloads below this size rarely repay compression setup and metadata costs.
pub(crate) const MINIMUM_COMPRESSION_SIZE: usize = 1024;
const MINIMUM_SAVINGS_DIVISOR: usize = 8;

/// Bytes in each window the compressibility probe compresses.
const PROBE_WINDOW_SIZE: usize = 16 * 1024;
/// Windows the probe spreads over a payload, from its first byte to its last.
const PROBE_WINDOWS: usize = 4;

/// Selects the explicit storage and delivery encoding for one resource payload.
///
/// Read [`SerializableResource::encoding`](crate::SerializableResource::encoding)
/// before constructing the payload reader.
#[derive(
	Clone,
	Copy,
	Debug,
	Default,
	PartialEq,
	Eq,
	serde::Serialize,
	serde::Deserialize,
	rkyv::Archive,
	rkyv::Serialize,
	rkyv::Deserialize,
)]
pub enum ResourcePayloadEncoding {
	/// Stores the resource bytes as authored.
	#[default]
	Raw,
	/// Stores one checked LZ4 block that expands to the resource's declared size.
	CpuLz4,
	/// Stores one Metal I/O LZ4 container that transfers directly into a GPU resource.
	MetalIoLz4,
}

impl ResourcePayloadEncoding {
	/// Returns the stable name used by inspection tools.
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Raw => "raw",
			Self::CpuLz4 => "cpu-lz4",
			Self::MetalIoLz4 => "metal-io-lz4",
		}
	}

	/// Returns whether clients must receive the complete payload through CPU decompression.
	pub const fn requires_cpu_decompression(self) -> bool {
		matches!(self, Self::CpuLz4)
	}

	/// Returns whether the payload must be transferred through native GPU resource I/O.
	pub const fn is_gpu_backed(self) -> bool {
		matches!(self, Self::MetalIoLz4)
	}
}

/// Controls whether a complete resource payload may use CPU compression.
///
/// Pass this policy to [`ProcessedAsset::with_compression`](crate::ProcessedAsset::with_compression)
/// before a whole-resource store call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceCompressionPolicy {
	/// Applies the size and savings heuristic before storing an LZ4 payload.
	#[default]
	Enabled,
	/// Stores the resource without CPU compression.
	Disabled,
}

/// The `PreparedCompression` struct carries one CPU LZ4 block and the identity of its decoded bytes.
pub(crate) struct PreparedCompression {
	pub(crate) bytes: Vec<u8>,
	pub(crate) decoded_hash: u64,
	pub(crate) decoded_size: usize,
}

/// Compresses a complete payload when it is large enough and saves more than 12.5%.
pub(crate) fn prepare(data: &[u8], policy: ResourceCompressionPolicy) -> Option<PreparedCompression> {
	if policy == ResourceCompressionPolicy::Disabled || data.len() < MINIMUM_COMPRESSION_SIZE {
		return None;
	}
	if !probe_suggests_savings(data) {
		return None;
	}

	let Some(maximum_size) = maximum_compressed_size(data.len()) else {
		log::warn!(
			"Resource compression was skipped. The most likely cause is that the complete payload is too large for this platform."
		);
		return None;
	};
	let mut compressed = Vec::new();
	if compressed.try_reserve_exact(maximum_size).is_err() {
		log::warn!(
			"Resource compression was skipped. The most likely cause is insufficient memory for the temporary LZ4 output."
		);
		return None;
	}
	compressed.resize(maximum_size, 0);
	let Ok(compressed_size) = lz4_flex::block::compress_into(data, &mut compressed) else {
		log::warn!(
			"Resource compression was skipped. The most likely cause is that the prepared LZ4 output bound was too small."
		);
		return None;
	};

	if !is_worthwhile(data.len(), compressed_size) {
		return None;
	}

	compressed.truncate(compressed_size);
	Some(PreparedCompression {
		bytes: compressed,
		decoded_hash: payload_hash(data),
		decoded_size: data.len(),
	})
}

/// Decodes one complete LZ4 block into the exact post-decompression buffer.
pub(crate) fn decompress_into(compressed: &[u8], output: &mut [u8]) -> Result<(), ()> {
	let written = lz4_flex::block::decompress_into(compressed, output).map_err(|_| ())?;
	if written != output.len() {
		return Err(());
	}
	Ok(())
}

/// Returns whether compression saves enough storage to repay decoding work.
fn is_worthwhile(decoded_size: usize, compressed_size: usize) -> bool {
	compressed_size < decoded_size - decoded_size / MINIMUM_SAVINGS_DIVISOR
}

/// Predicts from a few spread-out windows whether compressing all of `data` can save enough to keep the result.
///
/// Block-compressed textures and other dense payloads rarely compress, and trying costs a full pass over them.
/// Each window sees fewer earlier matches than the whole payload does, so it compresses slightly worse, and the probe
/// rejects only payloads whose windows save less than half the required amount. Payloads too small for the windows to
/// be much cheaper than a full attempt always get one.
fn probe_suggests_savings(data: &[u8]) -> bool {
	if data.len() < PROBE_WINDOW_SIZE * PROBE_WINDOWS * 2 {
		return true;
	}

	// The encoder's output bound for one window, so the probe needs no heap allocation.
	let mut scratch = [0_u8; lz4_flex::block::get_maximum_output_size(PROBE_WINDOW_SIZE)];
	let last_start = data.len() - PROBE_WINDOW_SIZE;
	let mut compressed = 0;
	for window in 0..PROBE_WINDOWS {
		let start = last_start * window / (PROBE_WINDOWS - 1);
		let Ok(size) = lz4_flex::block::compress_into(&data[start..start + PROBE_WINDOW_SIZE], &mut scratch) else {
			return true;
		};
		compressed += size;
	}
	let sampled = PROBE_WINDOW_SIZE * PROBE_WINDOWS;
	compressed < sampled - sampled / (MINIMUM_SAVINGS_DIVISOR * 2)
}

/// Computes the encoder's 110% plus 20-byte output bound without integer overflow.
fn maximum_compressed_size(input_size: usize) -> Option<usize> {
	input_size.checked_mul(110)?.checked_div(100)?.checked_add(20)
}

/// Returns the identity hash of a complete payload, the same value the resource writer computes while streaming it.
///
/// It's rapidhash V3 with the reference secrets, whose output stays the same across crate versions and platforms.
pub(crate) fn payload_hash(data: &[u8]) -> u64 {
	rapidhash::v3::rapidhash_v3(data)
}

#[cfg(test)]
mod tests {
	use super::{ResourceCompressionPolicy, payload_hash, prepare};

	/// Returns deterministic bytes that no general-purpose compressor can shrink.
	fn noise(len: usize) -> Vec<u8> {
		let mut state = 0x9E37_79B9_7F4A_7C15_u64;
		(0..len)
			.map(|_| {
				state ^= state << 13;
				state ^= state >> 7;
				state ^= state << 17;
				(state >> 32) as u8
			})
			.collect()
	}

	#[test]
	fn incompressible_payloads_are_stored_raw() {
		assert!(prepare(&noise(512 * 1024), ResourceCompressionPolicy::Enabled).is_none());
	}

	#[test]
	fn payloads_that_compress_are_compressed_even_when_their_start_does_not() {
		// A noisy header followed by a long repetitive body must still compress, because the probe samples the
		// whole payload rather than only its first bytes.
		let mut data = noise(64 * 1024);
		data.extend(std::iter::repeat_n([12_u8, 34, 56, 255], 256 * 1024).flatten());

		let prepared = prepare(&data, ResourceCompressionPolicy::Enabled).expect("a mostly repetitive payload should compress");

		assert!(prepared.bytes.len() < data.len() / 2);
		assert_eq!(prepared.decoded_size, data.len());
		assert_eq!(prepared.decoded_hash, payload_hash(&data));
	}
}
