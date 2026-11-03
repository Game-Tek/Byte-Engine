//! Compress complete CPU-readable resource payloads without changing their client-facing bytes.
//!
//! A [`ResourcePayloadEncoding::CpuLz4`] payload is a block table followed by one block per segment of the decoded
//! payload. Segments end at every stream boundary, so a reader decodes only the blocks a requested stream covers.

use std::ops::Range;

use crate::StreamDescription;

/// Payloads below this size rarely repay compression setup and metadata costs.
pub(crate) const MINIMUM_COMPRESSION_SIZE: usize = 1024;
const MINIMUM_SAVINGS_DIVISOR: usize = 8;

/// Bytes in each window the compressibility probe compresses.
const PROBE_WINDOW_SIZE: usize = 16 * 1024;
/// Windows the probe spreads over a payload, from its first byte to its last.
const PROBE_WINDOWS: usize = 4;

/// Bytes in the block count that opens a CPU LZ4 payload.
const BLOCK_COUNT_SIZE: usize = 4;
/// Bytes in each block table entry: the decoded size, then the stored size.
const BLOCK_ENTRY_SIZE: usize = 16;

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
	/// Stores a block table and one block per stream segment, each raw or LZ4, so streams decode independently.
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

	/// Returns whether clients must receive decoded bytes through CPU decompression.
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

/// The `PreparedCompression` struct carries one CPU LZ4 payload and the identity of its decoded bytes.
pub(crate) struct PreparedCompression {
	pub(crate) bytes: Vec<u8>,
	pub(crate) decoded_hash: u64,
	pub(crate) decoded_size: usize,
}

/// Builds a CPU LZ4 payload with one block per stream segment, compressing each segment that is large enough and
/// saves more than 12.5%.
///
/// Returns `None` when no segment compresses, so the caller stores the payload raw.
pub(crate) fn prepare(
	data: &[u8],
	streams: Option<&[StreamDescription]>,
	policy: ResourceCompressionPolicy,
) -> Option<PreparedCompression> {
	if policy == ResourceCompressionPolicy::Disabled || data.len() < MINIMUM_COMPRESSION_SIZE {
		return None;
	}

	let boundaries = segment_boundaries(data.len(), streams);
	let segments = || boundaries.windows(2).map(|window| window[0]..window[1]);
	let block_count = boundaries.len() - 1;

	// Every segment's encoder bound is reserved up front, so compressing one segment never reallocates the output.
	let Some(maximum_size) = segments().try_fold(
		BLOCK_COUNT_SIZE.checked_add(BLOCK_ENTRY_SIZE.checked_mul(block_count)?)?,
		|total, segment| total.checked_add(maximum_compressed_size(segment.len())?),
	) else {
		log::warn!(
			"Resource compression was skipped. The most likely cause is that the complete payload is too large for this platform. Store the resource with `ResourceCompressionPolicy::Disabled`."
		);
		return None;
	};
	let mut payload = Vec::new();
	if payload.try_reserve_exact(maximum_size).is_err() {
		log::warn!(
			"Resource compression was skipped. The most likely cause is insufficient memory for the temporary LZ4 output. Free memory or store the resource with `ResourceCompressionPolicy::Disabled`."
		);
		return None;
	}

	payload.extend_from_slice(&u32::try_from(block_count).ok()?.to_le_bytes());
	payload.resize(BLOCK_COUNT_SIZE + BLOCK_ENTRY_SIZE * block_count, 0);
	let mut compressed_any = false;
	for (index, segment) in segments().enumerate() {
		let decoded_size = segment.len();
		let stored_size = append_block(&mut payload, &data[segment]);
		compressed_any |= stored_size != decoded_size;

		let entry = BLOCK_COUNT_SIZE + BLOCK_ENTRY_SIZE * index;
		payload[entry..entry + 8].copy_from_slice(&(decoded_size as u64).to_le_bytes());
		payload[entry + 8..entry + 16].copy_from_slice(&(stored_size as u64).to_le_bytes());
	}

	if !compressed_any {
		return None;
	}

	Some(PreparedCompression {
		bytes: payload,
		decoded_hash: payload_hash(data),
		decoded_size: data.len(),
	})
}

