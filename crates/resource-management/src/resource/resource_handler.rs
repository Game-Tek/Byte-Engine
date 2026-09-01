use super::reader::ResourceReader;

pub type MultiResourceReader = Box<dyn ResourceReader>;

#[cfg(test)]
pub mod tests {
	use crate::{
		StreamDescription,
		r#async::{self, BoxedFuture},
		resource::{
			ReadTargets, ReadTargetsMut,
			reader::{ResourceReader, ResourceReaderBacking},
		},
		stream::StreamMut,
	};

	/// Copies every requested named stream from its described source range.
	fn read_streams<'a>(
		data: &[u8],
		stream_descriptions: Option<&[StreamDescription]>,
		mut streams: Vec<StreamMut<'a>>,
	) -> Result<ReadTargets<'a>, ()> {
		let stream_descriptions = stream_descriptions.ok_or(())?;
		for description in stream_descriptions {
			let Some(stream) = streams.iter_mut().find(|stream| stream.name() == description.name) else {
				continue;
			};
			let offset = stream.offset();
			let read_len = stream
				.size()
				.unwrap_or(stream.buffer().len())
				.min(stream.buffer().len())
				.min(data.len().saturating_sub(description.offset + offset));
			stream.buffer_mut()[..read_len].copy_from_slice(&data[(description.offset + offset)..][..read_len]);
		}

		Ok(ReadTargets::Streams(
			streams.into_iter().map(|stream| stream.into()).collect(),
		))
	}

	#[derive(Debug)]
	pub struct MemoryResourceReader {
		data: Box<[u8]>,
	}

	impl MemoryResourceReader {
		pub fn new(data: Box<[u8]>) -> Self {
			Self { data }
		}
	}

	impl ResourceReader for MemoryResourceReader {
		fn encoding(&self) -> crate::resource::ResourcePayloadEncoding {
			crate::resource::ResourcePayloadEncoding::Raw
		}

		fn read_into<'b, 'c: 'b, 'a: 'b>(
			&'b mut self,
			stream_descriptions: Option<&'c [StreamDescription]>,
			read_target: ReadTargetsMut<'a>,
		) -> BoxedFuture<'b, Result<ReadTargets<'a>, ()>> {
			r#async::future(async move {
				match read_target {
					ReadTargetsMut::Buffer { buffer, offset, size } => {
						let read_len = size
							.unwrap_or(buffer.len())
							.min(buffer.len())
							.min(self.data.len().saturating_sub(offset));
						buffer[..read_len].copy_from_slice(&self.data[offset..][..read_len]);
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
							.min(self.data.len().saturating_sub(offset));
						buffer[..read_len].copy_from_slice(&self.data[offset..][..read_len]);
						if read_len < buffer.len() {
							let mut v = buffer.into_vec();
							v.truncate(read_len);
							Ok(ReadTargets::Box(v.into_boxed_slice()))
						} else {
							Ok(ReadTargets::Box(buffer))
						}
					}
					ReadTargetsMut::Streams(streams) => read_streams(&self.data, stream_descriptions, streams),
					ReadTargetsMut::BackingStorage => {
						Ok(ReadTargets::Backing(ResourceReaderBacking::Buffer(self.data.clone())))
					}
				}
			})
		}

		fn into_backing_storage(
			self: Box<Self>,
		) -> BoxedFuture<'static, Result<ResourceReaderBacking, Box<dyn ResourceReader>>> {
			r#async::future(async move { Ok(ResourceReaderBacking::Buffer(self.data)) })
		}
	}
}
