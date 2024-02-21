use smol::{fs::File, io::{AsyncReadExt, AsyncSeekExt}};

use crate::{GenericResourceSerialization, ResourceResponse, Stream};

pub enum ReadTargets<'a> {
	Buffer(&'a mut [u8]),
	Streams(&'a mut [Stream<'a>]),
}

pub trait ResourceReader {
	fn read_into<'a>(&'a mut self, offset: usize, buffer: &'a mut [u8]) -> utils::BoxedFuture<'a, Option<()>>;
}

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

pub trait ResourceHandler {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&[]
	}

	fn read<'a>(&'a self, resource: &'a GenericResourceSerialization, reader: &'a mut dyn ResourceReader, read_target: &'a mut ReadTargets<'a>) -> utils::BoxedFuture<Option<ResourceResponse>>;
}