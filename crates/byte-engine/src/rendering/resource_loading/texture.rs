//! Resource Manager to GHI utilities for ordinary sampled textures.
//!
//! Request an image through the Resource Manager, then pass its [`Reference`] to [`load_texture`] from a loader
//! lane. The utility validates every mip, chooses staged or native I/O, hands the upload to the
//! [`Loader`](crate::rendering::loading::Loader), and returns the finished image detached. Pipeline code keeps only
//! renderer policy such as request identity, bindless slots, samplers, and readiness.

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

use std::sync::Arc;

use resource_management::{
	Reference, StreamDescription,
	resource::{ReadTargets, ResourceGpuBacking, ResourcePayloadEncoding, ResourceReaderBacking},
	resources::image::Image as ResourceImage,
	stream::StreamMut,
};
use smallvec::SmallVec;
use utils::Extent;

use super::{StagingLease, UploadStagingArena};
use crate::rendering::loading::{
	ImageDescription, ImageRegion, ImageUpload, LoadError, LoadPipeline, LoaderLane, NativeImageRegion, NativeImageUpload,
};

/// Loads the mips of an image resource from `first_level` on through the loader and returns the finished image.
///
/// The image's base level is the resource's mip `first_level`, so pass 0 for the whole chain, or a later level to load
/// only the smaller mips. Samplers pick mips from the image's own extent, so either image samples like the resource.
/// CPU-readable resources are staged and copied. GPU-backed resources are read straight from their file through
/// native I/O. Next, return the image in the pipeline's resident value so the render thread interns it and binds
/// it with a sampler the pipeline owns.
///
/// # Panics
///
/// Panics when `first_level` is past the resource's last mip.
pub async fn load_texture<P: LoadPipeline>(
	mut reference: Reference<ResourceImage>,
	name: &str,
	first_level: u32,
	lane: &LoaderLane<P>,
) -> Result<ghi::implementation::DetachedImage, LoadError> {
	let failed = |stage: &str, error: &dyn std::fmt::Display| LoadError(format!("Texture {stage} failed for {name}. {error}"));
	// The baker writes 2D images whose mip count fits their extent.
	let image = reference.resource();
	let [width, height, _] = image.extent;
	let mip_count = image.mip_count.max(1);
	assert!(
		first_level < mip_count,
		"Texture first mip level {first_level} is past the last of the {mip_count} mips of {name}. The most likely cause is that the caller chose the first level without reading the resource's mip count. Pass a level below the mip count."
	);
	let metadata = TextureMetadata {
		format: image.format.ghi(),
		extent: Extent::rectangle(width, height).mip(first_level),
		mip_count: mip_count - first_level,
		first_level,
	};
	let description = ImageDescription {
		name: name.to_owned(),
		format: metadata.format,
		extent: metadata.extent,
		mip_levels: metadata.mip_count,
		cube: false,
	};

	if !reference.is_gpu_backed() {
		let (staging, layouts) = prepare_staged_texture(&mut reference, lane.staging().clone(), metadata)
			.await
			.map_err(|error| failed("preparation", &error))?;
		// Every mip sits at its own offset inside the staging lease.
		let regions = (0..)
			.zip(&layouts)
			.map(|(mip_level, layout)| layout.region(mip_level))
			.collect();
		let [image] = lane
			.upload(staging, [ImageUpload { description, regions }], SmallVec::new())
			.await
			.map_err(|error| failed("upload", &error))?;
		return Ok(image);
	}

	let Ok(ResourceReaderBacking::Gpu(backing)) = reference.consume_reader().into_backing_storage().await else {
		return Err(failed("preparation", &TexturePreparationError::NativeBacking));
	};
	// The reference keeps its stream table after giving up its reader, so the upload reads the table in place.
	let source = NativeTextureUpload { backing, reference };
	let compression = source.compression().map_err(|error| failed("native encoding", &error))?;
	let regions = source.regions(metadata).map_err(|error| failed("I/O request", &error))?;
	lane.upload_native_image(NativeImageUpload {
		description,
		path: source.backing.path().to_owned(),
		compression,
		regions,
	})
	.await
	.map_err(|error| failed("I/O", &error))
}

/// The `TextureMetadata` struct keeps the shape of the image being loaded private to the texture utility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextureMetadata {
	format: ghi::Formats,
	extent: Extent,
	mip_count: u32,
	/// The resource mip that becomes the image's base level, which names the stream each image level reads.
	first_level: u32,
}

