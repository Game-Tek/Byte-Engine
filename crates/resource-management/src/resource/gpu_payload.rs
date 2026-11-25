//! Lay out texture payloads that DirectStorage streams straight into GPU images.
//!
//! A [`ResourcePayloadEncoding::DirectStorageGDeflate`](super::ResourcePayloadEncoding::DirectStorageGDeflate) file
//! is a header, a block table, then the blocks back to back:
//!
//! ```text
//! header:   magic "BEGD", version u32, block count u32, reserved u32          (16 bytes)
//! block[i]: mip u32, row origin u32, row count u32, decoded size u32,
//!           stored offset u64, stored size u32, reserved u32                  (32 bytes)
//! ```
//!
//! Every value is little endian. Each block holds one full-width band of rows of one mip, laid out as the D3D12 copy
//! footprint of that band, so DirectStorage copies it into the image without a CPU pass. A block is GDeflate unless
//! its stored size equals its decoded size, in which case it is stored raw because it would not shrink.

use smallvec::SmallVec;

/// Identifies a DirectStorage texture payload, so a reader rejects files written in another format.
const MAGIC: [u8; 4] = *b"BEGD";
/// Layout version of the header and block table.
const VERSION: u32 = 1;
/// Bytes in the payload header.
pub(crate) const HEADER_SIZE: usize = 16;
/// Bytes in each block table entry.
const BLOCK_ENTRY_SIZE: usize = 32;
/// Upper bound on the blocks of one payload, which rejects corrupt counts before allocating a table.
const MAXIMUM_BLOCK_COUNT: usize = 1 << 16;

/// Largest decoded size of one block.
///
/// DirectStorage stages every request in a 32 MiB buffer by default and rejects requests that do not fit, so large
/// mips are split into bands well below that size. Several bands then stay in flight together.
#[cfg_attr(
	not(all(target_os = "windows", feature = "gpu-processing")),
	allow(dead_code, reason = "Only Windows bakes DirectStorage payloads.")
)]
pub(crate) const MAXIMUM_BLOCK_DECODED_SIZE: usize = 8 * 1024 * 1024;

/// The `GpuPayloadBlock` struct locates one independently loadable band of a mip inside a DirectStorage payload.
///
/// Texture loaders turn each block into one native image request: the band starting at texel row `row_origin` of
/// mip `mip_level`, read from the stored range and decoded into `decoded_size` bytes of copy-footprint rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuPayloadBlock {
	/// The resource mip the band belongs to.
	pub mip_level: u32,
	/// The first texel row of the band.
	pub row_origin: u32,
	/// The texel rows of the band.
	pub row_count: u32,
	/// The bytes the band occupies after decompression, in copy-footprint layout.
	pub decoded_size: u32,
	/// The byte offset of the stored block in the payload file.
	pub stored_offset: u64,
	/// The bytes the block occupies in the file. Equal to `decoded_size` when the block is stored raw.
	pub stored_size: u32,
}

impl GpuPayloadBlock {
	/// Returns whether the block is stored without compression.
	pub fn is_raw(&self) -> bool {
		self.stored_size == self.decoded_size
	}

	/// Encodes this block as one little-endian table entry.
	fn to_bytes(self) -> [u8; BLOCK_ENTRY_SIZE] {
		let mut bytes = [0; BLOCK_ENTRY_SIZE];
		bytes[0..4].copy_from_slice(&self.mip_level.to_le_bytes());
		bytes[4..8].copy_from_slice(&self.row_origin.to_le_bytes());
		bytes[8..12].copy_from_slice(&self.row_count.to_le_bytes());
		bytes[12..16].copy_from_slice(&self.decoded_size.to_le_bytes());
		bytes[16..24].copy_from_slice(&self.stored_offset.to_le_bytes());
		bytes[24..28].copy_from_slice(&self.stored_size.to_le_bytes());
		bytes
	}

	/// Decodes one little-endian table entry.
	fn from_bytes(bytes: &[u8]) -> Self {
		let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
		Self {
			mip_level: u32_at(0),
			row_origin: u32_at(4),
			row_count: u32_at(8),
			decoded_size: u32_at(12),
			stored_offset: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
			stored_size: u32_at(24),
		}
	}
}

/// Blocks of one payload. Most textures have fewer than sixteen, so their table needs no heap allocation.
pub type GpuPayloadBlocks = SmallVec<[GpuPayloadBlock; 16]>;

