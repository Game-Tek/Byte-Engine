//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;
pub mod image_resource_handler;
pub mod mesh_resource_handler;
pub mod material_resource_handler;
pub mod audio_resource_handler;

#[cfg(test)]
pub mod tests {
    use crate::StreamDescription;

    use super::resource_handler::{LoadTargets, ReadTargets, ResourceReader};

	#[derive(Debug)]
	pub struct TestResourceReader {
		data: Box<[u8]>,
	}

	impl TestResourceReader {
		pub fn new(data: Box<[u8]>) -> Self {
			Self {
				data,
			}
		}
	}

	impl ResourceReader for TestResourceReader {
		fn read_into<'b, 'c: 'b, 'a: 'b>(mut self, streams: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> utils::BoxedFuture<'a, Result<LoadTargets<'a>, ()>> {
			Box::pin(async move {
				let offset = 0;

				match read_target {
					ReadTargets::Buffer(buffer) => {
						let l = buffer.len();
						buffer[..self.data.len().min(l)].copy_from_slice(&self.data[offset..][..self.data.len().min(l)]);
						Ok(LoadTargets::Buffer(&buffer[..self.data.len().min(l)]))
					}
					ReadTargets::Box(mut buffer) => {
						let l = buffer.len();
						buffer[..self.data.len().min(l)].copy_from_slice(&self.data[offset..][..self.data.len().min(l)]);
						Ok(LoadTargets::Box(buffer))
					}
					_ => {
						Err(())
					}
				}
			})
		}
	}
}