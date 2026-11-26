//! Metal I/O implementation for direct file-to-resource loading.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_metal::{
	MTLDevice as _, MTLIOCommandBuffer as _, MTLIOCommandQueue as _, MTLIOCommandQueueDescriptor, MTLIOCommandQueueType,
	MTLIOCompressionMethod, MTLIOFileHandle, MTLIOPriority, MTLIOStatus,
};

use super::{context, mtl};
use crate::io::{
	FileSlots, ResourceIoBufferLoad, ResourceIoCapabilities, ResourceIoCompression, ResourceIoCompressionMethods,
	ResourceIoContext, ResourceIoDestinationKinds, ResourceIoError, ResourceIoFeatures, ResourceIoFileDescriptor,
	ResourceIoFileHandle, ResourceIoFileRegion, ResourceIoImageLoad, ResourceIoImageSourceLayout, ResourceIoPriority,
	ResourceIoQueueDescriptor, ResourceIoQueueType, ResourceIoRequest, ResourceIoSourceKinds, ResourceIoStatus,
	validate_source_range,
};

/// The `ResourceIoQueue` struct owns a Metal I/O command queue and its opened source files.
pub struct ResourceIoQueue {
	device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	queue: Retained<ProtocolObject<dyn mtl::MTLIOCommandQueue>>,
	/// Source files registered with this queue.
	files: FileSlots<Retained<ProtocolObject<dyn MTLIOFileHandle>>>,
	debug_labels: bool,
}

/// The `ResourceIoTicket` struct retains one submitted Metal I/O batch until callers finish observing it.
pub struct ResourceIoTicket {
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLIOCommandBuffer>>,
	cancellation_requested: AtomicBool,
}

// SAFETY: Metal I/O queues, file handles, and I/O command buffers are `!Send` only because objc2 cannot know an
// object's thread rules. Metal documents these as usable from any thread, and the engine's resource loaders create
// and submit batches away from the render thread.
//
// Ownership may move between threads; concurrent use may not. `Sync` is deliberately not claimed, so sharing either
// value requires a lock that hands out one borrow at a time.
unsafe impl Send for ResourceIoQueue {}
unsafe impl Send for ResourceIoTicket {}

impl ResourceIoQueue {
	/// Creates the native queue used by later file batches.
	fn new(context: &mut context::Context, descriptor: ResourceIoQueueDescriptor<'_>) -> Result<Self, ResourceIoError> {
		let native_descriptor = MTLIOCommandQueueDescriptor::new();
		native_descriptor.setPriority(match descriptor.priority {
			ResourceIoPriority::High => MTLIOPriority::High,
			ResourceIoPriority::Normal => MTLIOPriority::Normal,
			ResourceIoPriority::Low => MTLIOPriority::Low,
		});
		native_descriptor.setType(match descriptor.queue_type {
			ResourceIoQueueType::Concurrent => MTLIOCommandQueueType::Concurrent,
			ResourceIoQueueType::Serial => MTLIOCommandQueueType::Serial,
		});
		if descriptor.max_commands_in_flight > 0 {
			native_descriptor.setMaxCommandsInFlight(descriptor.max_commands_in_flight);
		}
		if descriptor.max_batches_in_flight > 0 {
			// SAFETY: A positive count is a valid Metal I/O command-buffer limit for this new descriptor.
			unsafe {
				native_descriptor.setMaxCommandBufferCount(descriptor.max_batches_in_flight);
			}
		}

		let device = context.device.clone();
		let queue = device
			.newIOCommandQueueWithDescriptor_error(&native_descriptor)
			.map_err(|error| ResourceIoError::QueueCreation(native_error_message(&error)))?;
		if context.settings.debug_labels {
			queue.setLabel(descriptor.name.map(NSString::from_str).as_deref());
		}

		Ok(Self {
			device,
			queue,
			files: FileSlots::new(),
			debug_labels: context.settings.debug_labels,
		})
	}

	fn file(&self, region: ResourceIoFileRegion) -> Result<&ProtocolObject<dyn MTLIOFileHandle>, ResourceIoError> {
		self.files.get(region.file).map(|file| &**file)
	}

