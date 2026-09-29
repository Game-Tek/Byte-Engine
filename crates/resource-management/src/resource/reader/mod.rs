use std::{fmt::Debug, ops::Range, sync::Arc};

use memmap2::{Mmap, MmapOptions};

use super::{ReadTargets, ReadTargetsMut};
use super::{ResourcePayloadEncoding, compression};
use crate::{StreamDescription, r#async::BoxedFuture, stream::StreamMut};

#[derive(Debug)]
/// The `ResourceReaderBacking` enum provides reusable, reader-owned storage for resource bytes.
pub enum ResourceReaderBacking {
	Buffer(Box<[u8]>),
	MappedFile(MappedFileBacking),
	/// Data that must be decoded directly into a GPU resource by a native storage queue.
	Gpu(ResourceGpuBacking),
}

impl ResourceReaderBacking {
	/// Returns resource bytes when the backing is CPU-readable.
	pub fn try_as_slice(&self) -> Option<&[u8]> {
		match self {
			ResourceReaderBacking::Buffer(buffer) => Some(buffer),
			ResourceReaderBacking::MappedFile(mapped_file) => Some(mapped_file.as_slice()),
			ResourceReaderBacking::Gpu(_) => None,
		}
	}

	/// Returns resource bytes from CPU-readable backing storage.
	///
	/// # Panics
	///
	/// Panics when the backing requires native GPU resource I/O. Use
	/// [`Self::try_as_slice`] when either backing kind is valid.
	pub fn as_slice(&self) -> &[u8] {
		self.try_as_slice().expect(
			"Resource backing is not CPU-readable. The most likely cause is that compressed GPU data was passed to a CPU-only consumer.",
		)
	}
}

/// The `ResourceGpuBacking` struct provides the source file required by native GPU resource I/O.
#[derive(Debug)]
pub struct ResourceGpuBacking {
	path: std::path::PathBuf,
	encoding: ResourcePayloadEncoding,
}

impl ResourceGpuBacking {
	/// Creates a direct GPU source for one compressed resource file.
	pub fn new(path: std::path::PathBuf, encoding: ResourcePayloadEncoding) -> Self {
		assert!(
			encoding.is_gpu_backed(),
			"GPU resource backing requires a native GPU encoding. The most likely cause is that a CPU payload encoding reached the GPU reader."
		);
		Self { path, encoding }
	}

	pub fn path(&self) -> &std::path::Path {
		&self.path
	}

	pub fn encoding(&self) -> ResourcePayloadEncoding {
		self.encoding
	}
}

#[derive(Debug)]
/// The `MappedFileBacking` struct provides borrowed access to a file payload without a heap copy.
pub struct MappedFileBacking {
	map: Mmap,
	range: Range<usize>,
	// Packed backends attach a lease so a replaced slot cannot be reused while
	// this mapping still exposes its previous bytes.
	_lease: Option<Arc<()>>,
}

impl MappedFileBacking {
	/// Creates a mapped-file backing for the full file contents.
	pub fn new(file: impl memmap2::MmapAsRawDesc) -> Result<Self, ()> {
		// SAFETY: The mapping owns its OS mapping independently of the borrowed
		// descriptor and exposes it only as immutable bytes for the backing lifetime.
		let map = unsafe { MmapOptions::new().map(file) }.map_err(|_| ())?;
		let range = 0..map.len();
		Ok(Self {
			map,
			range,
			_lease: None,
		})
	}

	/// Creates a mapped-file backing for one optionally leased range of a shared payload file.
	///
	/// Only the pages covering the range are mapped, so reading one resource from a large packed file does not map
	/// the whole file.
	pub(crate) fn new_range(
		file: impl memmap2::MmapAsRawDesc,
		file_size: u64,
		offset: u64,
		size: u64,
		lease: Option<Arc<()>>,
	) -> Result<Self, ()> {
		let end = offset.checked_add(size).ok_or(())?;
		// Mapping past the end of the file would fault on access instead of failing here.
		if end > file_size {
			return Err(());
		}
		let size = usize::try_from(size).map_err(|_| ())?;
		// SAFETY: The mapping owns its OS mapping independently of the borrowed
		// descriptor and exposes it only as immutable bytes for the backing lifetime.
		// The mapped window was checked above to lie within the file. memmap2 aligns the offset to a page itself.
		let map = unsafe { MmapOptions::new().offset(offset).len(size).map(file) }.map_err(|_| ())?;
		Ok(Self {
			map,
			range: 0..size,
			_lease: lease,
		})
	}

	/// Returns the logical resource bytes from the mapped file.
	pub fn as_slice(&self) -> &[u8] {
		&self.map[self.range.clone()]
	}
}

/// The `ResourceReader` trait provides binary data for one [`Reference`](crate::Reference).
pub trait ResourceReader: Send + Sync + Debug {
	/// Returns the stored payload encoding without exposing encoded bytes to the caller.
	fn encoding(&self) -> ResourcePayloadEncoding;

	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>>;

	/// Consumes the reader and returns its owned backing when the caller can reuse it directly.
	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>>;
}

/// The `StoredResourceReader` struct is the reader storage backends hand to a [`Reference`](crate::Reference).
///
/// It keeps stored bytes private until CPU compression has been decoded, so every consumer reads decoded bytes
/// through [`Reference::load`](crate::Reference::load) whatever the payload encoding.
#[derive(Debug)]
pub struct StoredResourceReader {
	backing: ResourceReaderBacking,
	encoding: ResourcePayloadEncoding,
	decoded_size: usize,
}

impl StoredResourceReader {
	/// Creates a reader over one stored payload and its explicit delivery encoding.
	pub(crate) fn new(backing: ResourceReaderBacking, encoding: ResourcePayloadEncoding, decoded_size: usize) -> Self {
		Self {
			backing,
			encoding,
			decoded_size,
		}
	}

	/// Maps one optionally leased stored range of a payload file.
	///
	/// `stored_size` is the encoded extent in the file and `decoded_size` the size clients receive. An empty range
	/// maps nothing.
	pub(crate) fn mapped(
		file: impl memmap2::MmapAsRawDesc,
		file_size: u64,
		offset: u64,
		stored_size: u64,
		decoded_size: usize,
		encoding: ResourcePayloadEncoding,
		lease: Option<Arc<()>>,
	) -> Result<Self, ()> {
		let backing = if stored_size == 0 {
			ResourceReaderBacking::Buffer(Box::new([]))
		} else {
			ResourceReaderBacking::MappedFile(MappedFileBacking::new_range(file, file_size, offset, stored_size, lease)?)
		};
		Ok(Self::new(backing, encoding, decoded_size))
	}

	/// Creates a reader that transfers ownership of a native GPU I/O source instead of mapping CPU bytes.
	pub fn gpu(path: std::path::PathBuf, encoding: ResourcePayloadEncoding) -> Self {
		Self::new(
			ResourceReaderBacking::Gpu(ResourceGpuBacking::new(path, encoding)),
			encoding,
			0,
		)
	}

	/// Decodes a compressed payload into exact caller-owned post-decompression storage or into named streams.
	fn read_compressed<'a>(
		&self,
		stream_descriptions: Option<&[StreamDescription]>,
		mut read_target: ReadTargetsMut<'a>,
	) -> Result<ReadTargets<'a>, ()> {
		let compressed = self.backing.try_as_slice().ok_or(())?;

		// Stream ranges address decoded bytes, so the payload is decoded once and every stream is copied from it.
		if let ReadTargetsMut::Streams(streams) = read_target {
			let decoded = decode_owned(compressed, self.decoded_size)?;
			return copy_streams(&decoded, stream_descriptions, streams);
		}

		let (buffer, offset, size) = match &mut read_target {
			ReadTargetsMut::Buffer { buffer, offset, size } => (&mut **buffer, *offset, *size),
			ReadTargetsMut::Box { buffer, offset, size } => (&mut **buffer, *offset, *size),
			ReadTargetsMut::BackingStorage => {
				return Ok(ReadTargets::Backing(ResourceReaderBacking::Buffer(decode_owned(
					compressed,
					self.decoded_size,
				)?)));
			}
			ReadTargetsMut::Streams(_) => unreachable!("stream targets were decoded above"),
		};
		validate_full_decode_target(buffer.len(), offset, size, self.decoded_size)?;
		decode_resource(compressed, buffer)?;
		Ok(read_target.into())
	}

	/// Copies uncompressed stored bytes into the caller-selected range or streams.
	fn read_uncompressed<'a>(
		&self,
		stream_descriptions: Option<&[StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> Result<ReadTargets<'a>, ()> {
		let data = self.backing.try_as_slice().ok_or(())?;
		match read_target {
			ReadTargetsMut::Buffer { buffer, offset, size } => {
				let read_len = copy_resource_range(data, buffer, offset, size);
				Ok(ReadTargets::Buffer(&buffer[..read_len]))
			}
			ReadTargetsMut::Box {
				mut buffer,
				offset,
				size,
			} => {
				let read_len = copy_resource_range(data, &mut buffer, offset, size);
				if read_len < buffer.len() {
					let mut data = buffer.into_vec();
					data.truncate(read_len);
					Ok(ReadTargets::Box(data.into_boxed_slice()))
				} else {
					Ok(ReadTargets::Box(buffer))
				}
			}
			ReadTargetsMut::Streams(streams) => copy_streams(data, stream_descriptions, streams),
			ReadTargetsMut::BackingStorage => Err(()),
		}
	}
}

