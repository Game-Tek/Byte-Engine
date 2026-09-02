//! DirectStorage implementation for file-to-resource loading on DX12.

use std::ffi::{CString, c_void};
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use direct_storage::{
	DSTORAGE_COMPRESSION_DEFAULT, DSTORAGE_COMPRESSION_FORMAT_GDEFLATE, DSTORAGE_COMPRESSION_FORMAT_NONE, DSTORAGE_DESTINATION,
	DSTORAGE_DESTINATION_BUFFER, DSTORAGE_DESTINATION_TEXTURE_REGION, DSTORAGE_MAX_QUEUE_CAPACITY, DSTORAGE_MIN_QUEUE_CAPACITY,
	DSTORAGE_PRIORITY_HIGH, DSTORAGE_PRIORITY_LOW, DSTORAGE_PRIORITY_NORMAL, DSTORAGE_QUEUE_DESC, DSTORAGE_REQUEST,
	DSTORAGE_REQUEST_DESTINATION_BUFFER, DSTORAGE_REQUEST_DESTINATION_TEXTURE_REGION, DSTORAGE_REQUEST_OPTIONS,
	DSTORAGE_REQUEST_SOURCE_FILE, DSTORAGE_SOURCE, DSTORAGE_SOURCE_FILE, IDStorageCompressionCodec, IDStorageFactory,
	IDStorageFile, IDStorageQueue, IDStorageStatusArray, readonly_copy,
};
use libloading::Library;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{
	D3D12_BOX, D3D12_FENCE_FLAG_NONE, D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_RESOURCE_DESC,
	D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_NONE, D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12Fence,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::core::{GUID, HRESULT, Interface as _, PCSTR, PCWSTR};

use super::context;
use crate::io::{
	ResourceIoBufferLoad, ResourceIoCapabilities, ResourceIoCompression, ResourceIoCompressionMethods, ResourceIoContext,
	ResourceIoDestinationKinds, ResourceIoError, ResourceIoFeatures, ResourceIoFileDescriptor, ResourceIoFileHandle,
	ResourceIoFileRegion, ResourceIoImageLoad, ResourceIoImageSourceLayout, ResourceIoPriority, ResourceIoQueueDescriptor,
	ResourceIoQueueType, ResourceIoRequest, ResourceIoSourceKinds, ResourceIoStatus, validate_source_range,
};

const DIRECT_STORAGE_RUNTIME_GUIDANCE: &str = "Install the Microsoft.Direct3D.DirectStorage 1.3 runtime DLLs beside the executable; see https://www.nuget.org/packages/Microsoft.Direct3D.DirectStorage/1.3.0";
const COMPLETION_INDEX: u32 = 0;

static DIRECT_STORAGE_RUNTIME: LazyLock<Result<DirectStorageRuntime, String>> = LazyLock::new(|| {
	let core = unsafe { Library::new("dstoragecore.dll") }.map_err(runtime_load_message)?;
	let storage = unsafe { Library::new("dstorage.dll") }.map_err(runtime_load_message)?;
	Ok(DirectStorageRuntime { storage, _core: core })
});

type DStorageGetFactory = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type DStorageCreateCompressionCodec =
	unsafe extern "system" fn(direct_storage::DSTORAGE_COMPRESSION_FORMAT, u32, *const GUID, *mut *mut c_void) -> HRESULT;

/// The `DirectStorageRuntime` struct keeps the app-local DirectStorage modules loaded while their COM objects are alive.
struct DirectStorageRuntime {
	storage: Library,
	_core: Library,
}

impl DirectStorageRuntime {
	/// Loads both redistributable modules so missing runtime files become recoverable GHI errors.
	fn load() -> Result<&'static Self, String> {
		match &*DIRECT_STORAGE_RUNTIME {
			Ok(runtime) => Ok(runtime),
			Err(error) => Err(error.clone()),
		}
	}

	/// Creates a DirectStorage factory through the dynamically loaded ABI.
	fn create_factory(&self) -> Result<IDStorageFactory, ResourceIoError> {
		let function = unsafe { self.storage.get::<DStorageGetFactory>(b"DStorageGetFactory\0") }
			.map_err(runtime_load_message)
			.map_err(ResourceIoError::QueueCreation)?;
		let mut raw = std::ptr::null_mut();
		let result = unsafe { function(&IDStorageFactory::IID, &mut raw) };
		result
			.ok()
			.map_err(|error| ResourceIoError::QueueCreation(native_error_message(&error)))?;
		if raw.is_null() {
			return Err(ResourceIoError::QueueCreation(
				"DStorageGetFactory succeeded without returning a factory".to_string(),
			));
		}
		Ok(unsafe { IDStorageFactory::from_raw(raw) })
	}

	/// Creates the offline GDeflate codec through the same runtime used by resource queues.
	fn create_compression_codec(&self) -> Result<IDStorageCompressionCodec, ResourceIoError> {
		let function = unsafe {
			self.storage
				.get::<DStorageCreateCompressionCodec>(b"DStorageCreateCompressionCodec\0")
		}
		.map_err(runtime_load_message)
		.map_err(ResourceIoError::Execution)?;
		let mut raw = std::ptr::null_mut();
		let result = unsafe {
			function(
				DSTORAGE_COMPRESSION_FORMAT_GDEFLATE,
				0,
				&IDStorageCompressionCodec::IID,
				&mut raw,
			)
		};
		result
			.ok()
			.map_err(|error| ResourceIoError::Execution(native_error_message(&error)))?;
		if raw.is_null() {
			return Err(ResourceIoError::Execution(
				"DStorageCreateCompressionCodec succeeded without returning a codec".to_string(),
			));
		}
		Ok(unsafe { IDStorageCompressionCodec::from_raw(raw) })
	}
}

/// The `OpenFile` struct retains one source and the decoding contract selected when it was opened.
struct OpenFile {
	handle: IDStorageFile,
	compression: ResourceIoCompression,
}

