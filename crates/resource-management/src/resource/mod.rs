//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;

pub mod storage_backend;

pub mod resource_id;

pub mod read_target;
pub mod reader;

pub use read_target::ReadTargets;
pub use read_target::ReadTargetsMut;
pub use resource_id::ResourceId;
pub use storage_backend::redb_storage_backend::RedbStorageBackend;
pub use storage_backend::ReadStorageBackend;
pub use storage_backend::StorageBackend;
pub use storage_backend::WriteStorageBackend;

use crate::Model;

/// Trait that defines a resource.
pub trait Resource: Send + Sync {
	/// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
	/// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// Is needed by the deserialize function.
	fn get_class(&self) -> &'static str {
		<Self::Model as Model>::get_class()
	}

	fn queryable_properties(&self, id: &str) -> Vec<crate::QueryableProperty> {
		vec![crate::QueryableProperty {
			name: "name".to_string(),
			value: crate::QueryableValue::String(id.to_string()),
		}]
	}

	type Model: Model;
}

#[cfg(test)]
pub mod tests {
	use super::{
		reader::{ResourceReader, ResourceReaderBacking},
		ReadTargets, ReadTargetsMut,
	};
	use crate::StreamDescription;

	#[derive(Debug)]
	pub struct TestResourceReader {
		data: Box<[u8]>,
	}

	impl TestResourceReader {
		pub fn new(data: Box<[u8]>) -> Self {
			Self { data }
		}
	}

	impl ResourceReader for TestResourceReader {
		fn read_into<'b, 'c: 'b, 'a: 'b>(
			&mut self,
			_: Option<&'c [StreamDescription]>,
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
				ReadTargetsMut::Streams { .. } => Err(()),
				ReadTargetsMut::BackingStorage => Ok(ReadTargets::Backing(ResourceReaderBacking::Buffer(self.data.clone()))),
			}
		}

		fn into_backing_storage(self: Box<Self>) -> Result<ResourceReaderBacking, Box<dyn ResourceReader>> {
			Ok(ResourceReaderBacking::Buffer(self.data))
		}
	}
}
