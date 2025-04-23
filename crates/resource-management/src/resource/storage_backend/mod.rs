//! The storage backend provides a way to store and retrieve assets and resources from a storage backend.

pub mod redb_storage_backend;

use crate::{asset::ResourceId, ProcessedAsset, SerializableResource};

use super::resource_handler::MultiResourceReader;

pub struct Query<'a> {
	class: Option<&'a [&'a str]>,
}

impl<'a> Query<'a> {
	pub fn new() -> Self {
		Self {
			class: None,
		}
	}

	pub fn classes(mut self, classes: &'a [&'a str]) -> Self {
		self.class = Some(classes);
		self
	}	
}

pub trait ReadStorageBackend: Sync + Send + downcast_rs::Downcast {
	fn list<'a>(&'a self) -> Result<Vec<String>, String>;
	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> Option<(SerializableResource, MultiResourceReader)>;

	fn query<'a>(&'a self, query: Query<'a>) -> Result<Vec<(SerializableResource, MultiResourceReader)>, ()>;

	/// Returns the type of the asset, if attainable from the url.
    /// Can serve as a filter for the asset handler to not attempt to load assets it can't handle.
    fn get_type<'a>(&'a self, url: ResourceId<'a>) -> Option<&'a str> {
        Some(url.get_extension())
    }

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> bool {
        self.read(id).is_some()
    }
}

pub trait WriteStorageBackend: Sync + Send + downcast_rs::Downcast {
    fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;
    fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> Result<SerializableResource, ()>;
    fn sync<'s, 'a>(&'s self, _: &'a dyn ReadStorageBackend) -> () {}

    fn start(&self, _: ResourceId<'_>) {}
}

downcast_rs::impl_downcast!(ReadStorageBackend);
downcast_rs::impl_downcast!(WriteStorageBackend);

pub trait StorageBackend: ReadStorageBackend + WriteStorageBackend {}

downcast_rs::impl_downcast!(StorageBackend);

#[cfg(test)]
pub mod tests {
	use std::{hash::Hasher, sync::Arc};

	use gxhash::HashMapExt;
	use utils::{sync::Mutex, hash::HashMap};

	use crate::resource::resource_handler::tests::MemoryResourceReader;

	use super::*;

	#[derive(Clone)]
	pub struct TestStorageBackend(pub Arc<Mutex<HashMap<String, (Box<[u8]>, Box<[u8]>)>>>);
	
	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::new())))
		}
	
		pub fn get_resources(&self) -> Vec<ProcessedAsset> {
			self.0.lock().iter().map(|x| {
				let resource: SerializableResource = pot::from_slice(&x.1.0).unwrap();
				ProcessedAsset {
					id: resource.id,
					class: resource.class,
					resource: resource.resource,
					streams: resource.streams,
				}
			}).collect()
		}
	
		pub fn get_resource(&self, name: ResourceId<'_>) -> Option<ProcessedAsset> {
			self.0.lock().iter().find(|x| {
				let resource: SerializableResource = pot::from_slice(&x.1.0).unwrap();
				resource.id == name.as_ref()
			}).map(|x| {
				let resource: SerializableResource = pot::from_slice(&x.1.0).unwrap();
				ProcessedAsset {
					id: resource.id,
					class: resource.class,
					resource: resource.resource,
					streams: resource.streams,
				}
			})
		}
	
		pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
			Some(self.0.lock().iter().find(|x| {
				let resource: SerializableResource = pot::from_slice(&x.1.0).unwrap();
				resource.id == name.as_ref()
			})?.1 .1.clone())
		}
	}
	
	impl ReadStorageBackend for TestStorageBackend {
		fn list<'a>(&'a self) -> Result<Vec<String>, String> {
			Ok(self.0.lock().keys().map(|x| x.to_string()).collect())
		}
	
		fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> Option<(SerializableResource, MultiResourceReader)> {
			let (resource, data) = if let Some(e) = self.0.lock().get(id.as_ref()) {
				(e.0.clone(), e.1.clone())
			} else {
				return None;
			};
	
			let _ = id.get_base().to_string();
	
			let resource: SerializableResource = pot::from_slice(&resource).unwrap();
	
			let resource_reader = Box::new(MemoryResourceReader::new(data));
	
			Some((resource, resource_reader))
		}

		fn query<'a>(&'a self, _: Query<'a>) -> Result<Vec<(SerializableResource, MultiResourceReader)>, ()> {
			Err(())
		}
	}
	
	impl WriteStorageBackend for TestStorageBackend {
		fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
			self.0.lock().remove(id.as_ref());
			Ok(())
		}
	
		fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> Result<SerializableResource, ()> {
			let id = resource.id.clone();
			let size = data.len();
			let class = resource.class.clone();
			let streams = resource.streams.clone();
	
			let hash = {
				let mut hasher = gxhash::GxHasher::with_seed(961961961961961);
	
				std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)
	
				hasher.finish()
			};
	
			let serialized_resource_bytes = resource.resource.clone();
	
			let container = SerializableResource { id: id.clone(), hash, class, size, streams, resource: resource.resource.clone() };
	
			let serialized_container = pot::to_vec(&container).unwrap();
	
			self.0.lock().insert(id.clone(), (serialized_container.into(), data.into()));
	
			Ok(SerializableResource::new(id, hash, container.class.clone(), size, serialized_resource_bytes, container.streams.clone()))
		}
	
		fn sync<'s, 'a>(&'s self, _: &'a dyn ReadStorageBackend) -> () {
			{}
		}
	}
	
	impl StorageBackend for TestStorageBackend {}
}