/// Returns the sorted, distinct offsets where segments start and end: 0, `size`, and every stream's start and end.
///
/// Stream ranges past the payload are clamped to it, so malformed descriptions can't create out-of-range segments.
fn segment_boundaries(size: usize, streams: Option<&[StreamDescription]>) -> Vec<usize> {
	let streams = streams.unwrap_or_default();
	let mut boundaries = Vec::with_capacity(2 + streams.len() * 2);
	boundaries.extend([0, size]);
	for stream in streams {
		let start = stream.offset().min(size);
		boundaries.extend([start, stream.offset().saturating_add(stream.size()).min(size)]);
	}
	boundaries.sort_unstable();
	boundaries.dedup();
	boundaries
}

/// Appends one segment's block to `payload` and returns its stored size.
///
/// The block is LZ4 when the segment passes the size, probe, and savings rules, and the segment's raw bytes otherwise.
/// `payload` must already have capacity for the segment's encoder bound.
fn append_block(payload: &mut Vec<u8>, segment: &[u8]) -> usize {
	if segment.len() >= MINIMUM_COMPRESSION_SIZE
		&& probe_suggests_savings(segment)
		&& let Some(bound) = maximum_compressed_size(segment.len())
	{
		let start = payload.len();
		payload.resize(start + bound, 0);
		if let Ok(size) = lz4_flex::block::compress_into(segment, &mut payload[start..])
			&& is_worthwhile(segment.len(), size)
		{
			payload.truncate(start + size);
			return size;
		}
		payload.truncate(start);
	}

	payload.extend_from_slice(segment);
	segment.len()
}

/// Decodes a complete CPU LZ4 payload into the exact post-decompression buffer.
pub(crate) fn decompress_into(payload: &[u8], output: &mut [u8]) -> Result<(), ()> {
	let mut blocks = Blocks::parse(payload)?;
	let mut decoded_end = 0;
	for block in &mut blocks {
		let block = block?;
		block.decode_into(output.get_mut(block.decoded.clone()).ok_or(())?)?;
		decoded_end = block.decoded.end;
	}
	if decoded_end != output.len() || !blocks.data.is_empty() {
		return Err(());
	}
	Ok(())
}

/// Decodes the `range` of decoded bytes into `output`, which must be exactly `range.len()` long, touching only the
/// blocks that overlap it.
///
/// A block that `range` covers completely is decoded straight into `output`. A block it covers only in part is
/// decoded into `scratch` first, which callers reuse across reads to avoid one allocation per block.
pub(crate) fn decode_range_into(
	payload: &[u8],
	range: Range<usize>,
	output: &mut [u8],
	scratch: &mut Vec<u8>,
) -> Result<(), ()> {
	debug_assert_eq!(range.len(), output.len());
	if range.is_empty() {
		return Ok(());
	}

	for block in Blocks::parse(payload)? {
		let block = block?;
		if block.decoded.end <= range.start {
			continue;
		}

		let start = block.decoded.start.max(range.start);
		let end = block.decoded.end.min(range.end);
		let destination = &mut output[start - range.start..end - range.start];
		if start == block.decoded.start && end == block.decoded.end {
			block.decode_into(destination)?;
		} else {
			scratch.clear();
			scratch.try_reserve(block.decoded.len()).map_err(|_| ())?;
			scratch.resize(block.decoded.len(), 0);
			block.decode_into(scratch)?;
			destination.copy_from_slice(&scratch[start - block.decoded.start..end - block.decoded.start]);
		}

		if block.decoded.end >= range.end {
			return Ok(());
		}
	}

	// The table ended before the range did.
	Err(())
}

/// The `Block` struct locates one stored block of a CPU LZ4 payload and the decoded range it expands to.
struct Block<'a> {
	decoded: Range<usize>,
	stored: &'a [u8],
}

impl Block<'_> {
	/// Writes the block's decoded bytes into `output`, which must be exactly the block's decoded size.
	fn decode_into(&self, output: &mut [u8]) -> Result<(), ()> {
		if output.len() != self.decoded.len() {
			return Err(());
		}
		// A block stored at its decoded size holds raw bytes, because a kept LZ4 block is always smaller.
		if self.stored.len() == output.len() {
			output.copy_from_slice(self.stored);
			return Ok(());
		}
		let written = lz4_flex::block::decompress_into(self.stored, output).map_err(|_| ())?;
		if written != output.len() {
			return Err(());
		}
		Ok(())
	}
}