/// Copies each requested named range of decoded payload bytes into its stream.
fn copy_streams<'a>(
	data: &[u8],
	stream_descriptions: Option<&[StreamDescription]>,
	mut streams: Vec<StreamMut<'a>>,
) -> Result<ReadTargets<'a>, ()> {
	let Some(stream_descriptions) = stream_descriptions else {
		log::error!("Resource streams could not be loaded. The most likely cause is that stream descriptions are missing.");
		return Err(());
	};

	for stream in &mut streams {
		let description = find_stream_description(stream_descriptions, stream.name())?;
		copy_stream(data, description, stream)?;
	}

	Ok(ReadTargets::Streams(streams.into_iter().map(Into::into).collect()))
}

/// Finds one requested stream and reports mismatched resource metadata at the read boundary.
fn find_stream_description<'a>(stream_descriptions: &'a [StreamDescription], name: &str) -> Result<&'a StreamDescription, ()> {
	stream_descriptions
		.iter()
		.find(|description| description.name() == name)
		.ok_or_else(|| {
			log::error!(
				"Resource stream '{}' is missing. The most likely cause is that the requested stream does not exist in the stored resource metadata.",
				name
			);
		})
}

/// Copies one contiguous range without panicking when the requested offset is past the payload.
fn copy_resource_range(data: &[u8], destination: &mut [u8], offset: usize, requested_size: Option<usize>) -> usize {
	let source = data.get(offset..).unwrap_or_default();
	let read_len = requested_size
		.unwrap_or(destination.len())
		.min(destination.len())
		.min(source.len());
	destination[..read_len].copy_from_slice(&source[..read_len]);
	read_len
}

