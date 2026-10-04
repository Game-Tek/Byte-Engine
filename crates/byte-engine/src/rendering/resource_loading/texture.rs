//! Resource Manager to GHI utilities for ordinary sampled textures.
//!
//! Request an image through the Resource Manager, then pass its [`Reference`] to [`load_texture`] from a loader
//! lane. The utility validates every mip, chooses staged or native I/O, hands the upload to the
//! [`Loader`](crate::rendering::loading::Loader), and returns the finished image detached. Pipeline code keeps only
//! renderer policy such as request identity, bindless slots, samplers, and readiness.

use std::sync::Arc;

use resource_management::{
	Reference, StreamDescription,
	resource::{ReadTargets, ResourceGpuBacking, ResourcePayloadEncoding, ResourceReaderBacking},
	resources::image::Image as ResourceImage,
	stream::StreamMut,
	types::Formats as ResourceFormat,
};
use smallvec::SmallVec;
use utils::Extent;

use super::{StagingLease, UploadStagingArena};
use crate::rendering::loading::{
	ImageDescription, ImageRegion, ImageUpload, LoadPipeline, LoaderLane, NativeImageRegion, NativeImageUpload,
};

/// Loads every mip of an image resource through the loader and returns the finished image.
///
/// CPU-readable resources are staged and copied. GPU-backed resources are read straight from their file through
/// native I/O. Next, return the image in the pipeline's resident value so the render thread interns it and binds
/// it with a sampler the pipeline owns.
pub async fn load_texture<P: LoadPipeline>(
	reference: Reference<ResourceImage>,
	name: &str,
	lane: &LoaderLane<P>,
) -> Result<ghi::implementation::DetachedImage, TextureTransferError> {
	let (metadata, source) = prepare_texture(reference, lane.staging().clone())
		.await
		.map_err(|error| TextureTransferError(format!("Texture preparation failed for {name}. {error}")))?;
	let description = ImageDescription {
		name: name.to_owned(),
		format: metadata.format,
		extent: metadata.extent,
		mip_levels: metadata.mip_count,
		cube: false,
	};
	match source {
		PreparedTextureSource::Staged(source) => {
			// Every mip sits at its own offset inside the staging lease.
			let regions = source
				.layouts
				.iter()
				.enumerate()
				.map(|(mip_level, layout)| layout.region(mip_level as u32))
				.collect();
			let [image] = lane
				.upload(source.staging, [ImageUpload { description, regions }], SmallVec::new())
				.await
				.map_err(|error| TextureTransferError(format!("Texture upload failed for {name}. {error}")))?;
			Ok(image)
		}
		PreparedTextureSource::Native(source) => {
			let compression = source
				.compression()
				.map_err(|error| TextureTransferError(format!("Texture native encoding is unsupported for {name}. {error}")))?;
			let regions = source
				.regions(metadata)
				.map_err(|error| TextureTransferError(format!("Texture I/O requests are invalid for {name}. {error}")))?;
			lane.upload_native_image(NativeImageUpload {
				description,
				path: source.backing.path().to_owned(),
				compression,
				regions,
			})
			.await
			.map_err(|error| TextureTransferError(format!("Texture I/O failed for {name}. {error}")))
		}
	}
}

/// The `TextureTransferError` struct reports why a Resource Manager image could not become a GHI texture.
#[derive(Debug)]
pub struct TextureTransferError(String);

impl std::fmt::Display for TextureTransferError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl std::error::Error for TextureTransferError {}

/// The `TextureMetadata` struct keeps validated image shape private to the texture utility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextureMetadata {
	format: ghi::Formats,
	extent: Extent,
	mip_count: u32,
}

/// Prepares all persisted mips without choosing a renderer destination, and returns the validated image shape with
/// the source the loader uploads from.
///
/// CPU-readable resources receive one exclusive staging lease with rows
/// already padded for GHI copies. GPU-backed resources retain their native
/// file and stream metadata without decoding on the CPU. The caller supplies
/// logical identity when reporting [`TexturePreparationError`].
async fn prepare_texture(
	mut reference: Reference<ResourceImage>,
	staging: Arc<UploadStagingArena>,
) -> Result<(TextureMetadata, PreparedTextureSource), TexturePreparationError> {
	let image = reference.resource();
	let [width, height, depth] = image.extent;
	if width == 0 || height == 0 || depth != 0 {
		return Err(TexturePreparationError::Dimensions);
	}
	let mip_count = image.mip_count.max(1);
	let available_mips = u32::BITS - width.max(height).leading_zeros();
	if mip_count > available_mips {
		return Err(TexturePreparationError::MipCount);
	}
	let metadata = TextureMetadata {
		format: resource_format_to_ghi(image.format),
		extent: Extent::rectangle(width, height),
		mip_count,
	};

	let source = if reference.is_gpu_backed() {
		let streams = reference.streams().map(<[StreamDescription]>::to_vec);
		let backing = reference
			.consume_reader()
			.into_backing_storage()
			.await
			.map_err(|_| TexturePreparationError::NativeBacking)?;
		let ResourceReaderBacking::Gpu(backing) = backing else {
			return Err(TexturePreparationError::NativeBacking);
		};
		PreparedTextureSource::Native(NativeTextureUpload { backing, streams })
	} else {
		PreparedTextureSource::Staged(prepare_staged_texture(&mut reference, staging, metadata).await?)
	};

	Ok((metadata, source))
}