/// Encodes the payload header for `block_count` blocks.
fn header_bytes(block_count: u32) -> [u8; HEADER_SIZE] {
	let mut bytes = [0; HEADER_SIZE];
	bytes[0..4].copy_from_slice(&MAGIC);
	bytes[4..8].copy_from_slice(&VERSION.to_le_bytes());
	bytes[8..12].copy_from_slice(&block_count.to_le_bytes());
	bytes
}

/// Returns the bytes of the header and block table of a payload with `block_count` blocks.
pub(crate) fn table_size(block_count: usize) -> Option<usize> {
	block_count.checked_mul(BLOCK_ENTRY_SIZE)?.checked_add(HEADER_SIZE)
}

/// Validates a payload header and returns the number of blocks its table holds.
pub(crate) fn parse_header(header: &[u8]) -> Result<usize, &'static str> {
	if header.len() < HEADER_SIZE || header[0..4] != MAGIC {
		return Err(
			"DirectStorage texture payload has no valid header. The most likely cause is a corrupt file or one written by another payload encoding.",
		);
	}
	if u32::from_le_bytes(header[4..8].try_into().unwrap()) != VERSION {
		return Err(
			"DirectStorage texture payload has an unknown layout version. The most likely cause is a resource store baked by a different engine version; rebake it.",
		);
	}
	let block_count = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
	if block_count == 0 || block_count > MAXIMUM_BLOCK_COUNT {
		return Err(
			"DirectStorage texture payload has an invalid block count. The most likely cause is a corrupt or truncated payload file.",
		);
	}
	Ok(block_count)
}

/// Decodes and validates `block_count` table entries of a payload file that is `file_size` bytes long.
///
/// Every block must be nonempty, no larger stored than decoded, and lie entirely after the table and inside the file,
/// so a native request built from it never reads past the payload.
pub(crate) fn parse_blocks(table: &[u8], block_count: usize, file_size: u64) -> Result<GpuPayloadBlocks, &'static str> {
	const INVALID_BLOCK: &str =
		"DirectStorage texture payload has an invalid block. The most likely cause is a corrupt or truncated payload file.";
	let table_end = table_size(block_count).ok_or(INVALID_BLOCK)? as u64;
	if table.len() != block_count * BLOCK_ENTRY_SIZE || table_end > file_size {
		return Err(INVALID_BLOCK);
	}
	table
		.chunks_exact(BLOCK_ENTRY_SIZE)
		.map(GpuPayloadBlock::from_bytes)
		.map(|block| {
			let stored_end = block.stored_offset.checked_add(u64::from(block.stored_size));
			let valid = block.row_count != 0
				&& block.decoded_size != 0
				&& block.stored_size != 0
				&& block.stored_size <= block.decoded_size
				&& block.stored_offset >= table_end
				&& stored_end.is_some_and(|end| end <= file_size);
			valid.then_some(block).ok_or(INVALID_BLOCK)
		})
		.collect()
}

/// Returns whether DirectStorage can load images of `format`.
///
/// Three-channel formats have no D3D12 texture format, so those images keep a CPU encoding.
pub(crate) fn supports_format(format: crate::types::Formats) -> bool {
	!matches!(format, crate::types::Formats::RGB8 | crate::types::Formats::RGB16)
}

/// Returns the texel rows one copy row spans: a 4x4 block row for a block-compressed format, one row otherwise.
#[cfg(feature = "gpu-processing")]
fn texel_rows_per_copy_row(format: ghi::Formats) -> u32 {
	if format.bc_bytes_per_block().is_some() { 4 } else { 1 }
}

/// Returns the texel rows of each band of a mip of `extent` in `format`, so every band's copy footprint stays within
/// `maximum_block_size` bytes.
///
/// Bands of a block-compressed format span whole 4x4 block rows, which D3D12 requires of a copy region that does not
/// end at the image edge. A mip whose single row already exceeds the limit gets one row per band.
#[cfg(feature = "gpu-processing")]
pub(crate) fn band_height(format: ghi::Formats, extent: utils::Extent, maximum_block_size: usize) -> Option<u32> {
	let (bytes_per_row, ..) = format.copy_layout(extent)?;
	let row_pitch = bytes_per_row.checked_next_multiple_of(ghi::TEXTURE_COPY_PITCH_ALIGNMENT)?;
	// A band of n rows occupies (n - 1) padded rows plus one compact row.
	let rows_per_band = maximum_block_size.saturating_sub(bytes_per_row) / row_pitch + 1;
	Some(
		u32::try_from(rows_per_band)
			.unwrap_or(u32::MAX)
			.saturating_mul(texel_rows_per_copy_row(format)),
	)
}