/// The `NativeTextureUpload` struct retains a persisted GPU source and the reference whose stream table locates each mip.
///
/// The loader opens the backing file and waits for its reads before [`load_texture`] returns.
struct NativeTextureUpload {
	backing: ResourceGpuBacking,
	reference: Reference<ResourceImage>,
}

impl NativeTextureUpload {
	/// Returns the native decompression method declared by resource storage.
	fn compression(&self) -> Result<ghi::io::ResourceIoCompression, TexturePreparationError> {
		match self.backing.encoding() {
			ResourcePayloadEncoding::MetalIoLz4 => Ok(ghi::io::ResourceIoCompression::Lz4),
			ResourcePayloadEncoding::DirectStorageGDeflate => Ok(ghi::io::ResourceIoCompression::GDeflate1),
			ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => Err(TexturePreparationError::NativeEncoding),
		}
	}

	/// Locates every loaded region inside the native file: one band per stored block for a block-addressed payload,
	/// or one whole mip per decoded stream for a container addressed by decoded offset.
	fn regions(&self, metadata: TextureMetadata) -> Result<SmallVec<[NativeImageRegion; 16]>, TexturePreparationError> {
		if self.backing.encoding() == ResourcePayloadEncoding::DirectStorageGDeflate {
			return block_regions(self.backing.blocks(), metadata);
		}
		let mut regions = SmallVec::new();
		for mip_level in 0..metadata.mip_count {
			let name = MipStreamName::new(metadata.first_level + mip_level);
			let decoded_offset = match self.reference.streams() {
				Some(streams) => streams
					.iter()
					.find(|stream| stream.name() == name.as_str())
					.map(StreamDescription::offset)
					.ok_or(TexturePreparationError::Streams)?,
				// Only a single-level resource may store its one level without a stream table.
				None if metadata.mip_count == 1 && metadata.first_level == 0 => 0,
				None => return Err(TexturePreparationError::Streams),
			};
			regions.push(NativeImageRegion {
				file_offset: decoded_offset,
				stored: None,
				mip_level,
				first_row: 0,
				extent: metadata.extent.mip(mip_level),
			});
		}
		Ok(regions)
	}
}

/// Turns the stored blocks of the loaded mips into native regions, skipping mips below `metadata.first_level`.
///
/// Each block is one band of rows of one mip, so its region starts at the band's first row and spans the mip's width.
/// Every loaded mip must be covered by at least one block.
fn block_regions(
	blocks: &[resource_management::resource::GpuPayloadBlock],
	metadata: TextureMetadata,
) -> Result<SmallVec<[NativeImageRegion; 16]>, TexturePreparationError> {
	let mut regions = SmallVec::new();
	let mut covered_rows = SmallVec::<[u32; 16]>::from_elem(0, metadata.mip_count as usize);
	for block in blocks {
		let Some(mip_level) = block.mip_level.checked_sub(metadata.first_level) else {
			continue;
		};
		if mip_level >= metadata.mip_count {
			return Err(TexturePreparationError::Streams);
		}
		let extent = metadata.extent.mip(mip_level);
		let end_row = block
			.row_origin
			.checked_add(block.row_count)
			.ok_or(TexturePreparationError::Streams)?;
		if end_row > extent.height().max(1) {
			return Err(TexturePreparationError::Streams);
		}
		covered_rows[mip_level as usize] += block.row_count;
		regions.push(NativeImageRegion {
			file_offset: 0,
			stored: Some((block.stored_offset, block.stored_size as usize)),
			mip_level,
			first_row: block.row_origin,
			extent: Extent::rectangle(extent.width(), block.row_count),
		});
	}
	// Bands never overlap, so every mip is fully written exactly when its band rows add up to its height.
	let complete = covered_rows
		.iter()
		.enumerate()
		.all(|(mip_level, rows)| *rows == metadata.extent.mip(mip_level as u32).height().max(1));
	complete.then_some(regions).ok_or(TexturePreparationError::Streams)
}

/// The `TextureUploadLayout` struct keeps one mip's compact and GPU-aligned byte geometry consistent.
///
/// Texture preparation owns this internal representation so resource reading
/// and copy recording cannot derive different offsets or row pitches.
#[derive(Clone, Copy)]
pub(crate) struct TextureUploadLayout {
	pub(crate) offset: usize,
	pub(crate) copy: ghi::ImageCopyLayout,
}

