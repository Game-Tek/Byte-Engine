//! Compress complete CPU-readable resource payloads without changing their client-facing bytes.

/// Payloads below this size rarely repay compression setup and metadata costs.
pub(crate) const MINIMUM_COMPRESSION_SIZE: usize = 1024;
const MINIMUM_SAVINGS_DIVISOR: usize = 8;

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

/// The `PreparedCompression` struct carries an encoded payload and the hash of its decoded bytes.
pub(crate) struct PreparedCompression {
	pub(crate) bytes: Vec<u8>,
	pub(crate) decoded_hash: u64,
	pub(crate) decoded_size: usize,
	pub(crate) encoding: ResourcePayloadEncoding,
}

/// Compresses a complete payload when it is large enough and saves more than 12.5%.
pub(crate) fn prepare(data: &[u8], policy: ResourceCompressionPolicy) -> Option<PreparedCompression> {
	if policy == ResourceCompressionPolicy::Disabled || data.len() < MINIMUM_COMPRESSION_SIZE {
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
		encoding: ResourcePayloadEncoding::CpuLz4,
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

/// Computes the encoder's 110% plus 20-byte output bound without integer overflow.
fn maximum_compressed_size(input_size: usize) -> Option<usize> {
	input_size.checked_mul(110)?.checked_div(100)?.checked_add(20)
}

pub(crate) fn payload_hash(data: &[u8]) -> u64 {
	let digest = md5::compute(data);
	u64::from_le_bytes(digest.0[..8].try_into().expect("MD5 digest should contain eight bytes"))
}

#[cfg(test)]
mod tests {
	use super::{
		MINIMUM_COMPRESSION_SIZE, ResourceCompressionPolicy, decompress_into, is_worthwhile, maximum_compressed_size, prepare,
	};

	#[test]
	fn skips_small_and_explicitly_disabled_payloads() {
		assert!(prepare(&vec![7; MINIMUM_COMPRESSION_SIZE - 1], ResourceCompressionPolicy::Enabled).is_none());
		assert!(prepare(&vec![7; MINIMUM_COMPRESSION_SIZE * 2], ResourceCompressionPolicy::Disabled).is_none());
	}

	#[test]
	fn keeps_only_material_space_savings() {
		assert!(is_worthwhile(1024, 895));
		assert!(!is_worthwhile(1024, 896));
		assert!(!is_worthwhile(1024, 1023));
	}

	#[test]
	fn computes_the_encoder_bound_without_overflow() {
		assert_eq!(
			maximum_compressed_size(4096),
			Some(lz4_flex::block::get_maximum_output_size(4096))
		);
		assert_eq!(maximum_compressed_size(usize::MAX), None);
	}

	#[test]
	fn compresses_and_decodes_redundant_payloads_into_exact_storage() {
		let decoded = vec![42; MINIMUM_COMPRESSION_SIZE * 4];
		let compressed = prepare(&decoded, ResourceCompressionPolicy::Enabled)
			.expect("repeated bytes should pass the compression heuristic");
		let mut output = vec![0; decoded.len()];

		decompress_into(&compressed.bytes, &mut output).unwrap();

		assert_eq!(output, decoded);
		assert!(compressed.bytes.len() < decoded.len() - decoded.len() / 8);
		assert_eq!(compressed.decoded_size, decoded.len());
		assert_eq!(compressed.encoding, super::ResourcePayloadEncoding::CpuLz4);
	}

	#[test]
	fn rejects_an_output_buffer_with_the_wrong_decoded_size() {
		let decoded = vec![11; MINIMUM_COMPRESSION_SIZE * 2];
		let compressed = prepare(&decoded, ResourceCompressionPolicy::Enabled)
			.expect("repeated bytes should pass the compression heuristic");
		let mut output = vec![0; decoded.len() - 1];

		assert!(decompress_into(&compressed.bytes, &mut output).is_err());
	}
}
