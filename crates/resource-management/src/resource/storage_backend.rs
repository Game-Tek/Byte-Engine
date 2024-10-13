use std::{hash::Hasher, sync::{Arc, Mutex}};

use base64::Engine;
use gxhash::{HashMap, HashMapExt};
use redb::ReadableTable;
use utils::{r#async::AsyncWriteExt, remove_file, File};

use crate::{asset::{read_asset_from_source, BEADType, ResourceId}, BaseResource, Data, GenericResourceResponse, ProcessedAsset, StreamDescription};

use super::resource_handler::FileResourceReader;

pub trait ReadStorageBackend: Sync + Send + downcast_rs::Downcast {
	fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>>;
	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> utils::SendSyncBoxedFuture<'a, Option<(GenericResourceResponse, FileResourceReader)>>;

	/// Returns the type of the asset, if attainable from the url.
    /// Can serve as a filter for the asset handler to not attempt to load assets it can't handle.
    fn get_type<'a>(&'a self, url: ResourceId<'a>) -> Option<&'a str> {
        Some(url.get_extension())
    }

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> utils::BoxedFuture<'a, bool> {
        Box::pin(async move { self.read(id).await.is_some() })
    }
}

pub trait WriteStorageBackend: Sync + Send + downcast_rs::Downcast {
    fn delete<'a>(&'a self, id: ResourceId<'a>) -> utils::BoxedFuture<'a, Result<(), String>>;
    fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> utils::SendSyncBoxedFuture<'a, Result<GenericResourceResponse, ()>>;
    fn sync<'s, 'a>(&'s self, _: &'a dyn ReadStorageBackend) -> utils::BoxedFuture<'a, ()> {
        Box::pin(async move {})
    }

    fn start(&self, name: ResourceId<'_>) {}
}

downcast_rs::impl_downcast!(ReadStorageBackend);
downcast_rs::impl_downcast!(WriteStorageBackend);

pub struct DbStorageBackend {
    db: redb::Database,
    base_path: std::path::PathBuf,

	// #[cfg(test)]
	// resources: Arc<Mutex<Vec<(ProcessedAsset, Box<[u8]>)>>>,
	// #[cfg(test)]
	// files: Arc<Mutex<HashMap<&'static str, Box<[u8]>>>>,
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
			panic!();
            // polodb_core::Database::
        };

        let db = match db_res {
            Ok(db) => db,
            Err(_) => {
                // // Delete file and try again
                // std::fs::remove_file(path).unwrap();

                // log::warn!("Database file was corrupted, deleting and trying again.");

                // let db_res = polodb_core::Database::open_file(path);

                // match db_res {
                // 	Ok(db) => db,
                // 	Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
                // 		Ok(db) => {
                // 			log::error!("Could not create database file, using memory database instead.");
                // 			db
                // 		},
                // 		Err(_) => panic!("Could not create database"),
                // 	}
                // }
                panic!("Could not create database")
            }
        };

        // db.collection::<bson::Document>("resources").create_index(polodb_core::IndexModel{ keys: bson::doc! { "id": 1 }, options: Some(polodb_core::IndexOptions{ name: None, unique: Some(true) }) });

		{
			let write = db.begin_write().unwrap();
			let _ = write.open_table(TABLE); // Create table if it doesn't exist
			write.commit();
		}

        DbStorageBackend {
            db,
            base_path,

			// #[cfg(test)]
			// resources: Arc::new(Mutex::new(Vec::new())),
			// #[cfg(test)]
			// files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

	// #[cfg(test)]
	// pub fn add_file(&self, name: &'static str, data: &[u8]) {
	// 	self.files.lock().unwrap().insert(name, data.into());
	// }

	// #[cfg(test)]
	// pub fn get_resources(&self) -> Vec<ProcessedAsset> {
	// 	self.resources.lock().unwrap().iter().map(|x| x.0.clone()).collect()
	// }

	// #[cfg(test)]
	// pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
	// 	Some(self.resources.lock().unwrap().iter().find(|x| x.0.id == name.as_ref())?.1.clone())
	// }
}

// #[cfg(not(test))]
impl ReadStorageBackend for DbStorageBackend {
    fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async move {
            let mut resources = Vec::new();

            let read = self.db.begin_read().unwrap();
			let table = read.open_table(TABLE).unwrap();

            for doc in table.iter().unwrap() {
                let doc = doc.unwrap();
                resources.push(doc.0.value().to_string());
            }

            Ok(resources)
        })
    }

    fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> utils::SendSyncBoxedFuture<'a, Option<(GenericResourceResponse, FileResourceReader)>> {
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
	
			Box::pin(async move {
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
	
				let resource_reader = FileResourceReader::new(
					File::open(base_path.join(&uid)).await.ok()?,
				);
	
				Some((resource, resource_reader))
			})
		} else {
			Box::pin(async move { None })
		}
    }
}

impl WriteStorageBackend for DbStorageBackend {
    fn delete<'a>(&'a self, id: ResourceId<'a>) -> utils::BoxedFuture<'a, Result<(), String>> {
		let write = self.db.begin_write().unwrap();
		{
			let mut table = write.open_table(TABLE).unwrap();
			let _ = table.remove(id.as_ref());
		}
		write.commit();

		let uid = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.get_base().as_ref().as_bytes());
		let resource_path = self.base_path.join(std::path::Path::new(&uid));

