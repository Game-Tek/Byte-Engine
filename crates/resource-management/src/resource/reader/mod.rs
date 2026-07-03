pub mod redb;

use std::fmt::Debug;

use memmap2::{Mmap, MmapOptions};

use super::{ReadTargets, ReadTargetsMut};
use crate::StreamDescription;

#[derive(Debug)]
/// The `ResourceReaderBacking` enum keeps resource bytes in an owned backing so clients can reuse them without forcing another read.
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
/// The `MappedFileBacking` struct keeps a resource payload mapped from disk so clients can borrow bytes without copying them into heap memory.
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

/// The resource reader trait provides methods to read a single resource's binary data.
/// An instance of a resource reader will usually be paired with a resource `Reference` to perform the loading of the resource.
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> Result<ReadTargets<'a>, ()>;

	/// Consumes the reader and returns its owned backing when the caller can reuse it directly.
	fn into_backing_storage(self: Box<Self>) -> Result<ResourceReaderBacking, Box<dyn ResourceReader>>;
}