	/// Validates and encodes one file-to-buffer request without committing the command buffer.
	fn encode_buffer_load(
		&self,
		context: &context::Context,
		command_buffer: &ProtocolObject<dyn mtl::MTLIOCommandBuffer>,
		request_index: usize,
		load: ResourceIoBufferLoad,
	) -> Result<(), ResourceIoError> {
		validate_source_range(request_index, load.source)?;
		let source = self.file(load.source)?;
		let destination = context
			.buffers
			.get_single(load.destination)
			.ok_or(ResourceIoError::InvalidBufferHandle)?;
		let destination_end = load
			.destination_offset
			.checked_add(load.size)
			.ok_or(ResourceIoError::InvalidDestinationRange { request: request_index })?;
		if load.size == 0 || destination_end > destination.size {
			return Err(ResourceIoError::InvalidDestinationRange { request: request_index });
		}

		// SAFETY: Validation above bounds the source file region and destination buffer range for this retained batch.
		unsafe {
			command_buffer.loadBuffer_offset_size_sourceHandle_sourceHandleOffset(
				destination.buffer.as_ref(),
				load.destination_offset,
				load.size,
				source,
				load.source.decoded_offset,
			);
		}
		Ok(())
	}

	/// Validates and encodes one file-to-image request without committing the command buffer.
	fn encode_image_load(
		&self,
		context: &context::Context,
		command_buffer: &ProtocolObject<dyn mtl::MTLIOCommandBuffer>,
		request_index: usize,
		load: ResourceIoImageLoad,
	) -> Result<(), ResourceIoError> {
		validate_source_range(request_index, load.source)?;
		let source = self.file(load.source)?;
		let destination = context
			.images
			.get_single(load.destination)
			.ok_or(ResourceIoError::InvalidImageHandle)?;
		if load.mip_level >= destination.description.mip_levels || load.array_layer >= destination.description.array_layers {
			return Err(ResourceIoError::InvalidDestinationRange { request: request_index });
		}

		let mip_extent = crate::image::mip_extent(destination.description.extent, load.mip_level);
		let requested_extent = [load.extent.width(), load.extent.height().max(1), load.extent.depth().max(1)];
		let origin = [load.origin.width(), load.origin.height(), load.origin.depth()];
		let destination_extent = [mip_extent.width(), mip_extent.height().max(1), mip_extent.depth().max(1)];
		let (minimum_row_bytes, source_row_count, _) = destination
			.description
			.format
			.compact_copy_layout(requested_extent[0], requested_extent[1]);
		let minimum_image_bytes = load.source_bytes_per_row.checked_mul(source_row_count);
		let in_bounds = requested_extent[0] > 0
			&& load.source_bytes_per_row >= minimum_row_bytes
			&& minimum_image_bytes.is_some_and(|minimum| load.source_bytes_per_image >= minimum)
			&& origin
				.into_iter()
				.zip(requested_extent)
				.zip(destination_extent)
				.all(|((origin, size), destination)| origin.checked_add(size).is_some_and(|end| end <= destination));
		if !in_bounds {
			return Err(ResourceIoError::InvalidDestinationRange { request: request_index });
		}

		// SAFETY: Validation above bounds the file layout and destination texture subresource for this retained batch.
		unsafe {
			command_buffer
				.loadTexture_slice_level_size_sourceBytesPerRow_sourceBytesPerImage_destinationOrigin_sourceHandle_sourceHandleOffset(
					destination.texture.as_ref(),
					load.array_layer as usize,
					load.mip_level as usize,
					super::utils::mtl_size(load.extent),
					load.source_bytes_per_row,
					load.source_bytes_per_image,
					mtl::MTLOrigin {
						x: origin[0] as usize,
						y: origin[1] as usize,
						z: origin[2] as usize,
					},
					source,
					load.source.decoded_offset,
				);
		}
		Ok(())
	}
}

impl crate::io::ResourceIoQueue for ResourceIoQueue {
	type Ticket = ResourceIoTicket;
	type Context = context::Context;

