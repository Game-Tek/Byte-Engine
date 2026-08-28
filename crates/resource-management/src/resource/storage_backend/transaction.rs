use std::future::Future;

use compio::{
	buf::{BufResult, IoBuf},
	io::{AsyncWrite, AsyncWriteAtExt},
};

use crate::{
	ProcessedAsset, SerializableResource,
	resource::{ResourceId, ResourcePayloadEncoding},
};

const FILE_WRITE_BUFFER_SIZE: usize = 64 * 1024;

/// The `ResourceTransaction` struct reserves exact storage before a processor writes a resource payload.
///
/// Call [`Self::write_all`] with borrowed payload bytes, then pass the completed
/// transaction to [`Self::commit`]. File targets buffer small writes and flush
/// them asynchronously. Memory-backed targets complete without suspending.
/// Transactions are incremental, so they preserve payload bytes without CPU compression.
pub struct ResourceTransaction<'a> {
	// Backend dispatch happens once during commit. Payload writes only touch the
	// concrete writer below, so the processor's hot path never uses a dyn writer.
	guard: ResourceReservationGuard<'a>,
	resource_id: ResourceId,
	writer: ResourceWriter,
	decoded_payload: Option<DecodedResourcePayload>,
}

/// The `DecodedResourcePayload` struct carries a precomputed client-facing identity for encoded bytes.
struct DecodedResourcePayload {
	decoded_size: usize,
	encoding: ResourcePayloadEncoding,
}

#[derive(Clone, Copy)]
/// The `ResourceReservation` struct identifies backend storage that must be returned after a canceled write.
pub(super) struct ResourceReservation {
	pub(super) offset: u64,
	pub(super) size: u64,
}

/// The `ResourceReservationGuard` struct returns unpublished storage when a transaction is canceled or fails.
struct ResourceReservationGuard<'a> {
	backend: &'a dyn ResourceTransactionCommit,
	reservation: Option<ResourceReservation>,
}

impl Drop for ResourceReservationGuard<'_> {
	fn drop(&mut self) {
		if let Some(reservation) = self.reservation.take() {
			self.backend.abort_resource(reservation);
		}
	}
}

impl<'a> ResourceTransaction<'a> {
	pub(super) fn new(
		backend: &'a dyn ResourceTransactionCommit,
		resource_id: ResourceId,
		reservation: Option<ResourceReservation>,
		writer: ResourceWriter,
	) -> Self {
		Self {
			guard: ResourceReservationGuard { backend, reservation },
			resource_id,
			decoded_payload: None,
			writer,
		}
	}

	/// Records the decoded identity of a complete payload compressed before this transaction began.
	pub(crate) fn with_compressed_payload(
		mut self,
		decoded_size: usize,
		decoded_hash: u64,
		encoding: ResourcePayloadEncoding,
	) -> Self {
		assert!(
			encoding.requires_cpu_decompression(),
			"Compressed resource metadata requires a CPU encoding. The most likely cause is that a prepared payload was marked as raw or GPU-backed."
		);
		// The decoded hash is already known, so hashing the encoded bytes while
		// writing would add work and produce an identity that is immediately discarded.
		self.writer.set_hash(decoded_hash);
		self.decoded_payload = Some(DecodedResourcePayload { decoded_size, encoding });
		self
	}

	/// Returns the exact payload size reserved by the storage backend.
	pub fn expected_size(&self) -> usize {
		self.writer.expected_size
	}

	/// Returns the number of payload bytes accepted so far.
	pub fn written_size(&self) -> usize {
		self.writer.written_size
	}

	#[cfg(test)]
	pub(super) fn direct_write_count(&self) -> usize {
		self.writer.target.direct_write_count()
	}

	#[cfg(test)]
	pub(super) fn buffered_size(&self) -> usize {
		self.writer.target.buffered_size()
	}

	#[cfg(test)]
	pub(super) fn staging_buffer_capacity(&self) -> usize {
		self.writer.target.staging_buffer_capacity()
	}

	/// Writes all borrowed bytes into the reserved payload.
	///
	/// Memory-backed targets copy immediately. File-backed targets suspend only
	/// when their internal buffer must be flushed through Compio.
	pub async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
		self.writer.write_all(data).await
	}

	/// Publishes the resource metadata after the exact reserved payload has been written.
	///
	/// Commit fails when the resource ID differs from the reserved ID or when the
	/// processor wrote a different number of bytes than it declared.
	pub async fn commit(
		self,
		resource: ProcessedAsset,
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()> {
		let Self {
			mut guard,
			resource_id,
			writer,
			decoded_payload,
		} = self;
		if ResourceId::from(resource.id()) != resource_id {
			return Err(());
		}

		let output = writer.finish(decoded_payload).await.map_err(|_| ())?;
		let stored = guard
			.backend
			.commit_resource(resource_id, guard.reservation, resource, output, allocator)?;
		guard.reservation = None;
		Ok(stored)
	}
}

