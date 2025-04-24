use std::io::{Read, Seek};

use utils::sync::File;

use crate::{resource::resource_handler::{LoadTargets, ReadTargets}, StreamDescription};

use super::ResourceReader;

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
	fn read_into<'b, 'c: 'b, 'a: 'b>(&'b mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> Result<LoadTargets<'a>, ()> {
		match read_target {
			ReadTargets::Buffer(buffer) => {
				self.file.seek(std::io::SeekFrom::Start(0 as u64)).or(Err(()))?;
				self.file.read_exact(buffer).or(Err(()))?;
				Ok(LoadTargets::Buffer(buffer))
			}
			ReadTargets::Box(mut buffer) => {
				self.file.seek(std::io::SeekFrom::Start(0 as u64)).or(Err(()))?;
				self.file.read_exact(&mut buffer[..]).or(Err(()))?;
				Ok(LoadTargets::Box(buffer))
			}
			ReadTargets::Streams(mut streams) => {
				if let Some(stream_descriptions) = stream_descriptions{
					for sd in stream_descriptions {
						let offset = sd.offset;
						if let Some(s) = streams.iter_mut().find(|s| s.name() == sd.name) {
							self.file.seek(std::io::SeekFrom::Start(offset as u64)).or(Err(()))?;
							self.file.read_exact(s.buffer_mut()).or(Err(()))?;
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