	/// Reports the file, destination, compression, and scheduling paths exposed by Metal I/O.
	fn capabilities(&self) -> ResourceIoCapabilities {
		ResourceIoCapabilities {
			sources: ResourceIoSourceKinds::FILE,
			destinations: ResourceIoDestinationKinds::BUFFER | ResourceIoDestinationKinds::IMAGE_REGION,
			compression: ResourceIoCompressionMethods::ZLIB
				| ResourceIoCompressionMethods::LZFSE
				| ResourceIoCompressionMethods::LZ4
				| ResourceIoCompressionMethods::LZMA
				| ResourceIoCompressionMethods::LZ_BITMAP,
			features: ResourceIoFeatures::CANCELLATION,
		}
	}

	/// Opens a raw or Metal-compression-container file on this queue's device.
	fn open_file(&mut self, descriptor: ResourceIoFileDescriptor<'_>) -> Result<ResourceIoFileHandle, ResourceIoError> {
		if !self.capabilities().supports_compression(descriptor.compression) {
			return Err(ResourceIoError::UnsupportedCompression(descriptor.compression));
		}
		let path = descriptor.path.to_str().ok_or(ResourceIoError::InvalidPath)?;
		let path = NSString::from_str(path);
		let url = NSURL::fileURLWithPath(&path);
		let handle = match metal_compression_method(descriptor.compression) {
			Some(compression) => self.device.newIOFileHandleWithURL_compressionMethod_error(&url, compression),
			None => self.device.newIOFileHandleWithURL_error(&url),
		}
		.map_err(|error| ResourceIoError::FileOpen(native_error_message(&error)))?;
		if self.debug_labels {
			handle.setLabel(descriptor.name.map(NSString::from_str).as_deref());
		}

		Ok(self.files.open(handle))
	}

	/// Releases the Metal file handle, which closes its file descriptor.
	fn close_file(&mut self, file: ResourceIoFileHandle) {
		self.files.close(file);
	}

	/// Submits one independently completing Metal I/O command buffer.
	fn submit(
		&mut self,
		context: &Self::Context,
		name: Option<&str>,
		requests: &[ResourceIoRequest],
	) -> Result<Self::Ticket, ResourceIoError> {
		if requests.is_empty() {
			return Err(ResourceIoError::EmptyBatch);
		}
		let command_buffer = self.queue.commandBuffer();
		if self.debug_labels {
			command_buffer.setLabel(name.map(NSString::from_str).as_deref());
		}

		for (request_index, request) in requests.iter().copied().enumerate() {
			match request {
				ResourceIoRequest::Buffer(load) => {
					self.encode_buffer_load(context, command_buffer.as_ref(), request_index, load)?
				}
				ResourceIoRequest::Image(load) => {
					self.encode_image_load(context, command_buffer.as_ref(), request_index, load)?
				}
			}
		}

		command_buffer.commit();

		Ok(ResourceIoTicket {
			command_buffer,
			cancellation_requested: AtomicBool::new(false),
		})
	}
}

impl crate::io::ResourceIoTicket for ResourceIoTicket {
	fn status(&self) -> ResourceIoStatus {
		match self.command_buffer.status() {
			MTLIOStatus::Pending => ResourceIoStatus::Pending,
			MTLIOStatus::Complete if self.cancellation_requested.load(Ordering::Acquire) => ResourceIoStatus::Cancelled,
			MTLIOStatus::Complete => ResourceIoStatus::Complete,
			MTLIOStatus::Cancelled => ResourceIoStatus::Cancelled,
			_ => ResourceIoStatus::Failed,
		}
	}

	fn wait(&self) -> Result<(), ResourceIoError> {
		self.command_buffer.waitUntilCompleted();
		match self.status() {
			ResourceIoStatus::Complete => Ok(()),
			ResourceIoStatus::Cancelled => Err(ResourceIoError::Cancelled),
			ResourceIoStatus::Failed => Err(ResourceIoError::Execution(
				self.command_buffer
					.error()
					.as_deref()
					.map(native_error_message)
					.unwrap_or_else(|| "Metal I/O reported an unspecified command-buffer error".to_string()),
			)),
			ResourceIoStatus::Pending => Err(ResourceIoError::Execution(
				"Metal I/O remained pending after a synchronous completion wait".to_string(),
			)),
		}
	}

