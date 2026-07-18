//! The Redb storage backend provides a way to store and retrieve assets and resources using a Redb database.
//! This backend stores the resource's metadata/definition in a Redb database and the resource's binary data in a file.
//! Resource urls are hashed into `ResourceId`s which are the primary key of the database.
//! Resource metadata is stored in the database by archiving the `SerializableResource` struct into a byte array with rkyv.

use std::{hash::Hasher, path::Path};

use redb::{ReadableDatabase as _, ReadableTable};
use utils::sync::{remove_file, File, Write};

use super::{Query, QueryCursor, QueryError, QueryPage, ReadStorageBackend, StorageBackend, WriteStorageBackend};
use crate::{
	asset,
	resource::{reader::redb::FileResourceReader, resource_handler::MultiResourceReader, ResourceId},
	ProcessedAsset, QueryableProperty, QueryableValue, SerializableResource,
};

pub struct RedbStorageBackend {
	db: RedbDatabase,
	base_path: std::path::PathBuf,
}

/// The `RedbDatabase` enum keeps the runtime database read-only while allowing explicit resource producers to write.
enum RedbDatabase {
	Writable(redb::Database),
	ReadOnly(redb::ReadOnlyDatabase),
}

const RESOURCES_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resources");
const RESOURCE_CLASS_INDEX_TABLE: redb::TableDefinition<&[u8], [u8; 16]> = redb::TableDefinition::new("resource-class-index");
const RESOURCE_PROPERTY_INDEX_TABLE: redb::TableDefinition<&[u8], [u8; 16]> =
	redb::TableDefinition::new("resource-property-index");
const RESOURCE_MANAGEMENT_CODE_HASH: &str = env!("RESOURCE_MANAGEMENT_CODE_HASH");
const RESOURCE_MANAGEMENT_SIGNATURE_FILE: &str = ".resource-management-version";
const RESOURCE_PRODUCER_SIGNATURE_FILE: &str = ".resource-producer-version";

fn read_resource_cache_signature(base_path: &Path, signature_file: &str) -> Option<String> {
	std::fs::read_to_string(base_path.join(signature_file))
		.ok()
		.map(|signature| signature.trim().to_string())
}

/// Rejects a release resource store produced by a different resource schema before archived values are read.
fn validate_resource_management_signature(base_path: &Path) -> Result<(), String> {
	match read_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE) {
		Some(signature) if signature == RESOURCE_MANAGEMENT_CODE_HASH => Ok(()),
		Some(signature) => Err(format!(
			"resource-management signature '{signature}' does not match this engine's expected signature '{RESOURCE_MANAGEMENT_CODE_HASH}'"
		)),
		None => Err(format!(
			"resource-management signature marker '{}' is missing",
			RESOURCE_MANAGEMENT_SIGNATURE_FILE
		)),
	}
}

/// Writes one cache-owner signature beside the resource database.
fn write_resource_cache_signature(base_path: &Path, signature_file: &str, signature: &str) {
	std::fs::write(base_path.join(signature_file), signature).unwrap_or_else(|error| {
		panic!(
			"Failed to write resource cache signature file '{}'. The most likely cause is that the resources directory '{}' is not writable. Error: {}",
			signature_file,
			base_path.display(),
			error
		)
	});
}

/// Removes every persisted value after a cache owner reports an incompatible signature.
fn reset_resource_cache(base_path: &Path) {
	std::fs::remove_dir_all(base_path).unwrap_or_else(|error| {
		panic!(
			"Failed to delete stale resources directory. The most likely cause is that another process is still using files inside '{}'. Error: {}",
			base_path.display(),
			error
		)
	});

	std::fs::create_dir_all(base_path).unwrap();
}

/// Synchronizes the resource-management implementation marker shared by every database opener.
fn sync_resource_management_signature(base_path: &Path) {
	std::fs::create_dir_all(base_path).unwrap();

	let stored_signature = read_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE);

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
		reset_resource_cache(base_path);
	} else if base_path.join("resources.db").exists() {
		log::info!(
			"Deleting resources at '{}' because the resource-management signature marker is missing.",
			base_path.display()
		);
		reset_resource_cache(base_path);
	}

	write_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE, RESOURCE_MANAGEMENT_CODE_HASH);
}

