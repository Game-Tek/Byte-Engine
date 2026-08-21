//! Metal I/O implementation for direct file-to-resource loading.

/// The `OpenFile` struct retains one source registered with a Metal I/O queue.
struct OpenFile {
	handle: Retained<ProtocolObject<dyn MTLIOFileHandle>>,
}

static NEXT_QUEUE_ID: AtomicU64 = AtomicU64::new(1);

/// The `ResourceIoQueue` struct owns a Metal I/O command queue and its opened source files.
pub struct ResourceIoQueue {
	id: u64,
	context: NonNull<context::Context>,
	device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	queue: Retained<ProtocolObject<dyn mtl::MTLIOCommandQueue>>,
	files: Vec<OpenFile>,
	completion_event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
	next_completion_value: u64,
}

// Context::create_resource_io_queue transfers one logical queue to one runtime owner. As with the
// existing graphics Queue, callers must not use the originating Context concurrently with this queue.
unsafe impl Send for ResourceIoQueue {}

/// The `ResourceIoTicket` struct retains one submitted Metal I/O batch until callers finish observing it.
pub struct ResourceIoTicket {
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLIOCommandBuffer>>,
	pub(crate) completion_event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
	completion_point: ResourceIoTimelinePoint,
}

impl ResourceIoQueue {
	/// Creates the native queue and completion timeline used by later file batches.
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
		let completion_event = device
			.newSharedEvent()
			.ok_or_else(|| ResourceIoError::QueueCreation("Metal could not allocate a shared completion event".to_string()))?;

		Ok(Self {
			id: NEXT_QUEUE_ID.fetch_add(1, Ordering::Relaxed),
			context: NonNull::from(context),
			device,
			queue,
			files: Vec::new(),
			completion_event,
			next_completion_value: 1,
		})
	}

	fn context(&self) -> &context::Context {
		// The queue is created from a live Context and follows the same ownership contract as the
		// existing owned graphics queue. Submission never mutates context resource collections.
		unsafe { self.context.as_ref() }
	}

	fn file(&self, region: ResourceIoFileRegion) -> Result<&OpenFile, ResourceIoError> {
		if region.file.queue != self.id {
			return Err(ResourceIoError::InvalidFileHandle);
		}
		let index = usize::try_from(region.file.index).map_err(|_| ResourceIoError::InvalidFileHandle)?;
		self.files.get(index).ok_or(ResourceIoError::InvalidFileHandle)
	}

	/// Validates and encodes one file-to-buffer request without committing the command buffer.
	fn encode_buffer_load(
		&self,
		command_buffer: &ProtocolObject<dyn mtl::MTLIOCommandBuffer>,
		request_index: usize,
		load: ResourceIoBufferLoad,
	) -> Result<(), ResourceIoError> {
		validate_source_range(request_index, load.source)?;
		let source = self.file(load.source)?;
		let destination = self
			.context()
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

		unsafe {
			command_buffer.loadBuffer_offset_size_sourceHandle_sourceHandleOffset(
				destination.buffer.as_ref(),
				load.destination_offset,
				load.size,
				source.handle.as_ref(),
				load.source.decoded_offset,
			);
		}
		Ok(())
	}

	/// Validates and encodes one file-to-image request without committing the command buffer.
	fn encode_image_load(
		&self,
		command_buffer: &ProtocolObject<dyn mtl::MTLIOCommandBuffer>,
		request_index: usize,
		load: ResourceIoImageLoad,
	) -> Result<(), ResourceIoError> {
		validate_source_range(request_index, load.source)?;
		let source = self.file(load.source)?;
		let destination = self
			.context()
			.images
			.get_single(load.destination)
			.ok_or(ResourceIoError::InvalidImageHandle)?;
		if load.mip_level >= destination.mip_levels || load.array_layer >= destination.array_layers {
			return Err(ResourceIoError::InvalidDestinationRange { request: request_index });
		}

		let mip_extent = crate::image::mip_extent(destination.extent, load.mip_level);
		let requested_extent = [load.extent.width(), load.extent.height().max(1), load.extent.depth().max(1)];
		let origin = [load.origin.width(), load.origin.height(), load.origin.depth()];
		let destination_extent = [mip_extent.width(), mip_extent.height().max(1), mip_extent.depth().max(1)];
		let (minimum_row_bytes, source_row_count, _) = destination
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

		unsafe {
			command_buffer
				.loadTexture_slice_level_size_sourceBytesPerRow_sourceBytesPerImage_destinationOrigin_sourceHandle_sourceHandleOffset(
					destination.texture.as_ref(),
					load.array_layer as usize,
					load.mip_level as usize,
					mtl::MTLSize {
						width: requested_extent[0] as usize,
						height: requested_extent[1] as usize,
						depth: requested_extent[2] as usize,
					},
					load.source_bytes_per_row,
					load.source_bytes_per_image,
					mtl::MTLOrigin {
						x: origin[0] as usize,
						y: origin[1] as usize,
						z: origin[2] as usize,
					},
					source.handle.as_ref(),
					load.source.decoded_offset,
				);
		}
		Ok(())
	}
}

