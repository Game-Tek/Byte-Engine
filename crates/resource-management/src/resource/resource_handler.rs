use super::reader::ResourceReader;

pub type MultiResourceReader = Box<dyn ResourceReader>;

#[cfg(test)]
pub mod tests {
    use crate::{resource::{reader::ResourceReader, ReadTargets, ReadTargetsMut}, StreamDescription};

	#[derive(Debug)]
	pub struct MemoryResourceReader {
		data: Box<[u8]>,
	}

	impl MemoryResourceReader {
		pub fn new(data: Box<[u8]>) -> Self {
			Self {
				data,
			}
		}
	}

	impl ResourceReader for MemoryResourceReader {
		fn read_into<'b, 'c: 'b, 'a: 'b>(&mut self, stream_descriptions: Option<&'c [StreamDescription]>, read_target: ReadTargetsMut<'a>) -> Result<ReadTargets<'a>, ()> {
			match read_target {
				ReadTargetsMut::Buffer(buffer) => {
					buffer.copy_from_slice(&self.data[..buffer.len()]);
					Ok(ReadTargets::Buffer(buffer))
				}
				ReadTargetsMut::Box(mut buffer) => {
					buffer.copy_from_slice(&self.data[..buffer.len()]);
					Ok(ReadTargets::Box(buffer))
				}
				ReadTargetsMut::Streams(mut streams) => {
					if let Some(stream_descriptions) = stream_descriptions {
						for sd in stream_descriptions {
							let offset = sd.offset;
							if let Some(s) = streams.iter_mut().find(|s| s.name() == sd.name) {
								let len = s.buffer_mut().len();
								s.buffer_mut().copy_from_slice(&self.data[offset..][..len]);
							}
						}
						
						Ok(ReadTargets::Streams(streams.into_iter().map(|stream| {
							stream.into()
						}).collect()))
					} else {
						Err(())
					}
				}
			}
		}
	}
}