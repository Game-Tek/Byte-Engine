use super::{MappedFileBacking, ResourceReader, ResourceReaderBacking, StoredResourceReader};
use crate::{
	StreamDescription,
	r#async::{self, BoxedFuture},
	resource::{ReadTargets, ReadTargetsMut},
};

/// The `FileResourceReader` struct provides mapped payload bytes for deferred resource loads.
#[derive(Debug)]
pub struct FileResourceReader {
	reader: StoredResourceReader,
}

impl FileResourceReader {
	/// Maps a complete payload file for direct reads into caller-provided memory.
	pub fn new(file: impl memmap2::MmapAsRawDesc, size: u64) -> Result<Self, ()> {
		Self::new_range(file, size, 0, size, None)
	}

	/// Maps one optionally leased range from a shared payload file.
	pub(crate) fn new_range(
		file: impl memmap2::MmapAsRawDesc,
		file_size: u64,
		offset: u64,
		size: u64,
		lease: Option<std::sync::Arc<()>>,
	) -> Result<Self, ()> {
		let decoded_size = usize::try_from(size).map_err(|_| ())?;
		Self::new_stored_range(
			file,
			file_size,
			offset,
			size,
			decoded_size,
			crate::resource::ResourcePayloadEncoding::Raw,
			lease,
		)
	}

	/// Maps one stored range while keeping its CPU encoding private from clients.
	pub(crate) fn new_stored_range(
		file: impl memmap2::MmapAsRawDesc,
		file_size: u64,
		offset: u64,
		stored_size: u64,
		decoded_size: usize,
		encoding: crate::resource::ResourcePayloadEncoding,
		lease: Option<std::sync::Arc<()>>,
	) -> Result<Self, ()> {
		let end = offset.checked_add(stored_size).ok_or(())?;
		if end > file_size {
			return Err(());
		}
		let backing = if stored_size == 0 {
			ResourceReaderBacking::Buffer(Box::new([]))
		} else {
			ResourceReaderBacking::MappedFile(MappedFileBacking::new_range(file, offset, stored_size, lease)?)
		};
		Ok(Self {
			reader: StoredResourceReader::new(backing, encoding, decoded_size),
		})
	}

	/// Creates a reader that transfers ownership of a native GPU I/O source instead of mapping CPU bytes.
	pub fn new_gpu(path: std::path::PathBuf, encoding: crate::resource::ResourcePayloadEncoding) -> Self {
		Self {
			reader: StoredResourceReader::new(
				ResourceReaderBacking::Gpu(super::ResourceGpuBacking::new(path, encoding)),
				encoding,
				0,
			),
		}
	}
}

impl ResourceReader for FileResourceReader {
	fn encoding(&self) -> crate::resource::ResourcePayloadEncoding {
		self.reader.encoding()
	}

	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>> {
		self.reader.read_into(stream_descriptions, read_target)
	}

	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>> {
		Box::new(self.reader).into_backing_storage()
	}
}

#[cfg(test)]
mod tests {
	use std::{
		fs,
		io::Write,
		path::PathBuf,
		sync::atomic::{AtomicUsize, Ordering},
	};

	use super::*;

	fn temporary_file_path() -> PathBuf {
		static NEXT_FILE_ID: AtomicUsize = AtomicUsize::new(0);
		std::env::temp_dir().join(format!(
			"byte-engine-file-resource-reader-{}-{}.bin",
			std::process::id(),
			NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed)
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
	async fn ranged_file_resource_reader_exposes_only_the_requested_resource() {
		let path = temporary_file_path();
		fs::write(&path, b"firstsecondthird").unwrap();

		let reader: Box<dyn ResourceReader> =
			Box::new(FileResourceReader::new_range(&fs::File::open(&path).unwrap(), 16, 5, 6, None).unwrap());
		let backing = reader.into_backing_storage().await.unwrap();

		assert_eq!(backing.as_slice(), b"second");
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
