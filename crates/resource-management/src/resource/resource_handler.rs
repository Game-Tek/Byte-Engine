use std::fmt::Debug;

use utils::{File, r#async::AsyncReadExt, r#async::AsyncWriteExt, r#async::AsyncSeekExt};

use crate::{Reference, Resource, Stream, StreamDescription, StreamMut};

#[derive(Debug)]
pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a mut [u8]),
	Streams(Vec<StreamMut<'a>>),
}

impl <'a> ReadTargets<'a> {
	pub fn create_buffer<T: Resource + 'a>(reference: &Reference<T>) -> Self {
		ReadTargets::Box(unsafe { let mut v = Vec::with_capacity(reference.size); v.set_len(reference.size); v }.into_boxed_slice())
	}

	pub fn get_buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargets::Box(buffer) => Some(buffer),
			ReadTargets::Buffer(buffer) => Some(buffer),
			_ => None,
		}
	}
	
	pub fn get_stream(&self, arg: &str) -> Option<&StreamMut> {
		match self {
			ReadTargets::Streams(streams) => streams.iter().find(|s| s.name == arg),
			_ => None,
		}
	}
}

impl <'a> From<&'a mut [u8]> for ReadTargets<'a> {
	fn from(buffer: &'a mut [u8]) -> Self {
		ReadTargets::Buffer(buffer)
	}
}

impl <'a> From<Vec<StreamMut<'a>>> for ReadTargets<'a> {
	fn from(streams: Vec<StreamMut<'a>>) -> Self {
		ReadTargets::Streams(streams)
	}
}

impl <'a, T: Resource + 'a> From<Reference<T>> for ReadTargets<'a> {
	fn from(reference: Reference<T>) -> Self {
		ReadTargets::create_buffer(&reference)
	}
}

impl <'a, T: Resource + 'a> From<&Reference<T>> for ReadTargets<'a> {
	fn from(reference: &Reference<T>) -> Self {
		ReadTargets::create_buffer(&reference)
	}
}

impl <'a, T: Resource + 'a> From<&mut Reference<T>> for ReadTargets<'a> {
	fn from(reference: &mut Reference<T>) -> Self {
		ReadTargets::create_buffer(&reference)
	}
}

#[derive(Debug)]
pub enum LoadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a [u8]),
	Streams(Vec<Stream<'a>>),
}

impl <'a> LoadTargets<'a> {
	pub fn get_buffer(&self) -> Option<&[u8]> {
		match self {
			LoadTargets::Box(buffer) => Some(buffer),
			LoadTargets::Buffer(buffer) => Some(buffer),
			_ => None,
		}
	}
	
	pub fn get_stream(&self, arg: &str) -> Option<&Stream> {
		match self {
			LoadTargets::Streams(streams) => streams.iter().find(|s| s.name == arg),
			_ => None,
		}
	}
}

impl <'a> From<ReadTargets<'a>> for LoadTargets<'a> {
	fn from(read_targets: ReadTargets<'a>) -> Self {
		match read_targets {
			ReadTargets::Box(buffer) => LoadTargets::Box(buffer),
			ReadTargets::Buffer(buffer) => LoadTargets::Buffer(buffer),
			ReadTargets::Streams(streams) => LoadTargets::Streams(streams.into_iter().map(|s| s.into()).collect()),
		}
	}
}

/// The resource reader trait provides methods to read a single resource.
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'b, 'c: 'b, 'a: 'b>(self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> utils::BoxedFuture<'b, Result<LoadTargets<'a>, ()>>;
}

#[derive(Debug)]
pub struct FileResourceReader {
	#[cfg(not(test))]
	file: File,
	#[cfg(test)]
	data: Box<[u8]>,
}

impl FileResourceReader {
	#[cfg(not(test))]
	pub fn new(file: File) -> Self {
		Self {
			file,
		}
	}

	#[cfg(test)]
	pub fn new(data: Box<[u8]>) -> Self {
		Self {
			data,
		}
	}
}

#[cfg(not(test))]
impl ResourceReader for FileResourceReader {
	fn read_into<'b, 'c: 'b, 'a: 'b>(mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> utils::BoxedFuture<'b, Result<LoadTargets<'a>, ()>> {
		Box::pin(async move {
			match read_target {
				ReadTargets::Buffer(buffer) => {
					self.file.seek(std::io::SeekFrom::Start(0 as u64)).await.or(Err(()))?;
					self.file.read_exact(buffer).await.or(Err(()))?;
					Ok(LoadTargets::Buffer(buffer))
				}
				ReadTargets::Box(mut buffer) => {
					self.file.seek(std::io::SeekFrom::Start(0 as u64)).await.or(Err(()))?;
					self.file.read_exact(&mut buffer[..]).await.or(Err(()))?;
					Ok(LoadTargets::Box(buffer))
				}
				ReadTargets::Streams(mut streams) => {
					if let Some(stream_descriptions) = stream_descriptions{
						for sd in stream_descriptions {
							let offset = sd.offset;
							if let Some(s) = streams.iter_mut().find(|s| s.name == sd.name) {
								self.file.seek(std::io::SeekFrom::Start(offset as u64)).await.or(Err(()))?;
								self.file.read_exact(s.buffer).await.or(Err(()))?;
							}
						}
						Ok(LoadTargets::Streams(streams.into_iter().map(|stream| {
							Stream {
								name: stream.name,
								buffer: stream.buffer,
							}
						}).collect()))
					} else {
						Err(())
					}
				}
			}
		})
	}
}

#[cfg(test)]
impl ResourceReader for FileResourceReader {
	fn read_into<'b, 'c: 'b, 'a: 'b>(mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> utils::BoxedFuture<'b, Result<LoadTargets<'a>, ()>> {
		Box::pin(async move {
			match read_target {
				ReadTargets::Buffer(buffer) => {
					buffer.copy_from_slice(&self.data[..buffer.len()]);
					Ok(LoadTargets::Buffer(buffer))
				}
				ReadTargets::Box(mut buffer) => {
					buffer.copy_from_slice(&self.data[..buffer.len()]);
					Ok(LoadTargets::Box(buffer))
				}
				ReadTargets::Streams(mut streams) => {
					if let Some(stream_descriptions) = stream_descriptions{
						for sd in stream_descriptions {
							let offset = sd.offset;
							if let Some(s) = streams.iter_mut().find(|s| s.name == sd.name) {
								s.buffer.copy_from_slice(&self.data[offset..][..s.buffer.len()]);
							}
						}
						Ok(LoadTargets::Streams(streams.into_iter().map(|stream| {
							Stream {
								name: stream.name,
								buffer: stream.buffer,
							}
						}).collect()))
					} else {
						Err(())
					}
				}
			}
		})
	}
}