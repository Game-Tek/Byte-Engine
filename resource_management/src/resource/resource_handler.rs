use std::fmt::Debug;

use smol::{fs::File, io::{AsyncReadExt, AsyncSeekExt}};

use crate::Stream;

#[derive(Debug)]
pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a mut [u8]),
	Streams(Vec<Stream<'a>>),
}

impl <'a> ReadTargets<'a> {
	pub fn get_buffer(&self) -> Option<&[u8]> {
		match self {
			ReadTargets::Box(buffer) => Some(buffer),
			ReadTargets::Buffer(buffer) => Some(buffer),
			_ => None,
		}
	}
}

/// The resource reader trait provides methods to read a single resource.
pub trait ResourceReader: Send + Sync + Debug {
	fn read_into<'a>(&'a mut self, offset: usize, buffer: &'a mut [u8]) -> utils::BoxedFuture<'a, Option<()>>;
}

#[derive(Debug)]
pub struct FileResourceReader {
	file: File,
}

impl FileResourceReader {
	pub fn new(file: File) -> Self {
		Self {
			file,
		}
	}
}

impl ResourceReader for FileResourceReader {
	fn read_into<'a>(&'a mut self, offset: usize, buffer: &'a mut [u8]) -> utils::BoxedFuture<'a, Option<()>> {
		Box::pin(async move {
			self.file.seek(std::io::SeekFrom::Start(offset as u64)).await.ok()?;
			self.file.read_exact(buffer).await.ok()?;
			Some(())
		})
	}
}