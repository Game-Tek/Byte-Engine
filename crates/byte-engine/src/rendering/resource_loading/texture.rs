//! Resource Manager to GHI utilities for ordinary sampled textures.
//!
//! Request an image through the Resource Manager, then pass its
//! [`Reference`] to [`TextureTransfer::load`]. The utility validates every mip,
//! chooses staged or native I/O, creates the GHI image and sampler, waits for
//! transfer completion, and returns [`LoadedTexture`]. Pipeline code keeps only
//! renderer policy such as request identity, bindless slots, and readiness.

use std::sync::{Arc, Mutex};

use ghi::Device as _;
use ghi::command_buffer::CommandBufferRecording as _;
use ghi::context::ContextCreate as _;
use ghi::io::{ResourceIoContext as _, ResourceIoQueue as _, ResourceIoTicket as _};
use resource_management::{
	Reference, StreamDescription,
	resource::{ReadTargets, ReadTargetsMut, ResourceGpuBacking, ResourcePayloadEncoding, ResourceReaderBacking},
	resources::image::Image as ResourceImage,
	stream::StreamMut,
	types::Formats as ResourceFormat,
};
use smallvec::SmallVec;
use utils::Extent;

use super::{StagingLease, UploadStagingArena};
use crate::rendering::{
	SharedContext,
	loading::{LoadPipeline, LoaderLane},
};

/// The `TextureAddressMode` enum selects how an ordinary sampled texture addresses coordinates outside its bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureAddressMode {
	/// Clamp sampling coordinates to the image edge.
	Clamp,
	/// Repeat the image outside its normalized coordinate range.
	Repeat,
}

/// The `TextureDescriptor` struct describes one sampled texture without exposing GHI builders.
pub struct TextureDescriptor<'a> {
	name: &'a str,
	address_mode: TextureAddressMode,
}

impl<'a> TextureDescriptor<'a> {
	/// Creates a linearly filtered, clamped texture description.
	///
	/// Next, pass this value to [`TextureTransfer::load`].
	pub fn new(name: &'a str) -> Self {
		Self {
			name,
			address_mode: TextureAddressMode::Clamp,
		}
	}

	/// Selects how sampling behaves outside the normalized image bounds.
	pub fn address_mode(mut self, address_mode: TextureAddressMode) -> Self {
		self.address_mode = address_mode;
		self
	}
}

/// The `LoadedTexture` struct identifies one upload-complete GHI image and sampler pair.
#[derive(Clone, Copy)]
pub struct LoadedTexture {
	image: ghi::BaseImageHandle,
	sampler: ghi::SamplerHandle,
}

impl LoadedTexture {
	/// Returns the image for renderer-owned descriptor publication.
	pub const fn image(self) -> ghi::BaseImageHandle {
		self.image
	}

	/// Returns the sampler for renderer-owned descriptor publication.
	pub const fn sampler(self) -> ghi::SamplerHandle {
		self.sampler
	}
}

/// The `TextureTransfer` struct turns Resource Manager image references into upload-complete GHI textures.
///
/// This is a utility owned by a pipeline loader, not an independent resource
/// loader. It has no request registry, lane pool, or renderer-visible state.
pub struct TextureTransfer {
	staging_buffer: ghi::BaseBufferHandle,
	io_queue: Option<Mutex<ghi::implementation::ResourceIoQueue>>,
}

impl TextureTransfer {
	/// Creates the texture utility used by one pipeline loader.
	///
	/// Native I/O remains optional so unsupported devices can still load staged
	/// textures. Next, pass Resource Manager image references to [`Self::load`].
	pub fn new(context: &SharedContext, staging_buffer: ghi::BaseBufferHandle, name: &str) -> Self {
		let io_queue = context
			.lock()
			.create_resource_io_queue(ghi::io::ResourceIoQueueDescriptor::new().name(name))
			.ok()
			.map(Mutex::new);
		Self {
			staging_buffer,
			io_queue,
		}
	}

