//! Store resource metadata in Redb and binary payloads in companion files.
//!
//! The backend hashes resource URLs into [`ResourceId`](crate::asset::ResourceId)
//! database keys and archives [`SerializableResource`]
//! metadata with rkyv.

mod packed_allocator;

/// The `ReDBStorageBackend` struct provides persistent storage for baked resource metadata and payloads.
///
/// Use one backend instance for a directory while it can be updated. Drop any
/// read-only backend and its returned mapped backings before opening that
/// directory for writing.
pub struct ReDBStorageBackend {
	db: RedbDatabase,
	base_path: std::path::PathBuf,
	storage_mode: ResourceStorageMode,
	image_compression: ResourceGpuCompressionPolicy,
	packed_allocator: Option<PackedResourceAllocator>,
}

/// The `RedbDatabase` enum lets applications read compatible stores without writes while tools and cache recovery can update them.
enum RedbDatabase {
	Writable(redb::Database),
	ReadOnly(redb::ReadOnlyDatabase),
}

impl ReDBStorageBackend {
	/// Opens an application resource store and synchronizes its resource-management signature.
	///
	/// Debug applications can update their resource cache. Release applications keep a compatible
	/// store read-only, but discard a mismatched store before reading stale values. Use
	/// [`Self::open_read_only`] when a tool must inspect a store without changing it. Use
	/// [`Self::new_writable_with_mode`] when a producer must select a payload storage mode.
	pub fn new(base_path: std::path::PathBuf) -> Self {
		if cfg!(debug_assertions) || validate_resource_management_signature(&base_path).is_err() {
			Self::new_writable(base_path)
		} else {
			Self::open_read_only(base_path).unwrap_or_else(|error| {
				panic!(
					"Failed to open a compatible resources database in read-only mode. The most likely cause is an incomplete or corrupt resource store. Rebuild the resource directory with BELD. See {}. Error: {error}",
					crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
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
			image_compression: ResourceGpuCompressionPolicy::Disabled,
			packed_allocator: None,
		};
		let settings = backend.persisted_settings()?;
		backend.storage_mode = settings.storage_mode;
		backend.image_compression = settings.image_compression;
		Ok(backend)
	}

	/// Opens a writable resource store and preserves its persisted payload mode.
	///
	/// New stores use [`ResourceStorageMode::Files`]. Use [`Self::new_writable_with_mode`] to select another mode.
	pub fn new_writable(base_path: std::path::PathBuf) -> Self {
		Self::open_writable(base_path, None, None).unwrap_or_else(|error| panic!("Failed to open resource store. {error}"))
	}

	/// Opens a writable resource store with the selected payload mode.
	///
	/// Existing stores must already use `storage_mode`. This prevents a bake from mixing incompatible payload layouts.
	///
	/// # Errors
	///
	/// Returns an error when an existing store uses another payload mode.
	pub fn new_writable_with_mode(base_path: std::path::PathBuf, storage_mode: ResourceStorageMode) -> Result<Self, String> {
		Self::open_writable(base_path, Some(storage_mode), None)
	}

	/// Opens a writable resource store with explicit payload and image-compression settings.
	pub fn new_writable_with_settings(
		base_path: std::path::PathBuf,
		settings: ResourceStorageSettings,
	) -> Result<Self, String> {
		settings.validate()?;
		Self::open_writable(base_path, Some(settings.storage_mode), Some(settings.image_compression))
	}

	/// Opens the database and establishes one payload mode before callers can store resources.
	fn open_writable(
		base_path: std::path::PathBuf,
		requested_mode: Option<ResourceStorageMode>,
		requested_compression: Option<ResourceGpuCompressionPolicy>,
	) -> Result<Self, String> {
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
		let settings = {
			let mut configuration = write.open_table(STORE_CONFIGURATION_TABLE).unwrap();
			let stored_mode = configuration.get(STORAGE_MODE_KEY).unwrap().map(|value| {
				ResourceStorageMode::from_bytes(value.value()).unwrap_or_else(|| {
					panic!("Failed to open resource store. The most likely cause is an unknown persisted payload storage mode.")
				})
			});
			let storage_mode = match (stored_mode, requested_mode) {
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
			};
			let stored_compression = configuration
				.get(IMAGE_COMPRESSION_KEY)
				.unwrap()
				.map(|value| ResourceGpuCompressionPolicy::from_bytes(value.value()))
				.transpose()?
				.unwrap_or_default();
			let image_compression = match requested_compression {
				Some(requested) => {
					configuration.insert(IMAGE_COMPRESSION_KEY, requested.as_bytes()).unwrap();
					requested
				}
				None => stored_compression,
			};
			let settings = ResourceStorageSettings {
				storage_mode,
				image_compression,
			};
			settings.validate()?;
			settings
		};
		write.commit().unwrap();
		let packed_allocator = match settings.storage_mode {
			ResourceStorageMode::Files => None,
			ResourceStorageMode::Packed => Some(PackedResourceAllocator::open(
				base_path.join(PACKED_RESOURCES_FILE),
				read_packed_ranges(writable_db)?,
			)?),
		};

		Ok(ReDBStorageBackend {
			db,
			base_path,
			storage_mode: settings.storage_mode,
			image_compression: settings.image_compression,
			packed_allocator,
		})
	}

	fn begin_read(&self) -> Result<redb::ReadTransaction, redb::TransactionError> {
		match &self.db {
			RedbDatabase::Writable(db) => db.begin_read(),
			RedbDatabase::ReadOnly(db) => db.begin_read(),
		}
	}

	/// Creates and pre-sizes a unique staging file before returning it to a processor.
	async fn reserve_staged_file(&self, resource_id: ResourceId, size: usize) -> Result<ResourceWriter, ()> {
		static NEXT_STAGING_FILE: AtomicU64 = AtomicU64::new(0);

		if !matches!(&self.db, RedbDatabase::Writable(_)) {
			return Err(());
		}

		let file_size = u64::try_from(size).map_err(|_| ())?;
		loop {
			let sequence = NEXT_STAGING_FILE.fetch_add(1, Ordering::Relaxed);
			let path = self.base_path.join(format!(
				"{STAGED_RESOURCE_FILE_PREFIX}-{}-{}-{sequence}.tmp",
				resource_key_hex(resource_id.0),
				std::process::id()
			));
			let file = match compio::fs::OpenOptions::new()
				.create_new(true)
				.read(true)
				.write(true)
				.open(&path)
				.await
			{
				Ok(file) => file,
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(_) => return Err(()),
			};

			// Pre-sizing tells the OS the final extent before the processor starts
			// writing, so writes inside the reservation never grow the file.
			if file.set_len(file_size).await.is_err() {
				let _ = file.close().await;
				let _ = compio::fs::remove_file(path).await;
				return Err(());
			}

			return Ok(ResourceWriter::staged_file(file, StagedResourceFile::new(path), size));
		}
	}

	/// Opens one transaction handle and reserves its packed range through the backend slot allocator.
	async fn reserve_packed_file(&self, size: usize) -> Result<(ResourceWriter, ResourceReservation), ()> {
		if !matches!(&self.db, RedbDatabase::Writable(_)) {
			return Err(());
		}

		let packed_allocator = self.packed_allocator.as_ref().ok_or(())?;
		let file = compio::fs::OpenOptions::new()
			.truncate(false)
			.write(true)
			.open(self.base_path.join(PACKED_RESOURCES_FILE))
			.await
			.map_err(|_| ())?;
		let range = packed_allocator
			.reserve(u64::try_from(size).map_err(|_| ())?)
			.map_err(|error| {
				log::error!("{error}");
			})?;

		Ok((
			ResourceWriter::reserved_file(file, range.offset, size),
			ResourceReservation {
				offset: range.offset,
				size: range.size,
			},
		))
	}

	/// Reads store-wide payload settings before any resource locations are interpreted.
	fn persisted_settings(&self) -> Result<ResourceStorageSettings, String> {
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
		let storage_mode = ResourceStorageMode::from_bytes(mode.value())
			.ok_or_else(|| "resource payload mode is not recognized".to_string())?;
		let image_compression = table
			.get(IMAGE_COMPRESSION_KEY)
			.map_err(|error| format!("resource image compression could not be read: {error}"))?
			.map(|value| ResourceGpuCompressionPolicy::from_bytes(value.value()))
			.transpose()?
			.unwrap_or_default();
		let settings = ResourceStorageSettings {
			storage_mode,
			image_compression,
		};
		settings.validate()?;
		Ok(settings)
	}

	/// Opens one reader whose byte zero is the start of the requested resource.
	async fn open_reader(
		&self,
		id: [u8; 16],
		resource: &SerializableResource,
		packed_offset: Option<u64>,
		packed_lease: Option<std::sync::Arc<()>>,
	) -> Option<MultiResourceReader> {
		let stored_size = u64::try_from(resource.stored_size()).ok()?;
		let (file, offset) = match self.storage_mode {
			ResourceStorageMode::Files => {
				let path = resource_payload_path(&self.base_path, id, resource.hash(), resource.encoding());
				let file = AsyncFile::open(&path).await.ok()?;
				if file.metadata().await.ok()?.len() != stored_size {
					log::error!(
						"Resource payload size does not match its metadata. The most likely cause is a corrupt or incomplete resource file."
					);
					return None;
				}
				if resource.encoding().is_gpu_backed() {
					return Some(Box::new(FileResourceReader::new_gpu(path, resource.encoding())));
				}
				(file, 0)
			}
			ResourceStorageMode::Packed => {
				let file = AsyncFile::open(self.base_path.join(PACKED_RESOURCES_FILE)).await.ok()?;
				(file, packed_offset?)
			}
		};
		let file_size = file.metadata().await.ok()?.len();
		let reader = FileResourceReader::new_stored_range(
			&file,
			file_size,
			offset,
			stored_size,
			resource.size(),
			resource.encoding(),
			packed_lease,
		)
		.ok()?;
		Some(Box::new(reader))
	}

	/// Reads one metadata snapshot and leases its packed range before a writer can retire it.
	fn read_location(&self, id: ResourceId) -> Option<(SerializableResource, Option<u64>, Option<std::sync::Arc<()>>)> {
		if let Some(allocator) = &self.packed_allocator {
			let mut state = allocator.lock_state();
			let (resource, packed_offset) = self.read_record(id)?;
			let range = PackedRange::new(packed_offset?, u64::try_from(resource.stored_size()).ok()?);
			let lease = state.lease(range).ok()?;
			return Some((resource, packed_offset, lease));
		}

		let (resource, packed_offset) = self.read_record(id)?;
		Some((resource, packed_offset, None))
	}

	/// Reads resource metadata and its optional packed offset from one redb snapshot.
	fn read_record(&self, id: ResourceId) -> Option<(SerializableResource, Option<u64>)> {
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
		Some((resource, packed_offset))
	}

	/// Selects the one persisted encoding after CPU preparation and backend policy have both run.
	fn payload_encoding_for(&self, resource: &ProcessedAsset, encoding: ResourcePayloadEncoding) -> ResourcePayloadEncoding {
		if self.image_compression == ResourceGpuCompressionPolicy::Disabled || encoding != ResourcePayloadEncoding::Raw {
			return encoding;
		}
		if is_direct_texture(resource.class(), resource.serialized_metadata()) {
			ResourcePayloadEncoding::MetalIoLz4
		} else {
			ResourcePayloadEncoding::Raw
		}
	}

	/// Opens one resource reader after securing any packed range that backs it.
	async fn read_resource(&self, id: ResourceId) -> Option<(SerializableResource, MultiResourceReader)> {
		let (resource, packed_offset, packed_lease) = self.read_location(id)?;
		let resource_reader = self.open_reader(id.0, &resource, packed_offset, packed_lease).await?;

		Some((resource, resource_reader))
	}

	pub fn read_uid(&self, id: ResourceId) -> BoxedFuture<'_, Option<(SerializableResource, MultiResourceReader)>> {
		r#async::future(self.read_resource(id))
	}

	/// Removes one resource from the live store and returns its standalone payload path when one exists.
	fn remove_resource_record(&self, id: asset::ResourceId<'_>) -> Result<Option<std::path::PathBuf>, String> {
		// Packed metadata changes and allocator transitions share this lock so a
		// reader cannot lease a slot between its removal and retirement.
		let packed_allocator = self.packed_allocator.as_ref();
		let mut allocator_state = packed_allocator.map(|allocator| allocator.lock_state());
		let write = match &self.db {
			RedbDatabase::Writable(db) => db
				.begin_write()
				.map_err(|_| "Failed to begin delete transaction".to_string())?,
			RedbDatabase::ReadOnly(_) => {
				return Err("Cannot delete from a read-only resources database".to_string());
			}
		};
		let id = ResourceId::from(id.as_ref());
		let mut deleted_hash = None;
		let mut deleted_encoding = ResourcePayloadEncoding::Raw;
		let mut deleted_range = None;

		{
			let mut resources_table = write.open_table(RESOURCES_TABLE).unwrap();
			let mut class_table = write.open_table(RESOURCE_CLASS_INDEX_TABLE).unwrap();
			let mut property_table = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).unwrap();
			let mut packed_offsets = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE).unwrap();
			#[cfg(debug_assertions)]
			let mut traces_table = write.open_table(RESOURCE_TRACES_TABLE).unwrap();

			if let Some(existing) = resources_table.get(&id).unwrap() {
				let resource: SerializableResource = crate::from_slice(existing.value()).unwrap();
				deleted_hash = Some(resource.hash());
				deleted_encoding = resource.encoding();
				if self.storage_mode == ResourceStorageMode::Packed {
					let offset = packed_offsets
						.get(&id)
						.unwrap()
						.expect("stored packed resource should have an offset")
						.value();
					deleted_range = Some(PackedRange::new(
						offset,
						u64::try_from(resource.stored_size()).expect("stored resource size should fit in u64"),
					));
				}
				remove_indexes(&mut class_table, &mut property_table, &resource, id.0);
			}
			let _ = resources_table.remove(&id);
			let _ = packed_offsets.remove(&id);
			#[cfg(debug_assertions)]
			let _ = traces_table.remove(&id);
		}

		write.commit().map_err(|_| "Failed to commit transaction".to_string())?;
		if let (Some(state), Some(deleted_range)) = (allocator_state.as_mut(), deleted_range) {
			state.retire(deleted_range);
		}
		drop(allocator_state);
		if let Some(allocator) = packed_allocator {
			allocator.warn_if_fragmented();
		}

		Ok(if self.storage_mode == ResourceStorageMode::Files {
			deleted_hash.map(|hash| resource_payload_path(&self.base_path, id.0, hash, deleted_encoding))
		} else {
			None
		})
	}

