//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;

pub mod storage_backend;

pub mod resource_id;

pub mod reader;
pub mod read_target;

pub use storage_backend::redb_storage_backend::RedbStorageBackend;
pub use storage_backend::ReadStorageBackend;
pub use storage_backend::WriteStorageBackend;
pub use storage_backend::StorageBackend;

pub use resource_id::ResourceId;
pub use read_target::ReadTargets;
pub use read_target::ReadTargetsMut;

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

    use super::{reader::ResourceReader, ReadTargets, ReadTargetsMut};

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
		fn read_into<'b, 'c: 'b, 'a: 'b>(&mut self, _: Option<&'c [StreamDescription]>, read_target: ReadTargetsMut<'a>) -> Result<ReadTargets<'a>, ()> {
			let offset = 0;

			match read_target {
				ReadTargetsMut::Buffer(buffer) => {
					let l = buffer.len();
					buffer[..self.data.len().min(l)].copy_from_slice(&self.data[offset..][..self.data.len().min(l)]);
					Ok(ReadTargets::Buffer(&buffer[..self.data.len().min(l)]))
				}
				ReadTargetsMut::Box(mut buffer) => {
					let l = buffer.len();
					buffer[..self.data.len().min(l)].copy_from_slice(&self.data[offset..][..self.data.len().min(l)]);
					Ok(ReadTargets::Box(buffer))
				}
				_ => {
					Err(())
				}
			}
		}
	}
}
