use std::{hash::Hasher, sync::{Arc, Mutex}};

use base64::Engine;
use gxhash::{HashMap, HashMapExt};
use redb::ReadableTable;
use utils::sync::{File, remove_file, Write};

use crate::{asset::{read_asset_from_source, BEADType, ResourceId}, BaseResource, Data, GenericResourceResponse, ProcessedAsset, StreamDescription};

use super::resource_handler::FileResourceReader;

pub trait ReadStorageBackend: Sync + Send + downcast_rs::Downcast {
	fn list<'a>(&'a self) -> Result<Vec<String>, String>;
	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> Option<(GenericResourceResponse, FileResourceReader)>;

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
    fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> Result<GenericResourceResponse, ()>;
    fn sync<'s, 'a>(&'s self, _: &'a dyn ReadStorageBackend) -> () {}

    fn start(&self, name: ResourceId<'_>) {}
}

downcast_rs::impl_downcast!(ReadStorageBackend);
downcast_rs::impl_downcast!(WriteStorageBackend);

pub struct DbStorageBackend {
    db: redb::Database,
    base_path: std::path::PathBuf,
}

const TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("resources");

impl DbStorageBackend {
    pub fn new(base_path: std::path::PathBuf) -> Self {
        let mut memory_only = false;

        if cfg!(test) {
            // If we are running tests we want to use memory database. This way we can run tests in parallel.
            memory_only = true;
        }

        let db_res = if !memory_only {
            redb::Database::create(base_path.join("resources.db"))
        } else {
            log::info!("Using memory database instead of file database.");
			redb::Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())
        };

        let db = match db_res {
            Ok(db) => db,
            Err(_) => {
                panic!("Could not create database")
            }
        };

		{
			let write = db.begin_write().unwrap();
			let _ = write.open_table(TABLE); // Create table if it doesn't exist
			write.commit();
		}

        DbStorageBackend {
            db,
            base_path,
        }
    }
}

impl ReadStorageBackend for DbStorageBackend {
    fn list<'a>(&'a self) -> Result<Vec<String>, String> {
		let mut resources = Vec::new();

		let read = self.db.begin_read().unwrap();
		let table = read.open_table(TABLE).unwrap();

		for doc in table.iter().unwrap() {
			let doc = doc.unwrap();
			resources.push(doc.0.value().to_string());
		}

		Ok(resources)
    }

    fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> Option<(GenericResourceResponse, FileResourceReader)> {
		let read = self.db.begin_read().unwrap();
		let table = read.open_table(TABLE).unwrap();
        if let Some(d) = table.get(id.get_base().as_ref()).unwrap() {
			let container: BaseResource = pot::from_slice(d.value()).unwrap();
			let base_path = self.base_path.clone();
			let uid = {
				base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.get_base().as_ref().as_bytes())
			};
			let id = id.to_string();
			let resource = container.resource.clone();

			let resource = {
				let hash = container.hash;
				let class = container.class;
				let size = container.size;
				let streams: Option<Vec<StreamDescription>> = if let Some(arr) = container.streams {
					arr.iter().map(|v| {
						Some(StreamDescription::new(&v.name, v.size as usize, v.offset as usize))
					}).collect::<Option<Vec<_>>>()
				} else {
					None
				};
				GenericResourceResponse::new(id, hash, class, size, resource, streams)
			};

			#[cfg(not(test))]
			{
				let resource_reader = FileResourceReader::new(
					File::open(base_path.join(&uid)).ok()?,
				);

				Some((resource, resource_reader))
			}

			#[cfg(test)]
			{
				unreachable!();
			}
		} else {
			None
		}
    }
}

impl WriteStorageBackend for DbStorageBackend {
    fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
		let write = self.db.begin_write().unwrap();
		{
			let mut table = write.open_table(TABLE).unwrap();
			let _ = table.remove(id.as_ref());
		}
		write.commit();

