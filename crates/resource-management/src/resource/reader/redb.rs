use std::io::{Read, Seek};

use utils::sync::File;

use crate::{
	StreamDescription,
	resource::{ReadTargets, ReadTargetsMut},
};

use super::{MappedFileBacking, ResourceReader, ResourceReaderBacking};

#[derive(Debug)]
pub struct FileResourceReader {
	file: File,
}

impl FileResourceReader {
	pub fn new(file: File) -> Self {
		Self { file }
	}
}

impl ResourceReader for FileResourceReader {
	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> Result<ReadTargets<'a>, ()> {
		match read_target {
			ReadTargetsMut::Buffer(buffer) => {
				self.file.seek(std::io::SeekFrom::Start(0 as u64)).or(Err(()))?;
				self.file.read_exact(buffer).or(Err(()))?;
				Ok(ReadTargets::Buffer(buffer))
			}
			ReadTargetsMut::Box(mut buffer) => {
				self.file.seek(std::io::SeekFrom::Start(0 as u64)).or(Err(()))?;
				self.file.read_exact(&mut buffer[..]).or(Err(()))?;
				Ok(ReadTargets::Box(buffer))
			}
			ReadTargetsMut::Streams(mut streams) => {
				if let Some(stream_descriptions) = stream_descriptions {
					for sd in stream_descriptions {
						let offset = sd.offset;
						if let Some(s) = streams.iter_mut().find(|s| s.name() == sd.name) {
							self.file.seek(std::io::SeekFrom::Start(offset as u64)).or(Err(()))?;
							self.file
								.read_exact(s.buffer_mut())
								.inspect_err(|e| {
									log::error!(
										"Failed to read stream '{}' from file resource. Expected to read: {}. Error: {}",
										s.name(),
										s.buffer().len(),
										e,
									)
								})
								.or(Err(()))?;
						}
					}

					Ok(ReadTargets::Streams(
						streams.into_iter().map(|stream| stream.into()).collect(),
					))
				} else {
					log::error!("Stream descriptions are required for reading into streams");
					Err(())
				}
			}
		}
	}

	fn into_backing_storage(self: Box<Self>) -> Result<ResourceReaderBacking, Box<dyn ResourceReader>> {
		let mapped_file = MappedFileBacking::new(&self.file).map_err(|_| self as Box<dyn ResourceReader>)?;
		Ok(ResourceReaderBacking::MappedFile(mapped_file))
	}
}

#[cfg(test)]
mod tests {
	use std::{
		fs,
		io::Write,
		path::PathBuf,
		time::{SystemTime, UNIX_EPOCH},
	};

	use super::*;

	fn temporary_file_path() -> PathBuf {
		std::env::temp_dir().join(format!(
			"byte-engine-file-resource-reader-{}-{}.bin",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		))
	}

	#[test]
	fn file_resource_reader_can_expose_mapped_backing_storage() {
		let path = temporary_file_path();
		let expected = b"shader-bytes";

		{
			let mut file = fs::File::create(&path).unwrap();
			file.write_all(expected).unwrap();
			file.sync_all().unwrap();
		}

		let reader: Box<dyn ResourceReader> = Box::new(FileResourceReader::new(fs::File::open(&path).unwrap()));
		let backing = reader.into_backing_storage().unwrap();

		assert_eq!(backing.as_slice(), expected);
		fs::remove_file(path).unwrap();
	}
}