/// The `PreparedTextureSource` enum selects CPU staging or native GPU resource I/O, and keeps the validated texture
/// data alive until the loader uploads it.
///
/// [`load_texture`] turns this internal delivery choice into the matching loader upload.
enum PreparedTextureSource {
	/// CPU-readable bytes arranged for transfer command recording.
	Staged(StagedTextureUpload),
	/// Persisted GPU backing arranged for native resource-I/O submission.
	Native(NativeTextureUpload),
}

/// The `StagedTextureUpload` struct keeps row-padded mip bytes until the loader copies them.
///
/// [`load_texture`] hands the lease to the loader, which returns it to the arena once the copies finished.
struct StagedTextureUpload {
	staging: StagingLease,
	layouts: SmallVec<[TextureUploadLayout; 16]>,
}

/// The `NativeTextureUpload` struct retains a persisted GPU source and decoded mip ranges.
///
/// The loader opens the backing file and waits for its reads before [`load_texture`] returns.
struct NativeTextureUpload {
	backing: ResourceGpuBacking,
	streams: Option<Vec<StreamDescription>>,
}

impl NativeTextureUpload {
	/// Returns the native decompression method declared by resource storage.
	fn compression(&self) -> Result<ghi::io::ResourceIoCompression, TexturePreparationError> {
		match self.backing.encoding() {
			ResourcePayloadEncoding::MetalIoLz4 => Ok(ghi::io::ResourceIoCompression::Lz4),
			ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => Err(TexturePreparationError::NativeEncoding),
		}
	}

	/// Locates every persisted mip inside the decoded file stream.
	fn regions(&self, metadata: TextureMetadata) -> Result<SmallVec<[NativeImageRegion; 16]>, TexturePreparationError> {
		let mut regions = SmallVec::new();
		for mip_level in 0..metadata.mip_count {
			let name = MipStreamName::new(mip_level);
			let decoded_offset = match self.streams.as_deref() {
				Some(streams) => streams
					.iter()
					.find(|stream| stream.name() == name.as_str())
					.map(StreamDescription::offset)
					.ok_or(TexturePreparationError::Streams)?,
				None if metadata.mip_count == 1 => 0,
				None => return Err(TexturePreparationError::Streams),
			};
			let extent = metadata.extent.mip(mip_level);
			let (bytes_per_row, _, bytes_per_image) = metadata.format.compact_copy_layout(extent.width(), extent.height());
			regions.push(NativeImageRegion {
				file_offset: decoded_offset,
				mip_level,
				extent,
				bytes_per_row,
				bytes_per_image,
			});
		}
		Ok(regions)
	}
}

/// The `TextureUploadLayout` struct keeps one mip's compact and GPU-aligned byte geometry consistent.
///
/// Texture preparation owns this internal representation so resource reading
/// and copy recording cannot derive different offsets or row pitches.
#[derive(Clone, Copy)]
pub(crate) struct TextureUploadLayout {
	pub(crate) offset: usize,
	pub(crate) compact_bytes_per_row: usize,
	pub(crate) row_count: usize,
	pub(crate) compact_bytes_per_image: usize,
	pub(crate) compact_size: usize,
	pub(crate) source_bytes_per_row: usize,
	pub(crate) source_bytes_per_image: usize,
	pub(crate) padded_size: usize,
}

impl TextureUploadLayout {
	/// Computes one GPU-row-aligned staging range and rejects arithmetic overflow.
	pub(crate) fn new(format: ghi::Formats, extent: Extent, layer_count: usize, offset: usize) -> Option<Self> {
		let (compact_bytes_per_row, row_count, compact_bytes_per_image) = format.copy_layout(extent)?;
		let compact_size = compact_bytes_per_image.checked_mul(layer_count)?;
		// Rows are padded the way every backend's buffer-to-texture copy reads them.
		let (source_bytes_per_row, source_bytes_per_image) = ghi::aligned_copy_pitches(compact_bytes_per_row, row_count)?;
		let padded_size = source_bytes_per_image.checked_mul(layer_count)?;
		Some(Self {
			offset,
			compact_bytes_per_row,
			row_count,
			compact_bytes_per_image,
			compact_size,
			source_bytes_per_row,
			source_bytes_per_image,
			padded_size,
		})
	}

