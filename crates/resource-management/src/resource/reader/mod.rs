pub mod redb;

use std::{fmt::Debug, ops::Range};

use memmap2::{Mmap, MmapOptions};

use super::{ReadTargets, ReadTargetsMut};
use crate::{r#async::BoxedFuture, StreamDescription};

#[derive(Debug)]
/// The `ResourceReaderBacking` enum provides reusable, reader-owned storage for resource bytes.
pub enum ResourceReaderBacking {
	Buffer(Box<[u8]>),
	MappedFile(MappedFileBacking),
}

impl ResourceReaderBacking {
	/// Returns the resource bytes from the current backing storage.
	pub fn as_slice(&self) -> &[u8] {
		match self {
			ResourceReaderBacking::Buffer(buffer) => buffer,
			ResourceReaderBacking::MappedFile(mapped_file) => mapped_file.as_slice(),
		}
	}
}

#[derive(Debug)]
/// The `MappedFileBacking` struct provides borrowed access to a file payload without a heap copy.
pub struct MappedFileBacking {
	map: Mmap,
	range: Range<usize>,
}

impl MappedFileBacking {
	/// Creates a mapped-file backing for the full file contents.
	pub fn new(file: impl memmap2::MmapAsRawDesc) -> Result<Self, ()> {
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		let range = 0..map.len();
		Ok(Self { map, range })
	}

	/// Creates a mapped-file backing that exposes one logical range from a shared payload file.
	pub fn new_range(file: impl memmap2::MmapAsRawDesc, offset: u64, size: u64) -> Result<Self, ()> {
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		let start = usize::try_from(offset).map_err(|_| ())?;
		let size = usize::try_from(size).map_err(|_| ())?;
		let end = start.checked_add(size).ok_or(())?;
		if end > map.len() {
			return Err(());
		}
		Ok(Self { map, range: start..end })
	}

	/// Returns the logical resource bytes from the mapped file.
	pub fn as_slice(&self) -> &[u8] {
		&self.map[self.range.clone()]
	}
}

/// The `ResourceReader` trait provides binary data for one [`Reference`](crate::Reference).
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>>;

	/// Consumes the reader and returns its owned backing when the caller can reuse it directly.
	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>>;
}