	/// Clears backend tables in one transaction and retires every live packed range.
	fn clear_metadata(&self) -> Result<(), String> {
		let packed_allocator = self.packed_allocator.as_ref();
		let mut allocator_state = packed_allocator.map(|allocator| allocator.lock_state());
		let write = match &self.db {
			RedbDatabase::Writable(db) => db
				.begin_write()
				.map_err(|error| resource_clear_error("begin the resource clear transaction", error))?,
			RedbDatabase::ReadOnly(_) => {
				return Err(
					"Cannot clear a read-only resources database. The most likely cause is that the backend was opened with `ReDBStorageBackend::open_read_only`."
						.to_string(),
				);
			}
		};
		let mut retired_ranges = Vec::new();

		{
			let mut resources_table = write
				.open_table(RESOURCES_TABLE)
				.map_err(|error| resource_clear_error("open the resources table", error))?;
			let mut class_table = write
				.open_table(RESOURCE_CLASS_INDEX_TABLE)
				.map_err(|error| resource_clear_error("open the resource class index", error))?;
			let mut property_table = write
				.open_table(RESOURCE_PROPERTY_INDEX_TABLE)
				.map_err(|error| resource_clear_error("open the resource property index", error))?;
			let mut packed_offsets = write
				.open_table(PACKED_RESOURCE_OFFSETS_TABLE)
				.map_err(|error| resource_clear_error("open the packed resource offsets", error))?;
			#[cfg(debug_assertions)]
			let mut traces_table = write
				.open_table(RESOURCE_TRACES_TABLE)
				.map_err(|error| resource_clear_error("open the resource traces table", error))?;

			if self.storage_mode == ResourceStorageMode::Packed {
				for entry in resources_table
					.iter()
					.map_err(|error| resource_clear_error("iterate the resources table", error))?
				{
					let (key, value) = entry.map_err(|error| resource_clear_error("read resource metadata", error))?;
					let id = ResourceId(key.value());
					let resource: SerializableResource = crate::from_slice(value.value()).map_err(|_| {
						"Failed to clear resource metadata. The most likely cause is a corrupt resources database.".to_string()
					})?;
					let offset = packed_offsets
						.get(&id)
						.map_err(|error| resource_clear_error("read a packed resource offset", error))?
						.ok_or_else(|| {
							"Failed to clear packed resources. The most likely cause is incomplete packed resource metadata."
								.to_string()
						})?
						.value();
					retired_ranges.push(PackedRange::new(
						offset,
						u64::try_from(resource.stored_size()).map_err(|_| {
							"Failed to clear packed resources. The most likely cause is an unsupported resource size."
								.to_string()
						})?,
					));
				}
			}

			resources_table
				.retain(|_, _| false)
				.map_err(|error| resource_clear_error("clear the resources table", error))?;
			class_table
				.retain(|_, _| false)
				.map_err(|error| resource_clear_error("clear the resource class index", error))?;
			property_table
				.retain(|_, _| false)
				.map_err(|error| resource_clear_error("clear the resource property index", error))?;
			packed_offsets
				.retain(|_, _| false)
				.map_err(|error| resource_clear_error("clear the packed resource offsets", error))?;
			#[cfg(debug_assertions)]
			traces_table
				.retain(|_, _| false)
				.map_err(|error| resource_clear_error("clear the resource traces table", error))?;
		}

		write
			.commit()
			.map_err(|error| resource_clear_error("commit the resource clear transaction", error))?;
		if let Some(state) = allocator_state.as_mut() {
			for range in retired_ranges {
				state.retire(range);
			}
		}
		drop(allocator_state);
		if let Some(allocator) = packed_allocator {
			allocator.warn_if_fragmented();
		}

		Ok(())
	}

