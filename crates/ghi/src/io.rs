//! Load file regions into GPU resources through backend-native storage queues.
//!
//! Resource I/O is separate from graphics command queues because APIs such as
//! Metal I/O and DirectStorage use dedicated queue and file-handle types. Create
//! a queue through [`ResourceIoContext`], open source files on that queue, then
//! submit batches of [`ResourceIoRequest`] values.

bitflags::bitflags! {
	/// Identifies the compression containers a resource-I/O backend can decode.
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub struct ResourceIoCompressionMethods: u16 {
		const GDEFLATE_1 = 1 << 0;
		const ZLIB = 1 << 1;
		const LZFSE = 1 << 2;
		const LZ4 = 1 << 3;
		const LZMA = 1 << 4;
		const LZ_BITMAP = 1 << 5;
	}
}

bitflags::bitflags! {
	/// Identifies the source locations a resource-I/O backend can read.
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub struct ResourceIoSourceKinds: u8 {
		const FILE = 1 << 0;
		const MEMORY = 1 << 1;
	}
}

bitflags::bitflags! {
	/// Identifies the destination locations a resource-I/O backend can populate.
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub struct ResourceIoDestinationKinds: u8 {
		const BUFFER = 1 << 0;
		const IMAGE_REGION = 1 << 1;
		const IMAGE_SUBRESOURCE_RANGE = 1 << 2;
		const IMAGE_TILES = 1 << 3;
		const HOST_MEMORY = 1 << 4;
	}
}

bitflags::bitflags! {
	/// Identifies optional scheduling behavior a resource-I/O backend provides.
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub struct ResourceIoFeatures: u8 {
		const CANCELLATION = 1 << 0;
		const TIMELINE_SYNCHRONIZATION = 1 << 1;
	}
}

/// Selects how a resource-I/O source file is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResourceIoCompression {
	#[default]
	None,
	GDeflate1,
	Zlib,
	Lzfse,
	Lz4,
	Lzma,
	LzBitmap,
}

impl ResourceIoCompression {
	/// Returns the capability bit for a compressed format, or `None` for raw data.
	pub fn method(self) -> Option<ResourceIoCompressionMethods> {
		match self {
			Self::None => None,
			Self::GDeflate1 => Some(ResourceIoCompressionMethods::GDEFLATE_1),
			Self::Zlib => Some(ResourceIoCompressionMethods::ZLIB),
			Self::Lzfse => Some(ResourceIoCompressionMethods::LZFSE),
			Self::Lz4 => Some(ResourceIoCompressionMethods::LZ4),
			Self::Lzma => Some(ResourceIoCompressionMethods::LZMA),
			Self::LzBitmap => Some(ResourceIoCompressionMethods::LZ_BITMAP),
		}
	}
}

/// Writes one backend-native compressed file from decoded bytes.
///
/// Offline resource tools use this function to create files accepted by
/// [`ResourceIoQueue::open_file`]. The selected backend must support the same
/// compression method at runtime.
pub fn write_compressed_file(path: &Path, compression: ResourceIoCompression, decoded: &[u8]) -> Result<(), ResourceIoError> {
	#[cfg(target_os = "macos")]
	{
		crate::metal::write_compressed_file(path, compression, decoded)
	}

	#[cfg(not(target_os = "macos"))]
	{
		let _ = (path, decoded);
		Err(ResourceIoError::UnsupportedCompression(compression))
	}
}

/// Selects the relative scheduling priority for a resource-I/O queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResourceIoPriority {
	High,
	#[default]
	Normal,
	Low,
}

/// Selects whether independent resource-I/O commands may execute concurrently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResourceIoQueueType {
	#[default]
	Concurrent,
	Serial,
}

/// The `ResourceIoCapabilities` struct describes the native loading paths available on one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIoCapabilities {
	/// Source locations the backend can read without a portable fallback.
	pub sources: ResourceIoSourceKinds,
	/// Destination locations the backend can populate without a portable fallback.
	pub destinations: ResourceIoDestinationKinds,
	/// Compressed container formats the backend can decode.
	pub compression: ResourceIoCompressionMethods,
	/// Optional queue behavior exposed by the backend.
	pub features: ResourceIoFeatures,
}