		Box::pin(async move {
			let _ = remove_file(&resource_path).await;

			Ok(())
		})
    }

    fn store<'a, 'b: 'a>(&'a self,resource: &'b ProcessedAsset,data: &'a [u8],) -> utils::SendSyncBoxedFuture<'a, Result<GenericResourceResponse, ()>> {
        Box::pin(async move {
            let id = resource.id.clone();
            let size = data.len();
            let class = resource.class.clone();
			let streams = resource.streams.clone();

            let hash = {
                let mut hasher = gxhash::GxHasher::with_seed(961961961961961);

                std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)

				hasher.finish()
            };

			let serialized_resource = Data::from_serialize(&resource.resource).unwrap();
			let serialized_resource_bytes = pot::to_vec(&serialized_resource).unwrap();
			
            {
				let resource = BaseResource { id, hash, class, size, streams, resource: pot::to_vec(&resource.resource).unwrap() };
				let write = self.db.begin_write().unwrap();
				{
					let mut table = write.open_table(TABLE).unwrap();
					table.insert(resource.id.as_str(), serialized_resource_bytes.as_slice()).unwrap();
				}
				write.commit();
			}

            let resource_path = self.base_path.join(std::path::Path::new(&resource.id));

            let mut file = File::create(resource_path).await.or(Err(()))?;

            file.write_all(data).await.or(Err(()))?;
            file.flush().await.or(Err(()))?; // Must flush to ensure the file is written to disk, or else reads can cause failures

            Ok(GenericResourceResponse::new(resource.id.clone(), hash, resource.class.clone(), size, serialized_resource_bytes, resource.streams.clone()))
        })
    }

    fn sync<'s, 'a>(&'s self, other: &'a dyn ReadStorageBackend) -> utils::BoxedFuture<'a, ()> {
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

        Box::pin(async move {})
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
				resource: pot::from_slice(&resource.resource).unwrap(),
				streams: resource.streams,
			}
		}).collect()
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
	fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String> > {
		Box::pin(async move {
			Ok(self.0.lock().unwrap().keys().map(|x| x.to_string()).collect())
		})
	}

	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>,) -> utils::SendSyncBoxedFuture<'a, Option<(GenericResourceResponse, FileResourceReader)>> {
		// if let Some((resource, data)) = self.0.lock().unwrap().get(id.as_ref()) {
		// 	Box::pin(async move {
		// 		let resource: BaseResource = pot::from_slice(&resource).unwrap();
		// 		let base_path = std::path::PathBuf::new();
		// 		let uid = {
		// 			base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.get_base().as_ref().as_bytes())
		// 		};
		// 		let id = id.to_string();
		
		// 		let resource = {
		// 			let hash = resource.hash;
		// 			let class = resource.class;
		// 			let size = resource.size;
		// 			let streams: Option<Vec<StreamDescription>> = if let Some(arr) = resource.streams {
		// 				arr.iter().map(|v| {
		// 					Some(StreamDescription::new(&v.name, v.size as usize, v.offset as usize))
		// 				}).collect::<Option<Vec<_>>>()
		// 			} else {
		// 				None
		// 			};
		// 			GenericResourceResponse::new(id, hash, class, size, resource.resource, streams)
		// 		};			
			
		// 		let resource_reader = FileResourceReader::new(
		// 			File::open(base_path.join(&uid)).await.ok()?,
		// 		);
		
		// 		Some((resource, resource_reader))
		// 	})
		// } else {
		// }
		Box::pin(async move { None })
	}
}

#[cfg(test)]
impl WriteStorageBackend for TestStorageBackend {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> utils::BoxedFuture<'a, Result<(), String>> {
		Box::pin(async move {
			self.0.lock().unwrap().remove(id.as_ref());
			Ok(())
		})
	}

	fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8],) -> utils::SendSyncBoxedFuture<'a, Result<GenericResourceResponse, ()>> {
		let id = resource.id.clone();
		Box::pin(async move {
			let size = data.len();
			let class = resource.class.clone();
			let streams = resource.streams.clone();

			let hash = {
				let mut hasher = gxhash::GxHasher::with_seed(961961961961961);

				std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)

				hasher.finish()
			};

			let serialized_resource = Data::from_serialize(&resource.resource).unwrap();
			let serialized_resource_bytes = pot::to_vec(&serialized_resource).unwrap();

			let resource = BaseResource { id: id.clone(), hash, class, size, streams, resource: pot::to_vec(&resource.resource).unwrap() };

			self.0.lock().unwrap().insert(id.clone(), (serialized_resource_bytes.clone().into(), data.into()));

			Ok(GenericResourceResponse::new(id, hash, resource.class.clone(), size, serialized_resource_bytes, resource.streams.clone()))
		})
	}

	fn sync<'s, 'a>(&'s self, other: &'a dyn ReadStorageBackend) -> utils::BoxedFuture<'a, ()> {
		Box::pin(async move {})
	}
}

#[cfg(test)]
impl StorageBackend for TestStorageBackend {}