	/// Finds matching resource IDs through the narrowest available index.
	fn query_index(&self, query: &Query) -> Result<QueryPage<[u8; 16]>, QueryError> {
		let cursor = query.cursor.as_ref().map(|cursor| cursor.token.as_slice());
		let read = self.begin_read().map_err(|_| QueryError::StorageFailure)?;
		let resources_table = read.open_table(RESOURCES_TABLE).map_err(|_| QueryError::StorageFailure)?;
		let (index_definition, prefix) = if let Some((property, value)) = query.first_indexed_predicate() {
			let value = extract_string(value).ok_or(QueryError::StorageFailure)?;
			(
				RESOURCE_PROPERTY_INDEX_TABLE,
				property_index_key(&query.class, property, value, [0; 16]),
			)
		} else {
			(RESOURCE_CLASS_INDEX_TABLE, class_index_key(&query.class, [0; 16]))
		};
		let prefix = &prefix[..prefix.len() - 32];
		let index_table = read.open_table(index_definition).map_err(|_| QueryError::StorageFailure)?;

		let mut items = Vec::new();
		let mut last_key = None;
		let mut has_more = false;

		for entry in index_table.iter().map_err(|_| QueryError::StorageFailure)? {
			let entry = entry.map_err(|_| QueryError::StorageFailure)?;
			let key = entry.0.value();
			if !key.starts_with(prefix) {
				continue;
			}

			if let Some(cursor) = cursor
				&& key <= cursor
			{
				continue;
			}

			let resource_key = entry.1.value();
			let serialized = resources_table.get(resource_key).map_err(|_| QueryError::StorageFailure)?;
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

			items.push(resource_key);
			last_key = Some(key.to_vec());
		}

		Ok(QueryPage {
			items,
			cursor: if has_more { last_key.map(QueryCursor::new) } else { None },
		})
	}
}

impl ReadStorageBackend for ReDBStorageBackend {
	fn list(&self) -> impl std::future::Future<Output = Result<Vec<String>, String>> {
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

	fn read<'a>(
		&'a self,
		id: asset::ResourceId<'a>,
	) -> impl std::future::Future<Output = Option<(SerializableResource, MultiResourceReader)>> + 'a {
		self.read_resource(ResourceId::from(id.as_ref()))
	}

	fn query(
		&self,
		query: Query,
	) -> impl std::future::Future<Output = Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>> {
		r#async::future(async move {
			if query.limit == 0 {
				return Ok(QueryPage {
					items: Vec::new(),
					cursor: None,
				});
			}

			if let Some(cursor) = &query.cursor
				&& cursor.token.is_empty()
			{
				return Err(QueryError::InvalidCursor);
			}

			let page = self.query_index(&query)?;
			let mut items = Vec::with_capacity(page.items.len());
			for resource_key in page.items {
				items.push(
					self.read_resource(ResourceId(resource_key))
						.await
						.ok_or(QueryError::StorageFailure)?,
				);
			}

			Ok(QueryPage {
				items,
				cursor: page.cursor,
			})
		})
	}

	#[cfg(debug_assertions)]
	fn read_trace<'a>(
		&'a self,
		id: asset::ResourceId<'a>,
	) -> impl std::future::Future<Output = Result<Vec<ResourceTraceItem>, String>> + 'a {
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

impl WriteStorageBackend for ReDBStorageBackend {
	async fn clear(&self) -> Result<(), String> {
		let payload_paths = if self.storage_mode == ResourceStorageMode::Files {
			discover_resource_payload_paths(self.base_path.clone()).await?
		} else {
			Vec::new()
		};

		self.clear_metadata()?;

		for payload_path in payload_paths {
			match compio::fs::remove_file(&payload_path).await {
				Ok(()) => {}
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(error) => {
					return Err(format!(
						"Failed to clear resource payload '{}'. The most likely cause is that the resource directory is not writable. Error: {error}",
						payload_path.display()
					));
				}
			}
		}

		Ok(())
	}