		let uid = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.get_base().as_ref().as_bytes());
		let resource_path = self.base_path.join(std::path::Path::new(&uid));

		let _ = remove_file(&resource_path);

		Ok(())
    }

    fn store<'a, 'b: 'a>(&'a self,resource: &'b ProcessedAsset,data: &'a [u8],) -> Result<GenericResourceResponse, ()> {
		let id = resource.id.clone();
		let size = data.len();
		let class = resource.class.clone();
		let streams = resource.streams.clone();

		let hash = {
		    let mut hasher = gxhash::GxHasher::with_seed(961961961961961);

		    std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)

			hasher.finish()
		};

		{
			let resource = BaseResource { id, hash, class, size, streams, resource: resource.resource.clone() };
			let write = self.db.begin_write().unwrap();
			{
				let mut table = write.open_table(TABLE).unwrap();
				table.insert(resource.id.as_str(), pot::to_vec(&resource).unwrap().as_slice()).unwrap();
			}
			write.commit();
		}

		let uid = {
			base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&resource.id)
		};

		let resource_path = self.base_path.join(std::path::Path::new(&uid));

		let mut file = File::create(resource_path).or(Err(()))?;

		file.write_all(data).or(Err(()))?;
		file.flush().or(Err(()))?; // Must flush to ensure the file is written to disk, or else reads can cause failures

		Ok(GenericResourceResponse::new(resource.id.clone(), hash, resource.class.clone(), size, resource.resource.clone(), resource.streams.clone()))
    }

    fn sync<'s, 'a>(&'s self, other: &'a dyn ReadStorageBackend) -> () {
        if let Some(other) = other.downcast_ref::<DbStorageBackend>() {
			{
				let write = self.db.begin_write().unwrap();
				write.delete_table(TABLE);
				write.open_table(TABLE);
			}

            {
				let read = other.db.begin_read().unwrap();
				let source_table = read.open_table(TABLE).unwrap();

				let write = self.db.begin_write().unwrap();

				{
					let mut dest_table = write.open_table(TABLE).unwrap();

					for doc in source_table.iter().unwrap() {
						let doc = doc.unwrap();
						dest_table.insert(doc.0.value(), doc.1.value()).unwrap();
					}
				}

				write.commit();
			}
        }
    }
}

pub trait StorageBackend: ReadStorageBackend + WriteStorageBackend {}

downcast_rs::impl_downcast!(StorageBackend);

impl StorageBackend for DbStorageBackend {}

#[cfg(test)]
#[derive(Clone)]
pub struct TestStorageBackend(pub Arc<Mutex<HashMap<String, (Box<[u8]>, Box<[u8]>)>>>);

#[cfg(test)]
impl TestStorageBackend {
	pub fn new() -> Self {
		Self(Arc::new(Mutex::new(HashMap::new())))
	}

	pub fn get_resources(&self) -> Vec<ProcessedAsset> {
		self.0.lock().unwrap().iter().map(|x| {
			let resource: BaseResource = pot::from_slice(&x.1.0).unwrap();
			ProcessedAsset {
				id: resource.id,
				class: resource.class,
				resource: resource.resource,
				streams: resource.streams,
			}
		}).collect()
	}

	pub fn get_resource(&self, name: ResourceId<'_>) -> Option<ProcessedAsset> {
		self.0.lock().unwrap().iter().find(|x| {
			let resource: BaseResource = pot::from_slice(&x.1.0).unwrap();
			resource.id == name.as_ref()
		}).map(|x| {
			let resource: BaseResource = pot::from_slice(&x.1.0).unwrap();
			ProcessedAsset {
				id: resource.id,
				class: resource.class,
				resource: resource.resource,
				streams: resource.streams,
			}
		})
	}

	pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
		Some(self.0.lock().unwrap().iter().find(|x| {
			let resource: BaseResource = pot::from_slice(&x.1.0).unwrap();
			resource.id == name.as_ref()
		})?.1 .1.clone())
	}
}

#[cfg(test)]
impl ReadStorageBackend for TestStorageBackend {
	fn list<'a>(&'a self) -> Result<Vec<String>, String> {
		Ok(self.0.lock().unwrap().keys().map(|x| x.to_string()).collect())
	}

	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> Option<(GenericResourceResponse, FileResourceReader)> {
		let (resource, data) = if let Some(e) = self.0.lock().unwrap().get(id.as_ref()) {
			(e.0.clone(), e.1.clone())
		} else {
			return None;
		};

		let id = id.get_base().to_string();

		let resource: BaseResource = pot::from_slice(&resource).unwrap();

		let resource = {
			let hash = resource.hash;
			let class = resource.class;
			let size = resource.size;
			let streams: Option<Vec<StreamDescription>> = if let Some(arr) = resource.streams {
				arr.iter().map(|v| {
					Some(StreamDescription::new(&v.name, v.size as usize, v.offset as usize))
				}).collect::<Option<Vec<_>>>()
			} else {
				None
			};
			GenericResourceResponse::new(id, hash, class, size, resource.resource, streams)
		};

		let resource_reader = FileResourceReader::new(data);

		Some((resource, resource_reader))
	}
}

#[cfg(test)]
impl WriteStorageBackend for TestStorageBackend {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
		self.0.lock().unwrap().remove(id.as_ref());
		Ok(())
	}

	fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> Result<GenericResourceResponse, ()> {
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

		let container = BaseResource { id: id.clone(), hash, class, size, streams, resource: resource.resource.clone() };

		let serialized_container = pot::to_vec(&container).unwrap();

		self.0.lock().unwrap().insert(id.clone(), (serialized_container.into(), data.into()));

		Ok(GenericResourceResponse::new(id, hash, container.class.clone(), size, serialized_resource_bytes, container.streams.clone()))
	}

	fn sync<'s, 'a>(&'s self, other: &'a dyn ReadStorageBackend) -> () {
		{}
	}
}

#[cfg(test)]
impl StorageBackend for TestStorageBackend {}
