pub mod redb;

use std::{fmt::Debug, ops::Range, sync::Arc};

use memmap2::{Mmap, MmapOptions};

use super::{ReadTargets, ReadTargetsMut};
use crate::{StreamDescription, r#async::BoxedFuture};

#[derive(Debug)]
/// The `ResourceReaderBacking` enum provides reusable, reader-owned storage for resource bytes.
pub enum ResourceReaderBacking {
	Buffer(Box<[u8]>),
	MappedFile(MappedFileBacking),
	/// Data that must be decoded directly into a GPU resource by a native storage queue.
	Gpu(ResourceGpuBacking),
}

impl ResourceReaderBacking {
	/// Returns resource bytes when the backing is CPU-readable.
	pub fn try_as_slice(&self) -> Option<&[u8]> {
		match self {
			ResourceReaderBacking::Buffer(buffer) => Some(buffer),
			ResourceReaderBacking::MappedFile(mapped_file) => Some(mapped_file.as_slice()),
			ResourceReaderBacking::Gpu(_) => None,
		}
	}

	/// Returns resource bytes from CPU-readable backing storage.
	///
	/// # Panics
	///
	/// Panics when the backing requires native GPU resource I/O. Use
	/// [`Self::try_as_slice`] when either backing kind is valid.
	pub fn as_slice(&self) -> &[u8] {
		self.try_as_slice().expect(
			"Resource backing is not CPU-readable. The most likely cause is that compressed GPU data was passed to a CPU-only consumer.",
		)
	}
}

/// Selects the native container encoding used by GPU-backed resource data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResourceCompression {
	#[default]
	None,
	MetalIoLz4,
}

/// The `ResourceGpuBacking` struct provides the source file required by native GPU resource I/O.
#[derive(Debug)]
pub struct ResourceGpuBacking {
	path: std::path::PathBuf,
	compression: ResourceCompression,
}

impl ResourceGpuBacking {
	/// Creates a direct GPU source for one compressed resource file.
	pub fn new(path: std::path::PathBuf, compression: ResourceCompression) -> Self {
		Self { path, compression }
	}

	pub fn path(&self) -> &std::path::Path {
		&self.path
	}

	pub fn compression(&self) -> ResourceCompression {
		self.compression
	}
}

#[derive(Debug)]
/// The `MappedFileBacking` struct provides borrowed access to a file payload without a heap copy.
pub struct MappedFileBacking {
	map: Mmap,
	range: Range<usize>,
	// Packed backends attach a lease so a replaced slot cannot be reused while
	// this mapping still exposes its previous bytes.
	_lease: Option<Arc<()>>,
}

impl MappedFileBacking {
	/// Creates a mapped-file backing for the full file contents.
	pub fn new(file: impl memmap2::MmapAsRawDesc) -> Result<Self, ()> {
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		let range = 0..map.len();
		Ok(Self {
			map,
			range,
			_lease: None,
		})
	}

	/// Creates a mapped-file backing for one optionally leased range of a shared payload file.
	pub(crate) fn new_range(
		file: impl memmap2::MmapAsRawDesc,
		offset: u64,
		size: u64,
		lease: Option<Arc<()>>,
	) -> Result<Self, ()> {
		// Map once and keep the logical range separate so consumers can borrow
		// exactly one payload without copying it out of the shared file.
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		let start = usize::try_from(offset).map_err(|_| ())?;
		let size = usize::try_from(size).map_err(|_| ())?;
		let end = start.checked_add(size).ok_or(())?;
		if end > map.len() {
			return Err(());
		}
		Ok(Self {
			map,
			range: start..end,
			_lease: lease,
		})
	}

	/// Returns the logical resource bytes from the mapped file.
	pub fn as_slice(&self) -> &[u8] {
		&self.map[self.range.clone()]
	}
}

/// The `ResourceReader` trait provides binary data for one [`Reference`](crate::Reference).
pub trait ResourceReader: Send + Sync + Debug {
	/// Returns whether this reader owns a native GPU I/O source instead of CPU-readable bytes.
	fn is_gpu_backed(&self) -> bool {
		false
	}

	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>>;

	/// Consumes the reader and returns its owned backing when the caller can reuse it directly.
	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>>;
}