	/// Loads every mip, creates the GHI image and sampler, and waits until the texture is ready.
	pub async fn load<P: LoadPipeline>(
		&self,
		reference: Reference<ResourceImage>,
		descriptor: TextureDescriptor<'_>,
		lane: &mut LoaderLane<P>,
	) -> Result<LoadedTexture, TextureTransferError> {
		let transfer = PreparedTextureTransfer::prepare(reference, lane.staging().clone())
			.await
			.map_err(|error| TextureTransferError(format!("Texture preparation failed for {}. {error}", descriptor.name)))?;
		let metadata = transfer.metadata;
		let factory = lane.factory();
		let image = factory.build_image(
			ghi::image::Builder::new(metadata.format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(descriptor.name)
				.extent(metadata.extent)
				.mip_levels(metadata.mip_count)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let addressing_mode = match descriptor.address_mode {
			TextureAddressMode::Clamp => ghi::SamplerAddressingModes::Clamp,
			TextureAddressMode::Repeat => ghi::SamplerAddressingModes::Repeat,
		};
		let sampler = factory.build_sampler(
			ghi::sampler::Builder::new()
				.addressing_mode(addressing_mode)
				.max_lod((metadata.mip_count - 1) as f32),
		);
		let (image, sampler) = lane.commit(|context| (context.intern_image(image).into(), context.intern_sampler(sampler)));
		self.upload(transfer, image, descriptor.name, lane)?;
		Ok(LoadedTexture { image, sampler })
	}

	/// Completes the storage-specific transfer after the destination image exists.
	fn upload<P: LoadPipeline>(
		&self,
		transfer: PreparedTextureTransfer,
		image: ghi::BaseImageHandle,
		name: &str,
		lane: &LoaderLane<P>,
	) -> Result<(), TextureTransferError> {
		let (metadata, source) = transfer.into_parts();
		match source {
			PreparedTextureSource::Staged(source) => {
				// `source` retains its staging lease until the lane has waited for GPU completion.
				lane.transfer(|recording| {
					recording.copy_buffer_to_images(&source.copy_descriptors(self.staging_buffer, image));
				});
			}
			PreparedTextureSource::Native(source) => {
				let compression = source.compression().map_err(|error| {
					TextureTransferError(format!("Texture native encoding is unsupported for {name}. {error}"))
				})?;
				let ticket = {
					let mut queue = self
						.io_queue
						.as_ref()
						.ok_or_else(|| {
							TextureTransferError(format!(
								"Texture native I/O is unavailable for {name}. The most likely cause is unsupported storage capabilities."
							))
						})?
						.lock()
						.unwrap_or_else(|error| error.into_inner());
					let file = queue
						.open_file(
							ghi::io::ResourceIoFileDescriptor::new(source.path())
								.compression(compression)
								.name(name),
						)
						.map_err(|error| {
							TextureTransferError(format!(
								"Texture I/O file could not be opened for {name}. The most likely cause is missing or unreadable native backing storage. {error}"
							))
						})?;
					let requests = source.requests(metadata, file, image).map_err(|error| {
						TextureTransferError(format!("Texture I/O requests are invalid for {name}. {error}"))
					})?;
					lane.commit(|context| queue.submit(context, Some(name), &requests))
						.map_err(|error| {
							TextureTransferError(format!(
								"Texture I/O submission failed for {name}. The most likely cause is an unsupported request or unavailable native queue. {error}"
							))
						})?
				};
				ticket.wait().map_err(|error| {
					TextureTransferError(format!(
						"Texture I/O failed for {name}. The most likely cause is unreadable or incompatible compressed texture data. {error}"
					))
				})?;
			}
		}
		Ok(())
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

/// The `PreparedTextureTransfer` struct keeps validated texture data alive until its GHI transfer completes.
struct PreparedTextureTransfer {
	metadata: TextureMetadata,
	source: PreparedTextureSource,
}

impl PreparedTextureTransfer {
	/// Prepares all persisted mips without choosing a renderer destination.
	///
	/// CPU-readable resources receive one exclusive staging lease with rows
	/// already padded for GHI copies. GPU-backed resources retain their native
	/// file and stream metadata without decoding on the CPU. The caller supplies
	/// logical identity when reporting [`TexturePreparationError`].
	async fn prepare(
		mut reference: Reference<ResourceImage>,
		staging: Arc<UploadStagingArena>,
	) -> Result<Self, TexturePreparationError> {
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

		Ok(Self { metadata, source })
	}

	/// Splits preparation into the renderer-creation metadata and delivery source.
	fn into_parts(self) -> (TextureMetadata, PreparedTextureSource) {
		(self.metadata, self.source)
	}
}

/// The `PreparedTextureSource` enum selects CPU staging or native GPU resource I/O.
///
/// [`TextureTransfer`] consumes this internal delivery choice after creating the destination.
enum PreparedTextureSource {
	/// CPU-readable bytes arranged for transfer command recording.
	Staged(StagedTextureUpload),
	/// Persisted GPU backing arranged for native resource-I/O submission.
	Native(NativeTextureUpload),
}

/// The `StagedTextureUpload` struct retains row-padded mip bytes through one loader transfer.
///
/// [`TextureTransfer`] keeps this value alive until
/// [`crate::rendering::loading::LoaderLane::transfer`] returns. Its staging
/// lease then returns to the arena automatically.
struct StagedTextureUpload {
	staging: StagingLease,
	layouts: SmallVec<[TextureUploadLayout; 16]>,
}

impl StagedTextureUpload {
	/// Builds every buffer-to-image copy for validated renderer-selected destinations.
	fn copy_descriptors(
		&self,
		staging_buffer: ghi::BaseBufferHandle,
		image: ghi::BaseImageHandle,
	) -> SmallVec<[ghi::BufferImageCopyDescriptor; 16]> {
		self.layouts
			.iter()
			.enumerate()
			.map(|(mip_level, layout)| layout.copy_descriptor(staging_buffer, self.staging.offset(), image, mip_level as u32))
			.collect()
	}
}

/// The `NativeTextureUpload` struct retains a persisted GPU source and decoded mip ranges.
///
/// [`TextureTransfer`] opens the backing file and retains the resulting
/// ticket until completion before the pipeline publishes its resident value.
struct NativeTextureUpload {
	backing: ResourceGpuBacking,
	streams: Option<Vec<StreamDescription>>,
}

impl NativeTextureUpload {
	/// Returns the persisted file consumed by the native storage queue.
	fn path(&self) -> &std::path::Path {
		self.backing.path()
	}

	/// Returns the native decompression method declared by resource storage.
	fn compression(&self) -> Result<ghi::io::ResourceIoCompression, TexturePreparationError> {
		match self.backing.encoding() {
			ResourcePayloadEncoding::MetalIoLz4 => Ok(ghi::io::ResourceIoCompression::Lz4),
			ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => Err(TexturePreparationError::NativeEncoding),
		}
	}

	/// Builds one native request per persisted mip for one destination image.
	fn requests(
		&self,
		metadata: TextureMetadata,
		file: ghi::io::ResourceIoFileHandle,
		image: ghi::BaseImageHandle,
	) -> Result<SmallVec<[ghi::io::ResourceIoRequest; 16]>, TexturePreparationError> {
		let mut requests = SmallVec::new();
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
			let extent = texture_mip_extent(metadata.extent, mip_level);
			let (bytes_per_row, _, bytes_per_image) = metadata.format.compact_copy_layout(extent.width(), extent.height());
			requests.push(
				ghi::io::ResourceIoImageLoad::new(
					ghi::io::ResourceIoFileRegion::new(file, decoded_offset),
					image,
					0,
					mip_level,
					extent,
					bytes_per_row,
					bytes_per_image,
				)
				.into(),
			);
		}
		Ok(requests)
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
		let (compact_bytes_per_row, row_count, compact_bytes_per_image) =
			format.compact_copy_layout(extent.width().max(1), extent.height().max(1));
		let compact_size = compact_bytes_per_image.checked_mul(layer_count)?;
		let source_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
		let source_bytes_per_image = source_bytes_per_row.checked_mul(row_count)?;
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

	/// Builds one full-subresource copy descriptor for renderer-specific environment uploads.
	pub(crate) fn copy_descriptor(
		&self,
		staging_buffer: ghi::BaseBufferHandle,
		staging_offset: usize,
		image: ghi::BaseImageHandle,
		mip_level: u32,
	) -> ghi::BufferImageCopyDescriptor {
		ghi::BufferImageCopyDescriptor::new(
			staging_buffer,
			staging_offset + self.offset,
			self.source_bytes_per_row,
			self.source_bytes_per_image,
			image,
			mip_level,
		)
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
		let mut bytes = [0_u8; 16];
		bytes[..4].copy_from_slice(b"mip[");
		let mut digits = [0_u8; 10];
		let mut value = level;
		let mut digit_count = 0usize;
		loop {
			digits[digit_count] = b'0' + (value % 10) as u8;
			digit_count += 1;
			value /= 10;
			if value == 0 {
				break;
			}
		}
		for index in 0..digit_count {
			bytes[4 + index] = digits[digit_count - index - 1];
		}
		let len = digit_count + 5;
		bytes[len - 1] = b']';
		Self { bytes, len }
	}

	fn as_str(&self) -> &str {
		std::str::from_utf8(&self.bytes[..self.len]).expect("Mip stream names contain only ASCII bytes.")
	}
}

pub(crate) fn texture_mip_extent(base_extent: Extent, level: u32) -> Extent {
	debug_assert_eq!(
		base_extent.depth(),
		0,
		"Texture mip extent is not two-dimensional. The most likely cause is unvalidated image metadata."
	);
	Extent::rectangle((base_extent.width() >> level).max(1), (base_extent.height() >> level).max(1))
}

pub(crate) async fn load_image_streams<'a>(
	reference: &mut Reference<ResourceImage>,
	mut streams: SmallVec<[StreamMut<'a>; 16]>,
) -> Result<(), TexturePreparationError> {
	if reference.requires_cpu_decompression() {
		let loaded = reference
			.load(ReadTargetsMut::backing_storage())
			.await
			.map_err(|_| TexturePreparationError::Payload)?;
		let descriptions = reference.streams().ok_or(TexturePreparationError::Streams)?;
		let decoded = loaded.buffer().ok_or(TexturePreparationError::Payload)?;
		for stream in &mut streams {
			copy_decoded_stream(decoded, descriptions, stream)?;
		}
		return Ok(());
	}

	let loaded = reference
		.load(streams.into_vec().into())
		.await
		.map_err(|_| TexturePreparationError::Payload)?;
	if matches!(loaded, ReadTargets::Streams(_)) {
		Ok(())
	} else {
		Err(TexturePreparationError::Payload)
	}
}

async fn prepare_staged_texture(
	reference: &mut Reference<ResourceImage>,
	staging_arena: Arc<UploadStagingArena>,
	metadata: TextureMetadata,
) -> Result<StagedTextureUpload, TexturePreparationError> {
	let mut layouts = SmallVec::<[TextureUploadLayout; 16]>::new();
	let mut upload_byte_count = 0usize;
	for level in 0..metadata.mip_count {
		let mut layout = TextureUploadLayout::new(metadata.format, texture_mip_extent(metadata.extent, level), 1, 0)
			.ok_or(TexturePreparationError::Layout)?;
		layout.offset = upload_byte_count;
		upload_byte_count = upload_byte_count
			.checked_add(layout.padded_size)
			.ok_or(TexturePreparationError::Layout)?;
		layouts.push(layout);
	}
	let mut staging = staging_arena
		.allocate(upload_byte_count, 256)
		.await
		.ok_or(TexturePreparationError::StagingCapacity)?;
	load_texture_bytes(reference, &mut staging, &layouts).await?;
	for layout in &layouts {
		let range = layout.offset..layout.offset + layout.padded_size;
		layout.pack_rows(&mut staging.bytes_mut()[range]);
	}
	Ok(StagedTextureUpload { staging, layouts })
}

fn copy_decoded_stream(
	decoded: &[u8],
	descriptions: &[StreamDescription],
	stream: &mut StreamMut<'_>,
) -> Result<(), TexturePreparationError> {
	let description = descriptions
		.iter()
		.find(|description| description.name() == stream.name())
		.ok_or(TexturePreparationError::Streams)?;
	if description.size() != stream.buffer().len() {
		return Err(TexturePreparationError::Streams);
	}
	let end = description
		.offset()
		.checked_add(description.size())
		.ok_or(TexturePreparationError::Streams)?;
	let source = decoded
		.get(description.offset()..end)
		.ok_or(TexturePreparationError::Streams)?;
	stream.buffer_mut().copy_from_slice(source);
	Ok(())
}

fn texture_payload_is_compact(
	decoded_size: usize,
	descriptions: Option<&[StreamDescription]>,
	stream_names: &[MipStreamName],
	layouts: &[TextureUploadLayout],
) -> bool {
	let Some(descriptions) = descriptions else {
		return false;
	};
	let mut offset = 0usize;
	for (name, layout) in stream_names.iter().zip(layouts) {
		let Some(description) = descriptions.iter().find(|description| description.name() == name.as_str()) else {
			return false;
		};
		if description.offset() != offset || description.size() != layout.compact_size {
			return false;
		}
		let Some(next_offset) = offset.checked_add(layout.compact_size) else {
			return false;
		};
		offset = next_offset;
	}
	offset == decoded_size
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
	use std::path::PathBuf;

	use resource_management::{
		ReferenceModel,
		resource::{ResourcePayloadEncoding, reader::redb::FileResourceReader},
		resources::image::Image,
		types::Gamma,
	};

	use super::*;

	fn native_image_reference(depth: u32) -> Reference<ResourceImage> {
		let image = Image {
			format: ResourceFormat::BC5,
			gamma: Gamma::Linear,
			extent: [4, 4, depth],
			mip_count: 1,
			ibl: None,
			photometry: None,
		};
		let model = ReferenceModel::new("normal.image", 0, 16, &image, None);
		let reader = Box::new(FileResourceReader::new_gpu(
			PathBuf::from("normal.image"),
			ResourcePayloadEncoding::MetalIoLz4,
		));
		Reference::from_model(model, image, reader)
	}

	#[resource_management::r#async::test]
	async fn texture_preparation_accepts_only_zero_depth_for_two_dimensional_resources() {
		let bytes = Box::leak(vec![0_u8; 16].into_boxed_slice());
		let (staging, _worker) = UploadStagingArena::new_for_test(bytes);

		let prepared = PreparedTextureTransfer::prepare(native_image_reference(0), staging.clone())
			.await
			.expect("zero-depth image metadata should prepare");
		assert_eq!(prepared.metadata.extent, Extent::rectangle(4, 4));
		assert!(matches!(prepared.into_parts().1, PreparedTextureSource::Native(_)));
		assert!(matches!(
			PreparedTextureTransfer::prepare(native_image_reference(1), staging).await,
			Err(TexturePreparationError::Dimensions)
		));
	}

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
				let layout = TextureUploadLayout::new(metadata.format, texture_mip_extent(metadata.extent, level), 1, offset)
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
	fn row_packing_keeps_compact_texels_at_each_padded_row_start() {
		let layout =
			TextureUploadLayout::new(ghi::Formats::RGBA8UNORM, Extent::rectangle(2, 2), 1, 0).expect("valid texture layout");
		let mut bytes = vec![0; layout.padded_size];
		bytes[..16].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

		layout.pack_rows(&mut bytes);

		assert_eq!(&bytes[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
		assert_eq!(&bytes[256..264], &[9, 10, 11, 12, 13, 14, 15, 16]);
	}

	#[test]
	fn decoded_stream_copy_uses_explicit_named_ranges() {
		let decoded = [10_u8, 11, 12, 13, 14, 15];
		let descriptions = [StreamDescription::new("mip[0]", 3, 2)];
		let mut destination = [0_u8; 3];
		{
			let mut stream = StreamMut::new("mip[0]", &mut destination);
			copy_decoded_stream(&decoded, &descriptions, &mut stream).unwrap();
		}

		assert_eq!(destination, [12, 13, 14]);
		let mut missing = StreamMut::new("missing", &mut destination);
		assert_eq!(
			copy_decoded_stream(&decoded, &descriptions, &mut missing),
			Err(TexturePreparationError::Streams)
		);
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

	#[test]
	fn srgb_resource_format_preserves_srgb_gpu_sampling() {
		assert_eq!(resource_format_to_ghi(ResourceFormat::RGBA8SRGB), ghi::Formats::RGBA8sRGB);
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

	/// Ensures half-float rows (IES intensity maps, HDR environments) reach the transfer buffer unchanged.
	#[test]
	fn texture_upload_preserves_half_float_rows() {
		for (format, resource_format, bytes_per_texel) in [
			(ghi::Formats::R16F, ResourceFormat::R16F, 2),
			(ghi::Formats::RGBA16F, ResourceFormat::RGBA16F, 8),
		] {
			let compact_row = 2 * bytes_per_texel;
			let source = (0..compact_row * 2).map(|value| value as u8).collect::<Vec<_>>();
			let (data, upload) = staged_texture_bytes(format, Extent::rectangle(2, 2), 1, &source);

			assert_eq!(resource_format_to_ghi(resource_format), format);
			assert_eq!(upload.source_bytes_per_row, 256);
			assert_eq!(upload.source_bytes_per_image, 512);
			assert_eq!(&data[..compact_row], &source[..compact_row]);
			assert_eq!(&data[256..256 + compact_row], &source[compact_row..]);
		}
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