impl crate::io::ResourceIoQueue for ResourceIoQueue {
	type Ticket = ResourceIoTicket;

	fn capabilities(&self) -> ResourceIoCapabilities {
		metal_resource_io_capabilities()
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
		if self.context().settings.debug_labels {
			handle.setLabel(descriptor.name.map(NSString::from_str).as_deref());
		}

		let handle_index = self.files.len() as u64;
		self.files.push(OpenFile { handle });
		Ok(ResourceIoFileHandle {
			queue: self.id,
			index: handle_index,
		})
	}

	/// Submits one Metal I/O command buffer and signals its queue-local shared-event value.
	fn submit(&mut self, name: Option<&str>, requests: &[ResourceIoRequest]) -> Result<Self::Ticket, ResourceIoError> {
		if requests.is_empty() {
			return Err(ResourceIoError::EmptyBatch);
		}
		let command_buffer = self.queue.commandBuffer();
		if self.context().settings.debug_labels {
			command_buffer.setLabel(name.map(NSString::from_str).as_deref());
		}

		for (request_index, request) in requests.iter().copied().enumerate() {
			match request {
				ResourceIoRequest::Buffer(load) => self.encode_buffer_load(command_buffer.as_ref(), request_index, load)?,
				ResourceIoRequest::Image(load) => self.encode_image_load(command_buffer.as_ref(), request_index, load)?,
			}
		}

		let completion_value = self.next_completion_value;
		self.next_completion_value = completion_value.checked_add(1).ok_or_else(|| {
			ResourceIoError::Execution("Metal I/O completion timeline exhausted its 64-bit value range".to_string())
		})?;
		command_buffer.signalEvent_value(self.completion_event.as_ref(), completion_value);
		command_buffer.commit();

		Ok(ResourceIoTicket {
			command_buffer,
			completion_event: self.completion_event.clone(),
			completion_point: ResourceIoTimelinePoint {
				queue: self.id,
				value: completion_value,
			},
		})
	}
}

impl crate::io::ResourceIoTicket for ResourceIoTicket {
	fn status(&self) -> ResourceIoStatus {
		match self.command_buffer.status() {
			MTLIOStatus::Pending => ResourceIoStatus::Pending,
			MTLIOStatus::Complete => ResourceIoStatus::Complete,
			MTLIOStatus::Cancelled => ResourceIoStatus::Cancelled,
			MTLIOStatus::Error => ResourceIoStatus::Failed,
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
		self.command_buffer.tryCancel();
		Ok(())
	}

	fn completion_point(&self) -> ResourceIoTimelinePoint {
		self.completion_point
	}
}

impl ResourceIoContext for context::Context {
	type ResourceIoQueue = ResourceIoQueue;