/// Writes one DirectStorage texture payload for `image` from its compact decoded mips into `path`.
///
/// `decoded` holds the mips as the image processor stores them, located by the `mip[N]` streams. Each mip is split
/// into bands, each band is re-laid into copy-footprint rows and GDeflate compressed, and a band that would not shrink
/// is stored raw. Two scratch buffers are reused for every band, so the write allocates only once per payload.
#[cfg(all(target_os = "windows", feature = "gpu-processing"))]
pub(crate) fn write_direct_storage_payload(
	path: &std::path::Path,
	image: &crate::resources::image::Image,
	streams: Option<&[crate::StreamDescription]>,
	decoded: &[u8],
) -> Result<(), String> {
	write_direct_storage_payload_with_block_size(path, image, streams, decoded, MAXIMUM_BLOCK_DECODED_SIZE)
}

/// Writes a DirectStorage texture payload whose blocks decode to at most `maximum_block_size` bytes, so tests can
/// split small images into several bands.
#[cfg(all(target_os = "windows", feature = "gpu-processing"))]
fn write_direct_storage_payload_with_block_size(
	path: &std::path::Path,
	image: &crate::resources::image::Image,
	streams: Option<&[crate::StreamDescription]>,
	decoded: &[u8],
	maximum_block_size: usize,
) -> Result<(), String> {
	use std::io::{Seek as _, SeekFrom, Write as _};

	let format = image.format.ghi();
	let mip_count = image.mip_count.max(1);
	let base_extent = utils::Extent::rectangle(image.extent[0], image.extent[1]);
	let invalid_layout = || {
		"DirectStorage texture bake found an invalid mip layout. The most likely cause is image metadata that does not match its mip streams.".to_string()
	};
	let write_error = |error: std::io::Error| {
		format!(
			"DirectStorage texture payload could not be written. The most likely cause is that the resource directory is full or not writable. Error: {error}"
		)
	};

	// The table precedes the blocks, so count the bands before writing any of them.
	let mut block_count = 0;
	for mip_level in 0..mip_count {
		let extent = base_extent.mip(mip_level);
		let band = band_height(format, extent, maximum_block_size).ok_or_else(invalid_layout)?;
		block_count += extent.height().max(1).div_ceil(band) as usize;
	}
	let table_end = table_size(block_count).ok_or_else(invalid_layout)?;

	let encoder = ghi::io::GDeflateEncoder::new().map_err(|error| error.to_string())?;
	let mut file = std::io::BufWriter::new(std::fs::File::create(path).map_err(write_error)?);
	file.seek(SeekFrom::Start(table_end as u64)).map_err(write_error)?;

	let mut blocks = GpuPayloadBlocks::with_capacity(block_count);
	let mut footprint = Vec::new();
	let mut compressed = Vec::new();
	let mut stored_offset = table_end as u64;
	for mip_level in 0..mip_count {
		let extent = base_extent.mip(mip_level);
		let height = extent.height().max(1);
		let (compact_bytes_per_row, _, compact_size) = format.copy_layout(extent).ok_or_else(invalid_layout)?;
		let mip = mip_range(streams, mip_level, mip_count, decoded.len())
			.and_then(|range| decoded.get(range))
			.filter(|mip| mip.len() == compact_size)
			.ok_or_else(invalid_layout)?;
		let band = band_height(format, extent, maximum_block_size).ok_or_else(invalid_layout)?;

		for row_origin in (0..height).step_by(band as usize) {
			let row_count = band.min(height - row_origin);
			let layout = ghi::io::ResourceIoImageSourceLayout::d3d12_copy_footprint(
				format,
				utils::Extent::rectangle(extent.width(), row_count),
			)
			.map_err(|error| error.to_string())?;

			// Re-lay the band's compact rows at the footprint pitch. The padding is zeroed so equal inputs bake equal files.
			let first_byte = (row_origin / texel_rows_per_copy_row(format)) as usize * compact_bytes_per_row;
			let rows = mip
				.get(first_byte..)
				.ok_or_else(invalid_layout)?
				.chunks(compact_bytes_per_row);
			footprint.clear();
			footprint.resize(layout.total_bytes, 0);
			for (destination, source) in footprint.chunks_mut(layout.bytes_per_row).zip(rows) {
				destination[..compact_bytes_per_row].copy_from_slice(source);
			}

			let stored = match encoder
				.compress(&footprint, &mut compressed)
				.map_err(|error| error.to_string())?
			{
				Some(size) => &compressed[..size],
				None => &footprint[..],
			};
			file.write_all(stored).map_err(write_error)?;
			blocks.push(GpuPayloadBlock {
				mip_level,
				row_origin,
				row_count,
				decoded_size: u32::try_from(layout.total_bytes).map_err(|_| invalid_layout())?,
				stored_offset,
				stored_size: u32::try_from(stored.len()).map_err(|_| invalid_layout())?,
			});
			stored_offset += stored.len() as u64;
		}
	}

	file.seek(SeekFrom::Start(0)).map_err(write_error)?;
	file.write_all(&header_bytes(u32::try_from(blocks.len()).map_err(|_| invalid_layout())?))
		.map_err(write_error)?;
	for block in &blocks {
		file.write_all(&block.to_bytes()).map_err(write_error)?;
	}
	file.flush().map_err(write_error)?;
	Ok(())
}