/// The `ValidatedSource` struct retains the native file range used by one enqueued request.
struct ValidatedSource {
	file: IDStorageFile,
	compression: ResourceIoCompression,
	offset: u64,
	size: u32,
}

/// The `ValidatedRequest` struct retains every native object and value used by one enqueued request.
struct ValidatedRequest {
	source: ValidatedSource,
	resource: windows::Win32::Graphics::Direct3D12::ID3D12Resource,
	uncompressed_size: u32,
	destination: ValidatedDestination,
}

/// The `ValidatedDestination` enum carries the destination-specific ABI fields after portable validation succeeds.
enum ValidatedDestination {
	Buffer { offset: u64, size: u32 },
	Image { subresource: u32, region: D3D12_BOX },
}

/// The `ImageFootprint` struct records the exact DirectStorage source layout for one texture region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImageFootprint {
	row_pitch: usize,
	image_pitch: usize,
	total_bytes: u32,
}

/// The `ResourceIoQueue` struct owns a DirectStorage file queue and every file registered with it.
pub struct ResourceIoQueue {
	factory: IDStorageFactory,
	queue: IDStorageQueue,
	files: Vec<OpenFile>,
	next_cancellation_tag: u64,
}

/// The `ResourceIoTicket` struct retains one DirectStorage batch until callers finish observing it.
pub struct ResourceIoTicket {
	queue: IDStorageQueue,
	status: IDStorageStatusArray,
	fence: ID3D12Fence,
	cancellation_tag: u64,
	cancellation_requested: AtomicBool,
	_name: ManuallyDrop<Option<CString>>,
	_requests: ManuallyDrop<Vec<ValidatedRequest>>,
}

impl ResourceIoQueue {
	/// Creates a persistent DirectStorage file queue for one DX12 context.
	fn new(context: &context::Device, descriptor: ResourceIoQueueDescriptor<'_>) -> Result<Self, ResourceIoError> {
		if descriptor.queue_type == ResourceIoQueueType::Serial {
			return Err(ResourceIoError::QueueCreation(
				"DirectStorage file queues do not provide serial request execution".to_string(),
			));
		}
		if descriptor.max_batches_in_flight != 0 {
			return Err(ResourceIoError::QueueCreation(
				"DirectStorage does not expose an independent in-flight batch limit".to_string(),
			));
		}

		let capacity = direct_storage_queue_capacity(descriptor.max_commands_in_flight)?;
		let runtime = DirectStorageRuntime::load().map_err(ResourceIoError::QueueCreation)?;
		let factory = runtime.create_factory()?;
		let name = native_name(descriptor.name);
		let native_device = context.resource_io_native_device();
		let native_descriptor = DSTORAGE_QUEUE_DESC {
			SourceType: DSTORAGE_REQUEST_SOURCE_FILE,
			Capacity: capacity,
			Priority: match descriptor.priority {
				ResourceIoPriority::High => DSTORAGE_PRIORITY_HIGH,
				ResourceIoPriority::Normal => DSTORAGE_PRIORITY_NORMAL,
				ResourceIoPriority::Low => DSTORAGE_PRIORITY_LOW,
			},
			Name: native_name_pointer(name.as_ref()),
			Device: unsafe { readonly_copy(native_device) },
		};
		let queue = unsafe { factory.CreateQueue::<IDStorageQueue>(&native_descriptor) }
			.map_err(|error| ResourceIoError::QueueCreation(native_error_message(&error)))?;

		Ok(Self {
			factory,
			queue,
			files: Vec::new(),
			next_cancellation_tag: 1,
		})
	}

	/// Resolves a file index against this queue before any native request is enqueued.
	fn file(&self, region: ResourceIoFileRegion) -> Result<&OpenFile, ResourceIoError> {
		let index = usize::try_from(region.file.index).map_err(|_| ResourceIoError::InvalidFileHandle)?;
		self.files.get(index).ok_or(ResourceIoError::InvalidFileHandle)
	}

	/// Validates and retains the native file range used by one request.
	fn validate_source(
		&self,
		request_index: usize,
		region: ResourceIoFileRegion,
		uncompressed_size: u32,
	) -> Result<ValidatedSource, ResourceIoError> {
		validate_source_range(request_index, region)?;
		let file = self.file(region)?;
		let (offset, size) = native_source_range(request_index, region, file.compression, uncompressed_size)?;
		Ok(ValidatedSource {
			file: file.handle.clone(),
			compression: file.compression,
			offset,
			size,
		})
	}

	/// Validates one file-to-buffer request and retains its native inputs without enqueuing work.
	fn validate_buffer_load(
		&self,
		context: &context::Device,
		request_index: usize,
		load: ResourceIoBufferLoad,
	) -> Result<ValidatedRequest, ResourceIoError> {
		let destination = context
			.resource_io_buffer_destination(load.destination)
			.ok_or(ResourceIoError::InvalidBufferHandle)?;
		let destination_end = load
			.destination_offset
			.checked_add(load.size)
			.ok_or(ResourceIoError::InvalidDestinationRange { request: request_index })?;
		if load.size == 0 || destination_end > destination.size {
			return Err(ResourceIoError::InvalidDestinationRange { request: request_index });
		}
		if !destination.common_state || !destination.direct_storage_compatible {
			return Err(ResourceIoError::InvalidDestinationState { request: request_index });
		}

		let uncompressed_size =
			u32::try_from(load.size).map_err(|_| ResourceIoError::InvalidDestinationRange { request: request_index })?;
		let source = self.validate_source(request_index, load.source, uncompressed_size)?;
		Ok(ValidatedRequest {
			source,
			resource: destination.resource,
			uncompressed_size,
			destination: ValidatedDestination::Buffer {
				offset: u64::try_from(load.destination_offset)
					.map_err(|_| ResourceIoError::InvalidDestinationRange { request: request_index })?,
				size: uncompressed_size,
			},
		})
	}

