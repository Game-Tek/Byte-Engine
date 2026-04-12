//! The Redb storage backend provides a way to store and retrieve assets and resources using a Redb database.
//! This backend stores the resource's metadata/definition in a Redb database and the resource's binary data in a file.
//! Resource urls are hashed into `ResourceId`s which are the primary key of the database.
//! Resource metadata is stored in the database by serializing the `SerializableResource` struct into a byte array. The serialization is done using the `pot` crate.

use std::{hash::Hasher, path::Path};

use redb::{ReadableDatabase as _, ReadableTable};
use utils::sync::{remove_file, File, Write};

use crate::{
	asset,
	resource::{reader::redb::FileResourceReader, resource_handler::MultiResourceReader, ResourceId},
	ProcessedAsset, SerializableResource,
};

use super::{Query, ReadStorageBackend, StorageBackend, WriteStorageBackend};

pub struct RedbStorageBackend {
	db: redb::Database,
	base_path: std::path::PathBuf,
}

const RESOURCES_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resources");
const RESOURCE_MANAGEMENT_CODE_HASH: &str = env!("RESOURCE_MANAGEMENT_CODE_HASH");
const RESOURCE_MANAGEMENT_SIGNATURE_FILE: &str = ".resource-management-version";

fn sync_resource_management_signature(base_path: &Path) {
	std::fs::create_dir_all(base_path).unwrap();

	let signature_path = base_path.join(RESOURCE_MANAGEMENT_SIGNATURE_FILE);
	let database_path = base_path.join("resources.db");
	let stored_signature = std::fs::read_to_string(&signature_path)
		.ok()
		.map(|signature| signature.trim().to_string());

	if stored_signature.as_deref() == Some(RESOURCE_MANAGEMENT_CODE_HASH) {
		return;
	}

	if let Some(stored_signature) = stored_signature {
		log::info!(
			"Deleting resources at '{}' because the resource-management signature changed from '{}' to '{}'.",
			base_path.display(),
			stored_signature,
			RESOURCE_MANAGEMENT_CODE_HASH
		);

		std::fs::remove_dir_all(base_path).unwrap_or_else(|error| {
			panic!(
				"Failed to delete stale resources directory. The most likely cause is that another process is still using files inside '{}'. Error: {}",
				base_path.display(),
				error
			)
		});

		std::fs::create_dir_all(base_path).unwrap();
	} else if database_path.exists() {
		log::info!(
			"Deleting resources at '{}' because the resource-management signature marker is missing.",
			base_path.display()
		);

		std::fs::remove_dir_all(base_path).unwrap_or_else(|error| {
			panic!(
				"Failed to delete stale resources directory. The most likely cause is that another process is still using files inside '{}'. Error: {}",
				base_path.display(),
				error
			)
		});

		std::fs::create_dir_all(base_path).unwrap();
	}

	std::fs::write(&signature_path, RESOURCE_MANAGEMENT_CODE_HASH).unwrap_or_else(|error| {
		panic!(
			"Failed to write the resource-management signature file. The most likely cause is that the resources directory '{}' is not writable. Error: {}",
			base_path.display(),
			error
		)
	});
}

impl RedbStorageBackend {
	pub fn new(base_path: std::path::PathBuf) -> Self {
		let mut memory_only = false;

		if cfg!(test) {
			// If we are running tests we want to use memory database. This way we can run tests in parallel.
			memory_only = true;
		}

		let db_res = if !memory_only {
			sync_resource_management_signature(&base_path);
			redb::Database::create(base_path.join("resources.db"))
		} else {
			log::info!("Using memory database instead of file database.");
			redb::Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => match redb::Database::builder().create_with_backend(redb::backends::InMemoryBackend::new()) {
				Ok(db) => db,
				Err(_) => panic!("Could not create in-memory database"),
			},
		};

		{
			let write = db.begin_write().unwrap();
			let _ = write.open_table(RESOURCES_TABLE); // Create table if it doesn't exist
			let _ = write.commit();
		}

		RedbStorageBackend { db, base_path }
	}
}