	fn create_resource_io_queue(
		&mut self,
		descriptor: ResourceIoQueueDescriptor<'_>,
	) -> Result<Self::ResourceIoQueue, ResourceIoError> {
		ResourceIoQueue::new(self, descriptor)
	}
}

/// Rejects physical range metadata that cannot describe a nonempty stored block.
fn validate_source_range(request: usize, source: ResourceIoFileRegion) -> Result<(), ResourceIoError> {
	if source.stored_range.is_some_and(|range| {
		range.size == 0
			|| u64::try_from(range.size)
				.ok()
				.and_then(|size| range.offset.checked_add(size))
				.is_none()
	}) {
		return Err(ResourceIoError::InvalidSourceRange { request });
	}
	Ok(())
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

/// Reports the file, destination, compression, and scheduling paths exposed by Metal I/O.
fn metal_resource_io_capabilities() -> ResourceIoCapabilities {
	ResourceIoCapabilities {
		sources: ResourceIoSourceKinds::FILE,
		destinations: ResourceIoDestinationKinds::BUFFER | ResourceIoDestinationKinds::IMAGE_REGION,
		compression: ResourceIoCompressionMethods::ZLIB
			| ResourceIoCompressionMethods::LZFSE
			| ResourceIoCompressionMethods::LZ4
			| ResourceIoCompressionMethods::LZMA
			| ResourceIoCompressionMethods::LZ_BITMAP,
		features: ResourceIoFeatures::CANCELLATION | ResourceIoFeatures::TIMELINE_SYNCHRONIZATION,
	}
}

/// Creates one Metal I/O compression container without copying the decoded payload.
pub(crate) fn write_compressed_file(
	path: &std::path::Path,
	compression: ResourceIoCompression,
	decoded: &[u8],
) -> Result<(), ResourceIoError> {
	use std::ffi::{c_void, CString};

	use objc2_metal::{
		MTLIOCompressionContextAppendData, MTLIOCompressionContextDefaultChunkSize, MTLIOCompressionStatus,
		MTLIOCreateCompressionContext, MTLIOFlushAndDestroyCompressionContext,
	};

	let method = metal_compression_method(compression).ok_or(ResourceIoError::UnsupportedCompression(compression))?;
	let path = path.to_str().ok_or(ResourceIoError::InvalidPath)?;
	let path = CString::new(path).map_err(|_| ResourceIoError::InvalidPath)?;
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
		unsafe {
			MTLIOCompressionContextAppendData(
				context,
				NonNull::new(decoded.as_ptr() as *mut c_void).expect("A nonempty slice has a non-null data pointer."),
				decoded.len(),
			);
		}
	}
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

#[cfg(test)]
mod tests {
	use std::fs;
	use std::path::Path;
	use std::sync::atomic::{AtomicU64, Ordering};

	use super::*;
	use crate::device::Device as _;
	use crate::io::{ResourceIoQueue as _, ResourceIoTicket as _};

	fn test_context() -> context::Context {
		let features = crate::device::Features::new();
		let mut instance = crate::metal::Instance::new(features).expect(
			"Failed to create the Metal resource-I/O test instance. The most likely cause is that no Metal device is available.",
		);
		let device = instance
			.create_device(features, &mut [])
			.expect("Failed to create the Metal resource-I/O test device. The most likely cause is unavailable Metal support.");
		device.create_context().expect(
			"Failed to create the Metal resource-I/O test context. The most likely cause is unavailable Metal 4 support.",
		)
	}

	fn temporary_path(name: &str) -> std::path::PathBuf {
		static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
		std::env::temp_dir().join(format!(
			"byte-engine-metal-resource-io-{name}-{}-{}",
			std::process::id(),
			NEXT_FILE.fetch_add(1, Ordering::Relaxed)
		))
	}

	/// Produces a native Metal compression container for exercising runtime decode.
	fn write_lz4_container(path: &Path, bytes: &[u8]) {
		super::write_compressed_file(path, ResourceIoCompression::Lz4, bytes)
			.expect("Metal compression context should produce the test container");
	}

	#[test]
	fn raw_file_load_populates_a_context_buffer() {
		const BYTES: &[u8; 16] = b"metal-io-buffer!";
		let path = temporary_path("raw-buffer");
		fs::write(&path, BYTES).expect("raw resource-I/O test file");
		let mut context = test_context();
		let destination = context.build_buffer::<[u8; BYTES.len()]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination)
				.name("Metal I/O Raw Buffer")
				.device_accesses(crate::DeviceAccesses::HostOnly),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new().name("Metal I/O Test"))
			.expect("Metal I/O test queue");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path).name("Raw Buffer Source"))
			.expect("raw Metal I/O source");
		let request = ResourceIoBufferLoad::new(ResourceIoFileRegion::new(file, 0), destination, 0, BYTES.len()).into();
		let ticket = queue
			.submit(Some("Raw Buffer Load"), &[request])
			.expect("raw Metal I/O batch");

		assert_eq!(ticket.completion_point().value, 1);
		ticket.wait().expect("raw Metal I/O completion");
		drop(ticket);
		drop(queue);
		assert_eq!(context.get_buffer_slice(destination), BYTES);
		fs::remove_file(path).expect("remove raw resource-I/O test file");
	}

	#[test]
	fn lz4_file_load_decompresses_into_a_context_buffer() {
		const BYTES: &[u8; 32] = b"metal-io-native-lz4-decode-data!";
		let path = temporary_path("lz4-buffer");
		write_lz4_container(&path, BYTES);
		let mut context = test_context();
		let destination = context.build_buffer::<[u8; BYTES.len()]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination)
				.name("Metal I/O LZ4 Buffer")
				.device_accesses(crate::DeviceAccesses::HostOnly),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("Metal I/O test queue");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path).compression(ResourceIoCompression::Lz4))
			.expect("compressed Metal I/O source");
		let request = ResourceIoBufferLoad::new(ResourceIoFileRegion::new(file, 0), destination, 0, BYTES.len()).into();
		let ticket = queue
			.submit(Some("LZ4 Buffer Load"), &[request])
			.expect("compressed Metal I/O batch");

		ticket.wait().expect("compressed Metal I/O completion");
		drop(ticket);
		drop(queue);
		assert_eq!(context.get_buffer_slice(destination), BYTES);
		fs::remove_file(path).expect("remove compressed resource-I/O test file");
	}

	#[test]
	fn lz4_file_load_decompresses_into_an_image() {
		const BYTES: &[u8; 16] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
		let path = temporary_path("lz4-image");
		write_lz4_container(&path, BYTES);
		let mut context = test_context();
		let image = context.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.name("Metal I/O Compressed Image")
			.extent(utils::Extent::rectangle(2, 2))
			.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("Metal I/O compressed image queue");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path).compression(ResourceIoCompression::Lz4))
			.expect("compressed Metal I/O image source");
		let request = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(file, 0),
			image,
			0,
			0,
			utils::Extent::rectangle(2, 2),
			8,
			16,
		)
		.into();
		let ticket = queue
			.submit(Some("LZ4 Image Load"), &[request])
			.expect("compressed Metal I/O image batch");

		ticket.wait().expect("compressed Metal I/O image completion");
		drop(ticket);
		drop(queue);
		let copy = crate::TextureCopyHandle(image.0 .0);
		assert_eq!(context.get_image_data(copy), BYTES);
		fs::remove_file(path).expect("remove compressed resource-I/O image test file");
	}

	#[test]
	fn raw_file_load_populates_an_image_region() {
		const BYTES: &[u8; 16] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
		let path = temporary_path("raw-image");
		fs::write(&path, BYTES).expect("raw image resource-I/O test file");
		let mut context = test_context();
		let image = context.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.name("Metal I/O Image")
			.extent(utils::Extent::rectangle(2, 2))
			.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("Metal I/O image queue");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path))
			.expect("raw Metal I/O image source");
		let request = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(file, 0),
			image,
			0,
			0,
			utils::Extent::rectangle(2, 2),
			8,
			16,
		)
		.into();
		let ticket = queue
			.submit(Some("Raw Image Load"), &[request])
			.expect("raw Metal I/O image batch");

		ticket.wait().expect("raw Metal I/O image completion");
		drop(ticket);
		drop(queue);
		let copy = crate::TextureCopyHandle(image.0 .0);
		assert_eq!(context.get_image_data(copy), BYTES);
		fs::remove_file(path).expect("remove raw image resource-I/O test file");
	}

	#[test]
	fn submission_rejects_a_buffer_write_past_its_destination() {
		let path = temporary_path("invalid-buffer");
		fs::write(&path, [1, 2, 3, 4]).expect("invalid range resource-I/O test file");
		let mut context = test_context();
		let destination = context.build_buffer::<[u8; 4]>(crate::buffer::Builder::new(crate::Uses::TransferDestination));
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("Metal I/O validation queue");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path))
			.expect("raw Metal I/O validation source");
		let request = ResourceIoBufferLoad::new(ResourceIoFileRegion::new(file, 0), destination, 3, 2).into();

		assert!(matches!(
			queue.submit(Some("Invalid Buffer Load"), &[request]),
			Err(ResourceIoError::InvalidDestinationRange { request: 0 })
		));
		drop(queue);
		fs::remove_file(path).expect("remove invalid range resource-I/O test file");
	}
}

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_metal::{
	MTLDevice as _, MTLIOCommandBuffer as _, MTLIOCommandQueue as _, MTLIOCommandQueueDescriptor, MTLIOCommandQueueType,
	MTLIOCompressionMethod, MTLIOFileHandle, MTLIOPriority, MTLIOStatus, MTLSharedEvent,
};

use super::{context, mtl};
use crate::io::{
	ResourceIoBufferLoad, ResourceIoCapabilities, ResourceIoCompression, ResourceIoCompressionMethods, ResourceIoContext,
	ResourceIoDestinationKinds, ResourceIoError, ResourceIoFeatures, ResourceIoFileDescriptor, ResourceIoFileHandle,
	ResourceIoFileRegion, ResourceIoImageLoad, ResourceIoPriority, ResourceIoQueueDescriptor, ResourceIoQueueType,
	ResourceIoRequest, ResourceIoSourceKinds, ResourceIoStatus, ResourceIoTimelinePoint,
};
