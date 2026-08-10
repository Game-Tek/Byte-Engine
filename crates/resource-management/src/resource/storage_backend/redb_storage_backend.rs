//! Store resource metadata in Redb and binary payloads in companion files.
//!
//! The backend hashes resource URLs into [`ResourceId`](crate::asset::ResourceId)
//! database keys and archives [`SerializableResource`]
//! metadata with rkyv.

/// Selects how one resource store persists binary payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceStorageMode {
	/// Stores each resource payload in its own hash-named file.
	#[default]
	Files,
	/// Appends every resource payload to one shared file.
	Packed,
}

impl ResourceStorageMode {
	fn as_bytes(self) -> &'static [u8] {
		match self {
			Self::Files => b"files",
			Self::Packed => b"packed",
		}
	}

	fn from_bytes(bytes: &[u8]) -> Option<Self> {
		match bytes {
			b"files" => Some(Self::Files),
			b"packed" => Some(Self::Packed),
			_ => None,
		}
	}
}

/// The `RedbStorageBackend` struct provides persistent storage for baked resource metadata and payloads.
pub struct RedbStorageBackend {
	db: RedbDatabase,
	base_path: std::path::PathBuf,
	storage_mode: ResourceStorageMode,
}

/// The `RedbDatabase` enum keeps the runtime database read-only while allowing explicit resource producers to write.
enum RedbDatabase {
	Writable(redb::Database),
	ReadOnly(redb::ReadOnlyDatabase),
}

const RESOURCES_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resources");
const STORE_CONFIGURATION_TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("store-configuration");
const PACKED_RESOURCE_OFFSETS_TABLE: redb::TableDefinition<[u8; 16], u64> =
	redb::TableDefinition::new("packed-resource-offsets");
const RESOURCE_CLASS_INDEX_TABLE: redb::TableDefinition<&[u8], [u8; 16]> = redb::TableDefinition::new("resource-class-index");
const RESOURCE_PROPERTY_INDEX_TABLE: redb::TableDefinition<&[u8], [u8; 16]> =
	redb::TableDefinition::new("resource-property-index");
