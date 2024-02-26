use std::borrow::Cow;

use smol::{fs::File, io::{AsyncReadExt, AsyncSeekExt}};

use crate::{GenericResourceResponse, ResourceResponse, Stream,};

pub enum ReadTargets<'a> {
	Box(Box<[u8]>),
	Buffer(&'a mut [u8]),
	Streams(&'a mut [Stream<'a>]),
}

/// The resource reader trait provides methods to read a single resource.
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

pub trait ResourceHandler: Send {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&[]
	}

	fn read<'s, 'a>(&'s self, resource: GenericResourceResponse<'a>, reader: Box<dyn ResourceReader>,) -> utils::BoxedFuture<'a, Option<ResourceResponse<'a>>>;
}