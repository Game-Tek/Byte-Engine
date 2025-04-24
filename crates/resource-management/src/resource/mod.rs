//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;

pub mod storage_backend;

pub mod resource_id;

pub mod reader;

pub use storage_backend::redb_storage_backend::RedbStorageBackend;
pub use storage_backend::ReadStorageBackend;
pub use storage_backend::WriteStorageBackend;
pub use storage_backend::StorageBackend;

pub use resource_id::ResourceId;

use crate::Model;

/// Trait that defines a resource.
pub trait Resource: Send + Sync {
    /// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
    /// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
    /// Is needed by the deserialize function.
    fn get_class(&self) -> &'static str;

    type Model: Model;
}

#[cfg(test)]
pub mod tests {
    use crate::StreamDescription;

    use super::{reader::ResourceReader, resource_handler::{LoadTargets, ReadTargets,}};

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
		fn read_into<'b, 'c: 'b, 'a: 'b>(&mut self, _: Option<&'c [StreamDescription]>, read_target: ReadTargets<'a>) -> Result<LoadTargets<'a>, ()> {
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
		}
	}
}