impl ResourceIoCapabilities {
	/// Returns whether the device can open a file using `compression`.
	pub fn supports_compression(&self, compression: ResourceIoCompression) -> bool {
		compression.method().is_none_or(|method| self.compression.contains(method))
	}
}

/// The `ResourceIoQueueDescriptor` struct configures a persistent native storage queue.
#[derive(Clone, Copy, Debug)]
pub struct ResourceIoQueueDescriptor<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) priority: ResourceIoPriority,
	pub(crate) queue_type: ResourceIoQueueType,
	pub(crate) max_commands_in_flight: usize,
	pub(crate) max_batches_in_flight: usize,
}

impl<'a> ResourceIoQueueDescriptor<'a> {
	/// Creates a concurrent, normal-priority queue with backend-selected limits.
	pub fn new() -> Self {
		Self {
			name: None,
			priority: ResourceIoPriority::Normal,
			queue_type: ResourceIoQueueType::Concurrent,
			max_commands_in_flight: 0,
			max_batches_in_flight: 0,
		}
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	pub fn priority(mut self, priority: ResourceIoPriority) -> Self {
		self.priority = priority;
		self
	}

	pub fn queue_type(mut self, queue_type: ResourceIoQueueType) -> Self {
		self.queue_type = queue_type;
		self
	}

	/// Limits simultaneous native I/O commands, or selects the backend default with `0`.
	pub fn max_commands_in_flight(mut self, count: usize) -> Self {
		self.max_commands_in_flight = count;
		self
	}

	/// Limits queued batches, or selects the backend default with `0`.
	pub fn max_batches_in_flight(mut self, count: usize) -> Self {
		self.max_batches_in_flight = count;
		self
	}
}

impl Default for ResourceIoQueueDescriptor<'_> {
	fn default() -> Self {
		Self::new()
	}
}

/// The `ResourceIoFileDescriptor` struct identifies a raw or compressed file opened by a native storage API.
#[derive(Clone, Copy, Debug)]
pub struct ResourceIoFileDescriptor<'a> {
	pub(crate) path: &'a Path,
	pub(crate) compression: ResourceIoCompression,
	pub(crate) name: Option<&'a str>,
}

impl<'a> ResourceIoFileDescriptor<'a> {
	/// Creates a descriptor for one raw file.
	pub fn new(path: &'a Path) -> Self {
		Self {
			path,
			compression: ResourceIoCompression::None,
			name: None,
		}
	}

	pub fn compression(mut self, compression: ResourceIoCompression) -> Self {
		self.compression = compression;
		self
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}
}

/// The `ResourceIoFileHandle` struct identifies a file opened on one resource-I/O queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceIoFileHandle {
	pub(crate) queue: u64,
	pub(crate) index: u64,
}

/// The `ResourceIoTimelinePoint` struct identifies when one queue batch has completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceIoTimelinePoint {
	pub(crate) queue: u64,
	pub(crate) value: u64,
}

/// The `ResourceIoFileRegion` struct selects bytes from an opened source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIoFileRegion {
	pub(crate) file: ResourceIoFileHandle,
	pub(crate) decoded_offset: usize,
	pub(crate) stored_range: Option<ResourceIoStoredRange>,
}

impl ResourceIoFileRegion {
	/// Selects bytes starting at `offset` in the file's logical decoded address space.
	pub fn new(file: ResourceIoFileHandle, offset: usize) -> Self {
		Self {
			file,
			decoded_offset: offset,
			stored_range: None,
		}
	}

	/// Supplies the physical compressed range for APIs that load independent blocks.
	pub fn stored_range(mut self, offset: u64, size: usize) -> Self {
		self.stored_range = Some(ResourceIoStoredRange { offset, size });
		self
	}
}

/// The `ResourceIoStoredRange` struct selects one physical compressed block in a source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIoStoredRange {
	/// Byte offset in the physical compressed file.
	pub offset: u64,
	/// Compressed byte count available to the request.
	pub size: usize,
}