	/// Expands compact rows backward inside one final padded staging range.
	pub(crate) fn pack_rows(&self, bytes: &mut [u8]) {
		assert_eq!(bytes.len(), self.padded_size);
		let layer_count = self.compact_size / self.compact_bytes_per_image;
		for layer in (0..layer_count).rev() {
			for row in (0..self.row_count).rev() {
				let source = layer * self.compact_bytes_per_image + row * self.compact_bytes_per_row;
				let destination = layer * self.source_bytes_per_image + row * self.source_bytes_per_row;
				bytes.copy_within(source..source + self.compact_bytes_per_row, destination);
			}
		}
	}

	/// Places this layout inside its staging lease as the full subresource of one mip.
	pub(crate) fn region(&self, mip_level: u32) -> ImageRegion {
		ImageRegion {
			offset: self.offset,
			bytes_per_row: self.source_bytes_per_row,
			bytes_per_image: self.source_bytes_per_image,
			mip_level,
		}
	}
}

/// Errors produced while validating or preparing baked texture transfer data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TexturePreparationError {
	/// The resource is zero-sized or is not a 2D image.
	Dimensions,
	/// The declared mip count exceeds the image dimensions.
	MipCount,
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
			Self::Dimensions => {
				"Texture dimensions are unsupported. The most likely cause is a zero-sized or non-2D baked image."
			}
			Self::MipCount => {
				"Texture mip metadata is invalid. The most likely cause is a declared mip count larger than its dimensions permit."
			}
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
		write!(unwritten, "mip[{level}]").expect("A `mip[u32]` name fits in 16 bytes.");
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

async fn prepare_staged_texture(
	reference: &mut Reference<ResourceImage>,
	staging_arena: Arc<UploadStagingArena>,
	metadata: TextureMetadata,
) -> Result<StagedTextureUpload, TexturePreparationError> {
	let mut layouts = SmallVec::<[TextureUploadLayout; 16]>::new();
	let mut upload_byte_count = 0usize;
	for level in 0..metadata.mip_count {
		let layout = TextureUploadLayout::new(metadata.format, metadata.extent.mip(level), 1, upload_byte_count)
			.ok_or(TexturePreparationError::Layout)?;
		upload_byte_count = upload_byte_count
			.checked_add(layout.padded_size)
			.ok_or(TexturePreparationError::Layout)?;
		layouts.push(layout);
	}
	let mut staging = staging_arena
		.allocate(upload_byte_count, ghi::TEXTURE_COPY_PITCH_ALIGNMENT)
		.await
		.ok_or(TexturePreparationError::StagingCapacity)?;
	load_texture_bytes(reference, &mut staging, &layouts).await?;
	for layout in &layouts {
		let range = layout.offset..layout.offset + layout.padded_size;
		layout.pack_rows(&mut staging.bytes_mut()[range]);
	}
	Ok(StagedTextureUpload { staging, layouts })
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
			if description.offset() != offset || description.size() != layout.compact_size {
				return None;
			}
			offset.checked_add(layout.compact_size)
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
			.checked_sub(layout.compact_size)
			.ok_or(TexturePreparationError::Layout)?;
		let destination_end = layout
			.offset
			.checked_add(layout.compact_size)
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

async fn load_texture_bytes(
	reference: &mut Reference<ResourceImage>,
	staging: &mut StagingLease,
	layouts: &[TextureUploadLayout],
) -> Result<(), TexturePreparationError> {
	if let [layout] = layouts
		&& (!reference.requires_cpu_decompression() || reference.size == layout.compact_size)
	{
		return load_texture_into(reference, &mut staging.bytes_mut()[..layout.compact_size]).await;
	}

	let stream_names: [MipStreamName; u32::BITS as usize] = std::array::from_fn(|level| MipStreamName::new(level as u32));
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
		let region = allocator.take(layout.padded_size);
		streams.push(StreamMut::new(name.as_str(), &mut region[..layout.compact_size]));
	}
	load_image_streams(reference, streams).await
}

