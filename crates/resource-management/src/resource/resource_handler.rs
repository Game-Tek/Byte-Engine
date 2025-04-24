use std::fmt::Debug;

use crate::{stream::StreamMut, Reference, Resource, Stream};

use super::reader::ResourceReader;

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
			ReadTargets::Streams(streams) => streams.iter().find(|s| s.name() == arg),
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
			LoadTargets::Streams(streams) => streams.iter().find(|s| s.name() == arg),
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

pub type MultiResourceReader = Box<dyn ResourceReader>;

#[cfg(test)]
pub mod tests {
    use crate::StreamDescription;

    use super::{LoadTargets, ReadTargets, ResourceReader};

	#[derive(Debug)]
	pub struct MemoryResourceReader {
		data: Box<[u8]>,
	}

	impl MemoryResourceReader {
		pub fn new(data: Box<[u8]>) -> Self {
			Self {
				data,
			}
		}
	}

	impl ResourceReader for MemoryResourceReader {
		fn read_into<'b, 'c: 'b, 'a: 'b>(&mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> Result<LoadTargets<'a>, ()> {
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
					if let Some(stream_descriptions) = stream_descriptions {
						for sd in stream_descriptions {
							let offset = sd.offset;
							if let Some(s) = streams.iter_mut().find(|s| s.name() == sd.name) {
								let len = s.buffer_mut().len();
								s.buffer_mut().copy_from_slice(&self.data[offset..][..len]);
							}
						}
						
						Ok(LoadTargets::Streams(streams.into_iter().map(|stream| {
							stream.into()
						}).collect()))
					} else {
						Err(())
					}
				}
			}
		}
	}
}