/// Copies one requested named range while enforcing its persisted bounds.
fn copy_stream(data: &[u8], description: &StreamDescription, stream: &mut StreamMut<'_>) -> Result<(), ()> {
	let description_end = description.offset.checked_add(description.size).ok_or_else(|| {
		log::error!("Resource stream range overflowed. The most likely cause is corrupt stored stream metadata.");
	})?;
	let described = data.get(description.offset..description_end).ok_or_else(|| {
		log::error!("Resource stream is outside its payload. The most likely cause is corrupt stored stream metadata.");
	})?;
	copy_resource_range(described, stream.buffer_mut(), 0, None);
	Ok(())
}

impl ResourceReader for StoredResourceReader {
	fn encoding(&self) -> ResourcePayloadEncoding {
		self.encoding
	}

	fn read_into<'b, 'c: 'b, 'a: 'b>(
		&'b mut self,
		stream_descriptions: Option<&'c [StreamDescription]>,
		read_target: ReadTargetsMut<'a>,
	) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>> {
		crate::r#async::future(async move {
			match self.encoding {
				ResourcePayloadEncoding::Raw => self.read_uncompressed(stream_descriptions, read_target),
				ResourcePayloadEncoding::CpuLz4 => self.read_compressed(stream_descriptions, read_target),
				ResourcePayloadEncoding::MetalIoLz4 => {
					log::error!(
						"GPU-encoded resource data cannot be read through a CPU target. The most likely cause is that a native GPU payload reached a CPU-only consumer."
					);
					Err(())
				}
			}
		})
	}

	fn into_backing_storage(self: Box<Self>) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>> {
		crate::r#async::future(async move {
			match self.encoding {
				ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::MetalIoLz4 => Ok(self.backing),
				ResourcePayloadEncoding::CpuLz4 => {
					let decoded = match self.backing.try_as_slice() {
						Some(compressed) => decode_owned(compressed, self.decoded_size),
						None => return Err(self as Box<dyn ResourceReader>),
					};

					match decoded {
						Ok(decoded) => Ok(ResourceReaderBacking::Buffer(decoded)),
						Err(()) => Err(self as Box<dyn ResourceReader>),
					}
				}
			}
		})
	}
}