impl AsyncWrite for ResourceTransaction<'_> {
	async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
		let size = buffer.buf_len();
		let BufResult(result, buffer) = self.writer.write_owned(buffer).await;
		BufResult(result.map(|()| size), buffer)
	}

	async fn flush(&mut self) -> std::io::Result<()> {
		self.writer.flush().await
	}

	async fn shutdown(&mut self) -> std::io::Result<()> {
		self.writer.flush().await
	}
}

/// Prepares and writes one complete owned payload through a caller-selected transaction factory.
pub(crate) async fn write_complete_owned_resource<'a, T, Begin, BeginFuture>(
	data: T,
	compression_policy: crate::resource::ResourceCompressionPolicy,
	begin: Begin,
) -> Result<ResourceTransaction<'a>, ()>
where
	T: IoBuf,
	Begin: FnOnce(usize) -> BeginFuture,
	BeginFuture: Future<Output = Result<ResourceTransaction<'a>, ()>>,
{
	let prepared = crate::resource::compression::prepare(data.as_init(), compression_policy);
	if let Some(prepared) = prepared {
		let mut transaction = begin(prepared.bytes.len()).await?.with_compressed_payload(
			prepared.decoded_size,
			prepared.decoded_hash,
			prepared.encoding,
		);
		let BufResult(result, _) = compio::io::AsyncWriteExt::write_all(&mut transaction, prepared.bytes).await;
		result.map_err(|_| ())?;
		Ok(transaction)
	} else {
		let mut transaction = begin(data.buf_len()).await?;
		let BufResult(result, _) = compio::io::AsyncWriteExt::write_all(&mut transaction, data).await;
		result.map_err(|_| ())?;
		Ok(transaction)
	}
}

/// The `ResourceTransactionCommit` trait keeps backend publication outside the payload write hot path.
pub(super) trait ResourceTransactionCommit: Sync {
	fn abort_resource(&self, _reservation: ResourceReservation) {}

	fn commit_resource(
		&self,
		resource_id: ResourceId,
		backend_reservation: Option<ResourceReservation>,
		resource: ProcessedAsset,
		output: ResourceWriteOutput,
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()>;
}

/// The `ResourceWriter` struct enforces one reservation while routing bytes to a concrete storage target.
pub(super) struct ResourceWriter {
	target: ResourceWriteTarget,
	expected_size: usize,
	written_size: usize,
	hash: ResourceHash,
}

/// The `ResourceHash` enum either computes stored-byte identity or carries a known decoded identity.
enum ResourceHash {
	Compute(md5::Context),
	Known(u64),
}

impl ResourceHash {
	fn consume(&mut self, data: &[u8]) {
		if let Self::Compute(hasher) = self {
			hasher.consume(data);
		}
	}

	fn finish(self) -> u64 {
		match self {
			Self::Compute(hasher) => digest_hash(hasher.finalize()),
			Self::Known(hash) => hash,
		}
	}
}

impl ResourceWriter {
	#[cfg(test)]
	pub(super) fn memory(expected_size: usize) -> Result<Self, ()> {
		let mut data = Vec::new();
		data.try_reserve_exact(expected_size).map_err(|_| ())?;
		Ok(Self::new(ResourceWriteTarget::Memory(data), expected_size))
	}

	pub(super) fn reserved_file(file: compio::fs::File, offset: u64, expected_size: usize) -> Self {
		Self::new(
			ResourceWriteTarget::ReservedFile(AsyncResourceFile::new(file, offset, expected_size)),
			expected_size,
		)
	}

	pub(super) fn staged_file(file: compio::fs::File, staging: StagedResourceFile, expected_size: usize) -> Self {
		Self::new(
			ResourceWriteTarget::StagedFile {
				file: AsyncResourceFile::new(file, 0, expected_size),
				staging,
			},
			expected_size,
		)
	}

	fn new(target: ResourceWriteTarget, expected_size: usize) -> Self {
		Self {
			target,
			expected_size,
			written_size: 0,
			hash: ResourceHash::Compute(md5::Context::new()),
		}
	}

	/// Uses a precomputed decoded hash instead of hashing the stored payload bytes.
	fn set_hash(&mut self, hash: u64) {
		self.hash = ResourceHash::Known(hash);
	}