	fn delete<'a>(&'a self, id: asset::ResourceId<'a>) -> Result<(), String> {
		if let Some(payload_path) = self.remove_resource_record(id)? {
			let _ = remove_file(payload_path);
		}

		Ok(())
	}

	fn cpu_compression_policy(&self, resource: &ProcessedAsset) -> ResourceCompressionPolicy {
		if self.image_compression != ResourceGpuCompressionPolicy::Disabled
			&& is_direct_texture(resource.class(), resource.serialized_metadata())
		{
			ResourceCompressionPolicy::Disabled
		} else {
			resource.compression_policy()
		}
	}

	fn begin_resource<'a>(
		&'a self,
		id: asset::ResourceId<'_>,
		size: usize,
	) -> impl Future<Output = Result<ResourceTransaction<'a>, ()>> + 'a {
		let resource_id = ResourceId::from(id.as_ref());
		async move {
			let (writer, reservation) = match self.storage_mode {
				ResourceStorageMode::Files => (self.reserve_staged_file(resource_id, size).await?, None),
				ResourceStorageMode::Packed => {
					let (writer, reservation) = self.reserve_packed_file(size).await?;
					(writer, Some(reservation))
				}
			};

			Ok(ResourceTransaction::new(self, resource_id, reservation, writer))
		}
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
}

impl ResourceTransactionCommit for ReDBStorageBackend {
	fn abort_resource(&self, reservation: ResourceReservation) {
		if let Some(allocator) = &self.packed_allocator {
			allocator.abort(PackedRange::new(reservation.offset, reservation.size));
		}
	}

	/// Publishes payload location and metadata only after the writer satisfies its exact-size reservation.
	fn commit_resource(
		&self,
		resource_id: ResourceId,
		backend_reservation: Option<ResourceReservation>,
		resource: ProcessedAsset,
		output: ResourceWriteOutput,
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()> {
		let hash = output.hash();
		let size = output.size();
		let prepared_stored_size = output.stored_size();
		let encoding = self.payload_encoding_for(&resource, output.encoding());
		let packed_range = backend_reservation.map(|reservation| PackedRange::new(reservation.offset, reservation.size));

		let stored_size = match self.storage_mode {
			ResourceStorageMode::Files => {
				if packed_range.is_some() {
					return Err(());
				}
				let destination = resource_payload_path(&self.base_path, resource_id.0, hash, encoding);
				match encoding {
					ResourcePayloadEncoding::MetalIoLz4 => output.persist_compressed_file(&destination, encoding)?,
					ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => {
						output.persist_staged_file(&destination)?;
						prepared_stored_size
					}
				}
			}
			ResourceStorageMode::Packed => {
				if packed_range.is_none() || encoding.is_gpu_backed() {
					return Err(());
				}
				output.finish_reserved_file()?;
				prepared_stored_size
			}
		};
		let resource = resource.into_serializable(hash, size, stored_size, encoding);
		let serialized_resource = crate::to_vec_in(&resource, allocator).map_err(|_| ())?;
		let packed_allocator = self.packed_allocator.as_ref();
		let mut allocator_state = packed_allocator.map(|allocator| allocator.lock_state());

		let write = match &self.db {
			RedbDatabase::Writable(db) => db.begin_write().map_err(|_| ())?,
			RedbDatabase::ReadOnly(_) => return Err(()),
		};

		let mut replaced_range = None;
		{
			let mut resources_table = write.open_table(RESOURCES_TABLE).map_err(|_| ())?;
			let mut class_table = write.open_table(RESOURCE_CLASS_INDEX_TABLE).map_err(|_| ())?;
			let mut property_table = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE).map_err(|_| ())?;
			let mut packed_offsets = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE).map_err(|_| ())?;

			if let Some(existing) = resources_table.get(&resource_id).map_err(|_| ())? {
				let existing: SerializableResource = crate::from_slice(existing.value()).map_err(|_| ())?;
				if self.storage_mode == ResourceStorageMode::Packed {
					let offset = packed_offsets.get(&resource_id).map_err(|_| ())?.ok_or(())?.value();
					replaced_range = Some(PackedRange::new(
						offset,
						u64::try_from(existing.stored_size()).map_err(|_| ())?,
					));
				}
				remove_indexes(&mut class_table, &mut property_table, &existing, resource_id.0);
			}

			resources_table
				.insert(&resource_id, serialized_resource.as_slice())
				.map_err(|_| ())?;
			if let Some(range) = packed_range {
				packed_offsets.insert(&resource_id, range.offset).map_err(|_| ())?;
			} else {
				let _ = packed_offsets.remove(&resource_id);
			}
			insert_indexes(&mut class_table, &mut property_table, &resource, resource_id.0);
		}

		write.commit().map_err(|_| ())?;
		if let (Some(state), Some(packed_range)) = (allocator_state.as_mut(), packed_range) {
			state.publish(packed_range, replaced_range);
		}
		drop(allocator_state);
		if let Some(allocator) = packed_allocator {
			allocator.warn_if_fragmented();
		}

		// File mode retains replaced content-addressed extents so a reader that
		// observed old metadata can still open its payload.
		Ok(resource)
	}
}

impl StorageBackend for ReDBStorageBackend {}

/// Selects how one resource store persists binary payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceStorageMode {
	/// Stores each resource payload in its own resource-and-content-hash-named file.
	#[default]
	Files,
	/// Stores every resource payload in reusable ranges of one shared file.
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

/// Selects whether a file-mode store prepares ordinary textures for native Metal I/O.
///
/// This policy selects the final [`ResourcePayloadEncoding`]; it is not stored as
/// separate per-resource compression state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceGpuCompressionPolicy {
	#[default]
	Disabled,
	MetalIoLz4,
}

impl ResourceGpuCompressionPolicy {
	fn as_bytes(self) -> &'static [u8] {
		match self {
			Self::Disabled => b"none",
			Self::MetalIoLz4 => b"metal-io-lz4",
		}
	}

	fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
		match bytes {
			b"none" => Ok(Self::Disabled),
			b"metal-io-lz4" => Ok(Self::MetalIoLz4),
			_ => Err("resource image compression is not recognized".to_string()),
		}
	}
}

/// The `ResourceStorageSettings` struct selects persistent payload layout and native texture transport compression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceStorageSettings {
	storage_mode: ResourceStorageMode,
	image_compression: ResourceGpuCompressionPolicy,
}

impl ResourceStorageSettings {
	/// Creates storage settings with native GPU compression disabled.
	pub fn new(storage_mode: ResourceStorageMode) -> Self {
		Self {
			storage_mode,
			image_compression: ResourceGpuCompressionPolicy::Disabled,
		}
	}

	/// Selects the native GPU compression policy for compatible textures.
	pub fn image_compression(mut self, compression: ResourceGpuCompressionPolicy) -> Self {
		self.image_compression = compression;
		self
	}