/// Requires one exact full-resource destination before decoding can begin.
fn validate_full_decode_target(
	buffer_size: usize,
	offset: usize,
	requested_size: Option<usize>,
	decoded_size: usize,
) -> Result<(), ()> {
	if buffer_size == decoded_size && offset == 0 && requested_size.is_none_or(|size| size == decoded_size) {
		return Ok(());
	}

	log::error!(
		"Compressed resource destination has the wrong size. The most likely cause is that the caller requested a partial range or did not allocate the resource's complete post-decompression size of {decoded_size} bytes."
	);
	Err(())
}

fn decode_resource(compressed: &[u8], output: &mut [u8]) -> Result<(), ()> {
	compression::decompress_into(compressed, output).map_err(|_| {
		log::error!(
			"Compressed resource could not be decoded. The most likely cause is corrupt payload data or mismatched stored compression metadata."
		);
	})
}

fn decode_owned(compressed: &[u8], decoded_size: usize) -> Result<Box<[u8]>, ()> {
	let mut decoded = Vec::new();
	decoded.try_reserve_exact(decoded_size).map_err(|_| {
		log::error!(
			"Compressed resource buffer could not be allocated. The most likely cause is insufficient memory for the post-decompression payload."
		);
	})?;
	decoded.resize(decoded_size, 0);
	decode_resource(compressed, &mut decoded)?;
	Ok(decoded.into_boxed_slice())
}

#[cfg(test)]
mod tests {
	use std::{fs, path::PathBuf};

	use super::*;