impl ReadStorageBackend for RedbStorageBackend {
	fn list<'a>(&'a self) -> Result<Vec<String>, String> {
		let mut resources = Vec::new();

		let read = self.db.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		for doc in table.iter().unwrap() {
			let doc = doc.unwrap();
			let resource: SerializableResource = pot::from_slice(doc.1.value()).unwrap();
			resources.push(resource.id);
		}

		Ok(resources)
	}

	fn read<'s, 'a, 'b>(&'s self, id: asset::ResourceId<'b>) -> Option<(SerializableResource, MultiResourceReader)> {
		let read = self.db.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		let id = ResourceId::from(id.as_ref());

		if let Some(d) = table.get(&id).unwrap() {
			let resource: SerializableResource = pot::from_slice(d.value()).unwrap();
			let base_path = self.base_path.clone();

			let id: String = id.into();

			let resource_reader = Box::new(FileResourceReader::new(File::open(base_path.join(id)).ok()?));

			Some((resource, resource_reader))
		} else {
			None
		}
	}

	fn query<'a>(&'a self, query: Query<'a>) -> Result<Vec<(SerializableResource, MultiResourceReader)>, ()> {
		let base_path = self.base_path.clone();

		let read = self.db.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		let resources = table
			.iter()
			.unwrap()
			.filter_map(|d| {
				let d = d.unwrap();
				let resource: SerializableResource = pot::from_slice(d.1.value()).unwrap();

				let class = query.class.map_or(false, |c| c.contains(&resource.class.as_str()));

				if class {
					let hash = ResourceId(d.0.value());

					let id: String = hash.into();

					let resource_reader = Box::new(FileResourceReader::new(File::open(base_path.join(id)).unwrap()));

					Some((resource, resource_reader as MultiResourceReader))
				} else {
					None
				}
			})
			.collect::<Vec<_>>();

		Ok(resources)
	}
}

impl WriteStorageBackend for RedbStorageBackend {
	fn delete<'a>(&'a self, id: asset::ResourceId<'a>) -> Result<(), String> {
		let id = ResourceId::from(id.as_ref());

		let write = self.db.begin_write().unwrap();
		{
			let mut table = write.open_table(RESOURCES_TABLE).unwrap();
			let _ = table.remove(&id);
		}

		write.commit().map_err(|_| "Failed to commit transaction".to_string())?;

		let id: String = id.into();

		let resource_path = self.base_path.join(id);

		let _ = remove_file(&resource_path);

		Ok(())
	}

	fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &'a [u8]) -> Result<SerializableResource, ()> {
		let id = resource.id.clone();
		let size = data.len();
		let class = resource.class.clone();
		let streams = resource.streams.clone();

		let hash = {
			let mut hasher = gxhash::GxHasher::with_seed(961961961961961);

			std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)

			hasher.finish()
		};

		let rid = ResourceId::from(resource.id.as_ref());

		let resource = {
			let resource = SerializableResource {
				id,
				hash,
				class,
				size,
				streams,
				resource: resource.resource.clone(),
			};

			let write = self.db.begin_write().unwrap();

			{
				let mut table = write.open_table(RESOURCES_TABLE).unwrap();
				table.insert(&rid, pot::to_vec(&resource).unwrap().as_slice()).unwrap();
			}

			write.commit().map_err(|_| ())?;

			resource
		};

		let id: String = rid.into();

		let resource_path = self.base_path.join(id);

		let mut file = File::create(resource_path).unwrap();

		file.write_all(data).or(Err(()))?;
		file.flush().or(Err(()))?; // Must flush to ensure the file is written to disk, or else reads can cause failures

		Ok(resource)
	}

	fn sync<'s, 'a>(&'s self, other: &'a dyn ReadStorageBackend) -> () {
		if let Some(other) = other.downcast_ref::<RedbStorageBackend>() {
			{
				let write = self.db.begin_write().unwrap();
				write.delete_table(RESOURCES_TABLE).expect("Failed to delete table");
				write.open_table(RESOURCES_TABLE).expect("Failed to open table");
			}

			{
				let read = other.db.begin_read().unwrap();
				let source_table = read.open_table(RESOURCES_TABLE).unwrap();

				let write = self.db.begin_write().unwrap();

				{
					let mut dest_table = write.open_table(RESOURCES_TABLE).unwrap();

					for doc in source_table.iter().unwrap() {
						let doc = doc.unwrap();
						dest_table.insert(doc.0.value(), doc.1.value()).unwrap();
					}
				}

				write.commit().expect("Failed to commit transaction");
			}
		}
	}
}

impl StorageBackend for RedbStorageBackend {}