	fn validate(self) -> Result<(), String> {
		if self.storage_mode == ResourceStorageMode::Packed && self.image_compression != ResourceGpuCompressionPolicy::Disabled
		{
			return Err(
				"Compressed image storage requires file mode. The most likely cause is that native texture compression was combined with the current raw packed layout."
					.to_string(),
			);
		}
		#[cfg(not(all(target_os = "macos", feature = "gpu-processing")))]
		if self.image_compression == ResourceGpuCompressionPolicy::MetalIoLz4 {
			return Err(
				"Metal I/O LZ4 image compression is unavailable. The most likely cause is that BELD is not running on macOS with GPU processing enabled."
					.to_string(),
			);
		}
		Ok(())
	}
}

/// Reconstructs every live packed extent so allocator state needs no separate persistent journal.
fn read_packed_ranges(db: &redb::Database) -> Result<Vec<PackedRange>, String> {
	let read = db
		.begin_read()
		.map_err(|error| packed_database_error("Packed resource metadata could not be read", error))?;
	let resources = read
		.open_table(RESOURCES_TABLE)
		.map_err(|error| packed_database_error("Packed resource metadata could not be opened", error))?;
	let offsets = read
		.open_table(PACKED_RESOURCE_OFFSETS_TABLE)
		.map_err(|error| packed_database_error("Packed resource offsets could not be opened", error))?;
	let mut ranges = Vec::new();

	for entry in resources
		.iter()
		.map_err(|error| packed_database_error("Packed resource metadata could not be scanned", error))?
	{
		let entry = entry.map_err(|error| packed_database_error("Packed resource metadata could not be scanned", error))?;
		let key = entry.0.value();
		let resource: SerializableResource = crate::from_slice(entry.1.value()).map_err(|_| {
			format!(
				"Packed resource metadata is invalid for '{}'. The most likely cause is a corrupt or incompatible resource database.",
				resource_key_hex(key)
			)
		})?;
		let offset = offsets
			.get(&ResourceId(key))
			.map_err(|error| {
				format!(
					"Packed resource offset could not be read for '{}': {error}. The most likely cause is a corrupt resource database.",
					resource.id()
				)
			})?
			.ok_or_else(|| {
				format!(
					"Packed resource offset is missing for '{}'. The most likely cause is an incomplete resource database.",
					resource.id()
				)
			})?
			.value();
		let size = u64::try_from(resource.stored_size()).map_err(|_| {
			format!(
				"Packed resource size is invalid for '{}'. The most likely cause is incompatible resource metadata.",
				resource.id()
			)
		})?;
		ranges.push(PackedRange::new(offset, size));
	}

	Ok(ranges)
}

fn packed_database_error(action: &str, error: impl std::fmt::Display) -> String {
	format!("{action}: {error}. The most likely cause is an incomplete or corrupt resource database.")
}

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

/// Creates a consistent storage error for one REDB clear step.
fn resource_clear_error(action: &str, error: impl std::fmt::Display) -> String {
	format!(
		"Failed to {action}. The most likely cause is that the resources database is corrupt or cannot be updated. Error: {error}"
	)
}

fn resource_payload_path(base_path: &Path, key: [u8; 16], hash: u64, encoding: ResourcePayloadEncoding) -> std::path::PathBuf {
	base_path.join(format!("{}-{hash:016x}-{}", resource_key_hex(key), encoding.as_str()))
}

/// Enumerates backend-owned payload files on a blocking worker because Compio does not provide directory iteration.
async fn discover_resource_payload_paths(base_path: std::path::PathBuf) -> Result<Vec<std::path::PathBuf>, String> {
	match r#async::offload(move || resource_payload_paths_in(&base_path)).await {
		Ok(result) => result,
		Err(error) => {
			error.resume_unwind();
			Err(
				"Resource payload discovery stopped. The most likely cause is that its blocking worker was cancelled."
					.to_string(),
			)
		}
	}
}

/// Finds files that match the backend's content-addressed payload naming scheme.
fn resource_payload_paths_in(base_path: &Path) -> Result<Vec<std::path::PathBuf>, String> {
	let entries = std::fs::read_dir(base_path).map_err(|error| {
		format!(
			"Failed to scan resource payloads in '{}'. The most likely cause is that the resource directory cannot be read. Error: {error}",
			base_path.display()
		)
	})?;
	let mut payload_paths = Vec::new();

	for entry in entries {
		let entry = entry.map_err(|error| {
			format!(
				"Failed to read a resource directory entry. The most likely cause is that '{}' changed while it was being cleared. Error: {error}",
				base_path.display()
			)
		})?;
		let file_type = entry.file_type().map_err(|error| {
			format!(
				"Failed to inspect resource directory entry '{}'. The most likely cause is that it changed while the store was being cleared. Error: {error}",
				entry.path().display()
			)
		})?;
		let file_name = entry.file_name();
		let Some(name) = file_name.to_str() else {
			continue;
		};

		if file_type.is_file() && is_resource_payload_name(name) {
			payload_paths.push(entry.path());
		}
	}

	Ok(payload_paths)
}

/// Returns whether a file name is one of this backend's content-addressed payload extents.
fn is_resource_payload_name(name: &str) -> bool {
	let Some((resource_id, remainder)) = name.split_once('-') else {
		return false;
	};
	let Some((hash, encoding)) = remainder.split_once('-') else {
		return false;
	};

	ResourceId::from_uid_hex(resource_id).is_some()
		&& hash.len() == 16
		&& hash.bytes().all(|byte| byte.is_ascii_hexdigit())
		&& matches!(encoding, "raw" | "cpu-lz4" | "metal-io-lz4")
}