/// The `ResourceIoBufferLoad` struct describes one file-to-buffer request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIoBufferLoad {
	pub(crate) source: ResourceIoFileRegion,
	pub(crate) destination: BaseBufferHandle,
	pub(crate) destination_offset: usize,
	pub(crate) size: usize,
}

impl ResourceIoBufferLoad {
	/// Creates a request that writes decoded bytes into one buffer region.
	pub fn new(
		source: ResourceIoFileRegion,
		destination: impl Into<BaseBufferHandle>,
		destination_offset: usize,
		size: usize,
	) -> Self {
		Self {
			source,
			destination: destination.into(),
			destination_offset,
			size,
		}
	}
}

/// The `ResourceIoImageLoad` struct describes one file-to-image-subresource request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIoImageLoad {
	pub(crate) source: ResourceIoFileRegion,
	pub(crate) destination: BaseImageHandle,
	pub(crate) array_layer: u32,
	pub(crate) mip_level: u32,
	pub(crate) origin: Extent,
	pub(crate) extent: Extent,
	pub(crate) source_bytes_per_row: usize,
	pub(crate) source_bytes_per_image: usize,
}

impl ResourceIoImageLoad {
	/// Creates a request that writes decoded texels into one image subresource region.
	pub fn new(
		source: ResourceIoFileRegion,
		destination: impl Into<BaseImageHandle>,
		array_layer: u32,
		mip_level: u32,
		extent: Extent,
		source_bytes_per_row: usize,
		source_bytes_per_image: usize,
	) -> Self {
		Self {
			source,
			destination: destination.into(),
			array_layer,
			mip_level,
			origin: Extent::new(0, 0, 0),
			extent,
			source_bytes_per_row,
			source_bytes_per_image,
		}
	}

	pub fn origin(mut self, origin: Extent) -> Self {
		self.origin = origin;
		self
	}
}

/// Describes one independently schedulable file-to-resource operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceIoRequest {
	Buffer(ResourceIoBufferLoad),
	Image(ResourceIoImageLoad),
}

impl From<ResourceIoBufferLoad> for ResourceIoRequest {
	fn from(value: ResourceIoBufferLoad) -> Self {
		Self::Buffer(value)
	}
}

impl From<ResourceIoImageLoad> for ResourceIoRequest {
	fn from(value: ResourceIoImageLoad) -> Self {
		Self::Image(value)
	}
}

/// Describes the observable state of one submitted resource-I/O batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceIoStatus {
	Pending,
	Complete,
	Cancelled,
	Failed,
}

/// Identifies a resource-I/O setup, validation, or execution failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceIoError {
	EmptyBatch,
	UnsupportedCompression(ResourceIoCompression),
	InvalidPath,
	InvalidFileHandle,
	InvalidBufferHandle,
	InvalidImageHandle,
	InvalidSourceRange { request: usize },
	InvalidDestinationRange { request: usize },
	QueueCreation(String),
	FileOpen(String),
	Execution(String),
	Cancelled,
}