impl TextureUploadLayout {
	/// Computes one GPU-row-aligned staging range and rejects arithmetic overflow.
	pub(crate) fn new(format: ghi::Formats, extent: Extent, layer_count: usize, offset: usize) -> Option<Self> {
		Some(Self {
			offset,
			copy: ghi::ImageCopyLayout::new(format, extent, layer_count)?,
		})
	}

	/// Places this layout inside its staging lease as the full subresource of one mip.
	pub(crate) fn region(&self, mip_level: u32) -> ImageRegion {
		ImageRegion {
			offset: self.offset,
			bytes_per_row: self.copy.padded_bytes_per_row,
			bytes_per_image: self.copy.padded_bytes_per_image,
			mip_level,
		}
	}
}

/// Errors produced while validating or preparing baked texture transfer data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TexturePreparationError {
	/// Size arithmetic or staging placement overflowed.
	Layout,
	/// The complete padded mip chain does not fit the supplied staging arena.
	StagingCapacity,
	/// Named mip stream metadata is missing or inconsistent.
	Streams,
	/// CPU-readable payload bytes could not be decoded or read.
	Payload,
	/// GPU-backed storage did not return its persisted native source.
	NativeBacking,
	/// Native backing declared a CPU-only resource encoding.
	NativeEncoding,
}

impl std::fmt::Display for TexturePreparationError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(match self {
			Self::Layout => {
				"Texture upload layout is invalid. The most likely cause is overflowing dimensions or inconsistent mip metadata."
			}
			Self::StagingCapacity => {
				"Texture exceeds upload staging capacity. The most likely cause is a padded mip chain larger than the configured arena."
			}
			Self::Streams => {
				"Texture mip streams are invalid. The most likely cause is missing or mismatched baked stream metadata."
			}
			Self::Payload => {
				"Texture payload could not be loaded. The most likely cause is missing, corrupt, or incorrectly encoded resource bytes."
			}
			Self::NativeBacking => {
				"Texture native backing could not be extracted. The most likely cause is inconsistent GPU encoding metadata or an unavailable persisted file."
			}
			Self::NativeEncoding => {
				"Texture native backing has an invalid encoding. The most likely cause is CPU-readable storage routed to a native GPU queue."
			}
		})
	}
}

impl std::error::Error for TexturePreparationError {}

/// The `MipStreamName` struct formats a baked mip identifier without a transient allocation.
struct MipStreamName {
	bytes: [u8; 16],
	len: usize,
}

impl MipStreamName {
	/// Formats one bounded `mip[level]` identifier without allocating.
	fn new(level: u32) -> Self {
		use std::io::Write as _;

		let mut bytes = [0_u8; 16];
		// Writing into the slice advances it, so what remains unwritten gives the length.
		let mut unwritten = &mut bytes[..];
		write!(unwritten, "mip[{level}]").expect(
			"Mip stream name overflowed its 16-byte buffer. The most likely cause is that MipStreamName's buffer was shrunk below the 15 bytes `mip[4294967295]` needs.",
		);
		let len = 16 - unwritten.len();
		Self { bytes, len }
	}

	fn as_str(&self) -> &str {
		std::str::from_utf8(&self.bytes[..self.len]).expect("Mip stream names contain only ASCII bytes.")
	}
}

/// Loads each named mip stream of an image into its destination, whatever the stored payload encoding.
pub(crate) async fn load_image_streams<'a>(
	reference: &mut Reference<ResourceImage>,
	streams: SmallVec<[StreamMut<'a>; 16]>,
) -> Result<(), TexturePreparationError> {
	let loaded = reference
		.load(streams.into_vec().into())
		.await
		.map_err(|_| TexturePreparationError::Payload)?;
	matches!(loaded, ReadTargets::Streams(_))
		.then_some(())
		.ok_or(TexturePreparationError::Payload)
}