/// Returns the compact byte range of one mip in an image payload.
///
/// A single-level image may store its one level without a stream table, in which case it spans all `payload_size`
/// bytes.
#[cfg_attr(
	not(all(target_os = "windows", feature = "gpu-processing")),
	allow(dead_code, reason = "Only Windows bakes DirectStorage payloads.")
)]
fn mip_range(
	streams: Option<&[crate::StreamDescription]>,
	mip_level: u32,
	mip_count: u32,
	payload_size: usize,
) -> Option<std::ops::Range<usize>> {
	let Some(streams) = streams else {
		return (mip_count <= 1 && mip_level == 0).then_some(0..payload_size);
	};
	let name = format!("mip[{mip_level}]");
	let stream = streams.iter().find(|stream| stream.name() == name)?;
	Some(stream.offset()..stream.offset().checked_add(stream.size())?)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn block(stored_offset: u64, stored_size: u32, decoded_size: u32) -> GpuPayloadBlock {
		GpuPayloadBlock {
			mip_level: 1,
			row_origin: 4,
			row_count: 8,
			decoded_size,
			stored_offset,
			stored_size,
		}
	}

	fn table(blocks: &[GpuPayloadBlock]) -> Vec<u8> {
		blocks.iter().flat_map(|block| block.to_bytes()).collect()
	}

	#[test]
	fn header_and_blocks_round_trip() {
		let blocks = [block(80, 10, 64), block(90, 64, 64)];
		let header = header_bytes(2);
		assert_eq!(parse_header(&header), Ok(2));
		let parsed = parse_blocks(&table(&blocks), 2, 154).expect("valid table should parse");
		assert_eq!(&parsed[..], &blocks);
		assert!(!parsed[0].is_raw());
		assert!(parsed[1].is_raw());
	}

	#[test]
	fn header_rejects_foreign_or_empty_payloads() {
		assert!(parse_header(b"BEGX\x01\0\0\0\x01\0\0\0\0\0\0\0").is_err());
		assert!(parse_header(b"BEGD\x02\0\0\0\x01\0\0\0\0\0\0\0").is_err());
		assert!(parse_header(&header_bytes(0)).is_err());
		assert!(parse_header(&[0; 4]).is_err());
	}

	#[test]
	fn blocks_must_lie_after_the_table_and_inside_the_file() {
		let table_end = table_size(1).unwrap() as u64;
		// Overlaps the table.
		assert!(parse_blocks(&table(&[block(table_end - 1, 8, 16)]), 1, 100).is_err());
		// Runs past the end of the file.
		assert!(parse_blocks(&table(&[block(table_end, 60, 64)]), 1, table_end + 59).is_err());
		// Stored larger than decoded.
		assert!(parse_blocks(&table(&[block(table_end, 17, 16)]), 1, 100).is_err());
		// Empty.
		assert!(parse_blocks(&table(&[block(table_end, 0, 16)]), 1, 100).is_err());
		// The table is shorter than its count.
		assert!(parse_blocks(&table(&[block(table_end, 8, 16)]), 2, 100).is_err());
		assert!(parse_blocks(&table(&[block(table_end, 8, 16)]), 1, table_end + 8).is_ok());
	}

	#[test]
	fn three_channel_formats_are_not_streamed() {
		assert!(!supports_format(crate::types::Formats::RGB8));
		assert!(!supports_format(crate::types::Formats::RGB16));
		assert!(supports_format(crate::types::Formats::BC7SRGB));
		assert!(supports_format(crate::types::Formats::RGBA8));
	}

	#[cfg(feature = "gpu-processing")]
	#[test]
	fn bands_fit_the_block_limit_and_span_whole_block_rows() {
		// 4096 RGBA8 texels are 16 KiB per row, so 8 MiB fits 512 rows.
		assert_eq!(
			band_height(
				ghi::Formats::RGBA8UNORM,
				utils::Extent::rectangle(4096, 4096),
				MAXIMUM_BLOCK_DECODED_SIZE
			),
			Some(512)
		);
		// 4096 BC7 texels are 16 KiB per block row, so 8 MiB fits 512 block rows of four texel rows.
		assert_eq!(
			band_height(
				ghi::Formats::BC7,
				utils::Extent::rectangle(4096, 4096),
				MAXIMUM_BLOCK_DECODED_SIZE
			),
			Some(2048)
		);
		// A row larger than the limit still gets a band of one row.
		assert_eq!(
			band_height(ghi::Formats::RGBA8UNORM, utils::Extent::rectangle(64, 8), 16),
			Some(1)
		);
		// Small rows pad to the pitch alignment: a 1x1 mip has one 4-byte row.
		assert_eq!(
			band_height(ghi::Formats::RGBA8UNORM, utils::Extent::rectangle(1, 1), 4 + 256 * 3),
			Some(4)
		);
	}

	/// Returns one byte of an integer hash of `value`, so consecutive inputs give bytes no compressor can shrink.
	#[cfg(all(target_os = "windows", feature = "gpu-processing"))]
	fn hash_noise(mut value: u32) -> u8 {
		value ^= value >> 16;
		value = value.wrapping_mul(0x7feb_352d);
		value ^= value >> 15;
		value = value.wrapping_mul(0x846c_a68b);
		value ^= value >> 16;
		value as u8
	}

	/// Bakes a banded payload and streams it into a DX12 image through DirectStorage, then checks every texel of the
	/// base mip. Skips when the machine has no DX12 device or DirectStorage runtime.
	#[cfg(all(target_os = "windows", feature = "gpu-processing"))]
	#[test]
	fn baked_payload_streams_into_a_dx12_image() {
		use ghi::command_buffer::CommandBufferRecording as _;
		use ghi::context::Context as _;
		use ghi::io::{ResourceIoContext as _, ResourceIoQueue as _, ResourceIoTicket as _};

		if let Err(error) = ghi::io::GDeflateEncoder::new() {
			eprintln!("Skipping a DirectStorage payload test: {error}");
			return;
		}
		let mut queue_handle = None;
		let mut context = match ghi::dx12::Context::new(
			ghi::device::Features::new().validation(false),
			&mut [(ghi::QueueSelection::new(ghi::WorkloadTypes::TRANSFER), &mut queue_handle)],
		) {
			Ok(context) => context,
			Err(error) => {
				eprintln!("Skipping a DirectStorage payload test: {error}");
				return;
			}
		};
		let queue_handle = queue_handle.expect("The DX12 test device should expose a transfer queue");

		for (format, width, height) in [
			(crate::types::Formats::RGBA8, 64u32, 40u32),
			(crate::types::Formats::BC7, 64, 40),
		] {
			let ghi_format = format.ghi();
			let image = crate::resources::image::Image {
				format,
				gamma: crate::types::Gamma::Linear,
				extent: [width, height, 0],
				mip_count: 3,
				cpu_readable: false,
				ibl: None,
				photometry: None,
			};
			// The first eight rows of every mip are noise that GDeflate can't shrink, so they're stored raw; the rest
			// is a smooth gradient that compresses.
			let mut decoded = Vec::new();
			let mut streams = Vec::new();
			for mip_level in 0..image.mip_count {
				let extent = utils::Extent::rectangle(width, height).mip(mip_level);
				let (bytes_per_row, row_count, size) = ghi_format.copy_layout(extent).unwrap();
				streams.push(crate::StreamDescription::new(
					format!("mip[{mip_level}]"),
					size,
					decoded.len(),
				));
				for row in 0..row_count {
					for column in 0..bytes_per_row {
						decoded.push(if row < 8 {
							hash_noise((mip_level as usize * 65_536 + row * 4096 + column) as u32)
						} else {
							(row * 3 + column / 16) as u8
						});
					}
				}
			}

			let path = std::env::temp_dir().join(format!("byte-engine-dstorage-payload-{format:?}-{}", std::process::id()));
			// Eight 256-byte rows per block splits every 64-texel-wide mip into several bands.
			write_direct_storage_payload_with_block_size(&path, &image, Some(&streams), &decoded, 2048)
				.expect("The test payload should bake");
			let file_bytes = std::fs::read(&path).unwrap();
			let block_count = parse_header(&file_bytes).expect("The baked header should parse");
			let blocks = parse_blocks(
				&file_bytes[HEADER_SIZE..table_size(block_count).unwrap()],
				block_count,
				file_bytes.len() as u64,
			)
			.expect("The baked table should parse");
			assert!(blocks.iter().filter(|block| block.mip_level == 0).count() > 1, "{format:?}");
			assert!(blocks.iter().any(GpuPayloadBlock::is_raw), "{format:?}");
			assert!(blocks.iter().any(|block| !block.is_raw()), "{format:?}");

			let destination = context.build_image(
				ghi::image::Builder::new(
					ghi_format,
					ghi::Uses::Image | ghi::Uses::TransferSource | ghi::Uses::TransferDestination,
				)
				.extent(utils::Extent::rectangle(width, height))
				.mip_levels(image.mip_count),
			);
			let mut queue = context
				.create_resource_io_queue(ghi::io::ResourceIoQueueDescriptor::new())
				.expect("The DirectStorage test queue should be created");
			let file = queue
				.open_file(ghi::io::ResourceIoFileDescriptor::new(&path).compression(ghi::io::ResourceIoCompression::GDeflate1))
				.expect("DirectStorage should open the baked payload");
			let requests = blocks
				.iter()
				.map(|block| {
					let mip_width = utils::Extent::rectangle(width, height).mip(block.mip_level).width();
					let extent = utils::Extent::rectangle(mip_width, block.row_count);
					let layout = context.resource_io_image_source_layout(ghi_format, extent).unwrap();
					assert_eq!(layout.total_bytes, block.decoded_size as usize);
					ghi::io::ResourceIoImageLoad::new(
						ghi::io::ResourceIoFileRegion::new(file, 0)
							.stored_range(block.stored_offset, block.stored_size as usize),
						destination,
						0,
						block.mip_level,
						extent,
						layout.bytes_per_row,
						layout.bytes_per_image,
					)
					.origin(utils::Extent::new(0, block.row_origin, 0))
					.into()
				})
				.collect::<Vec<ghi::io::ResourceIoRequest>>();
			let ticket = queue
				.submit(&context, Some("Baked Payload Load"), &requests)
				.expect("DirectStorage should accept every baked block");
			ticket.wait().expect("DirectStorage should decode every baked block");
			drop(ticket);
			queue.close_file(file);
			drop(queue);

			let copy = context.record(queue_handle, None, |recording| {
				recording
					.transfer_texture(destination.into())
					.expect("The DX12 test image should support readback")
			});
			let readback = context.get_image_data(copy).expect("The DX12 test readback should complete");
			let (bytes_per_row, row_count, _) = ghi_format.copy_layout(utils::Extent::rectangle(width, height)).unwrap();
			for row in 0..row_count {
				let expected = &decoded[row * bytes_per_row..(row + 1) * bytes_per_row];
				let actual = &readback.bytes[row * readback.bytes_per_row..row * readback.bytes_per_row + bytes_per_row];
				assert_eq!(actual, expected, "{format:?} row {row}");
			}
			std::fs::remove_file(&path).unwrap();
		}
	}

	#[test]
	fn single_level_images_need_no_stream_table() {
		assert_eq!(mip_range(None, 0, 1, 64), Some(0..64));
		assert_eq!(mip_range(None, 1, 2, 64), None);
		let streams = [
			crate::StreamDescription::new("mip[0]", 64, 0),
			crate::StreamDescription::new("mip[1]", 16, 64),
		];
		assert_eq!(mip_range(Some(&streams), 1, 2, 80), Some(64..80));
	}
}