	/// Validates one file-to-image request and retains its native inputs without enqueuing work.
	fn validate_image_load(
		&self,
		context: &context::Device,
		request_index: usize,
		load: ResourceIoImageLoad,
	) -> Result<ValidatedRequest, ResourceIoError> {
		let destination = context
			.resource_io_image_destination(load.destination)
			.ok_or(ResourceIoError::InvalidImageHandle)?;
		if !destination.common_state {
			return Err(ResourceIoError::InvalidDestinationState { request: request_index });
		}

		let (subresource, region, footprint) = validate_image_destination(context, request_index, load, &destination)?;
		let source = self.validate_source(request_index, load.source, footprint.total_bytes)?;
		Ok(ValidatedRequest {
			source,
			resource: destination.resource,
			uncompressed_size: footprint.total_bytes,
			destination: ValidatedDestination::Image { subresource, region },
		})
	}

	/// Converts one fully validated request into the borrowed DirectStorage ABI and enqueues it.
	fn enqueue_request(&self, request: &ValidatedRequest, name: PCSTR, cancellation_tag: u64) {
		let mut options = DSTORAGE_REQUEST_OPTIONS::default();
		options.set_SourceType(DSTORAGE_REQUEST_SOURCE_FILE);
		options.set_CompressionFormat(match request.source.compression {
			ResourceIoCompression::None => DSTORAGE_COMPRESSION_FORMAT_NONE,
			ResourceIoCompression::GDeflate1 => DSTORAGE_COMPRESSION_FORMAT_GDEFLATE,
			_ => unreachable!("Unsupported compression formats are rejected while opening files."),
		});
		let source = DSTORAGE_SOURCE {
			File: ManuallyDrop::new(DSTORAGE_SOURCE_FILE {
				Source: unsafe { readonly_copy(&request.source.file) },
				Offset: request.source.offset,
				Size: request.source.size,
			}),
		};
		let (destination, destination_type) = match request.destination {
			ValidatedDestination::Buffer { offset, size } => (
				DSTORAGE_DESTINATION {
					Buffer: ManuallyDrop::new(DSTORAGE_DESTINATION_BUFFER {
						Resource: unsafe { readonly_copy(&request.resource) },
						Offset: offset,
						Size: size,
					}),
				},
				DSTORAGE_REQUEST_DESTINATION_BUFFER,
			),
			ValidatedDestination::Image { subresource, region } => (
				DSTORAGE_DESTINATION {
					Texture: ManuallyDrop::new(DSTORAGE_DESTINATION_TEXTURE_REGION {
						Resource: unsafe { readonly_copy(&request.resource) },
						SubresourceIndex: subresource,
						Region: region,
					}),
				},
				DSTORAGE_REQUEST_DESTINATION_TEXTURE_REGION,
			),
		};
		options.set_DestinationType(destination_type);
		let native_request = DSTORAGE_REQUEST {
			Options: options,
			Source: source,
			Destination: destination,
			UncompressedSize: request.uncompressed_size,
			CancellationTag: cancellation_tag,
			Name: name,
		};
		unsafe {
			self.queue.EnqueueRequest(&native_request);
		}
	}

	/// Allocates a queue-local cancellation tag without permitting wraparound reuse.
	fn allocate_cancellation_tag(&mut self) -> Result<u64, ResourceIoError> {
		let tag = self.next_cancellation_tag;
		self.next_cancellation_tag = tag
			.checked_add(1)
			.ok_or_else(|| ResourceIoError::Execution("DirectStorage batch cancellation-tag space is exhausted".to_string()))?;
		Ok(tag)
	}
}

impl crate::io::ResourceIoQueue for ResourceIoQueue {
	type Ticket = ResourceIoTicket;
	type Context = context::Device;

	fn capabilities(&self) -> ResourceIoCapabilities {
		direct_storage_resource_io_capabilities()
	}

	fn open_file(&mut self, descriptor: ResourceIoFileDescriptor<'_>) -> Result<ResourceIoFileHandle, ResourceIoError> {
		if !self.capabilities().supports_compression(descriptor.compression) {
			return Err(ResourceIoError::UnsupportedCompression(descriptor.compression));
		}
		let mut path: Vec<u16> = descriptor.path.as_os_str().encode_wide().collect();
		if path.contains(&0) {
			return Err(ResourceIoError::InvalidPath);
		}
		path.push(0);
		let handle = unsafe { self.factory.OpenFile::<_, IDStorageFile>(PCWSTR(path.as_ptr())) }
			.map_err(|error| ResourceIoError::FileOpen(native_error_message(&error)))?;
		let index = u64::try_from(self.files.len())
			.map_err(|_| ResourceIoError::FileOpen("DirectStorage queue file identity space is exhausted".to_string()))?;
		// DirectStorage files have no independent debug-name API, so the descriptor name is intentionally queue-only metadata.
		let _ = descriptor.name;
		self.files.push(OpenFile {
			handle,
			compression: descriptor.compression,
		});
		Ok(ResourceIoFileHandle { index })
	}

	/// Submits requests whose destinations are static default-heap resources tracked in the common state.
	fn submit(
		&mut self,
		context: &Self::Context,
		name: Option<&str>,
		requests: &[ResourceIoRequest],
	) -> Result<Self::Ticket, ResourceIoError> {
		if requests.is_empty() {
			return Err(ResourceIoError::EmptyBatch);
		}

		let mut validated = Vec::with_capacity(requests.len());
		for (request_index, request) in requests.iter().copied().enumerate() {
			validated.push(match request {
				ResourceIoRequest::Buffer(load) => self.validate_buffer_load(context, request_index, load)?,
				ResourceIoRequest::Image(load) => self.validate_image_load(context, request_index, load)?,
			});
		}

		let name = native_name(name);
		let status = unsafe {
			self.factory
				.CreateStatusArray::<_, IDStorageStatusArray>(1, native_name_pointer(name.as_ref()))
		}
		.map_err(|error| ResourceIoError::Execution(native_error_message(&error)))?;
		let fence = unsafe {
			context
				.resource_io_native_device()
				.CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE)
		}
		.map_err(|error| ResourceIoError::Execution(native_error_message(&error)))?;
		let cancellation_tag = self.allocate_cancellation_tag()?;
		let request_name = native_name_pointer(name.as_ref());
		for request in &validated {
			self.enqueue_request(request, request_name, cancellation_tag);
		}
		unsafe {
			self.queue.EnqueueStatus(&status, COMPLETION_INDEX);
			self.queue.EnqueueSignal(&fence, 1);
			self.queue.Submit();
		}