	/// Copies one borrowed input into the concrete target and updates transaction accounting.
	async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
		let remaining = self.expected_size - self.written_size;
		if data.len() > remaining {
			return Err(std::io::Error::new(
				std::io::ErrorKind::InvalidInput,
				"Resource transaction exceeds its reservation. The processor wrote more bytes than it declared.",
			));
		}

		self.target.write_all(data).await?;
		self.hash.consume(data);
		self.written_size += data.len();
		Ok(())
	}

	/// Routes an owned Compio buffer without copying when it is at least as large as the file buffer.
	async fn write_owned<T: IoBuf>(&mut self, buffer: T) -> BufResult<(), T> {
		let size = buffer.buf_len();
		let remaining = self.expected_size - self.written_size;
		if size > remaining {
			return BufResult(
				Err(std::io::Error::new(
					std::io::ErrorKind::InvalidInput,
					"Resource transaction exceeds its reservation. The processor wrote more bytes than it declared.",
				)),
				buffer,
			);
		}

		let BufResult(result, buffer) = self.target.write_owned(buffer).await;
		if result.is_ok() {
			self.hash.consume(buffer.as_init());
			self.written_size += size;
		}
		BufResult(result, buffer)
	}

	async fn flush(&mut self) -> std::io::Result<()> {
		self.target.flush().await
	}

	/// Verifies the exact-size contract and durably finishes payload I/O before metadata publication.
	async fn finish(self, decoded_payload: Option<DecodedResourcePayload>) -> std::io::Result<ResourceWriteOutput> {
		if self.written_size != self.expected_size {
			return Err(std::io::Error::new(
				std::io::ErrorKind::UnexpectedEof,
				"Resource transaction is incomplete. The processor wrote fewer bytes than it reserved.",
			));
		}

		let target = self.target.finish().await?;
		let hash = self.hash.finish();
		let (size, encoding) = if let Some(decoded) = decoded_payload {
			(decoded.decoded_size, decoded.encoding)
		} else {
			(self.written_size, ResourcePayloadEncoding::Raw)
		};

		Ok(ResourceWriteOutput {
			target,
			hash,
			size,
			stored_size: self.written_size,
			encoding,
		})
	}
}

// A closed target set keeps each write as a direct enum branch without a boxed writer.
enum ResourceWriteTarget {
	#[cfg(test)]
	Memory(Vec<u8>),
	ReservedFile(AsyncResourceFile),
	StagedFile {
		file: AsyncResourceFile,
		staging: StagedResourceFile,
	},
}

impl ResourceWriteTarget {
	async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
		match self {
			#[cfg(test)]
			Self::Memory(output) => {
				output.extend_from_slice(data);
				Ok(())
			}
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.write_all(data).await,
		}
	}

	async fn flush(&mut self) -> std::io::Result<()> {
		match self {
			#[cfg(test)]
			Self::Memory(_) => Ok(()),
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.flush().await,
		}
	}

	async fn write_owned<T: IoBuf>(&mut self, buffer: T) -> BufResult<(), T> {
		match self {
			#[cfg(test)]
			Self::Memory(output) => {
				output.extend_from_slice(buffer.as_init());
				BufResult(Ok(()), buffer)
			}
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.write_owned(buffer).await,
		}
	}

	#[cfg(test)]
	fn direct_write_count(&self) -> usize {
		match self {
			Self::Memory(_) => 0,
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.direct_write_count,
		}
	}

	#[cfg(test)]
	fn buffered_size(&self) -> usize {
		match self {
			Self::Memory(_) => 0,
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.buffer.len(),
		}
	}

	#[cfg(test)]
	fn staging_buffer_capacity(&self) -> usize {
		match self {
			Self::Memory(output) => output.capacity(),
			Self::ReservedFile(output) | Self::StagedFile { file: output, .. } => output.buffer.capacity(),
		}
	}

	async fn finish(self) -> std::io::Result<FinishedResourceWriteTarget> {
		match self {
			#[cfg(test)]
			Self::Memory(data) => Ok(FinishedResourceWriteTarget::Memory(data)),
			Self::ReservedFile(file) => {
				file.finish().await?;
				Ok(FinishedResourceWriteTarget::ReservedFile)
			}
			Self::StagedFile { file, staging } => {
				file.finish().await?;
				Ok(FinishedResourceWriteTarget::StagedFile(staging))
			}
		}
	}
}

/// The `AsyncResourceFile` struct bridges borrowed processor slices to Compio's owned-buffer file operations.
struct AsyncResourceFile {
	file: compio::fs::File,
	base_offset: u64,
	flushed_size: usize,
	buffer: Vec<u8>,
	desired_buffer_capacity: usize,
	#[cfg(test)]
	direct_write_count: usize,
}