/// The `Blocks` struct walks a CPU LZ4 payload's block table in decoded order without allocating.
struct Blocks<'a> {
	table: &'a [u8],
	data: &'a [u8],
	decoded_offset: usize,
}

impl<'a> Blocks<'a> {
	/// Splits a payload into its block table and block bytes.
	fn parse(payload: &'a [u8]) -> Result<Self, ()> {
		let (count, rest) = payload.split_first_chunk::<BLOCK_COUNT_SIZE>().ok_or(())?;
		let table_size = usize::try_from(u32::from_le_bytes(*count))
			.ok()
			.and_then(|count| count.checked_mul(BLOCK_ENTRY_SIZE))
			.ok_or(())?;
		let (table, data) = rest.split_at_checked(table_size).ok_or(())?;
		Ok(Self {
			table,
			data,
			decoded_offset: 0,
		})
	}
}

impl<'a> Iterator for Blocks<'a> {
	type Item = Result<Block<'a>, ()>;

	fn next(&mut self) -> Option<Self::Item> {
		let (entry, table) = self.table.split_first_chunk::<BLOCK_ENTRY_SIZE>()?;
		self.table = table;
		let size = |bytes: &[u8]| usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap())).map_err(|_| ());
		let block = (|| {
			let decoded_size = size(&entry[..8])?;
			let stored_size = size(&entry[8..])?;
			// An LZ4 block larger than its decoded bytes is never kept, so a larger stored size means corrupt metadata.
			if stored_size > decoded_size {
				return Err(());
			}
			let (stored, data) = self.data.split_at_checked(stored_size).ok_or(())?;
			let decoded_end = self.decoded_offset.checked_add(decoded_size).ok_or(())?;
			let block = Block {
				decoded: self.decoded_offset..decoded_end,
				stored,
			};
			self.data = data;
			self.decoded_offset = decoded_end;
			Ok(block)
		})();
		if block.is_err() {
			// Stop after the first corrupt entry, because later offsets depend on it.
			self.table = &[];
		}
		Some(block)
	}
}

/// Returns whether compression saves enough storage to repay decoding work.
fn is_worthwhile(decoded_size: usize, compressed_size: usize) -> bool {
	compressed_size < decoded_size - decoded_size / MINIMUM_SAVINGS_DIVISOR
}

/// Predicts from a few spread-out windows whether compressing all of one segment can save enough to keep the result.
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
	use super::{ResourceCompressionPolicy, decompress_into, payload_hash, prepare};
	use crate::StreamDescription;

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
		assert!(prepare(&noise(512 * 1024), None, ResourceCompressionPolicy::Enabled).is_none());
	}

	#[test]
	fn payloads_that_compress_are_compressed_even_when_their_start_does_not() {
		// A noisy header followed by a long repetitive body must still compress, because the probe samples the
		// whole payload rather than only its first bytes.
		let mut data = noise(64 * 1024);
		data.extend(std::iter::repeat_n([12_u8, 34, 56, 255], 256 * 1024).flatten());

		let prepared = prepare(&data, None, ResourceCompressionPolicy::Enabled).expect("a mostly repetitive payload should compress");

		assert!(prepared.bytes.len() < data.len() / 2);
		assert_eq!(prepared.decoded_size, data.len());
		assert_eq!(prepared.decoded_hash, payload_hash(&data));
	}

	#[test]
	fn streams_that_do_not_compress_are_stored_raw_beside_streams_that_do() {
		let mut data = noise(64 * 1024);
		data.extend(std::iter::repeat_n(9_u8, 64 * 1024));
		let streams = [
			StreamDescription::new("noise", 64 * 1024, 0),
			StreamDescription::new("runs", 64 * 1024, 64 * 1024),
		];

		let prepared =
			prepare(&data, Some(&streams), ResourceCompressionPolicy::Enabled).expect("the repetitive stream should compress");

		// The noise stream keeps its full size, and the runs stream shrinks to almost nothing.
		assert!(prepared.bytes.len() > 64 * 1024);
		assert!(prepared.bytes.len() < 64 * 1024 + 1024);
		let mut decoded = vec![0; data.len()];
		decompress_into(&prepared.bytes, &mut decoded).unwrap();
		assert_eq!(decoded, data);
	}
}