		Ok(ResourceIoTicket {
			queue: self.queue.clone(),
			status,
			fence,
			cancellation_tag,
			cancellation_requested: AtomicBool::new(false),
			_name: ManuallyDrop::new(name),
			_requests: ManuallyDrop::new(validated),
		})
	}
}

impl ResourceIoTicket {
	/// Blocks until the native queue has passed this batch's final lifetime marker.
	fn wait_for_completion_marker(&self) -> Result<(), ResourceIoError> {
		let completed = unsafe { self.fence.GetCompletedValue() };
		if completed == u64::MAX {
			return Err(ResourceIoError::Execution(
				"DirectStorage completion became unavailable. The most likely cause is that the DX12 device was removed."
					.to_string(),
			));
		}
		if completed < 1 {
			// A null event asks D3D12 to block in this call, so the ticket owns no native handle that can outlive it.
			unsafe { self.fence.SetEventOnCompletion(1, HANDLE::default()) }
				.map_err(|error| ResourceIoError::Execution(native_error_message(&error)))?;
		}
		// GetCompletedValue returns UINT64_MAX after device removal. That sentinel does not prove request completion.
		if unsafe { self.fence.GetCompletedValue() } == u64::MAX {
			return Err(ResourceIoError::Execution(
				"DirectStorage completion became unavailable. The most likely cause is that the DX12 device was removed."
					.to_string(),
			));
		}
		Ok(())
	}

	/// Maps the completed status slot and the conservative cancellation marker into the portable state.
	fn completed_status(&self) -> ResourceIoStatus {
		match unsafe { self.status.GetHResult(COMPLETION_INDEX) } {
			Ok(()) if self.cancellation_requested.load(Ordering::Acquire) => ResourceIoStatus::Cancelled,
			Ok(()) => ResourceIoStatus::Complete,
			Err(_) => ResourceIoStatus::Failed,
		}
	}

	/// Converts the completed native status into the portable synchronous result.
	fn completed_result(&self) -> Result<(), ResourceIoError> {
		match unsafe { self.status.GetHResult(COMPLETION_INDEX) } {
			Ok(()) if self.cancellation_requested.load(Ordering::Acquire) => Err(ResourceIoError::Cancelled),
			Ok(()) => Ok(()),
			Err(error) => Err(ResourceIoError::Execution(native_error_message(&error))),
		}
	}
}

impl crate::io::ResourceIoTicket for ResourceIoTicket {
	fn status(&self) -> ResourceIoStatus {
		if unsafe { self.status.IsComplete(COMPLETION_INDEX) } {
			self.completed_status()
		} else {
			ResourceIoStatus::Pending
		}
	}

	fn wait(&self) -> Result<(), ResourceIoError> {
		self.wait_for_completion_marker()?;
		if !unsafe { self.status.IsComplete(COMPLETION_INDEX) } {
			return Err(ResourceIoError::Execution(
				"DirectStorage remained pending after its completion fence was signalled".to_string(),
			));
		}
		self.completed_result()
	}

	fn cancel(&self) -> Result<(), ResourceIoError> {
		if unsafe { self.status.IsComplete(COMPLETION_INDEX) } {
			return Ok(());
		}
		self.cancellation_requested.store(true, Ordering::Release);
		unsafe {
			self.queue.CancelRequestsWithTag(u64::MAX, self.cancellation_tag);
		}
		Ok(())
	}
}

impl Drop for ResourceIoTicket {
	fn drop(&mut self) {
		// DirectStorage borrows the request's native objects and debug name until the completion marker runs.
		if self.wait_for_completion_marker().is_ok() {
			// The completion marker proves that DirectStorage no longer borrows either payload.
			unsafe {
				ManuallyDrop::drop(&mut self._requests);
				ManuallyDrop::drop(&mut self._name);
			}
			return;
		}
		// If the native wait fails, intentionally retain every object referenced by queued status/signal work.
		// DirectStorage does not promise that queue entries retain caller COM objects or the debug-name pointer.
		std::mem::forget(self.queue.clone());
		std::mem::forget(self.status.clone());
		std::mem::forget(self.fence.clone());
	}
}

impl ResourceIoContext for context::Device {
	type ResourceIoQueue = ResourceIoQueue;

	/// Computes the exact copy footprint accepted by a DirectStorage texture request.
	fn resource_io_image_source_layout(
		&self,
		format: crate::Formats,
		extent: utils::Extent,
	) -> Result<ResourceIoImageSourceLayout, ResourceIoError> {
		if extent.depth() > 1 {
			return Err(ResourceIoError::InvalidImageLayout);
		}
		let footprint = image_footprint(self, format, extent.width(), extent.height().max(1))?;
		Ok(ResourceIoImageSourceLayout {
			bytes_per_row: footprint.row_pitch,
			bytes_per_image: footprint.image_pitch,
			total_bytes: footprint.total_bytes as usize,
		})
	}

	fn create_resource_io_queue(
		&mut self,
		descriptor: ResourceIoQueueDescriptor<'_>,
	) -> Result<Self::ResourceIoQueue, ResourceIoError> {
		ResourceIoQueue::new(self, descriptor)
	}
}

