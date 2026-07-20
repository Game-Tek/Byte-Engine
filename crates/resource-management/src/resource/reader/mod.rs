pub mod redb;

use std::fmt::Debug;

use memmap2::{Mmap, MmapOptions};

use super::{ReadTargets, ReadTargetsMut};
use crate::StreamDescription;

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
}

impl MappedFileBacking {
	/// Creates a mapped-file backing for the full file contents.
	pub fn new(file: &std::fs::File) -> Result<Self, ()> {
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		Ok(Self { map })
	}

	/// Returns the mapped file contents as a byte slice.
	pub fn as_slice(&self) -> &[u8] {
		&self.map[..]
	}
}

/// The `ResourceReader` trait provides binary data for one [`Reference`](crate::Reference).
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> Result<ReadTargets<'a>, ()>;

	/// Consumes the reader and returns its owned backing when the caller can reuse it directly.
	fn into_backing_storage(self: Box<Self>) -> Result<ResourceReaderBacking, Box<dyn ResourceReader>>;
}
