//! Select where a resource reader stores binary data and how clients access the result.

use super::Resource;
use crate::{Reference, Stream, resource::reader::ResourceReaderBacking, stream::StreamMut};

#[derive(Debug)]
/// The `ReadTargets` enum provides read-only access to resource data after a read completes.
///
/// CPU readers return decoded bytes in every buffer and backing variant.
pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a [u8]),
	/// Selected named ranges of the decoded payload.
	Streams(Vec<Stream<'a>>),
	/// Storage owned by the reader, including mapped files when the backend supports them.
	Backing(ResourceReaderBacking),
}

impl<'a> ReadTargets<'a> {
	/// Returns the resource bytes when this target contains one contiguous buffer.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargets::Box(buffer) => Some(buffer),
			ReadTargets::Buffer(buffer) => Some(buffer),
			ReadTargets::Backing(backing) => backing.try_as_slice(),
			_ => None,
		}
	}

	/// Returns a reference to a stream if the data was read into a stream.
	pub fn stream(&self, arg: &str) -> Option<&Stream<'_>> {
		match self {
			ReadTargets::Streams(streams) => streams.iter().find(|s| s.name() == arg),
			_ => None,
		}
	}
}

impl<'a> From<ReadTargetsMut<'a>> for ReadTargets<'a> {
	fn from(read_targets: ReadTargetsMut<'a>) -> Self {
		match read_targets {
			ReadTargetsMut::Box { buffer, .. } => ReadTargets::Box(buffer),
			ReadTargetsMut::Buffer { buffer, .. } => ReadTargets::Buffer(buffer),
			ReadTargetsMut::Streams(streams) => ReadTargets::Streams(streams.into_iter().map(|s| s.into()).collect()),
			ReadTargetsMut::BackingStorage => panic!(
				"Backing storage cannot be produced without a resource reader. The most likely cause is that a backing-storage request was converted directly instead of being loaded through a resource reader."
			),
		}
	}
}

#[derive(Debug)]
/// The `ReadTargetsMut` enum lets callers select where a resource reader writes binary data.
///
/// CPU-compressed resources accept an exact full-size [`Self::Buffer`] or
/// [`Self::Box`], [`Self::Streams`], or [`Self::BackingStorage`] when the reader should allocate.
pub enum ReadTargetsMut<'a> {
	Box {
		buffer: Box<[u8]>,
		/// Byte offset into the source resource data to start reading from. Defaults to `0`.
		offset: usize,
		/// Number of bytes to read from the source. Defaults to `buffer.len()` when `None`.
		size: Option<usize>,
	},
	Buffer {
		buffer: &'a mut [u8],
		/// Byte offset into the source resource data to start reading from. Defaults to `0`.
		offset: usize,
		/// Number of bytes to read from the source. Defaults to `buffer.len()` when `None`.
		size: Option<usize>,
	},
	/// Selects named ranges of the decoded payload.
	Streams(Vec<StreamMut<'a>>),
	/// Requests reader-owned storage when the caller does not provide a buffer.
	BackingStorage,
}

impl<'a> ReadTargetsMut<'a> {
	/// Requests reader-owned backing storage for resource bytes.
	pub fn backing_storage() -> Self {
		ReadTargetsMut::BackingStorage
	}

	/// Creates an owned byte buffer sized for the referenced resource.
	pub fn create_buffer<T: Resource + 'a>(reference: &Reference<T>) -> Self {
		ReadTargetsMut::Box {
			buffer: vec![0; reference.size].into_boxed_slice(),
			offset: 0,
			size: None,
		}
	}

	/// Returns the buffer for a caller-provided or resource-manager-allocated target.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargetsMut::Box { buffer, .. } => Some(buffer),
			ReadTargetsMut::Buffer { buffer, .. } => Some(buffer),
			_ => None,
		}
	}
}

impl<'a> From<&'a mut [u8]> for ReadTargetsMut<'a> {
	fn from(buffer: &'a mut [u8]) -> Self {
		ReadTargetsMut::Buffer {
			buffer,
			offset: 0,
			size: None,
		}
	}
}

impl<'a> From<Vec<StreamMut<'a>>> for ReadTargetsMut<'a> {
	fn from(streams: Vec<StreamMut<'a>>) -> Self {
		ReadTargetsMut::Streams(streams)
	}
}