/// Maps the portable command limit to DirectStorage's fixed queue-capacity range.
fn direct_storage_queue_capacity(max_commands_in_flight: usize) -> Result<u16, ResourceIoError> {
	let capacity = if max_commands_in_flight == 0 {
		DSTORAGE_MAX_QUEUE_CAPACITY
	} else {
		u32::try_from(max_commands_in_flight).map_err(|_| {
			ResourceIoError::QueueCreation(
				"The DirectStorage command limit is larger than a native queue can represent".to_string(),
			)
		})?
	};
	if !(DSTORAGE_MIN_QUEUE_CAPACITY..=DSTORAGE_MAX_QUEUE_CAPACITY).contains(&capacity) {
		return Err(ResourceIoError::QueueCreation(format!(
			"DirectStorage queue capacity must be between {DSTORAGE_MIN_QUEUE_CAPACITY} and {DSTORAGE_MAX_QUEUE_CAPACITY} commands"
		)));
	}
	u16::try_from(capacity).map_err(|_| {
		ResourceIoError::QueueCreation("The DirectStorage queue capacity does not fit its native field".to_string())
	})
}

/// Selects the physical file range and decoded size used by one DirectStorage request.
fn native_source_range(
	request: usize,
	source: ResourceIoFileRegion,
	compression: ResourceIoCompression,
	uncompressed_size: u32,
) -> Result<(u64, u32), ResourceIoError> {
	match compression {
		ResourceIoCompression::None => Ok((
			u64::try_from(source.decoded_offset).map_err(|_| ResourceIoError::InvalidSourceRange { request })?,
			uncompressed_size,
		)),
		ResourceIoCompression::GDeflate1 => {
			let stored = source.stored_range.ok_or(ResourceIoError::InvalidSourceRange { request })?;
			let size = u32::try_from(stored.size).map_err(|_| ResourceIoError::InvalidSourceRange { request })?;
			if size >= uncompressed_size {
				return Err(ResourceIoError::InvalidSourceRange { request });
			}
			Ok((stored.offset, size))
		}
		_ => Err(ResourceIoError::UnsupportedCompression(compression)),
	}
}

/// Validates an image region and computes the native subresource, box, and file layout.
fn validate_image_destination(
	context: &context::Device,
	request: usize,
	load: ResourceIoImageLoad,
	destination: &context::ResourceIoImageDestination,
) -> Result<(u32, D3D12_BOX, ImageFootprint), ResourceIoError> {
	if load.mip_level >= destination.mip_levels || load.array_layer >= destination.array_layers {
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}
	let mip_extent = crate::image::mip_extent(destination.extent, load.mip_level);
	let width = load.extent.width();
	let height = load.extent.height().max(1);
	let depth = load.extent.depth().max(1);
	let origin = [load.origin.width(), load.origin.height(), load.origin.depth()];
	let destination_extent = [mip_extent.width(), mip_extent.height().max(1), 1];
	let requested_extent = [width, height, depth];
	let [right, bottom, back] = checked_region_end(origin, requested_extent, destination_extent)
		.ok_or(ResourceIoError::InvalidDestinationRange { request })?;
	if width == 0 || depth != 1 || load.origin.depth() != 0 {
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}
	if destination.format.is_depth()
		&& (load.origin != utils::Extent::new(0, 0, 0) || requested_extent != [destination_extent[0], destination_extent[1], 1])
	{
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}
	if destination.format.bc_bytes_per_block().is_some()
		&& !block_compressed_region_is_aligned(origin, [right, bottom, back], destination_extent)
	{
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}

	if unsafe { destination.resource.GetDesc() }.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D {
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}
	let footprint = image_footprint(context, destination.format, width, height)
		.map_err(|_| ResourceIoError::InvalidDestinationRange { request })?;
	if load.source_bytes_per_row != footprint.row_pitch || load.source_bytes_per_image != footprint.image_pitch {
		return Err(ResourceIoError::InvalidDestinationRange { request });
	}
	let subresource = load
		.array_layer
		.checked_mul(destination.mip_levels)
		.and_then(|layer| layer.checked_add(load.mip_level))
		.ok_or(ResourceIoError::InvalidDestinationRange { request })?;
	Ok((
		subresource,
		D3D12_BOX {
			left: load.origin.width(),
			top: load.origin.height(),
			front: 0,
			right,
			bottom,
			back: 1,
		},
		footprint,
	))
}

/// Computes a region end only when every axis fits within the destination.
fn checked_region_end(origin: [u32; 3], extent: [u32; 3], destination_extent: [u32; 3]) -> Option<[u32; 3]> {
	let end = [
		origin[0].checked_add(extent[0])?,
		origin[1].checked_add(extent[1])?,
		origin[2].checked_add(extent[2])?,
	];
	(end[0] <= destination_extent[0] && end[1] <= destination_extent[1] && end[2] <= destination_extent[2]).then_some(end)
}

/// Computes the exact D3D12 copy footprint DirectStorage expects for one 2D region.
fn image_footprint(
	context: &context::Device,
	format: crate::Formats,
	width: u32,
	height: u32,
) -> Result<ImageFootprint, ResourceIoError> {
	if width == 0 || height == 0 {
		return Err(ResourceIoError::InvalidImageLayout);
	}
	let descriptor = D3D12_RESOURCE_DESC {
		Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
		Alignment: 0,
		Width: u64::from(width),
		Height: height,
		DepthOrArraySize: 1,
		MipLevels: 1,
		Format: context::Device::dxgi_format(format).ok_or(ResourceIoError::InvalidImageLayout)?,
		SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
		Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
		Flags: D3D12_RESOURCE_FLAG_NONE,
	};
	let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
	let mut row_count = 0;
	let mut row_size = 0;
	let mut total_bytes = 0;
	unsafe {
		context.resource_io_native_device().GetCopyableFootprints(
			&descriptor,
			0,
			1,
			0,
			Some(&mut layout),
			Some(&mut row_count),
			Some(&mut row_size),
			Some(&mut total_bytes),
		);
	}
	let row_pitch = usize::try_from(layout.Footprint.RowPitch).map_err(|_| ResourceIoError::InvalidImageLayout)?;
	let image_pitch = row_pitch
		.checked_mul(usize::try_from(row_count).map_err(|_| ResourceIoError::InvalidImageLayout)?)
		.ok_or(ResourceIoError::InvalidImageLayout)?;
	let total_bytes = u32::try_from(total_bytes).map_err(|_| ResourceIoError::InvalidImageLayout)?;
	if row_pitch == 0 || row_size == 0 || image_pitch == 0 || total_bytes == 0 {
		return Err(ResourceIoError::InvalidImageLayout);
	}
	Ok(ImageFootprint {
		row_pitch,
		image_pitch,
		total_bytes,
	})
}