	fn cancel(&self) -> Result<(), ResourceIoError> {
		if !matches!(self.command_buffer.status(), MTLIOStatus::Pending) {
			return Ok(());
		}
		self.cancellation_requested.store(true, Ordering::Release);
		self.command_buffer.tryCancel();
		Ok(())
	}
}

impl ResourceIoContext for context::Context {
	type ResourceIoQueue = ResourceIoQueue;

	/// Computes the compact source layout accepted by a Metal I/O texture request.
	fn resource_io_image_source_layout(
		&self,
		format: crate::Formats,
		extent: utils::Extent,
	) -> Result<ResourceIoImageSourceLayout, ResourceIoError> {
		ResourceIoImageSourceLayout::compact(format, extent)
	}

	fn create_resource_io_queue(
		&mut self,
		descriptor: ResourceIoQueueDescriptor<'_>,
	) -> Result<Self::ResourceIoQueue, ResourceIoError> {
		ResourceIoQueue::new(self, descriptor)
	}
}

/// Maps portable compression names to Metal compression-container methods.
fn metal_compression_method(compression: ResourceIoCompression) -> Option<MTLIOCompressionMethod> {
	match compression {
		ResourceIoCompression::None | ResourceIoCompression::GDeflate1 => None,
		ResourceIoCompression::Zlib => Some(MTLIOCompressionMethod::Zlib),
		ResourceIoCompression::Lzfse => Some(MTLIOCompressionMethod::LZFSE),
		ResourceIoCompression::Lz4 => Some(MTLIOCompressionMethod::LZ4),
		ResourceIoCompression::Lzma => Some(MTLIOCompressionMethod::LZMA),
		ResourceIoCompression::LzBitmap => Some(MTLIOCompressionMethod::LZBitmap),
	}
}

/// Creates one Metal I/O compression container without copying the decoded payload.
pub(crate) fn write_compressed_file(
	path: &std::path::Path,
	compression: ResourceIoCompression,
	decoded: &[u8],
) -> Result<(), ResourceIoError> {
	use std::ffi::{CString, c_void};

	use objc2_metal::{
		MTLIOCompressionContextAppendData, MTLIOCompressionContextDefaultChunkSize, MTLIOCompressionStatus,
		MTLIOCreateCompressionContext, MTLIOFlushAndDestroyCompressionContext,
	};

	let method = metal_compression_method(compression).ok_or(ResourceIoError::UnsupportedCompression(compression))?;
	let path = path.to_str().ok_or(ResourceIoError::InvalidPath)?;
	let path = CString::new(path).map_err(|_| ResourceIoError::InvalidPath)?;
	// SAFETY: The CString remains alive for context creation and Metal copies the path into its compression context.
	let context = unsafe {
		MTLIOCreateCompressionContext(
			NonNull::new(path.as_ptr() as *mut _).ok_or(ResourceIoError::InvalidPath)?,
			method,
			MTLIOCompressionContextDefaultChunkSize(),
		)
	};
	if context.is_null() {
		return Err(ResourceIoError::FileOpen(
			"Metal could not create the compression container".to_string(),
		));
	}

	if !decoded.is_empty() {
		// SAFETY: `decoded` is initialized and remains alive for the synchronous append call.
		unsafe {
			MTLIOCompressionContextAppendData(
				context,
				NonNull::new(decoded.as_ptr() as *mut c_void).expect("A nonempty slice has a non-null data pointer."),
				decoded.len(),
			);
		}
	}
	// SAFETY: `context` was created successfully above and is destroyed exactly once here.
	let status = unsafe { MTLIOFlushAndDestroyCompressionContext(context) };
	if status != MTLIOCompressionStatus::Complete {
		let _ = std::fs::remove_file(path.to_string_lossy().as_ref());
		return Err(ResourceIoError::Execution(format!(
			"Metal compression ended with status {status:?}"
		)));
	}

	Ok(())
}

/// Converts a native error into the localized diagnostic retained by the GHI error.
fn native_error_message(error: &NSError) -> String {
	error.localizedDescription().to_string()
}