impl AsyncResourceFile {
	fn new(file: compio::fs::File, base_offset: u64, expected_size: usize) -> Self {
		Self {
			file,
			base_offset,
			flushed_size: 0,
			buffer: Vec::new(),
			desired_buffer_capacity: expected_size.min(FILE_WRITE_BUFFER_SIZE),
			#[cfg(test)]
			direct_write_count: 0,
		}
	}

	/// Buffers borrowed input and applies backpressure whenever the owned file buffer fills.
	async fn write_all(&mut self, mut data: &[u8]) -> std::io::Result<()> {
		if data.is_empty() {
			return Ok(());
		}
		self.ensure_buffer()?;

		while !data.is_empty() {
			let available = self.buffer.capacity() - self.buffer.len();
			if available == 0 {
				self.flush().await?;
				continue;
			}

			let copied = available.min(data.len());
			self.buffer.extend_from_slice(&data[..copied]);
			data = &data[copied..];
		}
		Ok(())
	}

	/// Allocates the reusable staging buffer only when a write must copy into it.
	fn ensure_buffer(&mut self) -> std::io::Result<()> {
		if self.buffer.capacity() != 0 {
			return Ok(());
		}

		self.buffer.try_reserve_exact(self.desired_buffer_capacity).map_err(|_| {
			std::io::Error::other(
				"Resource write buffer allocation failed. The process likely does not have enough memory for the staging buffer.",
			)
		})
	}

	/// Flushes pending small writes, then submits a large owned buffer directly to its reserved offset.
	async fn write_owned<T: IoBuf>(&mut self, buffer: T) -> BufResult<(), T> {
		if buffer.buf_len() < FILE_WRITE_BUFFER_SIZE {
			return BufResult(self.write_all(buffer.as_init()).await, buffer);
		}

		if let Err(error) = self.flush().await {
			return BufResult(Err(error), buffer);
		}

		let write_offset = match self.current_write_offset() {
			Ok(offset) => offset,
			Err(error) => return BufResult(Err(error), buffer),
		};
		let size = buffer.buf_len();
		let BufResult(result, buffer) = self.file.write_all_at(buffer, write_offset).await;
		if result.is_ok() {
			self.flushed_size += size;
			#[cfg(test)]
			{
				self.direct_write_count += 1;
			}
		}
		BufResult(result, buffer)
	}

	/// Submits the accumulated owned buffer at its reserved file offset and reuses it after completion.
	async fn flush(&mut self) -> std::io::Result<()> {
		if self.buffer.is_empty() {
			return Ok(());
		}

		let write_offset = self.current_write_offset()?;
		let buffer = std::mem::take(&mut self.buffer);
		let size = buffer.len();
		let BufResult(result, mut buffer) = self.file.write_all_at(buffer, write_offset).await;
		if let Err(error) = result {
			self.buffer = buffer;
			return Err(error);
		}
		buffer.clear();
		self.buffer = buffer;
		self.flushed_size += size;
		Ok(())
	}

	fn current_write_offset(&self) -> std::io::Result<u64> {
		self.base_offset
			.checked_add(u64::try_from(self.flushed_size).map_err(|_| {
				std::io::Error::other(
					"Resource write offset overflowed. The reserved payload is likely larger than a u64 file extent.",
				)
			})?)
			.ok_or_else(|| {
				std::io::Error::other(
					"Resource write offset overflowed. The reserved offset and payload size likely exceed a u64 file extent.",
				)
			})
	}

	/// Flushes and synchronizes the file, then closes its transaction-owned handle.
	async fn finish(mut self) -> std::io::Result<()> {
		self.flush().await?;
		self.file.sync_data().await?;
		self.file.close().await?;
		Ok(())
	}
}

fn digest_hash(digest: md5::Digest) -> u64 {
	u64::from_le_bytes(digest.0[..8].try_into().expect("MD5 digest should contain eight bytes"))
}

/// The `StagedResourceFile` struct keeps an unpublished file removable until metadata publication succeeds.
pub(super) struct StagedResourceFile {
	path: std::path::PathBuf,
	remove_on_drop: bool,
}

impl StagedResourceFile {
	pub(super) fn new(path: std::path::PathBuf) -> Self {
		Self {
			path,
			remove_on_drop: true,
		}
	}