#[cfg(debug_assertions)]
const RESOURCE_TRACES_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resource-traces");
const RESOURCE_MANAGEMENT_CODE_HASH: &str = env!("RESOURCE_MANAGEMENT_CODE_HASH");
const RESOURCE_MANAGEMENT_SIGNATURE_FILE: &str = ".resource-management-version";
const STORAGE_MODE_KEY: &str = "payload-storage-mode";
const PACKED_RESOURCES_FILE: &str = "resources.pack";

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
	/// Opens the resource database with the access required by the current engine build.
	///
	/// Debug builds can update their resource cache. Release builds only read resources that have a matching
	/// resource-management signature. Use [`Self::new_writable`] for an explicit resource-producing workflow.
	pub fn new(base_path: std::path::PathBuf) -> Self {
		if cfg!(debug_assertions) {
			Self::new_writable(base_path)
		} else {
			Self::open_read_only(base_path).unwrap_or_else(|error| {
				panic!(
					"Failed to open resources database in read-only mode. The baked resources are incompatible or incomplete; rerun BELD with the matching engine revision. Error: {error}"
				)
			})
		}
	}

	/// Opens a compatible resource database without modifying its directory.
	///
	/// Read-only tools use this constructor so a signature mismatch remains available for the user to inspect or replace.
	/// After handling an incompatibility, use [`Self::new_writable`] only when the user has requested a resource update.
	pub fn open_read_only(base_path: std::path::PathBuf) -> Result<Self, String> {
		validate_resource_management_signature(&base_path)?;
		let database_path = base_path.join("resources.db");
		let db = redb::ReadOnlyDatabase::open(&database_path)
			.map(RedbDatabase::ReadOnly)
			.map_err(|error| format!("resource database '{}' could not be opened: {error}", database_path.display()))?;
		let mut backend = Self {
			db,
			base_path,
			storage_mode: ResourceStorageMode::Files,
		};
		backend.storage_mode = backend.persisted_storage_mode()?;
		Ok(backend)
	}

	/// Opens a writable resource store and preserves its persisted payload mode.
	///
	/// New stores use [`ResourceStorageMode::Files`]. Use [`Self::new_writable_with_mode`] to select another mode.
	pub fn new_writable(base_path: std::path::PathBuf) -> Self {
		Self::open_writable(base_path, None).unwrap_or_else(|error| panic!("Failed to open resource store. {error}"))
	}

	/// Opens a writable resource store with the selected payload mode.
	///
	/// Existing stores must already use `storage_mode`. This prevents a bake from mixing incompatible payload layouts.
	///
	/// # Errors
	///
	/// Returns an error when an existing store uses another payload mode.
	pub fn new_writable_with_mode(base_path: std::path::PathBuf, storage_mode: ResourceStorageMode) -> Result<Self, String> {
		Self::open_writable(base_path, Some(storage_mode))
	}

	/// Opens the database and establishes one payload mode before callers can store resources.
	fn open_writable(base_path: std::path::PathBuf, requested_mode: Option<ResourceStorageMode>) -> Result<Self, String> {
		std::fs::create_dir_all(&base_path).unwrap();
		let db = if cfg!(test) {
			log::info!("Using memory database instead of file database.");
			RedbDatabase::Writable(
				redb::Database::builder()
					.create_with_backend(redb::backends::InMemoryBackend::new())
					.unwrap_or_else(|_| panic!("Could not create in-memory database")),
			)
		} else {
			sync_resource_management_signature(&base_path);
			let db = redb::Database::create(base_path.join("resources.db")).unwrap_or_else(|_| {
				redb::Database::builder()
					.create_with_backend(redb::backends::InMemoryBackend::new())
					.unwrap_or_else(|_| panic!("Could not create in-memory database"))
			});
			RedbDatabase::Writable(db)
		};

		let RedbDatabase::Writable(writable_db) = &db else {
			unreachable!();
		};
		let write = writable_db.begin_write().unwrap();
		let _ = write.open_table(RESOURCES_TABLE);
		let _ = write.open_table(RESOURCE_CLASS_INDEX_TABLE);
		let _ = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE);
		let _ = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE);
		#[cfg(debug_assertions)]
		let _ = write.open_table(RESOURCE_TRACES_TABLE);
		let storage_mode = {
			let mut configuration = write.open_table(STORE_CONFIGURATION_TABLE).unwrap();
			let stored_mode = configuration.get(STORAGE_MODE_KEY).unwrap().map(|value| {
				ResourceStorageMode::from_bytes(value.value()).unwrap_or_else(|| {
					panic!("Failed to open resource store. The most likely cause is an unknown persisted payload storage mode.")
				})
			});
			match (stored_mode, requested_mode) {
				(Some(stored), Some(requested)) if stored != requested => {
					return Err(format!(
						"Resource store mode does not match. The destination already uses '{stored:?}', but '{requested:?}' was requested."
					));
				}
				(Some(stored), _) => stored,
				(None, requested) => {
					let selected = requested.unwrap_or_default();
					configuration.insert(STORAGE_MODE_KEY, selected.as_bytes()).unwrap();
					selected
				}
			}
		};
		write.commit().unwrap();

		Ok(RedbStorageBackend {
			db,
			base_path,
			storage_mode,
		})
	}

	fn begin_read(&self) -> Result<redb::ReadTransaction, redb::TransactionError> {
		match &self.db {
			RedbDatabase::Writable(db) => db.begin_read(),
			RedbDatabase::ReadOnly(db) => db.begin_read(),
		}
	}

	/// Reads the store-wide payload mode before any resource locations are interpreted.
	fn persisted_storage_mode(&self) -> Result<ResourceStorageMode, String> {
		let read = self
			.begin_read()
			.map_err(|error| format!("resource database read failed: {error}"))?;
		let table = read
			.open_table(STORE_CONFIGURATION_TABLE)
			.map_err(|error| format!("resource store configuration is missing: {error}"))?;
		let mode = table
			.get(STORAGE_MODE_KEY)
			.map_err(|error| format!("resource payload mode could not be read: {error}"))?
			.ok_or_else(|| "resource payload mode is missing".to_string())?;
		ResourceStorageMode::from_bytes(mode.value()).ok_or_else(|| "resource payload mode is not recognized".to_string())
	}

	/// Opens one reader whose byte zero is the start of the requested resource.
	async fn open_reader(&self, id: [u8; 16], resource_size: usize, packed_offset: Option<u64>) -> Option<MultiResourceReader> {
		match self.storage_mode {
			ResourceStorageMode::Files => {
				let file = AsyncFile::open(self.base_path.join(resource_key_hex(id))).await.ok()?;
				let size = file.metadata().await.ok()?.len();
				Some(Box::new(FileResourceReader::new(&file, size).ok()?))
			}
			ResourceStorageMode::Packed => {
				let file = AsyncFile::open(self.base_path.join(PACKED_RESOURCES_FILE)).await.ok()?;
				let file_size = file.metadata().await.ok()?.len();
				let resource_size = u64::try_from(resource_size).ok()?;
				Some(Box::new(
					FileResourceReader::new_range(&file, file_size, packed_offset?, resource_size).ok()?,
				))
			}
		}
	}

	pub fn read_uid(&self, id: ResourceId) -> BoxedFuture<'_, Option<(SerializableResource, MultiResourceReader)>> {
		r#async::future(async move {
			let (resource, packed_offset) = {
				let read = self.begin_read().ok()?;
				let table = read.open_table(RESOURCES_TABLE).ok()?;
				let resource = table
					.get(&id)
					.ok()?
					.map(|data| crate::from_slice::<SerializableResource>(data.value()).ok())??;
				let packed_offset = if self.storage_mode == ResourceStorageMode::Packed {
					read.open_table(PACKED_RESOURCE_OFFSETS_TABLE)
						.ok()?
						.get(&id)
						.ok()?
						.map(|value| value.value())
				} else {
					None
				};
				(resource, packed_offset)
			};
			let resource_reader = self.open_reader(id.0, resource.size(), packed_offset).await?;

			Some((resource, resource_reader))
		})
	}

	fn query_index(
		&self,
		query: &Query,
		use_property_index: bool,
	) -> Result<QueryPage<(SerializableResource, [u8; 16])>, QueryError> {
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
				items.push((resource, resource_key));
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
				items.push((resource, resource_key));
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
	fn list(&self) -> BoxedFuture<'_, Result<Vec<String>, String>> {
		r#async::future(async {
			let mut resources = Vec::new();

			let read = self.begin_read().unwrap();
			let table = read.open_table(RESOURCES_TABLE).unwrap();

			for doc in table.iter().unwrap() {
				let doc = doc.unwrap();
				let resource: SerializableResource = crate::from_slice(doc.1.value()).unwrap();
				resources.push(resource.id);
			}

			Ok(resources)
		})
	}

	fn read<'a>(&'a self, id: asset::ResourceId<'a>) -> BoxedFuture<'a, Option<(SerializableResource, MultiResourceReader)>> {
		r#async::future(async move {
			let id = ResourceId::from(id.as_ref());
			let (resource, packed_offset) = {
				let read = self.begin_read().ok()?;
				let table = read.open_table(RESOURCES_TABLE).ok()?;
				let resource = table
					.get(&id)
					.ok()?
					.map(|data| crate::from_slice::<SerializableResource>(data.value()).ok())??;
				let packed_offset = if self.storage_mode == ResourceStorageMode::Packed {
					read.open_table(PACKED_RESOURCE_OFFSETS_TABLE)
						.ok()?
						.get(&id)
						.ok()?
						.map(|value| value.value())
				} else {
					None
				};
				(resource, packed_offset)
			};
			let resource_reader = self.open_reader(id.0, resource.size(), packed_offset).await?;

			Some((resource, resource_reader))
		})
	}

	fn query(
		&self,
		query: Query,
	) -> BoxedFuture<'_, Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>> {
		r#async::future(async move {
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

			let page = self.query_index(&query, query.first_indexed_predicate().is_some())?;
			let mut items = Vec::with_capacity(page.items.len());
			for (resource, resource_key) in page.items {
				let packed_offset = if self.storage_mode == ResourceStorageMode::Packed {
					let read = self.begin_read().map_err(|_| QueryError::StorageFailure)?;
					let offsets = read
						.open_table(PACKED_RESOURCE_OFFSETS_TABLE)
						.map_err(|_| QueryError::StorageFailure)?;
					Some(
						offsets
							.get(&resource_key)
							.map_err(|_| QueryError::StorageFailure)?
							.ok_or(QueryError::StorageFailure)?
							.value(),
					)
				} else {
					None
				};
				let reader = self
					.open_reader(resource_key, resource.size(), packed_offset)
					.await
					.ok_or(QueryError::StorageFailure)?;
				items.push((resource, reader));
			}

			Ok(QueryPage {
				items,
				cursor: page.cursor,
			})
		})
	}

	#[cfg(debug_assertions)]
	fn read_trace<'a>(&'a self, id: asset::ResourceId<'a>) -> BoxedFuture<'a, Result<Vec<ResourceTraceItem>, String>> {
		r#async::future(async move {
			let read = self
				.begin_read()
				.map_err(|_| "Failed to begin resource trace read".to_string())?;
			let table = read
				.open_table(RESOURCE_TRACES_TABLE)
				.map_err(|_| "Failed to open resource traces table".to_string())?;
			let id = ResourceId::from(id.as_ref());
			let Some(items) = table.get(&id).map_err(|_| "Failed to read resource trace".to_string())? else {
				return Ok(Vec::new());
			};

			crate::from_slice(items.value()).map_err(|_| "Failed to deserialize resource trace".to_string())
		})
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
			let mut packed_offsets = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE).unwrap();
			#[cfg(debug_assertions)]
			let mut traces_table = write.open_table(RESOURCE_TRACES_TABLE).unwrap();

			if let Some(existing) = resources_table.get(&id).unwrap() {
				let resource: SerializableResource = crate::from_slice(existing.value()).unwrap();
				remove_indexes(&mut class_table, &mut property_table, &resource, id.0);
			}

			let _ = resources_table.remove(&id);
			let _ = packed_offsets.remove(&id);
			#[cfg(debug_assertions)]
			let _ = traces_table.remove(&id);
		}

		write.commit().map_err(|_| "Failed to commit transaction".to_string())?;

		if self.storage_mode == ResourceStorageMode::Files {
			let id: String = id.into();
			let _ = remove_file(self.base_path.join(id));
		}

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

		// Holding the Redb write transaction serializes pack appends across writers and processes.
		let packed_offset = if self.storage_mode == ResourceStorageMode::Packed {
			let mut file = std::fs::OpenOptions::new()
				.create(true)
				.read(true)
				.append(true)
				.open(self.base_path.join(PACKED_RESOURCES_FILE))
				.map_err(|_| ())?;
			let offset = file.seek(SeekFrom::End(0)).map_err(|_| ())?;
			file.write_all(data).map_err(|_| ())?;
			file.flush().map_err(|_| ())?;
			file.sync_data().map_err(|_| ())?;
			Some(offset)
		} else {
			None
		};

		let resource = resource.into_serializable(hash, size);
		{
			let mut resources_table = write.open_table(RESOURCES_TABLE).map_err(|_| ())?;
			let mut class_table = write.open_table(RESOURCE_CLASS_INDEX_TABLE).map_err(|_| ())?;
			let mut property_table = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).map_err(|_| ())?;
			let mut packed_offsets = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE).map_err(|_| ())?;

			if let Some(existing) = resources_table.get(&rid).map_err(|_| ())? {
				let existing: SerializableResource = crate::from_slice(existing.value()).map_err(|_| ())?;
				remove_indexes(&mut class_table, &mut property_table, &existing, rid.0);
			}

			let serialized_resource = crate::to_vec_in(&resource, allocator).map_err(|_| ())?;
			resources_table.insert(&rid, serialized_resource.as_slice()).map_err(|_| ())?;
			if let Some(offset) = packed_offset {
				packed_offsets.insert(&rid, offset).map_err(|_| ())?;
			} else {
				let _ = packed_offsets.remove(&rid);
			}
			insert_indexes(&mut class_table, &mut property_table, &resource, rid.0);
		}

		write.commit().map_err(|_| ())?;

		if self.storage_mode == ResourceStorageMode::Files {
			let id: String = rid.into();
			let mut file = SyncFile::create(self.base_path.join(id)).map_err(|_| ())?;
			file.write_all(data).map_err(|_| ())?;
			file.flush().map_err(|_| ())?;
		}

		Ok(resource)
	}

	#[cfg(debug_assertions)]
	fn replace_trace(&self, id: asset::ResourceId<'_>, items: &[ResourceTraceItem]) -> Result<(), String> {
		let write = match &self.db {
			RedbDatabase::Writable(db) => db
				.begin_write()
				.map_err(|_| "Failed to begin resource trace write".to_string())?,
			RedbDatabase::ReadOnly(_) => {
				return Err("Cannot write traces to a read-only resources database".to_string());
			}
		};
		let id = ResourceId::from(id.as_ref());
		{
			let mut table = write
				.open_table(RESOURCE_TRACES_TABLE)
				.map_err(|_| "Failed to open resource traces table".to_string())?;
			if items.is_empty() {
				table.remove(&id).map_err(|_| "Failed to clear resource trace".to_string())?;
			} else {
				let serialized =
					crate::to_vec(&items.to_vec()).map_err(|_| "Failed to serialize resource trace".to_string())?;
				table
					.insert(&id, serialized.as_slice())
					.map_err(|_| "Failed to store resource trace".to_string())?;
			}
		}

		write.commit().map_err(|_| "Failed to commit resource trace".to_string())
	}

	fn sync(&self, other: &dyn ReadStorageBackend) {
		let RedbDatabase::Writable(db) = &self.db else {
			return;
		};
		let Some(other) = other.downcast_ref::<RedbStorageBackend>() else {
			return;
		};
		if std::ptr::eq(self, other) || self.base_path == other.base_path {
			return;
		}
		if self.storage_mode != other.storage_mode {
			log::error!(
				"Failed to synchronize resource stores. The most likely cause is that the source and destination use different payload storage modes."
			);
			return;
		}

		let read = other.begin_read().unwrap();
		let source_resources = read.open_table(RESOURCES_TABLE).unwrap();
		// Copy payloads before metadata so readers never receive a location that has not been copied.
		match self.storage_mode {
			ResourceStorageMode::Files => {
				for doc in source_resources.iter().unwrap() {
					let file_name = resource_key_hex(doc.unwrap().0.value());
					std::fs::copy(other.base_path.join(&file_name), self.base_path.join(file_name)).unwrap();
				}
			}
			ResourceStorageMode::Packed => {
				let source = other.base_path.join(PACKED_RESOURCES_FILE);
				let destination = self.base_path.join(PACKED_RESOURCES_FILE);
				if source.exists() {
					std::fs::copy(source, destination).unwrap();
				} else {
					SyncFile::create(destination).unwrap();
				}
			}
		}

		let source_classes = read.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
		let source_properties = read.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();
		let source_offsets = read.open_table(PACKED_RESOURCE_OFFSETS_TABLE).unwrap();
		let clear = db.begin_write().unwrap();
		clear.delete_table(RESOURCES_TABLE).unwrap();
		clear.delete_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
		clear.delete_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();
		clear.delete_table(PACKED_RESOURCE_OFFSETS_TABLE).unwrap();
		clear.commit().unwrap();

		let write = db.begin_write().unwrap();
		{
			let mut dest_resources = write.open_table(RESOURCES_TABLE).unwrap();
			let mut dest_classes = write.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
			let mut dest_properties = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();
			let mut dest_offsets = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE).unwrap();
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
			for doc in source_offsets.iter().unwrap() {
				let doc = doc.unwrap();
				dest_offsets.insert(doc.0.value(), doc.1.value()).unwrap();
			}
		}
		write.commit().unwrap();
	}
}