	/// Reserves a fresh file under the system temp directory.
	///
	/// `create_new` fails when the path already exists, so retrying with a new
	/// timestamp suffix gives each test its own file without shared counters.
	fn temporary_file_path() -> PathBuf {
		loop {
			let nanos = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos();
			let path = std::env::temp_dir().join(format!(
				"byte-engine-stored-resource-reader-{}-{nanos}.bin",
				std::process::id()
			));
			match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
				Ok(_) => return path,
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(error) => {
					panic!("Failed to create a test file: {error}. The most likely cause is an unwritable temp directory.")
				}
			}
		}
	}

	/// Maps `size` raw bytes starting at `offset` of a `file_size`-byte file.
	fn mapped_raw(file: &fs::File, file_size: u64, offset: u64, size: u64) -> Result<StoredResourceReader, ()> {
		StoredResourceReader::mapped(
			file,
			file_size,
			offset,
			size,
			size as usize,
			ResourcePayloadEncoding::Raw,
			None,
		)
	}

	#[crate::r#async::test]
	async fn mapped_reader_loads_directly_into_a_borrowed_target() {
		let path = temporary_file_path();
		let expected = b"mapped-resource-bytes";
		fs::write(&path, expected).unwrap();

		let length = expected.len() as u64;
		let mut reader = mapped_raw(&fs::File::open(&path).unwrap(), length, 0, length).unwrap();
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
	async fn ranged_reader_maps_an_unaligned_range_deep_in_a_large_file() {
		let path = temporary_file_path();
		let contents = (0..300_000_u32).map(|index| (index % 251) as u8).collect::<Vec<_>>();
		fs::write(&path, &contents).unwrap();
		let file = fs::File::open(&path).unwrap();

		// The offset sits past several mapping-alignment boundaries and between them.
		let reader: Box<dyn ResourceReader> = Box::new(mapped_raw(&file, contents.len() as u64, 200_003, 1_000).unwrap());
		let backing = reader.into_backing_storage().await.unwrap();

		assert_eq!(backing.as_slice(), &contents[200_003..201_003]);
		assert!(mapped_raw(&file, contents.len() as u64, 299_500, 1_000).is_err());
		fs::remove_file(path).unwrap();
	}

	#[crate::r#async::test]
	async fn empty_mapped_reader_returns_empty_backing_storage() {
		let path = temporary_file_path();
		fs::write(&path, []).unwrap();

		let reader: Box<dyn ResourceReader> = Box::new(mapped_raw(&fs::File::open(&path).unwrap(), 0, 0, 0).unwrap());
		let backing = reader.into_backing_storage().await.unwrap();

		assert!(backing.as_slice().is_empty());
		fs::remove_file(path).unwrap();
	}

	#[crate::r#async::test]
	async fn compressed_stream_reads_return_the_decoded_named_ranges() {
		// Runs of 64 equal bytes compress well, so the payload is stored as one LZ4 block.
		let decoded = (0..4096_usize).map(|index| (index / 64) as u8).collect::<Vec<_>>();
		let compressed = compression::prepare(&decoded, crate::resource::ResourceCompressionPolicy::Enabled)
			.expect("repeated bytes should pass the compression heuristic");
		let mut reader = StoredResourceReader::new(
			ResourceReaderBacking::Buffer(compressed.bytes.into_boxed_slice()),
			ResourcePayloadEncoding::CpuLz4,
			decoded.len(),
		);
		let descriptions = [StreamDescription::new("head", 8, 60), StreamDescription::new("tail", 4, 4092)];
		let mut head = [0_u8; 8];
		let mut tail = [0_u8; 4];
		let streams = vec![StreamMut::new("head", &mut head), StreamMut::new("tail", &mut tail)];

		let loaded = reader.read_into(Some(&descriptions), streams.into()).await.unwrap();

		assert_eq!(
			loaded.stream("head").map(crate::Stream::buffer),
			Some([0, 0, 0, 0, 1, 1, 1, 1].as_slice())
		);
		assert_eq!(loaded.stream("tail").map(crate::Stream::buffer), Some([63; 4].as_slice()));
	}

	#[crate::r#async::test]
	async fn uncompressed_ranges_past_the_payload_return_empty_without_touching_the_target() {
		let mut reader = StoredResourceReader::new(
			ResourceReaderBacking::Buffer(vec![1, 2, 3].into_boxed_slice()),
			ResourcePayloadEncoding::Raw,
			3,
		);
		let mut destination = [9_u8; 2];
		{
			let loaded = reader
				.read_into(
					None,
					ReadTargetsMut::Buffer {
						buffer: &mut destination,
						offset: 10,
						size: None,
					},
				)
				.await
				.unwrap();

			assert_eq!(loaded.buffer(), Some([].as_slice()));
		}
		assert_eq!(destination, [9, 9]);
	}

	#[crate::r#async::test]
	async fn uncompressed_stream_reads_require_a_named_description_and_stay_inside_it() {
		let mut reader = StoredResourceReader::new(
			ResourceReaderBacking::Buffer(vec![10, 11, 12, 13].into_boxed_slice()),
			ResourcePayloadEncoding::Raw,
			4,
		);
		let descriptions = [StreamDescription::new("known", 2, 1)];
		let mut missing_destination = [0_u8; 2];
		let missing = vec![StreamMut::new("missing", &mut missing_destination)];
		assert!(reader.read_into(Some(&descriptions), missing.into()).await.is_err());

		let mut destination = [0_u8; 4];
		let streams = vec![StreamMut::new("known", &mut destination)];
		let loaded = reader.read_into(Some(&descriptions), streams.into()).await.unwrap();

		assert_eq!(
			loaded.stream("known").map(crate::Stream::buffer),
			Some([11, 12, 0, 0].as_slice())
		);
	}
}