	/// Moves a completed staging file into place, or reuses an identical existing extent.
	fn persist(mut self, destination: &std::path::Path, expected_size: u64) -> Result<(), ()> {
		match std::fs::metadata(destination) {
			Ok(metadata) if metadata.len() == expected_size => return Ok(()),
			Ok(_) => return Err(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(_) => return Err(()),
		}

		match std::fs::rename(&self.path, destination) {
			Ok(()) => {
				self.remove_on_drop = false;
				Ok(())
			}
			// Another transaction may have published the same content between the
			// metadata check and rename. Reuse it only when the extent matches.
			Err(_) if std::fs::metadata(destination).is_ok_and(|metadata| metadata.len() == expected_size) => Ok(()),
			Err(_) => Err(()),
		}
	}

	/// Encodes the staged decoded bytes into one native GPU I/O container before publication.
	#[cfg(feature = "gpu-processing")]
	fn compress(self, destination: &std::path::Path, encoding: ResourcePayloadEncoding) -> Result<usize, ()> {
		if destination.exists() {
			return usize::try_from(std::fs::metadata(destination).map_err(|_| ())?.len()).map_err(|_| ());
		}

		let source = std::fs::File::open(&self.path).map_err(|_| ())?;
		// SAFETY: The completed staging file is not mutated while this immutable
		// mapping is alive, and the file handle remains open through compression.
		let decoded = unsafe { memmap2::MmapOptions::new().map(&source) }.map_err(|_| ())?;
		let compressed_staging = self.path.with_extension("compressed");
		let method = match encoding {
			ResourcePayloadEncoding::MetalIoLz4 => ghi::io::ResourceIoCompression::Lz4,
			ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => return Err(()),
		};

		if ghi::io::write_compressed_file(&compressed_staging, method, &decoded).is_err() {
			let _ = std::fs::remove_file(&compressed_staging);
			return Err(());
		}
		drop(decoded);

		match std::fs::rename(&compressed_staging, destination) {
			Ok(()) => usize::try_from(std::fs::metadata(destination).map_err(|_| ())?.len()).map_err(|_| ()),
			Err(_) if destination.exists() => {
				let _ = std::fs::remove_file(compressed_staging);
				usize::try_from(std::fs::metadata(destination).map_err(|_| ())?.len()).map_err(|_| ())
			}
			Err(_) => {
				let _ = std::fs::remove_file(compressed_staging);
				Err(())
			}
		}
	}
}

impl Drop for StagedResourceFile {
	fn drop(&mut self) {
		if self.remove_on_drop {
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

enum FinishedResourceWriteTarget {
	#[cfg(test)]
	Memory(Vec<u8>),
	ReservedFile,
	StagedFile(StagedResourceFile),
}

/// The `ResourceWriteOutput` struct carries one durable payload into backend metadata publication.
pub(super) struct ResourceWriteOutput {
	target: FinishedResourceWriteTarget,
	hash: u64,
	/// Number of bytes clients receive after CPU decompression.
	size: usize,
	/// Number of bytes occupied by the prepared payload before backend-specific encoding.
	stored_size: usize,
	encoding: ResourcePayloadEncoding,
}

impl ResourceWriteOutput {
	pub(super) fn hash(&self) -> u64 {
		self.hash
	}

	pub(super) fn size(&self) -> usize {
		self.size
	}

	pub(super) fn stored_size(&self) -> usize {
		self.stored_size
	}

	pub(super) fn encoding(&self) -> ResourcePayloadEncoding {
		self.encoding
	}

	#[cfg(test)]
	pub(super) fn into_memory(self) -> Result<Vec<u8>, ()> {
		match self.target {
			FinishedResourceWriteTarget::Memory(data) => Ok(data),
			_ => Err(()),
		}
	}

	pub(super) fn finish_reserved_file(self) -> Result<(), ()> {
		match self.target {
			FinishedResourceWriteTarget::ReservedFile => Ok(()),
			_ => Err(()),
		}
	}

	pub(super) fn persist_staged_file(self, destination: &std::path::Path) -> Result<(), ()> {
		let expected_size = u64::try_from(self.stored_size).map_err(|_| ())?;
		match self.target {
			FinishedResourceWriteTarget::StagedFile(file) => file.persist(destination, expected_size),
			_ => Err(()),
		}
	}

	pub(super) fn persist_compressed_file(
		self,
		destination: &std::path::Path,
		encoding: ResourcePayloadEncoding,
	) -> Result<usize, ()> {
		#[cfg(feature = "gpu-processing")]
		{
			match self.target {
				FinishedResourceWriteTarget::StagedFile(file) => file.compress(destination, encoding),
				_ => Err(()),
			}
		}

		#[cfg(not(feature = "gpu-processing"))]
		{
			let _ = (destination, encoding);
			Err(())
		}
	}
}
