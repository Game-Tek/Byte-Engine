use super::{MappedFileBacking, ResourceReader, ResourceReaderBacking};
use crate::{
	r#async::{self, BoxedFuture},
	resource::{ReadTargets, ReadTargetsMut},
	StreamDescription,
};

/// The `FileResourceReader` struct provides mapped payload bytes for deferred resource loads.
#[derive(Debug)]
pub struct FileResourceReader {
	backing: ResourceReaderBacking,
}

impl FileResourceReader {
	/// Maps an asynchronously opened payload file for direct reads into caller-provided memory.
	pub fn new(file: impl memmap2::MmapAsRawDesc, size: u64) -> Result<Self, ()> {
		let backing = if size == 0 {
			ResourceReaderBacking::Buffer(Box::new([]))
		} else {
			ResourceReaderBacking::MappedFile(MappedFileBacking::new(file)?)
		};
		Ok(Self { backing })
	}
}

impl ResourceReader for FileResourceReader {
	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>> {
		r#async::future(async move {
			let data = self.backing.as_slice();
			match read_target {
				ReadTargetsMut::Buffer { buffer, offset, size } => {
					let read_len = size
						.unwrap_or(buffer.len())
						.min(buffer.len())
						.min(data.len().saturating_sub(offset));
					buffer[..read_len].copy_from_slice(&data[offset..][..read_len]);
					Ok(ReadTargets::Buffer(&buffer[..read_len]))
				}
				ReadTargetsMut::Box {
					mut buffer,
					offset,
					size,
				} => {
					let read_len = size
						.unwrap_or(buffer.len())
						.min(buffer.len())
						.min(data.len().saturating_sub(offset));
					buffer[..read_len].copy_from_slice(&data[offset..][..read_len]);
					if read_len < buffer.len() {
						let mut v = buffer.into_vec();
						v.truncate(read_len);
						Ok(ReadTargets::Box(v.into_boxed_slice()))
					} else {
						Ok(ReadTargets::Box(buffer))
					}
				}
				ReadTargetsMut::Streams(mut streams) => {
					if let Some(stream_descriptions) = stream_descriptions {
						for sd in stream_descriptions {
							let stream_offset = sd.offset;
							if let Some(s) = streams.iter_mut().find(|s| s.name() == sd.name) {
								let offset = s.offset();
								let read_len = s
									.size()
									.unwrap_or(s.buffer().len())
									.min(s.buffer().len())
									.min(data.len().saturating_sub(stream_offset + offset));
								s.buffer_mut()[..read_len].copy_from_slice(&data[(stream_offset + offset)..][..read_len]);
							}
						}

						Ok(ReadTargets::Streams(
							streams.into_iter().map(|stream| stream.into()).collect(),
						))
					} else {
						log::error!(
							"Resource streams could not be loaded. The most likely cause is that stream descriptions are missing."
						);
						Err(())
					}
				}
				ReadTargetsMut::BackingStorage => Err(()),
			}
		})
	}

	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>> {
		r#async::future(async move { Ok(self.backing) })
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

	#[crate::r#async::test]
	async fn file_resource_reader_can_expose_mapped_backing_storage() {
		let path = temporary_file_path();
		let expected = b"shader-bytes";

		{
			let mut file = fs::File::create(&path).unwrap();
			file.write_all(expected).unwrap();
			file.sync_all().unwrap();
		}

		let reader: Box<dyn ResourceReader> =
			Box::new(FileResourceReader::new(&fs::File::open(&path).unwrap(), expected.len() as u64).unwrap());
		let backing = reader.into_backing_storage().await.unwrap();

		assert_eq!(backing.as_slice(), expected);
		fs::remove_file(path).unwrap();
	}

	#[crate::r#async::test]
	async fn file_resource_reader_loads_directly_into_a_borrowed_target() {
		let path = temporary_file_path();
		let expected = b"mapped-resource-bytes";
		fs::write(&path, expected).unwrap();

		let mut reader = FileResourceReader::new(&fs::File::open(&path).unwrap(), expected.len() as u64).unwrap();
		let mut destination = [0_u8; 8];
		let loaded = reader
			.read_into(
				None,
				ReadTargetsMut::Buffer {
					buffer: &mut destination,
					offset: 7,
					size: None,
				},
			)
			.await
			.unwrap();

		assert_eq!(loaded.buffer(), Some(&expected[7..15]));
		assert_eq!(&destination, &expected[7..15]);
		fs::remove_file(path).unwrap();
	}

	#[crate::r#async::test]
	async fn empty_file_resource_reader_returns_empty_backing_storage() {
		let path = temporary_file_path();
		fs::write(&path, []).unwrap();

		let reader: Box<dyn ResourceReader> = Box::new(FileResourceReader::new(&fs::File::open(&path).unwrap(), 0).unwrap());
		let backing = reader.into_backing_storage().await.unwrap();

		assert!(backing.as_slice().is_empty());
		fs::remove_file(path).unwrap();
	}
}