/// Checks the copy-region restrictions imposed by block-compressed DX12 formats.
fn block_compressed_region_is_aligned(origin: [u32; 3], end: [u32; 3], destination_extent: [u32; 3]) -> bool {
	origin[0].is_multiple_of(4)
		&& origin[1].is_multiple_of(4)
		&& (end[0].is_multiple_of(4) || end[0] == destination_extent[0])
		&& (end[1].is_multiple_of(4) || end[1] == destination_extent[1])
}

/// Reports the file, destination, compression, and scheduling paths exposed by DirectStorage.
fn direct_storage_resource_io_capabilities() -> ResourceIoCapabilities {
	ResourceIoCapabilities {
		sources: ResourceIoSourceKinds::FILE,
		destinations: ResourceIoDestinationKinds::BUFFER | ResourceIoDestinationKinds::IMAGE_REGION,
		compression: ResourceIoCompressionMethods::GDEFLATE_1,
		features: ResourceIoFeatures::CANCELLATION,
	}
}

/// Converts optional UTF-8 diagnostic metadata into a stable native C string.
fn native_name(name: Option<&str>) -> Option<CString> {
	name.and_then(|name| CString::new(name).ok())
}

/// Returns a null pointer when optional diagnostic metadata is unavailable.
fn native_name_pointer(name: Option<&CString>) -> PCSTR {
	name.map_or_else(PCSTR::null, |name| PCSTR(name.as_ptr().cast()))
}

/// Converts a runtime-loader failure into an actionable diagnostic for the calling operation.
fn runtime_load_message(error: libloading::Error) -> String {
	format!("{error}. {DIRECT_STORAGE_RUNTIME_GUIDANCE}")
}

/// Converts a native error into the diagnostic retained by the GHI error.
fn native_error_message(error: &windows::core::Error) -> String {
	format!("{} (HRESULT {:#010X})", error.message(), error.code().0 as u32)
}

/// Rejects codec output that DirectStorage cannot accept as a decompression request.
fn validate_gdeflate_output_size(decoded_size: usize, compressed_size: usize) -> Result<(), ResourceIoError> {
	if compressed_size >= decoded_size {
		return Err(ResourceIoError::Execution(
			"DirectStorage GDeflate output is not smaller than the decoded payload; store this block without compression"
				.to_string(),
		));
	}
	Ok(())
}