pub(crate) fn resource_format_to_ghi(format: ResourceFormat) -> ghi::Formats {
	match format {
		ResourceFormat::RG8 => ghi::Formats::RG8UNORM,
		ResourceFormat::RG16 => ghi::Formats::RG16UNORM,
		ResourceFormat::R16F => ghi::Formats::R16F,
		ResourceFormat::RGB8 => ghi::Formats::RGB8UNORM,
		ResourceFormat::RGB16 => ghi::Formats::RGB16UNORM,
		ResourceFormat::RGBA8 => ghi::Formats::RGBA8UNORM,
		ResourceFormat::RGBA16 => ghi::Formats::RGBA16UNORM,
		ResourceFormat::RGBA16F => ghi::Formats::RGBA16F,
		ResourceFormat::RGBA8SRGB => ghi::Formats::RGBA8sRGB,
		ResourceFormat::BC5 => ghi::Formats::BC5,
		ResourceFormat::BC5SNORM => ghi::Formats::BC5SNORM,
		ResourceFormat::BC7 => ghi::Formats::BC7,
		ResourceFormat::BC7SRGB => ghi::Formats::BC7SRGB,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn texture_layout_preserves_every_mip_and_gpu_row_pitch() {
		let metadata = TextureMetadata {
			format: ghi::Formats::RGBA8UNORM,
			extent: Extent::rectangle(17, 3),
			mip_count: 3,
		};
		let mut offset = 0;
		let layouts = (0..metadata.mip_count)
			.map(|level| {
				let layout = TextureUploadLayout::new(metadata.format, metadata.extent.mip(level), 1, offset)
					.expect("valid texture layout");
				offset += layout.padded_size;
				layout
			})
			.collect::<SmallVec<[_; 16]>>();

		assert_eq!(layouts.len(), 3);
		assert_eq!(layouts[0].compact_bytes_per_row, 68);
		assert_eq!(layouts[0].source_bytes_per_row, 256);
		assert_eq!(layouts[0].source_bytes_per_image, 768);
		assert_eq!(layouts[1].offset, layouts[0].padded_size);
		assert_eq!(layouts[2].offset, layouts[0].padded_size + layouts[1].padded_size);
	}

	#[test]
	fn compact_mips_expand_into_padded_regions_without_scratch_storage() {
		let layouts = [
			TextureUploadLayout {
				offset: 0,
				compact_bytes_per_row: 4,
				row_count: 1,
				compact_bytes_per_image: 4,
				compact_size: 4,
				source_bytes_per_row: 8,
				source_bytes_per_image: 8,
				padded_size: 8,
			},
			TextureUploadLayout {
				offset: 8,
				compact_bytes_per_row: 2,
				row_count: 1,
				compact_bytes_per_image: 2,
				compact_size: 2,
				source_bytes_per_row: 4,
				source_bytes_per_image: 4,
				padded_size: 4,
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

	/// Lays `source` out as the GPU expects it: compact rows expanded to the padded row pitch.
	fn staged_texture_bytes(
		format: ghi::Formats,
		extent: Extent,
		layer_count: usize,
		source: &[u8],
	) -> (Vec<u8>, TextureUploadLayout) {
		let layout = TextureUploadLayout::new(format, extent, layer_count, 0).expect("texture layout");
		assert_eq!(source.len(), layout.compact_size);
		let mut bytes = vec![0u8; layout.padded_size];
		bytes[..source.len()].copy_from_slice(source);
		layout.pack_rows(&mut bytes);
		(bytes, layout)
	}

	#[test]
	fn texture_upload_preserves_minimum_extent_and_bc_row_contents() {
		let compact_row = 2 * 16;
		let source = (0..(compact_row * 2)).map(|value| value as u8).collect::<Vec<_>>();
		let (data, upload) = staged_texture_bytes(ghi::Formats::BC7, Extent::rectangle(5, 7), 1, &source);

		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 256 * 2);
		assert_eq!(&data[0..compact_row], &source[0..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..compact_row * 2]);

		let (zero_data, zero_extent) =
			staged_texture_bytes(ghi::Formats::RGBA8UNORM, Extent::rectangle(0, 0), 1, &[1, 2, 3, 4]);
		assert_eq!(zero_extent.source_bytes_per_row, 256);
		assert_eq!(zero_extent.source_bytes_per_image, 256);
		assert_eq!(&zero_data[..4], &[1, 2, 3, 4]);
	}

	#[test]
	fn cubemap_upload_preserves_every_face_and_image_pitch() {
		let compact_face_size = 2 * 2 * 8;
		let source = (0..compact_face_size * 6).map(|value| value as u8).collect::<Vec<_>>();
		let (data, upload) = staged_texture_bytes(ghi::Formats::RGBA16F, Extent::square(2), 6, &source);

		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(data.len(), 512 * 6);
		for face in 0..6 {
			for row in 0..2 {
				let source_start = face * compact_face_size + row * 16;
				let upload_start = face * 512 + row * 256;
				assert_eq!(
					&data[upload_start..upload_start + 16],
					&source[source_start..source_start + 16]
				);
			}
		}
	}
}