/// Synchronizes an optional producer marker without changing the marker expected by plain BELD and release openers.
fn sync_resource_producer_signature(base_path: &Path, expected_signature: &str) {
	std::fs::create_dir_all(base_path).unwrap();

	let stored_signature = read_resource_cache_signature(base_path, RESOURCE_PRODUCER_SIGNATURE_FILE);
	if stored_signature.as_deref() == Some(expected_signature) {
		return;
	}

	if let Some(stored_signature) = stored_signature {
		log::info!(
			"Deleting resources at '{}' because the resource producer signature changed from '{}' to '{}'.",
			base_path.display(),
			stored_signature,
			expected_signature
		);
		reset_resource_cache(base_path);
	} else if base_path.join("resources.db").exists() {
		log::info!(
			"Deleting resources at '{}' because the resource producer signature marker is missing.",
			base_path.display()
		);
		reset_resource_cache(base_path);
	}

	// Producer invalidation removes the shared marker too, so restore both markers before the database is reopened.
	write_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE, RESOURCE_MANAGEMENT_CODE_HASH);
	write_resource_cache_signature(base_path, RESOURCE_PRODUCER_SIGNATURE_FILE, expected_signature);
}

fn resource_key_hex(key: [u8; 16]) -> String {
	ResourceId(key).into()
}

fn class_index_key(class: &str, key: [u8; 16]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(class.len() + 1 + 32);
	bytes.extend_from_slice(class.as_bytes());
	bytes.push(0);
	bytes.extend_from_slice(resource_key_hex(key).as_bytes());
	bytes
}

fn property_index_key(class: &str, property: &str, value: &str, key: [u8; 16]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(class.len() + property.len() + value.len() + 3 + 32);
	bytes.extend_from_slice(class.as_bytes());
	bytes.push(0);
	bytes.extend_from_slice(property.as_bytes());
	bytes.push(0);
	bytes.extend_from_slice(value.as_bytes());
	bytes.push(0);
	bytes.extend_from_slice(resource_key_hex(key).as_bytes());
	bytes
}

fn extract_string(value: &QueryableValue) -> Option<&str> {
	match value {
		QueryableValue::String(value) => Some(value.as_str()),
	}
}

fn remove_indexes(
	class_table: &mut redb::Table<&[u8], [u8; 16]>,
	property_table: &mut redb::Table<&[u8], [u8; 16]>,
	resource: &SerializableResource,
	resource_key: [u8; 16],
) {
	let class_key = class_index_key(&resource.class, resource_key);
	let _ = class_table.remove(class_key.as_slice());

	for property in &resource.queryable_properties {
		let QueryableProperty { name, value } = property;
		let Some(value) = extract_string(value) else {
			continue;
		};

		let property_key = property_index_key(&resource.class, name, value, resource_key);
		let _ = property_table.remove(property_key.as_slice());
	}
}

fn insert_indexes(
	class_table: &mut redb::Table<&[u8], [u8; 16]>,
	property_table: &mut redb::Table<&[u8], [u8; 16]>,
	resource: &SerializableResource,
	resource_key: [u8; 16],
) {
	let class_key = class_index_key(&resource.class, resource_key);
	class_table.insert(class_key.as_slice(), resource_key).unwrap();

	for property in &resource.queryable_properties {
		let QueryableProperty { name, value } = property;
		let Some(value) = extract_string(value) else {
			continue;
		};

		let property_key = property_index_key(&resource.class, name, value, resource_key);
		property_table.insert(property_key.as_slice(), resource_key).unwrap();
	}
}

impl RedbStorageBackend {
	pub fn new(base_path: std::path::PathBuf) -> Self {
		Self::new_with_optional_producer_signature(base_path, None, cfg!(not(debug_assertions)))
	}

	/// Opens a resource database with write access for tools that produce baked resources.
	pub fn new_writable(base_path: std::path::PathBuf) -> Self {
		Self::new_with_optional_producer_signature(base_path, None, false)
	}

	/// Opens a resource database whose persisted values also depend on an external resource producer.
	///
	/// Runtime asset pipelines use this constructor so changing their generated resource ABI invalidates values baked by
	/// the previous producer implementation.
	pub fn new_with_producer_signature(base_path: std::path::PathBuf, producer_signature: &str) -> Self {
		Self::new_with_optional_producer_signature(base_path, Some(producer_signature), false)
	}

