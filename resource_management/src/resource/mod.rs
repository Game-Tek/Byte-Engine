//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;
pub mod image_resource_handler;
pub mod mesh_resource_handler;
pub mod material_resource_handler;
pub mod audio_resource_handler;

#[cfg(test)]
pub mod tests {
    use super::resource_handler::ResourceReader;

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
		fn read_into<'a>(&'a mut self, offset: usize, buffer: &'a mut [u8]) -> utils::BoxedFuture<'a, Option<()>> {
			Box::pin(async move {
				let l = buffer.len();
				buffer[..self.data.len().min(l)].copy_from_slice(&self.data[offset..][..self.data.len().min(l)]);
				Some(())
			})
		}
	}
}