impl std::fmt::Display for ResourceIoError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::EmptyBatch => formatter.write_str(
				"Resource I/O batch is empty. The most likely cause is that submission occurred before any requests were collected.",
			),
			Self::UnsupportedCompression(compression) => write!(
				formatter,
				"Resource I/O compression is unsupported: {compression:?}. The most likely cause is that the payload was baked for another backend."
			),
			Self::InvalidPath => formatter.write_str(
				"Resource I/O path is invalid. The most likely cause is that the file path cannot be represented by the native storage API.",
			),
			Self::InvalidFileHandle => formatter.write_str(
				"Resource I/O file handle is invalid. The most likely cause is that the handle belongs to another queue or has not been opened.",
			),
			Self::InvalidBufferHandle => formatter.write_str(
				"Resource I/O buffer handle is invalid. The most likely cause is that the buffer belongs to another graphics context.",
			),
			Self::InvalidImageHandle => formatter.write_str(
				"Resource I/O image handle is invalid. The most likely cause is that the image belongs to another graphics context.",
			),
			Self::InvalidSourceRange { request } => write!(
				formatter,
				"Resource I/O request {request} has an invalid source range. The most likely cause is overflowing or inconsistent payload metadata."
			),
			Self::InvalidDestinationRange { request } => write!(
				formatter,
				"Resource I/O request {request} has an invalid destination range. The most likely cause is that the destination resource is smaller than the requested write."
			),
			Self::QueueCreation(message) => write!(
				formatter,
				"Resource I/O queue creation failed: {message}. The most likely cause is unavailable native storage support or exhausted device resources."
			),
			Self::FileOpen(message) => write!(
				formatter,
				"Resource I/O file open failed: {message}. The most likely cause is a missing file or a compression format that does not match its contents."
			),
			Self::Execution(message) => write!(
				formatter,
				"Resource I/O execution failed: {message}. The most likely cause is unreadable source data or an invalid native destination layout."
			),
			Self::Cancelled => formatter.write_str(
				"Resource I/O batch was cancelled. The most likely cause is that the caller abandoned an in-flight resource request.",
			),
		}
	}
}

impl std::error::Error for ResourceIoError {}

/// The `ResourceIoTicket` trait provides completion and cancellation access to one submitted batch.
pub trait ResourceIoTicket {
	/// Returns the latest nonblocking state reported by the native queue.
	fn status(&self) -> ResourceIoStatus;

	/// Waits for all requests in this batch and returns its execution result.
	fn wait(&self) -> Result<(), ResourceIoError>;

	/// Requests best-effort cancellation without waiting for the native queue.
	fn cancel(&self) -> Result<(), ResourceIoError>;

	/// Returns the queue-local timeline point signaled after every request completes.
	fn completion_point(&self) -> ResourceIoTimelinePoint;
}

/// The `ResourceIoQueue` trait provides file registration and batched native loading.
pub trait ResourceIoQueue {
	type Ticket: ResourceIoTicket;

	fn capabilities(&self) -> ResourceIoCapabilities;

	/// Opens a file using the same native device that owns this queue's destinations.
	fn open_file(&mut self, descriptor: ResourceIoFileDescriptor<'_>) -> Result<ResourceIoFileHandle, ResourceIoError>;

	/// Encodes and commits one independently completing request batch.
	fn submit(&mut self, name: Option<&str>, requests: &[ResourceIoRequest]) -> Result<Self::Ticket, ResourceIoError>;
}

/// The `ResourceIoContext` trait creates dedicated storage queues for context-owned GPU resources.
pub trait ResourceIoContext {
	type ResourceIoQueue: ResourceIoQueue;

	/// Creates a persistent queue that can populate resources owned by this context.
	fn create_resource_io_queue(
		&mut self,
		descriptor: ResourceIoQueueDescriptor<'_>,
	) -> Result<Self::ResourceIoQueue, ResourceIoError>;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn raw_compression_needs_no_capability_bit() {
		let capabilities = ResourceIoCapabilities {
			sources: ResourceIoSourceKinds::FILE,
			destinations: ResourceIoDestinationKinds::BUFFER,
			compression: ResourceIoCompressionMethods::empty(),
			features: ResourceIoFeatures::empty(),
		};

		assert!(capabilities.supports_compression(ResourceIoCompression::None));
		assert!(!capabilities.supports_compression(ResourceIoCompression::Lz4));
	}

	#[test]
	fn image_request_defaults_to_the_subresource_origin() {
		let request = ResourceIoImageLoad::new(
			ResourceIoFileRegion::new(ResourceIoFileHandle { queue: 1, index: 2 }, 32),
			BaseImageHandle(2),
			3,
			4,
			Extent::rectangle(16, 8),
			64,
			512,
		);

		assert_eq!(request.origin, Extent::new(0, 0, 0));
		assert_eq!(request.array_layer, 3);
		assert_eq!(request.mip_level, 4);
	}
}

use std::path::Path;

use utils::Extent;

use crate::{BaseBufferHandle, BaseImageHandle};