	/// Opens the database after synchronizing its shared and optional producer-specific cache owners.
	fn new_with_optional_producer_signature(
		base_path: std::path::PathBuf,
		producer_signature: Option<&str>,
		read_only: bool,
	) -> Self {
		let mut memory_only = false;

		if cfg!(test) {
			memory_only = true;
		}

		std::fs::create_dir_all(&base_path).unwrap();

		let db = if memory_only {
			log::info!("Using memory database instead of file database.");
			RedbDatabase::Writable(
				redb::Database::builder()
					.create_with_backend(redb::backends::InMemoryBackend::new())
					.unwrap_or_else(|_| panic!("Could not create in-memory database")),
			)
		} else if read_only {
			validate_resource_management_signature(&base_path).unwrap_or_else(|error| {
				panic!(
					"Failed to open resources database in read-only mode. The baked resources are incompatible or incomplete; rerun BELD with the matching engine revision. Error: {error}"
				)
			});
			RedbDatabase::ReadOnly(redb::ReadOnlyDatabase::open(base_path.join("resources.db")).unwrap_or_else(|error| {
				panic!(
					"Failed to open resources database in read-only mode. The most likely cause is that BELD has not baked the resources or the database is invalid. Error: {}",
					error
				)
			}))
		} else {
			sync_resource_management_signature(&base_path);
			if let Some(producer_signature) = producer_signature {
				sync_resource_producer_signature(&base_path, producer_signature);
			}
			let db = redb::Database::create(base_path.join("resources.db")).unwrap_or_else(|_| {
				redb::Database::builder()
					.create_with_backend(redb::backends::InMemoryBackend::new())
					.unwrap_or_else(|_| panic!("Could not create in-memory database"))
			});
			RedbDatabase::Writable(db)
		};

		if let RedbDatabase::Writable(db) = &db {
			let write = db.begin_write().unwrap();
			let _ = write.open_table(RESOURCES_TABLE);
			let _ = write.open_table(RESOURCE_CLASS_INDEX_TABLE);
			let _ = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE);
			let _ = write.commit();
		}