impl StorageBackend for RedbStorageBackend {}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::{
		validate_resource_management_signature, RedbStorageBackend, ResourceStorageMode, PACKED_RESOURCES_FILE,
		RESOURCE_MANAGEMENT_CODE_HASH, RESOURCE_MANAGEMENT_SIGNATURE_FILE,
	};
	use crate::{
		resource::storage_backend::{Query, QueryCursor, QueryError, ReadStorageBackend, WriteStorageBackend},
		Model, ProcessedAsset,
	};
	#[cfg(debug_assertions)]
	use crate::{ResourceTraceItem, ResourceTraceLevel};

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

	fn backend_with_mode(storage_mode: ResourceStorageMode) -> RedbStorageBackend {
		static NEXT_BACKEND_ID: AtomicUsize = AtomicUsize::new(0);

		let unique = format!(
			"byte-engine-redb-tests-{}-{}",
			std::process::id(),
			NEXT_BACKEND_ID.fetch_add(1, Ordering::Relaxed)
		);
		RedbStorageBackend::new_writable_with_mode(std::env::temp_dir().join(unique), storage_mode).unwrap()
	}

	fn backend() -> RedbStorageBackend {
		backend_with_mode(ResourceStorageMode::Files)
	}

	#[crate::r#async::test]
	async fn packed_storage_reads_resource_ranges_and_appends_replacements() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let first_id = crate::asset::ResourceId::new("first.test");
		let second_id = crate::asset::ResourceId::new("second.test");
		backend
			.store(
				ProcessedAsset::new(first_id, MockShaderModel { stage: "first".into() }),
				b"first-payload",
			)
			.unwrap();
		backend
			.store(
				ProcessedAsset::new(second_id, MockShaderModel { stage: "second".into() }),
				b"second-payload",
			)
			.unwrap();

		{
			let (_, reader) = backend.read(first_id).await.unwrap();
			let backing = reader.into_backing_storage().await.unwrap();
			assert_eq!(backing.as_slice(), b"first-payload");
		}
		{
			let (_, reader) = backend.read(second_id).await.unwrap();
			let backing = reader.into_backing_storage().await.unwrap();
			assert_eq!(backing.as_slice(), b"second-payload");
		}

		// Replacement appends a new extent so readers holding an old map remain valid.
		backend
			.store(
				ProcessedAsset::new(
					first_id,
					MockShaderModel {
						stage: "replacement".into(),
					},
				),
				b"replacement",
			)
			.unwrap();
		let (_, reader) = backend.read(first_id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), b"replacement");
		drop(backing);

		let expected_pack_size = b"first-payload".len() + b"second-payload".len() + b"replacement".len();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			expected_pack_size as u64
		);

		backend.delete(second_id).unwrap();
		assert!(backend.read(second_id).await.is_none());
		std::fs::remove_dir_all(&backend.base_path).unwrap();
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

	fn store_mock<T: Model>(backend: &RedbStorageBackend, id: &str, resource: T) {
		let asset = ProcessedAsset::new(crate::asset::ResourceId::new(id), resource);
		backend.store(asset, id.as_bytes()).unwrap();
	}

	async fn query_ids(backend: &RedbStorageBackend, query: Query) -> (Vec<String>, Option<super::QueryCursor>) {
		let page = backend.query(query).await.unwrap();
		(page.items.into_iter().map(|(resource, _)| resource.id).collect(), page.cursor)
	}

	#[crate::r#async::test]
	async fn query_by_class_pages_results() {
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

		let (first_ids, cursor) = query_ids(&backend, Query::new("MockMaterial").limit(2)).await;
		assert_eq!(first_ids.len(), 2);
		assert!(cursor.is_some());

		let (second_ids, cursor) = query_ids(&backend, Query::new("MockMaterial").limit(2).cursor(cursor.unwrap())).await;
		assert_eq!(second_ids.len(), 1);
		assert!(cursor.is_none());

		let mut ids = first_ids;
		ids.extend(second_ids);
		ids.sort();
		assert_eq!(ids, vec!["materials/a", "materials/b", "materials/c"]);
	}

	#[crate::r#async::test]
	async fn query_by_name_uses_property_index() {
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

		let (ids, cursor) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/b").limit(10)).await;
		assert_eq!(ids, vec!["materials/b"]);
		assert!(cursor.is_none());
	}

	#[crate::r#async::test]
	async fn query_filters_multiple_predicates() {
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
		)
		.await;
		assert_eq!(ids, vec!["materials/a"]);
	}

	#[crate::r#async::test]
	async fn query_isolates_types() {
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

		let (material_ids, _) = query_ids(&backend, Query::new("MockMaterial").limit(10)).await;
		let (shader_ids, _) = query_ids(&backend, Query::new("MockShader").limit(10)).await;

		assert_eq!(material_ids, vec!["materials/shared"]);
		assert_eq!(shader_ids, vec!["shaders/shared"]);
	}

	#[crate::r#async::test]
	async fn query_returns_empty_for_unknown_name() {
		let backend = backend();
		store_mock(
			&backend,
			"materials/a",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "hero".into(),
			},
		);

		let (ids, cursor) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/missing").limit(10)).await;
		assert!(ids.is_empty());
		assert!(cursor.is_none());
	}

	#[cfg(debug_assertions)]
	#[crate::r#async::test]
	async fn trace_round_trips_without_creating_a_resource_and_delete_clears_it() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("broken.asset");
		let items = vec![ResourceTraceItem::new(
			ResourceTraceLevel::Error,
			"Asset is malformed. The most likely cause is invalid fixture data.".to_string(),
		)];

		backend.replace_trace(id, &items).unwrap();

		assert_eq!(backend.read_trace(id).await.unwrap(), items);
		assert!(backend.read(id).await.is_none());
		assert!(backend.list().await.unwrap().is_empty());

		backend.delete(id).unwrap();
		assert!(backend.read_trace(id).await.unwrap().is_empty());
	}

	#[crate::r#async::test]
	async fn delete_updates_indexes() {
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

		let (ids, _) = query_ids(&backend, Query::new("MockMaterial").eq("name", "materials/a").limit(10)).await;
		assert!(ids.is_empty());
	}

	#[crate::r#async::test]
	async fn malformed_cursor_returns_error() {
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
			.await
			.unwrap_err();

		assert_eq!(error, QueryError::InvalidCursor);
	}
}

use std::{
	hash::Hasher,
	io::{Seek as _, SeekFrom},
	path::Path,
};

use redb::{ReadableDatabase as _, ReadableTable};
use utils::sync::{remove_file, File as SyncFile, Write};

use super::{Query, QueryCursor, QueryError, QueryPage, ReadStorageBackend, StorageBackend, WriteStorageBackend};
#[cfg(debug_assertions)]
use crate::ResourceTraceItem;
use crate::{
	asset,
	r#async::{self, BoxedFuture, File as AsyncFile},
	resource::{reader::redb::FileResourceReader, resource_handler::MultiResourceReader, ResourceId},
	ProcessedAsset, QueryableProperty, QueryableValue, SerializableResource,
};