/// Reads the mips `metadata` names into one staging lease, each at its own GPU-row-aligned offset, and returns the
/// lease with every mip's layout.
async fn prepare_staged_texture(
	reference: &mut Reference<ResourceImage>,
	staging_arena: Arc<UploadStagingArena>,
	metadata: TextureMetadata,
) -> Result<(StagingLease, SmallVec<[TextureUploadLayout; 16]>), TexturePreparationError> {
	let mut layouts = SmallVec::<[TextureUploadLayout; 16]>::new();
	let mut upload_byte_count = 0usize;
	for level in 0..metadata.mip_count {
		let layout = TextureUploadLayout::new(metadata.format, metadata.extent.mip(level), 1, upload_byte_count)
			.ok_or(TexturePreparationError::Layout)?;
		upload_byte_count = upload_byte_count
			.checked_add(layout.copy.padded_size)
			.ok_or(TexturePreparationError::Layout)?;
		layouts.push(layout);
	}
	let mut staging = staging_arena
		.allocate(upload_byte_count, ghi::TEXTURE_COPY_PITCH_ALIGNMENT)
		.await
		.ok_or(TexturePreparationError::StagingCapacity)?;
	load_texture_bytes(reference, &mut staging, &layouts, metadata.first_level).await?;
	for layout in &layouts {
		let range = layout.offset..layout.offset + layout.copy.padded_size;
		layout.copy.pack_rows(&mut staging.bytes_mut()[range]);
	}
	Ok((staging, layouts))
}

fn texture_payload_is_compact(
	decoded_size: usize,
	descriptions: Option<&[StreamDescription]>,
	stream_names: &[MipStreamName],
	layouts: &[TextureUploadLayout],
) -> bool {
	// Each mip's stream must start where the previous one ended, so together they end at the decoded size.
	let end = descriptions.and_then(|descriptions| {
		stream_names.iter().zip(layouts).try_fold(0usize, |offset, (name, layout)| {
			let description = descriptions.iter().find(|description| description.name() == name.as_str())?;
			if description.offset() != offset || description.size() != layout.copy.compact_size {
				return None;
			}
			offset.checked_add(layout.copy.compact_size)
		})
	});
	end == Some(decoded_size)
}

fn expand_compact_texture_levels(
	bytes: &mut [u8],
	decoded_size: usize,
	layouts: &[TextureUploadLayout],
) -> Result<(), TexturePreparationError> {
	let mut source_end = decoded_size;
	for layout in layouts.iter().rev() {
		let source_start = source_end
			.checked_sub(layout.copy.compact_size)
			.ok_or(TexturePreparationError::Layout)?;
		let destination_end = layout
			.offset
			.checked_add(layout.copy.compact_size)
			.ok_or(TexturePreparationError::Layout)?;
		if layout.offset < source_start || destination_end > bytes.len() {
			return Err(TexturePreparationError::Layout);
		}
		if layout.offset != source_start {
			bytes.copy_within(source_start..source_end, layout.offset);
		}
		source_end = source_start;
	}
	(source_end == 0).then_some(()).ok_or(TexturePreparationError::Layout)
}

async fn load_texture_into(
	reference: &mut Reference<ResourceImage>,
	destination: &mut [u8],
) -> Result<(), TexturePreparationError> {
	let expected_size = destination.len();
	let loaded = reference
		.load(destination.into())
		.await
		.map_err(|_| TexturePreparationError::Payload)?;
	if loaded.buffer().is_none_or(|buffer| buffer.len() != expected_size) {
		return Err(TexturePreparationError::Payload);
	}
	Ok(())
}

/// Reads the resource mips from `first_level` on into their staging layouts, one layout per loaded level.
async fn load_texture_bytes(
	reference: &mut Reference<ResourceImage>,
	staging: &mut StagingLease,
	layouts: &[TextureUploadLayout],
	first_level: u32,
) -> Result<(), TexturePreparationError> {
	// A single-level resource's payload is that level, so it reads whole. A later level alone must read its stream.
	if let [layout] = layouts
		&& first_level == 0
		&& (!reference.requires_cpu_decompression() || reference.size == layout.copy.compact_size)
	{
		return load_texture_into(reference, &mut staging.bytes_mut()[..layout.copy.compact_size]).await;
	}

	let stream_names: [MipStreamName; u32::BITS as usize] =
		std::array::from_fn(|level| MipStreamName::new(first_level + level as u32));
	if reference.requires_cpu_decompression()
		&& texture_payload_is_compact(reference.size, reference.streams(), &stream_names, layouts)
	{
		let decoded_size = reference.size;
		let destination = staging
			.bytes_mut()
			.get_mut(..decoded_size)
			.ok_or(TexturePreparationError::Layout)?;
		load_texture_into(reference, destination).await?;
		return expand_compact_texture_levels(staging.bytes_mut(), decoded_size, layouts);
	}

	let mut streams = SmallVec::new();
	let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
	for (name, layout) in stream_names.iter().zip(layouts) {
		let region = allocator.take(layout.copy.padded_size);
		streams.push(StreamMut::new(name.as_str(), &mut region[..layout.copy.compact_size]));
	}
	load_image_streams(reference, streams).await
}