		RedbStorageBackend { db, base_path }
	}

	fn begin_read(&self) -> Result<redb::ReadTransaction, redb::TransactionError> {
		match &self.db {
			RedbDatabase::Writable(db) => db.begin_read(),
			RedbDatabase::ReadOnly(db) => db.begin_read(),
		}
	}

	fn open_reader(&self, id: [u8; 16]) -> Option<MultiResourceReader> {
		let file_id = resource_key_hex(id);
		Some(Box::new(FileResourceReader::new(
			File::open(self.base_path.join(file_id)).ok()?,
		)))
	}

	pub fn read_uid(&self, id: ResourceId) -> Option<(SerializableResource, MultiResourceReader)> {
		let read = self.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		if let Some(d) = table.get(&id).unwrap() {
			let resource: SerializableResource = crate::from_slice(d.value()).unwrap();
			let resource_reader = self.open_reader(id.0)?;

			Some((resource, resource_reader))
		} else {
			None
		}
	}

	fn query_index(
		&self,
		query: &Query,
		use_property_index: bool,
	) -> Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError> {
		let cursor = query.cursor.as_ref().map(|cursor| cursor.token.as_slice());
		let read = self.begin_read().map_err(|_| QueryError::StorageFailure)?;
		let resources_table = read.open_table(RESOURCES_TABLE).map_err(|_| QueryError::StorageFailure)?;

		let mut items = Vec::new();
		let mut last_key = None;
		let mut has_more = false;

		if use_property_index {
			let (property, value) = query.first_indexed_predicate().ok_or(QueryError::StorageFailure)?;
			let value = extract_string(value).ok_or(QueryError::StorageFailure)?;
			let index_table = read
				.open_table(RESOURCE_PROPERTY_INDEX_TABLE)
				.map_err(|_| QueryError::StorageFailure)?;

			for entry in index_table.iter().map_err(|_| QueryError::StorageFailure)? {
				let entry = entry.map_err(|_| QueryError::StorageFailure)?;
				let key = entry.0.value();
				let prefix = property_index_key(&query.class, property, value, [0; 16]);
				let prefix = &prefix[..prefix.len() - 32];

				if !key.starts_with(prefix) {
					continue;
				}

				if let Some(cursor) = cursor {
					if key <= cursor {
						continue;
					}
				}

				let resource_key = entry.1.value();
				let serialized = resources_table.get(&resource_key).map_err(|_| QueryError::StorageFailure)?;
				let Some(serialized) = serialized else {
					continue;
				};

				let archived = crate::archived_from_slice::<SerializableResource>(serialized.value())
					.map_err(|_| QueryError::StorageFailure)?;
				if !query.matches_archived(archived) {
					continue;
				}

				if items.len() >= query.limit {
					has_more = true;
					break;
				}

				let resource: SerializableResource =
					crate::from_slice(serialized.value()).map_err(|_| QueryError::StorageFailure)?;
				let reader = self.open_reader(resource_key).ok_or(QueryError::StorageFailure)?;
				items.push((resource, reader));
				last_key = Some(key.to_vec());
			}
		} else {
			let index_table = read
				.open_table(RESOURCE_CLASS_INDEX_TABLE)
				.map_err(|_| QueryError::StorageFailure)?;

			for entry in index_table.iter().map_err(|_| QueryError::StorageFailure)? {
				let entry = entry.map_err(|_| QueryError::StorageFailure)?;
				let key = entry.0.value();
				let prefix = class_index_key(&query.class, [0; 16]);
				let prefix = &prefix[..prefix.len() - 32];

				if !key.starts_with(prefix) {
					continue;
				}

				if let Some(cursor) = cursor {
					if key <= cursor {
						continue;
					}
				}

				let resource_key = entry.1.value();
				let serialized = resources_table.get(&resource_key).map_err(|_| QueryError::StorageFailure)?;
				let Some(serialized) = serialized else {
					continue;
				};

				let archived = crate::archived_from_slice::<SerializableResource>(serialized.value())
					.map_err(|_| QueryError::StorageFailure)?;
				if !query.matches_archived(archived) {
					continue;
				}

				if items.len() >= query.limit {
					has_more = true;
					break;
				}

				let resource: SerializableResource =
					crate::from_slice(serialized.value()).map_err(|_| QueryError::StorageFailure)?;
				let reader = self.open_reader(resource_key).ok_or(QueryError::StorageFailure)?;
				items.push((resource, reader));
				last_key = Some(key.to_vec());
			}
		}

		Ok(QueryPage {
			items,
			cursor: if has_more { last_key.map(QueryCursor::new) } else { None },
		})
	}
}

impl ReadStorageBackend for RedbStorageBackend {
	fn list(&self) -> Result<Vec<String>, String> {
		let mut resources = Vec::new();

		let read = self.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		for doc in table.iter().unwrap() {
			let doc = doc.unwrap();
			let resource: SerializableResource = crate::from_slice(doc.1.value()).unwrap();
			resources.push(resource.id);
		}

		Ok(resources)
	}

	fn read<'s, 'a, 'b>(&'s self, id: asset::ResourceId<'b>) -> Option<(SerializableResource, MultiResourceReader)> {
		let read = self.begin_read().unwrap();
		let table = read.open_table(RESOURCES_TABLE).unwrap();

		let id = ResourceId::from(id.as_ref());

		if let Some(d) = table.get(&id).unwrap() {
			let resource: SerializableResource = crate::from_slice(d.value()).unwrap();
			let resource_reader = self.open_reader(id.0)?;

			Some((resource, resource_reader))
		} else {
			None
		}
	}

	fn query(&self, query: Query) -> Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError> {
		if query.limit == 0 {
			return Ok(QueryPage {
				items: Vec::new(),
				cursor: None,
			});
		}

		if let Some(cursor) = &query.cursor {
			if cursor.token.is_empty() {
				return Err(QueryError::InvalidCursor);
			}
		}

		self.query_index(&query, query.first_indexed_predicate().is_some())
	}
}

impl WriteStorageBackend for RedbStorageBackend {
	fn delete<'a>(&'a self, id: asset::ResourceId<'a>) -> Result<(), String> {
		let write = match &self.db {
			RedbDatabase::Writable(db) => db
				.begin_write()
				.map_err(|_| "Failed to begin delete transaction".to_string())?,
			RedbDatabase::ReadOnly(_) => {
				return Err("Cannot delete from a read-only resources database".to_string());
			}
		};
		let id = ResourceId::from(id.as_ref());