/// Creates one independent GDeflate block for later use with [`ResourceIoFileRegion::stored_range`].
pub(crate) fn write_compressed_file(
	path: &Path,
	compression: ResourceIoCompression,
	decoded: &[u8],
) -> Result<(), ResourceIoError> {
	if compression != ResourceIoCompression::GDeflate1 {
		return Err(ResourceIoError::UnsupportedCompression(compression));
	}
	let runtime = DirectStorageRuntime::load().map_err(ResourceIoError::Execution)?;
	let codec = runtime.create_compression_codec()?;
	let bound = unsafe { codec.CompressBufferBound(decoded.len()) };
	if bound == 0 && !decoded.is_empty() {
		return Err(ResourceIoError::Execution(
			"DirectStorage returned an empty compression bound for a nonempty payload".to_string(),
		));
	}
	let mut compressed = vec![0u8; bound];
	let mut compressed_size = 0;
	unsafe {
		codec
			.CompressBuffer(
				decoded.as_ptr().cast(),
				decoded.len(),
				DSTORAGE_COMPRESSION_DEFAULT,
				compressed.as_mut_ptr().cast(),
				compressed.len(),
				&mut compressed_size,
			)
			.map_err(|error| ResourceIoError::Execution(native_error_message(&error)))?;
	}
	if compressed_size > compressed.len() {
		return Err(ResourceIoError::Execution(
			"DirectStorage reported compressed output larger than the allocated codec bound".to_string(),
		));
	}
	validate_gdeflate_output_size(decoded.len(), compressed_size)?;
	compressed.truncate(compressed_size);
	if let Err(error) = std::fs::write(path, compressed) {
		let _ = std::fs::remove_file(path);
		return Err(ResourceIoError::FileOpen(error.to_string()));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::fs;
	use std::sync::atomic::{AtomicU64, Ordering};

	use super::*;
	use crate::command_buffer::CommandBufferRecording as _;
	use crate::io::{ResourceIoQueue as _, ResourceIoTicket as _};

	/// Creates a native DX12 context and transfer queue when the test machine exposes them.
	fn test_context() -> Option<(context::Device, crate::QueueHandle)> {
		let features = crate::device::Features::new().validation(false);
		let mut queue_handle = None;
		let context = context::Device::new(
			features,
			&mut [(crate::QueueSelection::new(crate::WorkloadTypes::TRANSFER), &mut queue_handle)],
		)
		.ok()?;
		Some((context, queue_handle?))
	}

	fn temporary_path(name: &str) -> std::path::PathBuf {
		static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
		std::env::temp_dir().join(format!(
			"byte-engine-dx12-resource-io-{name}-{}-{}",
			std::process::id(),
			NEXT_FILE.fetch_add(1, Ordering::Relaxed)
		))
	}

	/// Copies a DirectStorage destination buffer through a native readback heap.
	fn read_buffer(
		context: &mut context::Device,
		queue_handle: crate::QueueHandle,
		source: crate::BaseBufferHandle,
		size: usize,
	) -> Vec<u8> {
		let readback = context.build_buffer::<[u8; 4096]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = context.create_synchronizer(None, false);
		let command_buffer = context.create_command_buffer(None, queue_handle);
		let mut recording = context.create_command_buffer_recording(command_buffer);
		recording.copy_buffers(&[crate::BufferCopyDescriptor::new(source, 0, readback.into(), 0, size)]);
		recording.execute(synchronizer);
		context.wait_for_synchronizer(synchronizer);
		context
			.buffer_mapped_bytes_for_sequence(readback.into(), size, 0)
			.expect("DX12 resource-I/O readback buffer should remain mapped")
	}

	/// Copies a DirectStorage destination image through the public native readback path.
	fn read_image(context: &mut context::Device, queue_handle: crate::QueueHandle, image: crate::ImageHandle) -> Vec<u8> {
		let synchronizer = context.create_synchronizer(None, false);
		let command_buffer = context.create_command_buffer(None, queue_handle);
		let mut recording = context.create_command_buffer_recording(command_buffer);
		let copy = recording
			.transfer_texture(image.into())
			.expect("DX12 resource-I/O image should support native readback");
		recording.execute(synchronizer);
		context.wait_for_synchronizer(synchronizer);
		context
			.get_image_data(copy)
			.expect("DX12 resource-I/O image readback should complete")
			.bytes
	}

	#[test]
	fn capabilities_match_the_implemented_direct_storage_paths() {
		assert_eq!(
			direct_storage_resource_io_capabilities(),
			ResourceIoCapabilities {
				sources: ResourceIoSourceKinds::FILE,
				destinations: ResourceIoDestinationKinds::BUFFER | ResourceIoDestinationKinds::IMAGE_REGION,
				compression: ResourceIoCompressionMethods::GDEFLATE_1,
				features: ResourceIoFeatures::CANCELLATION,
			}
		);
	}

	#[test]
	fn compressed_sources_require_a_nonempty_stored_block() {
		let file = ResourceIoFileHandle { index: 0 };
		let missing = ResourceIoFileRegion::new(file, 0);
		let empty = ResourceIoFileRegion::new(file, 0).stored_range(20, 0);
		let valid = ResourceIoFileRegion::new(file, 64).stored_range(20, 12);

		assert_eq!(
			native_source_range(3, missing, ResourceIoCompression::GDeflate1, 64),
			Err(ResourceIoError::InvalidSourceRange { request: 3 })
		);
		assert_eq!(
			validate_source_range(4, empty),
			Err(ResourceIoError::InvalidSourceRange { request: 4 })
		);
		assert_eq!(
			native_source_range(5, valid, ResourceIoCompression::GDeflate1, 64),
			Ok((20, 12))
		);
		assert_eq!(
			native_source_range(6, valid, ResourceIoCompression::GDeflate1, 12),
			Err(ResourceIoError::InvalidSourceRange { request: 6 })
		);
	}

	#[test]
	fn gdeflate_output_must_be_smaller_than_its_decoded_block() {
		assert!(validate_gdeflate_output_size(64, 63).is_ok());
		assert!(matches!(
			validate_gdeflate_output_size(64, 64),
			Err(ResourceIoError::Execution(_))
		));
		assert!(matches!(
			validate_gdeflate_output_size(0, 0),
			Err(ResourceIoError::Execution(_))
		));
	}

	#[test]
	fn queue_capacity_preserves_direct_storage_native_limits() {
		assert_eq!(direct_storage_queue_capacity(0), Ok(DSTORAGE_MAX_QUEUE_CAPACITY as u16));
		assert_eq!(
			direct_storage_queue_capacity(DSTORAGE_MIN_QUEUE_CAPACITY as usize),
			Ok(DSTORAGE_MIN_QUEUE_CAPACITY as u16)
		);
		assert!(matches!(
			direct_storage_queue_capacity(DSTORAGE_MIN_QUEUE_CAPACITY as usize - 1),
			Err(ResourceIoError::QueueCreation(_))
		));
		assert!(matches!(
			direct_storage_queue_capacity(DSTORAGE_MAX_QUEUE_CAPACITY as usize + 1),
			Err(ResourceIoError::QueueCreation(_))
		));
	}

	#[test]
	fn block_compressed_regions_allow_aligned_blocks_and_short_edges() {
		assert!(block_compressed_region_is_aligned([4, 4, 0], [8, 8, 1], [10, 10, 1]));
		assert!(block_compressed_region_is_aligned([8, 8, 0], [10, 10, 1], [10, 10, 1]));
		assert!(!block_compressed_region_is_aligned([2, 4, 0], [8, 8, 1], [10, 10, 1]));
		assert!(!block_compressed_region_is_aligned([4, 4, 0], [9, 8, 1], [10, 10, 1]));
	}

	#[test]
	fn image_validation_uses_copy_footprints_and_native_subresources() {
		let Some((mut context, _)) = test_context() else {
			return;
		};
		let image = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image)
				.extent(utils::Extent::rectangle(16, 16))
				.mip_levels(4)
				.array_layers(std::num::NonZeroU32::new(3)),
		);
		let destination = context
			.resource_io_image_destination(image.into())
			.expect("DX12 validation image should have static native storage");
		let layout = context
			.resource_io_image_source_layout(crate::Formats::RGBA8UNORM, utils::Extent::rectangle(2, 2))
			.expect("DX12 validation image should expose a source layout");
		let file = ResourceIoFileHandle { index: 0 };
		let valid = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(file, 0),
			image,
			2,
			3,
			utils::Extent::rectangle(2, 2),
			layout.bytes_per_row,
			layout.bytes_per_image,
		);
		let compact = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(file, 0),
			image,
			2,
			3,
			utils::Extent::rectangle(2, 2),
			8,
			16,
		);

		let (subresource, _, validated_footprint) = validate_image_destination(&context, 0, valid, &destination)
			.expect("A copy-footprint image request should pass DX12 validation");
		assert_eq!(subresource, 11);
		assert_eq!(validated_footprint.row_pitch, layout.bytes_per_row);
		assert_eq!(validated_footprint.image_pitch, layout.bytes_per_image);
		assert_eq!(validated_footprint.total_bytes as usize, layout.total_bytes);
		assert_eq!(
			validate_image_destination(&context, 1, compact, &destination),
			Err(ResourceIoError::InvalidDestinationRange { request: 1 })
		);
	}

	#[test]
	fn raw_file_load_populates_a_dx12_buffer() {
		const BYTES: &[u8; 16] = b"dx12-io-buffer!!";
		if DirectStorageRuntime::load().is_err() {
			return;
		}
		let Some((mut context, queue_handle)) = test_context() else {
			return;
		};
		let path = temporary_path("raw-buffer");
		fs::write(&path, BYTES).expect("DX12 raw resource-I/O test file should be writable");
		let destination = context.build_buffer::<[u8; BYTES.len()]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new().name("DX12 Resource I/O Test"))
			.expect("DirectStorage test queue should be created when its runtime is available");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path).name("Raw Buffer Source"))
			.expect("DirectStorage should open the raw test file");
		let request = ResourceIoBufferLoad::new(ResourceIoFileRegion::new(file, 0), destination, 0, BYTES.len()).into();
		let ticket = queue
			.submit(&context, Some("Raw Buffer Load"), &[request])
			.expect("DirectStorage should submit the raw buffer batch");

		// Dropping a pending ticket must retain its native inputs through the final fence marker.
		drop(ticket);
		drop(queue);
		assert_eq!(
			read_buffer(&mut context, queue_handle, destination.into(), BYTES.len()),
			BYTES
		);
		fs::remove_file(path).expect("DX12 raw resource-I/O test file should be removable");
	}

	#[test]
	fn submission_rejects_incompatible_destinations() {
		if DirectStorageRuntime::load().is_err() {
			return;
		}
		let Some((mut context, _)) = test_context() else {
			return;
		};
		let path = temporary_path("validation");
		fs::write(&path, [1, 2, 3, 4]).expect("DX12 validation resource-I/O test file should be writable");
		let incompatible_destination = context.build_buffer::<[u8; 4]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("DirectStorage validation queue should be created");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path))
			.expect("DirectStorage validation test file should open");
		let incompatible_request =
			ResourceIoBufferLoad::new(ResourceIoFileRegion::new(file, 0), incompatible_destination, 0, 4).into();

		assert!(matches!(
			queue.submit(&context, None, &[incompatible_request]),
			Err(ResourceIoError::InvalidDestinationState { request: 0 })
		));

		drop(queue);
		fs::remove_file(path).expect("DX12 validation resource-I/O test file should be removable");
	}

	#[test]
	fn gdeflate_file_load_decompresses_into_a_dx12_buffer() {
		const BYTES: &[u8; 4096] = &[0x5A; 4096];
		if DirectStorageRuntime::load().is_err() {
			return;
		}
		let Some((mut context, queue_handle)) = test_context() else {
			return;
		};
		let path = temporary_path("gdeflate-buffer");
		write_compressed_file(&path, ResourceIoCompression::GDeflate1, BYTES)
			.expect("DirectStorage should create a GDeflate test block");
		let stored_size = fs::metadata(&path)
			.expect("DX12 compressed resource-I/O test file should have metadata")
			.len() as usize;
		let destination = context.build_buffer::<[u8; BYTES.len()]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("DirectStorage GDeflate test queue should be created");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path).compression(ResourceIoCompression::GDeflate1))
			.expect("DirectStorage should open the GDeflate test file");
		let source = ResourceIoFileRegion::new(file, 0).stored_range(0, stored_size);
		let request = ResourceIoBufferLoad::new(source, destination, 0, BYTES.len()).into();
		let ticket = queue
			.submit(&context, Some("GDeflate Buffer Load"), &[request])
			.expect("DirectStorage should submit the GDeflate buffer batch");

		ticket.wait().expect("DirectStorage GDeflate buffer batch should complete");
		drop(ticket);
		drop(queue);
		assert_eq!(
			read_buffer(&mut context, queue_handle, destination.into(), BYTES.len()),
			BYTES
		);
		fs::remove_file(path).expect("DX12 compressed resource-I/O test file should be removable");
	}

	#[test]
	fn raw_file_load_populates_a_dx12_image_region() {
		const PIXELS: &[u8; 16] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
		if DirectStorageRuntime::load().is_err() {
			return;
		}
		let Some((mut context, queue_handle)) = test_context() else {
			return;
		};
		let image = context.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			)
			.extent(utils::Extent::rectangle(2, 2))
			.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let layout = context
			.resource_io_image_source_layout(crate::Formats::RGBA8UNORM, utils::Extent::rectangle(2, 2))
			.expect("DX12 resource-I/O test image should expose a source layout");
		let mut stored = vec![0u8; layout.total_bytes];
		stored[0..8].copy_from_slice(&PIXELS[0..8]);
		stored[layout.bytes_per_row..layout.bytes_per_row + 8].copy_from_slice(&PIXELS[8..16]);
		let path = temporary_path("raw-image");
		fs::write(&path, stored).expect("DX12 raw image resource-I/O test file should be writable");
		let mut queue = context
			.create_resource_io_queue(ResourceIoQueueDescriptor::new())
			.expect("DirectStorage image test queue should be created");
		let file = queue
			.open_file(ResourceIoFileDescriptor::new(&path))
			.expect("DirectStorage should open the raw image test file");
		let request = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(file, 0),
			image,
			0,
			0,
			utils::Extent::rectangle(2, 2),
			layout.bytes_per_row,
			layout.bytes_per_image,
		)
		.into();
		let ticket = queue
			.submit(&context, Some("Raw Image Load"), &[request])
			.expect("DirectStorage should submit the raw image batch");

		ticket.wait().expect("DirectStorage raw image batch should complete");
		drop(ticket);
		drop(queue);
		assert_eq!(read_image(&mut context, queue_handle, image), PIXELS);
		fs::remove_file(path).expect("DX12 raw image resource-I/O test file should be removable");
	}
}
