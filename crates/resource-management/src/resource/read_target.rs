//! Read targets are involved in reading binary data for resources for a client.
//! The read targets specify whether resource bytes should be read into caller-provided memory, read into
//! an allocated box, split into streams, or served from reader-owned backing storage.
//! `ReadTargets` is used for read-only access, while `ReadTargetsMut` is used for mutable access.

use super::Resource;
use crate::{resource::reader::ResourceReaderBacking, stream::StreamMut, Reference, Stream};

#[derive(Debug)]
/// The read targets are used to specify where the binary data should be read into.
/// `ReadTargets` is used for read-only access and will be handed back to the client once the data was read.
pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a [u8]),
	Streams(Vec<Stream<'a>>),
	/// Reader-owned storage for resource bytes.
	/// File-backed resources use mapped files when the storage backend supports them.
	Backing(ResourceReaderBacking),
}

impl<'a> ReadTargets<'a> {
	/// Returns the resource bytes when the read target contains contiguous data.
	/// This includes caller-provided buffers, allocated boxes, and reader-owned backing storage.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargets::Box(buffer) => Some(buffer),
			ReadTargets::Buffer(buffer) => Some(buffer),
			ReadTargets::Backing(backing) => Some(backing.as_slice()),
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
/// The read targets are used to specify where the binary data should be read into.
/// `ReadTargetsMut` is used for mutable access and will be provided by the client when the data is to be read.
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
	Streams(Vec<StreamMut<'a>>),
	/// Requests reader-owned backing storage for resource bytes.
	/// This is the default target created from a `Reference` when the caller does not provide a buffer.
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

	/// Sets the byte offset into the source resource data to start reading from.
	/// Only applies to `Box` and `Buffer` variants; `Streams` carry their own per-stream offset.
	pub fn with_offset(self, offset: usize) -> Self {
		match self {
			ReadTargetsMut::Box { buffer, size, .. } => ReadTargetsMut::Box { buffer, offset, size },
			ReadTargetsMut::Buffer { buffer, size, .. } => ReadTargetsMut::Buffer { buffer, offset, size },
			other => other,
		}
	}

	/// Sets the number of bytes to read from the source.
	/// Only applies to `Box` and `Buffer` variants; `Streams` carry their own per-stream size.
	pub fn with_size(self, size: usize) -> Self {
		match self {
			ReadTargetsMut::Box { buffer, offset, .. } => ReadTargetsMut::Box {
				buffer,
				offset,
				size: Some(size),
			},
			ReadTargetsMut::Buffer { buffer, offset, .. } => ReadTargetsMut::Buffer {
				buffer,
				offset,
				size: Some(size),
			},
			other => other,
		}
	}

	/// Returns a reference to a buffer if the data was read into a buffer.
	/// Buffers can be a slice provided by the client or a boxed slice created by the resource manager.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargetsMut::Box { buffer, .. } => Some(buffer),
			ReadTargetsMut::Buffer { buffer, .. } => Some(buffer),
			_ => None,
		}
	}

	/// Returns a mutable reference to a buffer if the data was read into a buffer.
	pub fn stream(&self, arg: &str) -> Option<&StreamMut<'_>> {
		match self {
			ReadTargetsMut::Streams(streams) => streams.iter().find(|s| s.name() == arg),
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

impl<'a, T: Resource + 'a> From<Reference<T>> for ReadTargetsMut<'a> {
	fn from(_reference: Reference<T>) -> Self {
		ReadTargetsMut::backing_storage()
	}
}

impl<'a, T: Resource + 'a> From<&Reference<T>> for ReadTargetsMut<'a> {
	fn from(_reference: &Reference<T>) -> Self {
		ReadTargetsMut::backing_storage()
	}
}

impl<'a, T: Resource + 'a> From<&mut Reference<T>> for ReadTargetsMut<'a> {
	fn from(_reference: &mut Reference<T>) -> Self {
		ReadTargetsMut::backing_storage()
	}
}