/// Selects ordinary textures while environment images retain their existing multi-image CPU upload path.
fn is_direct_texture(class: &str, metadata: &[u8]) -> bool {
	class == "Image" && crate::from_slice::<crate::resources::image::Image>(metadata).is_ok_and(|image| image.ibl.is_none())
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
const IMAGE_COMPRESSION_KEY: &str = "image-compression";
const PACKED_RESOURCES_FILE: &str = "resources.pack";
const STAGED_RESOURCE_FILE_PREFIX: &str = ".resource-write";
const BAKING_APP_RESOURCES_DOCS_PATH: &str = "develop/resource-management/baking-app-resources";

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::{
		PACKED_RESOURCES_FILE, RESOURCE_MANAGEMENT_CODE_HASH, RESOURCE_MANAGEMENT_SIGNATURE_FILE, ReDBStorageBackend,
		ResourceStorageMode, ResourceStorageSettings, STAGED_RESOURCE_FILE_PREFIX, resource_payload_path,
		validate_resource_management_signature,
	};
	use crate::{
		Model, ProcessedAsset,
		resource::storage_backend::{Query, QueryCursor, QueryError, ReadStorageBackend, WriteStorageBackend},
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

	fn backend_with_mode(storage_mode: ResourceStorageMode) -> ReDBStorageBackend {
		static NEXT_BACKEND_ID: AtomicUsize = AtomicUsize::new(0);

		let unique = format!(
			"byte-engine-redb-tests-{}-{}",
			std::process::id(),
			NEXT_BACKEND_ID.fetch_add(1, Ordering::Relaxed)
		);
		ReDBStorageBackend::new_writable_with_mode(std::env::temp_dir().join(unique), storage_mode).unwrap()
	}

	fn backend() -> ReDBStorageBackend {
		backend_with_mode(ResourceStorageMode::Files)
	}

	#[crate::r#async::test]
	async fn clear_removes_all_resources_and_keeps_each_storage_mode_reusable() {
		for storage_mode in [ResourceStorageMode::Files, ResourceStorageMode::Packed] {
			let backend = backend_with_mode(storage_mode);
			let first_id = crate::asset::ResourceId::new("materials/first");
			let first = backend
				.store(
					ProcessedAsset::new(
						first_id,
						MockMaterialModel {
							group: "opaque".into(),
							tag: "first".into(),
						},
					),
					b"first-payload",
				)
				.await
				.unwrap();
			let first_payload_path = resource_payload_path(
				&backend.base_path,
				super::ResourceId::from(first_id.as_ref()).0,
				first.hash(),
				first.encoding(),
			);
			let stale_payload_path = resource_payload_path(
				&backend.base_path,
				super::ResourceId::from("stale.test").0,
				0x1234,
				crate::resource::ResourcePayloadEncoding::Raw,
			);
			if storage_mode == ResourceStorageMode::Files {
				std::fs::write(&stale_payload_path, b"stale").unwrap();
				std::fs::write(backend.base_path.join("keep-me"), b"unrelated").unwrap();
			}
			store_mock(
				&backend,
				"materials/second",
				MockMaterialModel {
					group: "transparent".into(),
					tag: "second".into(),
				},
			)
			.await;
			#[cfg(debug_assertions)]
			backend
				.replace_trace(
					crate::asset::ResourceId::new("failed.test"),
					&[ResourceTraceItem::new(
						ResourceTraceLevel::Error,
						"Expected failure.".to_string(),
					)],
				)
				.unwrap();

			assert_eq!(backend.list().await.unwrap().len(), 2);
			backend.clear().await.unwrap();

			assert!(backend.list().await.unwrap().is_empty());
			assert!(backend.read(first_id).await.is_none());
			assert!(backend.query(Query::new("MockMaterial")).await.unwrap().items.is_empty());
			if storage_mode == ResourceStorageMode::Files {
				assert!(!first_payload_path.exists());
				assert!(!stale_payload_path.exists());
				assert_eq!(std::fs::read(backend.base_path.join("keep-me")).unwrap(), b"unrelated");
			}
			#[cfg(debug_assertions)]
			assert!(
				backend
					.read_trace(crate::asset::ResourceId::new("failed.test"))
					.await
					.unwrap()
					.is_empty()
			);

			store_mock(
				&backend,
				"materials/rebuilt",
				MockMaterialModel {
					group: "opaque".into(),
					tag: "rebuilt".into(),
				},
			)
			.await;
			assert_eq!(backend.list().await.unwrap(), ["materials/rebuilt"]);

			std::fs::remove_dir_all(&backend.base_path).unwrap();
		}
	}

	#[test]
	fn compressed_images_reject_the_raw_packed_layout() {
		let settings = ResourceStorageSettings::new(ResourceStorageMode::Packed)
			.image_compression(crate::resource::ResourceGpuCompressionPolicy::MetalIoLz4);

		assert!(settings.validate().is_err());
	}

	#[test]
	fn direct_texture_selection_excludes_environment_images() {
		use crate::{
			resources::image::{Image, ImageIBL, ImageSubresource},
			types::{Formats, Gamma},
		};

		let subresource = ImageSubresource {
			format: Formats::RGBA16F,
			gamma: Gamma::Linear,
			extent: [4, 4, 0],
			mip_count: 1,
			array_layers: 6,
		};
		let ordinary = Image {
			format: Formats::RGBA8,
			gamma: Gamma::Linear,
			extent: [4, 4, 0],
			mip_count: 1,
			ibl: None,
			photometry: None,
		};
		let environment = Image {
			ibl: Some(ImageIBL {
				diffuse_irradiance: subresource.clone(),
				prefiltered_specular: subresource,
			}),
			..ordinary.clone()
		};

		assert!(super::is_direct_texture("Image", &crate::to_vec(&ordinary).unwrap()));
		assert!(!super::is_direct_texture("Image", &crate::to_vec(&environment).unwrap()));
	}

	#[cfg(all(target_os = "macos", feature = "gpu-processing"))]
	#[crate::r#async::test]
	async fn metal_lz4_image_files_return_gpu_backing() {
		static NEXT_COMPRESSED_IMAGE_ID: AtomicUsize = AtomicUsize::new(0);

		use crate::{
			StreamDescription,
			resource::{ResourceGpuCompressionPolicy, ResourcePayloadEncoding, ResourceReaderBacking},
			resources::image::Image,
			types::{Formats, Gamma},
		};

		let path = std::env::temp_dir().join(format!(
			"byte-engine-compressed-image-tests-{}-{}",
			std::process::id(),
			NEXT_COMPRESSED_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
		));
		let backend = ReDBStorageBackend::new_writable_with_settings(
			path.clone(),
			ResourceStorageSettings::new(ResourceStorageMode::Files)
				.image_compression(ResourceGpuCompressionPolicy::MetalIoLz4),
		)
		.unwrap();
		let id = crate::asset::ResourceId::new("texture.image");
		let decoded = [7_u8; 32 * 32 * 4];
		let resource = ProcessedAsset::new(
			id,
			Image {
				format: Formats::RGBA8,
				gamma: Gamma::Linear,
				extent: [32, 32, 0],
				mip_count: 1,
				ibl: None,
				photometry: None,
			},
		)
		.with_streams(vec![StreamDescription::new("mip[0]", decoded.len(), 0)]);
		let stored = backend.store(resource, &decoded).await.unwrap();
		let (_, reader) = backend.read(id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		let ResourceReaderBacking::Gpu(backing) = backing else {
			panic!(
				"Compressed image did not return GPU backing. The most likely cause is that the ReDB reader mapped the native container as raw bytes."
			);
		};

		assert_eq!(backing.encoding(), ResourcePayloadEncoding::MetalIoLz4);
		assert!(backing.path().exists());
		assert_eq!(stored.size(), decoded.len());
		assert_eq!(
			stored.stored_size(),
			std::fs::metadata(backing.path()).unwrap().len() as usize
		);
		assert_eq!(stored.encoding(), ResourcePayloadEncoding::MetalIoLz4);
		assert_eq!(
			backing.path(),
			resource_payload_path(
				&path,
				crate::resource::ResourceId::from(id.as_ref()).0,
				stored.hash(),
				ResourcePayloadEncoding::MetalIoLz4,
			)
		);

		drop(backend);
		std::fs::remove_dir_all(path).unwrap();
	}

	#[crate::r#async::test]
	async fn file_transaction_presizes_staging_file_and_abort_removes_it() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("presized.test");
		let transaction = WriteStorageBackend::begin_resource(&backend, id, 4096).await.unwrap();
		let staging_files = std::fs::read_dir(&backend.base_path)
			.unwrap()
			.filter_map(Result::ok)
			.filter(|entry| entry.file_name().to_string_lossy().starts_with(STAGED_RESOURCE_FILE_PREFIX))
			.collect::<Vec<_>>();

		assert_eq!(transaction.expected_size(), 4096);
		assert_eq!(staging_files.len(), 1);
		assert_eq!(staging_files[0].metadata().unwrap().len(), 4096);

		drop(transaction);

		assert!(!staging_files[0].path().exists());
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn packed_transactions_reserve_early_and_commit_out_of_order() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let first_id = crate::asset::ResourceId::new("first-reserved.test");
		let second_id = crate::asset::ResourceId::new("second-reserved.test");
		let mut first = WriteStorageBackend::begin_resource(&backend, first_id, 5).await.unwrap();

		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			5
		);

		let mut second = WriteStorageBackend::begin_resource(&backend, second_id, 6).await.unwrap();

		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			11
		);
		assert_eq!(backend.packed_allocator.as_ref().unwrap().high_water(), 11);
		assert!(backend.read(first_id).await.is_none());
		assert!(backend.read(second_id).await.is_none());

		second.write_all(b"second").await.unwrap();
		second
			.commit(
				ProcessedAsset::new(second_id, MockShaderModel { stage: "second".into() }),
				&std::alloc::Global,
			)
			.await
			.unwrap();
		first.write_all(b"first").await.unwrap();
		first
			.commit(
				ProcessedAsset::new(first_id, MockShaderModel { stage: "first".into() }),
				&std::alloc::Global,
			)
			.await
			.unwrap();

		for (id, expected) in [(first_id, b"first".as_slice()), (second_id, b"second".as_slice())] {
			let (_, reader) = backend.read(id).await.unwrap();
			let data = reader.into_backing_storage().await.unwrap();
			assert_eq!(data.as_slice(), expected);
		}

		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn concurrent_packed_reservations_never_overlap() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let (first, second) = std::future::join!(
			WriteStorageBackend::begin_resource(&backend, crate::asset::ResourceId::new("concurrent-first.test"), 7),
			WriteStorageBackend::begin_resource(&backend, crate::asset::ResourceId::new("concurrent-second.test"), 13),
		)
		.await;
		let first = first.unwrap();
		let second = second.unwrap();

		assert_eq!(first.expected_size(), 7);
		assert_eq!(second.expected_size(), 13);
		assert_eq!(backend.packed_allocator.as_ref().unwrap().high_water(), 20);
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			20
		);

		drop((first, second));
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn owned_store_utility_persists_a_large_vec() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("owned-store.test");
		let expected = (0..(64 * 1024 + 17)).map(|index| index as u8).collect::<Vec<_>>();

		let stored_resource = WriteStorageBackend::store_owned(
			&backend,
			ProcessedAsset::new(id, MockShaderModel { stage: "owned".into() }),
			expected.clone(),
		)
		.await
		.unwrap();
		assert_eq!(stored_resource.encoding(), crate::resource::ResourcePayloadEncoding::CpuLz4);
		assert!(stored_resource.stored_size() < stored_resource.size());

		let (_, reader) = backend.read(id).await.unwrap();
		let stored = reader.into_backing_storage().await.unwrap();
		assert_eq!(stored.as_slice(), expected);
		drop(stored);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn resource_transaction_enforces_exact_size_before_publication() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("exact-size.test");
		let mut short = WriteStorageBackend::begin_resource(&backend, id, 4).await.unwrap();
		short.write_all(b"abc").await.unwrap();

		assert!(
			short
				.commit(
					ProcessedAsset::new(id, MockShaderModel { stage: "short".into() }),
					&std::alloc::Global,
				)
				.await
				.is_err()
		);
		assert!(backend.read(id).await.is_none());

		let mut long = WriteStorageBackend::begin_resource(&backend, id, 4).await.unwrap();

		assert!(long.write_all(b"abcde").await.is_err());
		assert_eq!(long.written_size(), 0);
		drop(long);
		assert!(backend.read(id).await.is_none());

		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn file_resource_hash_is_independent_of_writer_chunk_boundaries() {
		let backend = backend();
		let contiguous_id = crate::asset::ResourceId::new("contiguous.test");
		let chunked_id = crate::asset::ResourceId::new("chunked.test");
		let contiguous = backend
			.store(
				ProcessedAsset::new(contiguous_id, MockShaderModel { stage: "one".into() }),
				b"one logical payload",
			)
			.await
			.unwrap();
		let mut chunked = WriteStorageBackend::begin_resource(&backend, chunked_id, 19).await.unwrap();
		chunked.write_all(b"one ").await.unwrap();
		chunked.write_all(b"logical ").await.unwrap();
		chunked.write_all(b"payload").await.unwrap();
		let chunked = chunked
			.commit(
				ProcessedAsset::new(chunked_id, MockShaderModel { stage: "many".into() }),
				&std::alloc::Global,
			)
			.await
			.unwrap();

		assert_eq!(contiguous.hash(), chunked.hash());
		let (_, reader) = backend.read(chunked_id).await.unwrap();
		let data = reader.into_backing_storage().await.unwrap();
		assert_eq!(data.as_slice(), b"one logical payload");
		drop(data);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn file_transaction_flushes_buffered_borrowed_writes_through_compio() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("buffered-async.test");
		let mut expected = Vec::with_capacity(2 * 64 * 1024 + 37);
		expected.extend_from_slice(b"compio");
		expected.extend((0..(2 * 64 * 1024 + 31)).map(|index| index as u8));
		let mut transaction = WriteStorageBackend::begin_resource(&backend, id, expected.len())
			.await
			.unwrap();
		assert_eq!(transaction.staging_buffer_capacity(), 0);

		transaction.write_all(b"compio").await.unwrap();
		assert!(transaction.staging_buffer_capacity() >= 64 * 1024);
		for bytes in expected[6..].chunks(997) {
			transaction.write_all(bytes).await.unwrap();
		}
		assert_eq!(transaction.direct_write_count(), 0);
		transaction
			.commit(
				ProcessedAsset::new(
					id,
					MockShaderModel {
						stage: "buffered".into(),
					},
				),
				&std::alloc::Global,
			)
			.await
			.unwrap();

		let (_, reader) = backend.read(id).await.unwrap();
		let stored = reader.into_backing_storage().await.unwrap();
		assert_eq!(stored.as_slice(), expected);
		drop(stored);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn large_owned_first_write_never_allocates_the_staging_buffer() {
		let backend = backend();
		let id = crate::asset::ResourceId::new("lazy-buffer.test");
		let expected = (0..(64 * 1024 + 17)).map(|index| index as u8).collect::<Vec<_>>();
		let mut transaction = WriteStorageBackend::begin_resource(&backend, id, expected.len())
			.await
			.unwrap();

		assert_eq!(transaction.staging_buffer_capacity(), 0);
		let compio::buf::BufResult(result, expected) = compio::io::AsyncWriteExt::write_all(&mut transaction, expected).await;
		result.unwrap();
		assert_eq!(transaction.direct_write_count(), 1);
		assert_eq!(transaction.staging_buffer_capacity(), 0);
		transaction
			.commit(
				ProcessedAsset::new(id, MockShaderModel { stage: "lazy".into() }),
				&std::alloc::Global,
			)
			.await
			.unwrap();

		let (_, reader) = backend.read(id).await.unwrap();
		let stored = reader.into_backing_storage().await.unwrap();
		assert_eq!(stored.as_slice(), expected);
		drop(stored);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn large_owned_write_flushes_pending_bytes_and_bypasses_the_file_buffer() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("earlier.test"),
					MockShaderModel { stage: "earlier".into() },
				),
				b"earlier",
			)
			.await
			.unwrap();
		let id = crate::asset::ResourceId::new("direct-owned.test");
		let large = (0..(64 * 1024 + 1)).map(|index| index as u8).collect::<Vec<_>>();
		let mut expected = Vec::with_capacity(6 + large.len() + 4);
		expected.extend_from_slice(b"prefix");
		expected.extend_from_slice(&large);
		expected.extend_from_slice(b"tail");
		let mut transaction = WriteStorageBackend::begin_resource(&backend, id, expected.len())
			.await
			.unwrap();

		transaction.write_all(b"prefix").await.unwrap();
		assert_eq!(transaction.buffered_size(), 6);
		let compio::buf::BufResult(result, large) = compio::io::AsyncWriteExt::write_all(&mut transaction, large).await;
		result.unwrap();
		assert_eq!(large.len(), 64 * 1024 + 1);
		assert_eq!(transaction.direct_write_count(), 1);
		assert_eq!(transaction.buffered_size(), 0);
		transaction.write_all(b"tail").await.unwrap();
		assert_eq!(transaction.buffered_size(), 4);
		transaction
			.commit(
				ProcessedAsset::new(id, MockShaderModel { stage: "direct".into() }),
				&std::alloc::Global,
			)
			.await
			.unwrap();

		let (_, reader) = backend.read(id).await.unwrap();
		let stored = reader.into_backing_storage().await.unwrap();
		assert_eq!(stored.as_slice(), expected);
		drop(stored);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn packed_replacements_reuse_copy_on_write_slots() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let id = crate::asset::ResourceId::new("replacement.test");
		backend
			.store(ProcessedAsset::new(id, MockShaderModel { stage: "first".into() }), b"old!")
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			4
		);

		// The first replacement needs a separate slot because publication is
		// copy-on-write. Later equal-sized replacements alternate between slots.
		backend
			.store(ProcessedAsset::new(id, MockShaderModel { stage: "second".into() }), b"new!")
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			8
		);

		for (stage, payload) in [("third", b"next".as_slice()), ("fourth", b"last".as_slice())] {
			backend
				.store(ProcessedAsset::new(id, MockShaderModel { stage: stage.into() }), payload)
				.await
				.unwrap();
			assert_eq!(
				std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
					.unwrap()
					.len(),
				8
			);
		}

		let (_, reader) = backend.read(id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), b"last");
		drop(backing);

		backend.delete(id).unwrap();
		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("after-delete.test"),
					MockShaderModel { stage: "reused".into() },
				),
				b"free",
			)
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			8
		);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn packed_slots_wait_for_mapped_readers_before_reuse() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let id = crate::asset::ResourceId::new("mapped.test");
		backend
			.store(ProcessedAsset::new(id, MockShaderModel { stage: "old".into() }), b"old!")
			.await
			.unwrap();
		let (_, reader) = backend.read(id).await.unwrap();
		let old_backing = reader.into_backing_storage().await.unwrap();

		backend
			.store(ProcessedAsset::new(id, MockShaderModel { stage: "new".into() }), b"new!")
			.await
			.unwrap();
		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("while-mapped.test"),
					MockShaderModel { stage: "held".into() },
				),
				b"held",
			)
			.await
			.unwrap();

		assert_eq!(old_backing.as_slice(), b"old!");
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			12
		);

		drop(old_backing);
		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("after-map.test"),
					MockShaderModel { stage: "free".into() },
				),
				b"free",
			)
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			12
		);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn packed_query_readers_also_lease_their_slots() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let id = crate::asset::ResourceId::new("query-mapped.test");
		let material = || MockMaterialModel {
			group: "opaque".into(),
			tag: "held".into(),
		};
		backend.store(ProcessedAsset::new(id, material()), b"old!").await.unwrap();
		let page = backend
			.query(Query::new("MockMaterial").eq("name", "query-mapped.test"))
			.await
			.unwrap();
		let (_, reader) = page.items.into_iter().next().unwrap();
		let old_backing = reader.into_backing_storage().await.unwrap();

		backend.store(ProcessedAsset::new(id, material()), b"new!").await.unwrap();
		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("query-held.test"),
					MockShaderModel { stage: "held".into() },
				),
				b"held",
			)
			.await
			.unwrap();

		assert_eq!(old_backing.as_slice(), b"old!");
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			12
		);
		drop(old_backing);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn abandoned_packed_reservations_are_reused() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let transaction = WriteStorageBackend::begin_resource(&backend, crate::asset::ResourceId::new("abandoned.test"), 7)
			.await
			.unwrap();
		drop(transaction);

		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("reused.test"),
					MockShaderModel { stage: "reused".into() },
				),
				b"reused!",
			)
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			7
		);
		std::fs::remove_dir_all(&backend.base_path).unwrap();
	}

	#[crate::r#async::test]
	async fn failed_packed_transactions_return_their_reservations() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		let failed_id = crate::asset::ResourceId::new("failed.test");
		let mut transaction = WriteStorageBackend::begin_resource(&backend, failed_id, 4).await.unwrap();
		transaction.write_all(b"bad").await.unwrap();
		assert!(
			transaction
				.commit(
					ProcessedAsset::new(failed_id, MockShaderModel { stage: "failed".into() }),
					&std::alloc::Global,
				)
				.await
				.is_err()
		);

		backend
			.store(
				ProcessedAsset::new(
					crate::asset::ResourceId::new("after-failure.test"),
					MockShaderModel { stage: "reused".into() },
				),
				b"good",
			)
			.await
			.unwrap();
		assert_eq!(
			std::fs::metadata(backend.base_path.join(PACKED_RESOURCES_FILE))
				.unwrap()
				.len(),
			4
		);
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

	async fn store_mock<T: Model>(backend: &ReDBStorageBackend, id: &str, resource: T) {
		let asset = ProcessedAsset::new(crate::asset::ResourceId::new(id), resource);
		backend.store(asset, id.as_bytes()).await.unwrap();
	}

	async fn query_ids(backend: &ReDBStorageBackend, query: Query) -> (Vec<String>, Option<super::QueryCursor>) {
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
		)
		.await;
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		)
		.await;
		store_mock(
			&backend,
			"materials/c",
			MockMaterialModel {
				group: "transparent".into(),
				tag: "hero".into(),
			},
		)
		.await;

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
		)
		.await;
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		)
		.await;

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
		)
		.await;
		store_mock(
			&backend,
			"materials/b",
			MockMaterialModel {
				group: "opaque".into(),
				tag: "prop".into(),
			},
		)
		.await;
		store_mock(
			&backend,
			"materials/c",
			MockMaterialModel {
				group: "transparent".into(),
				tag: "hero".into(),
			},
		)
		.await;

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
		)
		.await;
		store_mock(
			&backend,
			"shaders/shared",
			MockShaderModel {
				stage: "fragment".into(),
			},
		)
		.await;

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
		)
		.await;

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
		)
		.await;
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
		)
		.await;

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
	future::Future,
	path::Path,
	sync::atomic::{AtomicU64, Ordering},
};

use redb::{ReadableDatabase as _, ReadableTable};
use utils::sync::remove_file;

use self::packed_allocator::{PackedRange, PackedResourceAllocator};
use super::transaction::ResourceReservation;
use super::{
	Query, QueryCursor, QueryError, QueryPage, ReadStorageBackend, ResourceTransaction, ResourceTransactionCommit,
	ResourceWriteOutput, ResourceWriter, StagedResourceFile, StorageBackend, WriteStorageBackend,
};
#[cfg(debug_assertions)]
use crate::ResourceTraceItem;
use crate::{
	ProcessedAsset, QueryableProperty, QueryableValue, SerializableResource, asset,
	r#async::{self, BoxedFuture, File as AsyncFile},
	resource::{
		ResourceCompressionPolicy, ResourceId, ResourcePayloadEncoding, reader::redb::FileResourceReader,
		resource_handler::MultiResourceReader,
	},
};