		{
			let mut resources_table = write.open_table(RESOURCES_TABLE).unwrap();
			let mut class_table = write.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
			let mut property_table = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();

			if let Some(existing) = resources_table.get(&id).unwrap() {
				let resource: SerializableResource = crate::from_slice(existing.value()).unwrap();
				remove_indexes(&mut class_table, &mut property_table, &resource, id.0);
			}

			let _ = resources_table.remove(&id);
		}

		write.commit().map_err(|_| "Failed to commit transaction".to_string())?;

		let id: String = id.into();
		let resource_path = self.base_path.join(id);
		let _ = remove_file(&resource_path);

		Ok(())
	}

	fn store_in(
		&self,
		resource: ProcessedAsset,
		data: &[u8],
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()> {
		let write = match &self.db {
			RedbDatabase::Writable(db) => db.begin_write().map_err(|_| ())?,
			RedbDatabase::ReadOnly(_) => return Err(()),
		};
		let size = data.len();

		let hash = {
			let mut hasher = gxhash::GxHasher::with_seed(961961961961961);
			std::hash::Hasher::write(&mut hasher, data);
			hasher.finish()
		};

		let rid = ResourceId::from(resource.id.as_ref());

		let resource = {
			let resource = resource.into_serializable(hash, size);

			{
				let mut resources_table = write.open_table(RESOURCES_TABLE).unwrap();
				let mut class_table = write.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
				let mut property_table = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();

				if let Some(existing) = resources_table.get(&rid).unwrap() {
					let existing: SerializableResource = crate::from_slice(existing.value()).unwrap();
					remove_indexes(&mut class_table, &mut property_table, &existing, rid.0);
				}

				let serialized_resource = crate::to_vec_in(&resource, allocator).unwrap();
				resources_table.insert(&rid, serialized_resource.as_slice()).unwrap();
				insert_indexes(&mut class_table, &mut property_table, &resource, rid.0);
			}

			write.commit().map_err(|_| ())?;

			resource
		};

		let id: String = rid.into();
		let resource_path = self.base_path.join(id);
		let mut file = File::create(resource_path).unwrap();

		file.write_all(data).or(Err(()))?;
		file.flush().or(Err(()))?;

		Ok(resource)
	}

	fn sync(&self, other: &dyn ReadStorageBackend) {
		let RedbDatabase::Writable(db) = &self.db else {
			return;
		};

		if let Some(other) = other.downcast_ref::<RedbStorageBackend>() {
			{
				let write = db.begin_write().unwrap();
				write.delete_table(RESOURCES_TABLE).expect("Failed to delete table");
				write.open_table(RESOURCES_TABLE).expect("Failed to open table");
				write
					.delete_table(RESOURCE_CLASS_INDEX_TABLE)
					.expect("Failed to delete table");
				write.open_table(RESOURCE_CLASS_INDEX_TABLE).expect("Failed to open table");
				write
					.delete_table(RESOURCE_PROPERTY_INDEX_TABLE)
					.expect("Failed to delete table");
				write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).expect("Failed to open table");
			}

			{
				let read = other.begin_read().unwrap();
				let source_resources = read.open_table(RESOURCES_TABLE).unwrap();
				let source_classes = read.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
				let source_properties = read.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();

				let write = db.begin_write().unwrap();

				{
					let mut dest_resources = write.open_table(RESOURCES_TABLE).unwrap();
					let mut dest_classes = write.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
					let mut dest_properties = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();

					for doc in source_resources.iter().unwrap() {
						let doc = doc.unwrap();
						dest_resources.insert(doc.0.value(), doc.1.value()).unwrap();
					}

					for doc in source_classes.iter().unwrap() {
						let doc = doc.unwrap();
						dest_classes.insert(doc.0.value(), doc.1.value()).unwrap();
					}

					for doc in source_properties.iter().unwrap() {
						let doc = doc.unwrap();
						dest_properties.insert(doc.0.value(), doc.1.value()).unwrap();
					}
				}

				write.commit().expect("Failed to commit transaction");
			}
		}
	}
}