#[cfg(test)]
mod tests {
	use resource_management::resource::GpuPayloadBlock;

	use super::*;

	fn block(mip_level: u32, row_origin: u32, row_count: u32, stored_offset: u64) -> GpuPayloadBlock {
		GpuPayloadBlock {
			mip_level,
			row_origin,
			row_count,
			decoded_size: 1024,
			stored_offset,
			stored_size: 512,
		}
	}

	fn metadata(first_level: u32) -> TextureMetadata {
		TextureMetadata {
			format: ghi::Formats::RGBA8UNORM,
			extent: Extent::rectangle(16, 8).mip(first_level),
			mip_count: 3 - first_level,
			first_level,
		}
	}

	/// Each stored band becomes one region of its mip, mips below the first level are skipped, and a mip its bands
	/// don't cover exactly is rejected.
	#[test]
	fn block_regions_follow_each_band_of_the_loaded_mips() {
		let blocks = [
			block(0, 0, 5, 100),
			block(0, 5, 3, 200),
			block(1, 0, 4, 300),
			block(2, 0, 2, 400),
		];
		let regions = block_regions(&blocks, metadata(0)).expect("Complete bands should become regions");
		assert_eq!(regions.len(), 4);
		assert_eq!((regions[1].mip_level, regions[1].first_row), (0, 5));
		assert_eq!(regions[1].extent, Extent::rectangle(16, 3));
		assert_eq!(regions[1].stored, Some((200, 512)));
		assert_eq!(regions[2].extent, Extent::rectangle(8, 4));

		let later = [block(0, 0, 8, 100), blocks[2], blocks[3]];
		let regions = block_regions(&later, metadata(1)).expect("Later mips should load on their own");
		assert_eq!((regions[0].mip_level, regions[0].stored), (0, Some((300, 512))));
		assert_eq!((regions[1].mip_level, regions[1].extent), (1, Extent::rectangle(4, 2)));

		for first_band in [block(0, 0, 5, 100), block(0, 0, 9, 100)] {
			let incomplete = [first_band, block(1, 0, 4, 300), block(2, 0, 2, 400)];
			assert_eq!(
				block_regions(&incomplete, metadata(0)).err(),
				Some(TexturePreparationError::Streams)
			);
		}
	}

	#[test]
	fn compact_mips_expand_into_padded_regions_without_scratch_storage() {
		let layouts = [
			TextureUploadLayout {
				offset: 0,
				copy: ghi::ImageCopyLayout {
					compact_bytes_per_row: 4,
					row_count: 1,
					compact_bytes_per_image: 4,
					compact_size: 4,
					padded_bytes_per_row: 8,
					padded_bytes_per_image: 8,
					padded_size: 8,
				},
			},
			TextureUploadLayout {
				offset: 8,
				copy: ghi::ImageCopyLayout {
					compact_bytes_per_row: 2,
					row_count: 1,
					compact_bytes_per_image: 2,
					compact_size: 2,
					padded_bytes_per_row: 4,
					padded_bytes_per_image: 4,
					padded_size: 4,
				},
			},
		];
		let names = [MipStreamName::new(0), MipStreamName::new(1)];
		let descriptions = [StreamDescription::new("mip[0]", 4, 0), StreamDescription::new("mip[1]", 2, 4)];
		let mut staging = [1_u8, 2, 3, 4, 5, 6, 0, 0, 0, 0, 0, 0];

		assert!(texture_payload_is_compact(6, Some(&descriptions), &names, &layouts));
		expand_compact_texture_levels(&mut staging, 6, &layouts).unwrap();
		assert_eq!(&staging[..4], &[1, 2, 3, 4]);
		assert_eq!(&staging[8..10], &[5, 6]);
	}
}
