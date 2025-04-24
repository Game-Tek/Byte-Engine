//! Read targets are involved in reading binary data for resources for a client.
//! The read targets are used to specify where the binary data should be read into.
//! /// `ReadTargets` is used for read-only access, while `ReadTargetsMut` is used for mutable access.

use crate::{stream::StreamMut, Reference, Stream};

use super::Resource;

#[derive(Debug)]
/// The read targets are used to specify where the binary data should be read into.
/// `ReadTargets` is used for read-only access and will be handed back to the client once the data was read.
pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a [u8]),
	Streams(Vec<Stream<'a>>),
}

impl <'a> ReadTargets<'a> {
	/// Returns a reference to a buffer if the data was read into a buffer.
	/// Buffers can be a slice provided by the client or a boxed slice created by the resource manager.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargets::Box(buffer) => Some(buffer),
			ReadTargets::Buffer(buffer) => Some(buffer),
			_ => None,
		}
	}

	/// Returns a reference to a stream if the data was read into a stream.
	pub fn stream(&self, arg: &str) -> Option<&Stream> {
		match self {
			ReadTargets::Streams(streams) => streams.iter().find(|s| s.name() == arg),
			_ => None,
		}
	}
}

impl <'a> From<ReadTargetsMut<'a>> for ReadTargets<'a> {
	fn from(read_targets: ReadTargetsMut<'a>) -> Self {
		match read_targets {
			ReadTargetsMut::Box(buffer) => ReadTargets::Box(buffer),
			ReadTargetsMut::Buffer(buffer) => ReadTargets::Buffer(buffer),
			ReadTargetsMut::Streams(streams) => ReadTargets::Streams(streams.into_iter().map(|s| s.into()).collect()),
		}
	}
}

#[derive(Debug)]
/// The read targets are used to specify where the binary data should be read into.
/// `ReadTargetsMut` is used for mutable access and will be provided by the client when the data is to be read.
pub enum ReadTargetsMut<'a> {
	Box(Box<[u8]>),
	Buffer(&'a mut [u8]),
	Streams(Vec<StreamMut<'a>>),
}

impl <'a> ReadTargetsMut<'a> {
	pub fn create_buffer<T: Resource + 'a>(reference: &Reference<T>) -> Self {
		ReadTargetsMut::Box(unsafe { let mut v = Vec::with_capacity(reference.size); v.set_len(reference.size); v }.into_boxed_slice())
	}

	/// Returns a reference to a buffer if the data was read into a buffer.
	/// Buffers can be a slice provided by the client or a boxed slice created by the resource manager.
	pub fn buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargetsMut::Box(buffer) => Some(buffer),
			ReadTargetsMut::Buffer(buffer) => Some(buffer),
			_ => None,
		}
	}

	/// Returns a mutable reference to a buffer if the data was read into a buffer.
	pub fn stream(&self, arg: &str) -> Option<&StreamMut> {
		match self {
			ReadTargetsMut::Streams(streams) => streams.iter().find(|s| s.name() == arg),
			_ => None,
		}
	}
}

impl <'a> From<&'a mut [u8]> for ReadTargetsMut<'a> {
	fn from(buffer: &'a mut [u8]) -> Self {
		ReadTargetsMut::Buffer(buffer)
	}
}

impl <'a> From<Vec<StreamMut<'a>>> for ReadTargetsMut<'a> {
	fn from(streams: Vec<StreamMut<'a>>) -> Self {
		ReadTargetsMut::Streams(streams)
	}
}

impl <'a, T: Resource + 'a> From<Reference<T>> for ReadTargetsMut<'a> {
	fn from(reference: Reference<T>) -> Self {
		ReadTargetsMut::create_buffer(&reference)
	}
}

impl <'a, T: Resource + 'a> From<&Reference<T>> for ReadTargetsMut<'a> {
	fn from(reference: &Reference<T>) -> Self {
		ReadTargetsMut::create_buffer(&reference)
	}
}

impl <'a, T: Resource + 'a> From<&mut Reference<T>> for ReadTargetsMut<'a> {
	fn from(reference: &mut Reference<T>) -> Self {
		ReadTargetsMut::create_buffer(&reference)
	}
}