impl StorageBackend for RedbStorageBackend {}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::{
		sync_resource_management_signature, sync_resource_producer_signature, validate_resource_management_signature,
		RedbStorageBackend, RESOURCE_MANAGEMENT_CODE_HASH, RESOURCE_MANAGEMENT_SIGNATURE_FILE,
		RESOURCE_PRODUCER_SIGNATURE_FILE,
	};
	use crate::{
		resource::storage_backend::{Query, QueryCursor, QueryError, ReadStorageBackend, WriteStorageBackend},
		Model, ProcessedAsset,
	};

	#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct MockMaterialModel {
		group: String,
		tag: String,
	}

	impl Model for MockMaterialModel {
		fn get_class() -> &'static str {
			"MockMaterial"
		}

		fn queryable_properties(&self, id: &str) -> Vec<crate::QueryableProperty> {
			vec![
				crate::QueryableProperty {
					name: "name".to_string(),
					value: crate::QueryableValue::String(id.to_string()),
				},
				crate::QueryableProperty {
					name: "group".to_string(),
					value: crate::QueryableValue::String(self.group.clone()),
				},
				crate::QueryableProperty {
					name: "tag".to_string(),
					value: crate::QueryableValue::String(self.tag.clone()),
				},
			]
		}
	}

	#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct MockShaderModel {
		stage: String,
	}

	impl Model for MockShaderModel {
		fn get_class() -> &'static str {
			"MockShader"
		}

		fn queryable_properties(&self, id: &str) -> Vec<crate::QueryableProperty> {
			vec![
				crate::QueryableProperty {
					name: "name".to_string(),
					value: crate::QueryableValue::String(id.to_string()),
				},
				crate::QueryableProperty {
					name: "stage".to_string(),
					value: crate::QueryableValue::String(self.stage.clone()),
				},
			]
		}
	}

	fn backend() -> RedbStorageBackend {
		static NEXT_BACKEND_ID: AtomicUsize = AtomicUsize::new(0);

		let unique = format!(
			"byte-engine-redb-tests-{}-{}",
			std::process::id(),
			NEXT_BACKEND_ID.fetch_add(1, Ordering::Relaxed)
		);
		RedbStorageBackend::new(std::env::temp_dir().join(unique))
	}

	#[test]
	fn read_only_signature_validation_rejects_missing_and_stale_resource_stores() {
		static NEXT_SIGNATURE_VALIDATION_ID: AtomicUsize = AtomicUsize::new(0);
		let resources_path = std::env::temp_dir().join(format!(
			"byte-engine-read-only-signature-tests-{}-{}",
			std::process::id(),
			NEXT_SIGNATURE_VALIDATION_ID.fetch_add(1, Ordering::Relaxed)
		));
		std::fs::create_dir_all(&resources_path).unwrap();

		assert!(validate_resource_management_signature(&resources_path).is_err());
		std::fs::write(resources_path.join(RESOURCE_MANAGEMENT_SIGNATURE_FILE), "stale").unwrap();
		assert!(validate_resource_management_signature(&resources_path).is_err());
		std::fs::write(
			resources_path.join(RESOURCE_MANAGEMENT_SIGNATURE_FILE),
			RESOURCE_MANAGEMENT_CODE_HASH,
		)
		.unwrap();
		assert_eq!(validate_resource_management_signature(&resources_path), Ok(()));

		std::fs::remove_dir_all(resources_path).unwrap();
	}

	#[test]
	fn producer_signature_invalidates_stale_values_without_plain_marker_ping_pong() {
		static NEXT_SIGNATURE_TEST_ID: AtomicUsize = AtomicUsize::new(0);

		let resources_path = std::env::temp_dir().join(format!(
			"byte-engine-resource-signature-tests-{}-{}",
			std::process::id(),
			NEXT_SIGNATURE_TEST_ID.fetch_add(1, Ordering::Relaxed)
		));
		let retained_resource = resources_path.join("retained-resource");

		// An existing plain cache has no producer marker and must be invalidated once when debug first opens it.
		sync_resource_management_signature(&resources_path);
		std::fs::write(resources_path.join("resources.db"), b"old database").unwrap();
		std::fs::write(&retained_resource, b"persisted resource").unwrap();
		sync_resource_producer_signature(&resources_path, "producer-a");
		assert!(!retained_resource.exists());

		std::fs::write(&retained_resource, b"compatible resource").unwrap();
		// Plain BELD/release opens intentionally ignore the separate producer marker and must not erase a debug cache.
		sync_resource_management_signature(&resources_path);
		assert!(retained_resource.exists());
		assert_eq!(
			std::fs::read_to_string(resources_path.join(RESOURCE_PRODUCER_SIGNATURE_FILE)).unwrap(),
			"producer-a"
		);

		// Reopening with the same producer also retains values.
		sync_resource_producer_signature(&resources_path, "producer-a");
		assert!(retained_resource.exists());

		// A different producer can emit a different shader/resource ABI, so none of its predecessor's values are reusable.
		sync_resource_producer_signature(&resources_path, "producer-b");
		assert!(!retained_resource.exists());
		assert_eq!(
			std::fs::read_to_string(resources_path.join(RESOURCE_MANAGEMENT_SIGNATURE_FILE)).unwrap(),
			RESOURCE_MANAGEMENT_CODE_HASH
		);
		assert_eq!(
			std::fs::read_to_string(resources_path.join(RESOURCE_PRODUCER_SIGNATURE_FILE)).unwrap(),
			"producer-b"
		);

		std::fs::remove_dir_all(resources_path).unwrap();
	}

	fn store_mock<T: Model>(backend: &RedbStorageBackend, id: &str, resource: T) {
		let asset = ProcessedAsset::new(crate::asset::ResourceId::new(id), resource);
		backend.store(asset, id.as_bytes()).unwrap();
	}

	fn query_ids(backend: &RedbStorageBackend, query: Query) -> (Vec<String>, Option<super::QueryCursor>) {
		let page = backend.query(query).unwrap();
		(page.items.into_iter().map(|(resource, _)| resource.id).collect(), page.cursor)
	}

	#[test]
	fn query_by_class_pages_results() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		);
		store_mock(
			&backend,
			"materials/c",
			MockMaterialModel {
				group: "transparent".into(),
				tag: "hero".into(),
			},
		);

		let (first_ids, cursor) = query_ids(&backend, Query::new("MockMaterial").limit(2));
		assert_eq!(first_ids.len(), 2);
		assert!(cursor.is_some());

		let (second_ids, cursor) = query_ids(&backend, Query::new("MockMaterial").limit(2).cursor(cursor.unwrap()));
		assert_eq!(second_ids.len(), 1);
		assert!(cursor.is_none());

		let mut ids = first_ids;
		ids.extend(second_ids);
		ids.sort();
		assert_eq!(ids, vec!["materials/a", "materials/b", "materials/c"]);
	}

	#[test]
	fn query_by_name_uses_property_index() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		);

		let (ids, cursor) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/b").limit(10));
		assert_eq!(ids, vec!["materials/b"]);
		assert!(cursor.is_none());
	}

	#[test]
	fn query_filters_multiple_predicates() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		);
		store_mock(
			&backend,
			"materials/c",
			MockMaterialModel {
				group: "transparent".into(),
				tag: "hero".into(),
			},
		);

		let (ids, _) = query_ids(
			&backend,
			Query::new("MockMaterial").eq("group", "opaque").eq("tag", "hero").limit(10),
		);
		assert_eq!(ids, vec!["materials/a"]);
	}

	#[test]
	fn query_isolates_types() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/shared",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);
		store_mock(
			&backend,
			"shaders/shared",
			MockShaderModel {
				stage: "fragment".into(),
			},
		);

		let (material_ids, _) = query_ids(&backend, Query::new("MockMaterial").limit(10));
		let (shader_ids, _) = query_ids(&backend, Query::new("MockShader").limit(10));

		assert_eq!(material_ids, vec!["materials/shared"]);
		assert_eq!(shader_ids, vec!["shaders/shared"]);
	}

	#[test]
	fn query_returns_empty_for_unknown_name() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);

		let (ids, cursor) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/missing").limit(10));
		assert!(ids.is_empty());
		assert!(cursor.is_none());
	}

	#[test]
	fn delete_updates_indexes() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);
		backend.delete(crate::asset::ResourceId::new("materials/a")).unwrap();

		let (ids, _) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/a").limit(10));
		assert!(ids.is_empty());
	}

	#[test]
	fn malformed_cursor_returns_error() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);

		let error = backend
			.query(Query {
				class: "MockMaterial".to_string(),
				predicates: vec![],
				limit: 2,
				cursor: Some(QueryCursor::new(Vec::new())),
			})
			.unwrap_err();

		assert_eq!(error, QueryError::InvalidCursor);
	}
}
