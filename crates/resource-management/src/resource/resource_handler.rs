use super::reader::ResourceReader;

pub type MultiResourceReader = Box<dyn ResourceReader>;

#[cfg(test)]
pub mod tests {
	use crate::{
		resource::{
			reader::{ResourceReader, ResourceReaderBacking},
			ReadTargets, ReadTargetsMut,
		},
		StreamDescription,
	};

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
		fn read_into<'b, 'c: 'b, 'a: 'b>(
			&mut self,
			stream_descriptions: Option<&'c [StreamDescription]>,
			read_target: ReadTargetsMut<'a>,
		) -> Result<ReadTargets<'a>, ()> {
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
									.min(self.data.len().saturating_sub(stream_offset + offset));
								s.buffer_mut()[..read_len].copy_from_slice(&self.data[(stream_offset + offset)..][..read_len]);
							}
						}

						Ok(ReadTargets::Streams(
							streams.into_iter().map(|stream| stream.into()).collect(),
						))
					} else {
						Err(())
					}
				}
				ReadTargetsMut::BackingStorage => Ok(ReadTargets::Backing(ResourceReaderBacking::Buffer(self.data.clone()))),
			}
		}

		fn into_backing_storage(self: Box<Self>) -> Result<ResourceReaderBacking, Box<dyn ResourceReader>> {
			Ok(ResourceReaderBacking::Buffer(self.data))
		}
	}
}
