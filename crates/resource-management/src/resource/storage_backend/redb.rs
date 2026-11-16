//! Store resource metadata in Redb and binary payloads in companion files.
//!
//! The backend hashes resource URLs into [`ResourceId`](crate::asset::ResourceId)
//! database keys and archives [`SerializableResource`]
//! metadata with rkyv.

mod packed_allocator;

pub use packed_allocator::PackedFragmentation;

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
	/// Whether the resources table was empty when this backend opened the store.
	opened_empty: bool,
	/// Suffix for the next staging file name. Name clashes with other processes or
	/// backends sharing the directory are resolved by retrying with the next value.
	next_staging_file: AtomicU64,
	/// Payload files published since the last durable commit. [`Self::sync_payloads`] writes them to the disk right
	/// before that commit, so a bake syncs its payloads once per flush instead of once per resource.
	unsynced_payloads: Mutex<Vec<std::path::PathBuf>>,
}

/// The `RedbDatabase` enum lets applications read compatible stores without writes while tools and cache recovery can update them.
enum RedbDatabase {
	Writable(redb::Database),
	ReadOnly(redb::ReadOnlyDatabase),
}

/// The `ResourceStoreOpenError` enum tells a caller why a resources directory did not open, so a tool or application
/// can report a locked, damaged, or incompatible store instead of baking into or reading from nothing.
///
/// Every variant names the resources directory. No open repairs or deletes a store it rejects; the one exception is
/// the documented reset of a store whose resource-management signature is missing or changed, which happens before
/// the database is read. Its [`Display`](std::fmt::Display) output is the complete message to show the user.
#[derive(Debug)]
pub enum ResourceStoreOpenError {
	/// Another backend, in this process or another one, holds the store's database lock.
	InUse { path: std::path::PathBuf },
	/// `resources.db` or `resources.pack` holds data that is not a valid resource store. `detail` describes the
	/// damage.
	Corrupt { path: std::path::PathBuf, detail: String },
	/// The file system refused an operation the open needs. `action` completes "could not ...".
	Io {
		path: std::path::PathBuf,
		action: &'static str,
		source: std::io::Error,
	},
	/// The store, or the requested settings, cannot be used by this engine build. `reason` is a complete message
	/// with the cause and the fix, such as a payload storage mode that differs from the store's.
	Incompatible { path: std::path::PathBuf, reason: String },
}

impl std::fmt::Display for ResourceStoreOpenError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::InUse { path } => write!(
				f,
				"Resource store '{}' is already open. The most likely cause is that a running application, another BELD command, or another backend in this process holds its database. Close the other program or drop the other backend, then open the store again. See {}.",
				path.display(),
				crate::online_docs_url(RESOURCE_STORE_RECOVERY_DOCS_PATH)
			),
			Self::Corrupt { path, detail } => write!(
				f,
				"Resource store '{}' is corrupt or unreadable, and was left unchanged. The most likely cause is that a crash, an interrupted bake, a disk error, or another program damaged its files. Delete '{}' and bake again. See {}. Error: {detail}",
				path.display(),
				path.display(),
				crate::online_docs_url(RESOURCE_STORE_RECOVERY_DOCS_PATH)
			),
			Self::Io { path, action, source } => write!(
				f,
				"Resource store '{}' did not open because the engine could not {action}. The most likely cause is that this process cannot access the directory or its files, or the disk is full. Check the permissions and free space of '{}', then open the store again. Error: {source}",
				path.display(),
				path.display()
			),
			Self::Incompatible { path, reason } => write!(f, "Resource store '{}' cannot be opened. {reason}", path.display()),
		}
	}
}

impl std::error::Error for ResourceStoreOpenError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Io { source, .. } => Some(source),
			Self::InUse { .. } | Self::Corrupt { .. } | Self::Incompatible { .. } => None,
		}
	}
}

impl ReDBStorageBackend {
	/// Opens an application resource store and synchronizes its resource-management signature.
	///
	/// Debug applications can update their resource cache. Release applications keep a compatible
	/// store read-only, but discard a mismatched store before reading stale values. Use
	/// [`Self::open_read_only`] when a tool must inspect a store without changing it. Use
	/// [`Self::new_writable_with_mode`] when a producer must select a payload storage mode.
	///
	/// # Errors
	///
	/// Returns a [`ResourceStoreOpenError`] when the store is in use, corrupt, inaccessible, or incompatible. The
	/// backend never substitutes a temporary store; use [`MemoryStorageBackend`](super::MemoryStorageBackend) when
	/// resources must not persist.
	pub fn new(base_path: std::path::PathBuf) -> Result<Self, ResourceStoreOpenError> {
		if cfg!(debug_assertions) || validate_resource_management_signature(&base_path).is_err() {
			Self::new_writable(base_path)
		} else {
			Self::open_read_only(base_path)
		}
	}

	/// Opens a compatible resource database without modifying its directory.
	///
	/// Read-only tools use this constructor so a signature mismatch remains available for the user to inspect or replace.
	/// After handling an incompatibility, use [`Self::new_writable`] only when the user has requested a resource update.
	///
	/// # Errors
	///
	/// Returns [`ResourceStoreOpenError::Incompatible`] when another engine revision baked the store, and the other
	/// [`ResourceStoreOpenError`] variants when the store is in use, corrupt, or inaccessible.
	pub fn open_read_only(base_path: std::path::PathBuf) -> Result<Self, ResourceStoreOpenError> {
		validate_resource_management_signature(&base_path).map_err(|reason| ResourceStoreOpenError::Incompatible {
			path: base_path.clone(),
			reason,
		})?;
		let db = redb::ReadOnlyDatabase::open(base_path.join(RESOURCES_DATABASE_FILE))
			.map(RedbDatabase::ReadOnly)
			.map_err(|error| store_open_error(&base_path, "open 'resources.db'", error))?;
		let mut backend = Self {
			db,
			base_path,
			storage_mode: ResourceStorageMode::Files,
			image_compression: ResourceGpuCompressionPolicy::Disabled,
			packed_allocator: None,
			opened_empty: false,
			next_staging_file: AtomicU64::new(0),
			unsynced_payloads: Mutex::new(Vec::new()),
		};
		let settings = backend.persisted_settings()?;
		backend.storage_mode = settings.storage_mode;
		backend.image_compression = settings.image_compression;
		Ok(backend)
	}

	/// Opens a writable resource store and preserves its persisted payload mode.
	///
	/// New stores use [`ResourceStorageMode::Files`]. Use [`Self::new_writable_with_mode`] to select another mode.
	///
	/// # Errors
	///
	/// Returns a [`ResourceStoreOpenError`] when the store is in use, corrupt, or inaccessible. Every error leaves
	/// `resources.db` as it was.
	pub fn new_writable(base_path: std::path::PathBuf) -> Result<Self, ResourceStoreOpenError> {
		Self::open_writable(base_path, None, None)
	}

	/// Opens a writable resource store with the selected payload mode.
	///
	/// Existing stores must already use `storage_mode`. This prevents a bake from mixing incompatible payload layouts.
	///
	/// # Errors
	///
	/// Returns [`ResourceStoreOpenError::Incompatible`] when an existing store uses another payload mode, and the
	/// errors of [`Self::new_writable`].
	pub fn new_writable_with_mode(
		base_path: std::path::PathBuf,
		storage_mode: ResourceStorageMode,
	) -> Result<Self, ResourceStoreOpenError> {
		Self::open_writable(base_path, Some(storage_mode), None)
	}

	/// Opens a writable resource store with explicit payload and image-compression settings.
	///
	/// # Errors
	///
	/// Returns [`ResourceStoreOpenError::Incompatible`] when this build cannot store `settings` or an existing store
	/// uses another payload mode, and the errors of [`Self::new_writable`].
	pub fn new_writable_with_settings(
		base_path: std::path::PathBuf,
		settings: ResourceStorageSettings,
	) -> Result<Self, ResourceStoreOpenError> {
		settings.validate().map_err(|reason| ResourceStoreOpenError::Incompatible {
			path: base_path.clone(),
			reason,
		})?;
		Self::open_writable(base_path, Some(settings.storage_mode), Some(settings.image_compression))
	}

	/// Opens the database and establishes one payload mode before callers can store resources.
	///
	/// Every failure is returned. Substituting a temporary database would let a bake report success and lose its output.
	fn open_writable(
		base_path: std::path::PathBuf,
		requested_mode: Option<ResourceStorageMode>,
		requested_compression: Option<ResourceGpuCompressionPolicy>,
	) -> Result<Self, ResourceStoreOpenError> {
		sync_resource_management_signature(&base_path)?;
		let writable_db = redb::Database::create(base_path.join(RESOURCES_DATABASE_FILE))
			.map_err(|error| store_open_error(&base_path, "open 'resources.db'", error))?;

		let write = writable_db
			.begin_write()
			.map_err(|error| store_open_error(&base_path, "start a write to 'resources.db'", error))?;
		let opened_empty = write
			.open_table(RESOURCES_TABLE)
			.is_ok_and(|resources| resources.is_empty().unwrap_or(false));
		let _ = write.open_table(RESOURCE_CLASS_INDEX_TABLE);
		let _ = write.open_table(RESOURCE_PROPERTY_INDEX_TABLE);
		let _ = write.open_table(PACKED_RESOURCE_OFFSETS_TABLE);
		for table in RECORD_TABLES {
			let _ = write.open_table(table);
		}
		let settings = {
			let mut configuration = write
				.open_table(STORE_CONFIGURATION_TABLE)
				.map_err(|error| store_open_error(&base_path, "read the store configuration", error))?;
			let stored_mode = configuration
				.get(STORAGE_MODE_KEY)
				.map_err(|error| store_open_error(&base_path, "read the store configuration", error))?
				.map(|value| {
					ResourceStorageMode::from_bytes(value.value()).ok_or_else(|| ResourceStoreOpenError::Corrupt {
						path: base_path.clone(),
						detail: "the persisted payload storage mode is not recognized".to_string(),
					})
				})
				.transpose()?;
			let storage_mode = match (stored_mode, requested_mode) {
				(Some(stored), Some(requested)) if stored != requested => {
					return Err(ResourceStoreOpenError::Incompatible {
						path: base_path,
						reason: format!(
							"The store already uses '{}' payload storage, but '{}' was requested. The most likely cause is that the bake selects a different `--storage-mode` than the bake that created the store. Bake with `--storage-mode {}`, or delete the resources directory and bake again. See {}.",
							stored.as_str(),
							requested.as_str(),
							stored.as_str(),
							crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
						),
					});
				}
				(Some(stored), _) => stored,
				(None, requested) => {
					let selected = requested.unwrap_or_default();
					configuration
						.insert(STORAGE_MODE_KEY, selected.as_bytes())
						.map_err(|error| store_open_error(&base_path, "write the store configuration", error))?;
					selected
				}
			};
			let stored_compression = configuration
				.get(IMAGE_COMPRESSION_KEY)
				.map_err(|error| store_open_error(&base_path, "read the store configuration", error))?
				.map(|value| ResourceGpuCompressionPolicy::from_bytes(value.value()))
				.transpose()
				.map_err(|detail| ResourceStoreOpenError::Corrupt {
					path: base_path.clone(),
					detail,
				})?
				.unwrap_or_default();
			let image_compression = match requested_compression {
				Some(requested) => {
					configuration
						.insert(IMAGE_COMPRESSION_KEY, requested.as_bytes())
						.map_err(|error| store_open_error(&base_path, "write the store configuration", error))?;
					requested
				}
				None => stored_compression,
			};
			let settings = ResourceStorageSettings {
				storage_mode,
				image_compression,
			};
			// Returning drops the write uncommitted, so a rejected open leaves the store configuration unchanged.
			settings.validate().map_err(|reason| ResourceStoreOpenError::Incompatible {
				path: base_path.clone(),
				reason,
			})?;
			settings
		};
		write
			.commit()
			.map_err(|error| store_open_error(&base_path, "commit the store configuration", error))?;
		let packed_allocator = match settings.storage_mode {
			ResourceStorageMode::Files => None,
			ResourceStorageMode::Packed => {
				let read = writable_db
					.begin_read()
					.map_err(|error| store_open_error(&base_path, "read 'resources.db'", error))?;
				let live_ranges = read_packed_ranges(&read).map_err(|detail| ResourceStoreOpenError::Corrupt {
					path: base_path.clone(),
					detail,
				})?;
				Some(PackedResourceAllocator::open(&base_path, live_ranges)?)
			}
		};

		Ok(ReDBStorageBackend {
			db: RedbDatabase::Writable(writable_db),
			base_path,
			storage_mode: settings.storage_mode,
			image_compression: settings.image_compression,
			packed_allocator,
			opened_empty,
			next_staging_file: AtomicU64::new(0),
			unsynced_payloads: Mutex::new(Vec::new()),
		})
	}

	fn begin_read(&self) -> Result<redb::ReadTransaction, redb::TransactionError> {
		match &self.db {
			RedbDatabase::Writable(db) => db.begin_read(),
			RedbDatabase::ReadOnly(db) => db.begin_read(),
		}
	}

	/// Writes every payload published since the last durable commit to the disk.
	///
	/// Call it right before each durable commit. That commit flushes the drive's cache, which makes these writes
	/// permanent first, so no durable record can name a payload that a crash would lose.
	async fn sync_payloads(&self) -> Result<(), String> {
		use utils::r#async::stream::{self, TryStreamExt as _};

		let mut paths = std::mem::take(&mut *self.unsynced_payloads.lock().unwrap());
		// Packed payloads all live in the pack, which one sync covers. Syncing an unchanged pack costs almost nothing.
		if self.storage_mode == ResourceStorageMode::Packed {
			paths.push(self.base_path.join(PACKED_RESOURCES_FILE));
		}

		let synced = stream::iter(paths.iter().map(Ok))
			.try_for_each_concurrent(PAYLOAD_SYNCS_IN_FLIGHT, |path| async move {
				sync_payload(path).await.map_err(|error| (path, error))
			})
			.await;
		if let Err((path, error)) = synced {
			let message = format!(
				"Failed to sync resource payload '{}'. The most likely cause is that the disk is full or unavailable. Free disk space or check the resource directory, then bake again. Error: {error}",
				path.display()
			);
			// Keep every payload so the next durable commit syncs them again. A file that did sync syncs again almost
			// for free.
			self.unsynced_payloads.lock().unwrap().extend(paths);
			return Err(message);
		}
		Ok(())
	}

	/// Creates and pre-sizes a unique staging file before returning it to a processor.
	async fn reserve_staged_file(&self, resource_id: ResourceId, size: usize) -> Result<ResourceWriter, ()> {
		if !matches!(&self.db, RedbDatabase::Writable(_)) {
			return Err(());
		}

		let file_size = u64::try_from(size).map_err(|_| ())?;
		loop {
			let sequence = self.next_staging_file.fetch_add(1, Ordering::Relaxed);
			let path = self.base_path.join(format!(
				"{STAGED_RESOURCE_FILE_PREFIX}-{}-{}-{sequence}.tmp",
				resource_id,
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
	fn persisted_settings(&self) -> Result<ResourceStorageSettings, ResourceStoreOpenError> {
		let corrupt = |detail: String| ResourceStoreOpenError::Corrupt {
			path: self.base_path.clone(),
			detail,
		};
		let read = self
			.begin_read()
			.map_err(|error| store_open_error(&self.base_path, "read 'resources.db'", error))?;
		let table = read
			.open_table(STORE_CONFIGURATION_TABLE)
			.map_err(|error| store_open_error(&self.base_path, "read the store configuration", error))?;
		let mode = table
			.get(STORAGE_MODE_KEY)
			.map_err(|error| store_open_error(&self.base_path, "read the store configuration", error))?
			.ok_or_else(|| corrupt("the persisted payload storage mode is missing".to_string()))?;
		let storage_mode = ResourceStorageMode::from_bytes(mode.value())
			.ok_or_else(|| corrupt("the persisted payload storage mode is not recognized".to_string()))?;
		let image_compression = table
			.get(IMAGE_COMPRESSION_KEY)
			.map_err(|error| store_open_error(&self.base_path, "read the store configuration", error))?
			.map(|value| ResourceGpuCompressionPolicy::from_bytes(value.value()))
			.transpose()
			.map_err(corrupt)?
			.unwrap_or_default();
		let settings = ResourceStorageSettings {
			storage_mode,
			image_compression,
		};
		settings.validate().map_err(|reason| ResourceStoreOpenError::Incompatible {
			path: self.base_path.clone(),
			reason,
		})?;
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
		// Materials, variants, and other metadata-only resources have no payload, so no file is opened for them.
		if stored_size == 0 && !resource.encoding().is_gpu_backed() {
			return Some(Box::new(StoredResourceReader::new(
				ResourceReaderBacking::Buffer(Box::new([])),
				resource.encoding(),
				0,
			)));
		}
		let (file, file_size, offset) = match self.storage_mode {
			ResourceStorageMode::Files => {
				let path = resource_payload_path(&self.base_path, id, resource.hash(), resource.encoding());
				let file = AsyncFile::open(&path).await.ok()?;
				let file_size = file.metadata().await.ok()?.len();
				if file_size != stored_size {
					log::error!(
						"Resource payload size does not match its metadata. The most likely cause is a corrupt or incomplete resource file."
					);
					return None;
				}
				if resource.encoding().is_gpu_backed() {
					return Some(Box::new(StoredResourceReader::gpu(path, resource.encoding())));
				}
				(file, file_size, 0)
			}
			ResourceStorageMode::Packed => {
				let file = AsyncFile::open(self.base_path.join(PACKED_RESOURCES_FILE)).await.ok()?;
				let file_size = file.metadata().await.ok()?.len();
				(file, file_size, packed_offset?)
			}
		};
		let reader = StoredResourceReader::mapped(
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

	/// Measures the files of the store, so tools can report the space that no resource uses.
	///
	/// Compare [`StorageUsage::payload_files`] with the UID, hash, and encoding of each resource from
	/// [`ReadStorageBackend::list`] to find payload files that no resource records. This reads the directory and,
	/// in packed mode, every resource's metadata, so call it where blocking is acceptable.
	pub fn usage(&self) -> Result<StorageUsage, String> {
		let usage_error = |action: &str, error: &dyn std::fmt::Display| {
			format!(
				"Failed to measure the resource store: could not {action}. The most likely cause is that '{}' can't be read. Error: {error}",
				self.base_path.display()
			)
		};
		let mut usage = StorageUsage {
			database_size: std::fs::metadata(self.base_path.join(RESOURCES_DATABASE_FILE)).map_or(0, |metadata| metadata.len()),
			payload_files: Vec::new(),
			staging_files: (0, 0),
			pack: None,
		};

		for entry in std::fs::read_dir(&self.base_path).map_err(|error| usage_error("list the directory", &error))? {
			let entry = entry.map_err(|error| usage_error("read a directory entry", &error))?;
			let metadata = entry.metadata().map_err(|error| usage_error("read a file's size", &error))?;
			if !metadata.is_file() {
				continue;
			}
			let file_name = entry.file_name();
			let Some(name) = file_name.to_str() else {
				continue;
			};

			if let Some((uid, hash, encoding)) = parse_payload_name(name) {
				usage.payload_files.push(PayloadFile {
					uid,
					hash,
					encoding,
					size: metadata.len(),
				});
			} else if name.starts_with(STAGED_RESOURCE_FILE_PREFIX) {
				usage.staging_files.0 += 1;
				usage.staging_files.1 += metadata.len();
			}
		}

		if self.storage_mode == ResourceStorageMode::Packed {
			// The allocator's own reconstruction yields the free ranges, so the report matches what a bake would reuse.
			let pack_path = self.base_path.join(PACKED_RESOURCES_FILE);
			let pack_size = std::fs::metadata(&pack_path).map_or(0, |metadata| metadata.len());
			let read = self
				.begin_read()
				.map_err(|error| usage_error("read the resources database", &error))?;
			let state = packed_allocator::reconstruct_state(&pack_path, pack_size, read_packed_ranges(&read)?)?;
			usage.pack = Some(state.fragmentation());
		}

		Ok(usage)
	}

	/// Returns every recorded bake trace with its resource ID, including traces of bakes that stored no resource.
	///
	/// Debug builds record traces; a store baked by a release build has none. Call
	/// [`ReadStorageBackend::read_trace`] for one ID.
	#[cfg(debug_assertions)]
	pub fn traces(&self) -> Result<Vec<(String, Vec<ResourceTraceItem>)>, String> {
		self.records(RESOURCE_TRACES_TABLE, "traces", crate::from_slice)
	}

	/// Returns the IDs each stored resource's model names, by resource ID, for tools that follow them.
	///
	/// Debug builds record references; a store baked by a release build has none. A resource that names no other
	/// resource isn't listed.
	#[cfg(debug_assertions)]
	pub fn references(&self) -> Result<Vec<(String, Vec<String>)>, String> {
		self.records(RESOURCE_REFERENCES_TABLE, "references", crate::from_slice)
	}

	/// Returns the provenance of every resource ID that has one, by ID, for tools that explain freshness.
	///
	/// Next, pass a record to [`stale_sources`](crate::asset::manager::stale_sources) to find the sources that changed.
	pub fn provenances(&self) -> Result<Vec<(String, Provenance)>, String> {
		self.records(RESOURCE_PROVENANCE_TABLE, "provenance", crate::from_slice)
	}

	/// Decodes every value of a per-resource record table. A missing table has none.
	fn records<T>(
		&self,
		table: redb::TableDefinition<[u8; 16], &[u8]>,
		name: &str,
		decode: impl Fn(&[u8]) -> Result<T, crate::ResourceArchiveError>,
	) -> Result<Vec<T>, String> {
		let record_error = |error: &dyn std::fmt::Display| {
			format!(
				"Failed to read resource {name}. The most likely cause is a corrupt resources database. Delete the resources directory and bake again. Error: {error}"
			)
		};
		let read = self.begin_read().map_err(|error| record_error(&error))?;
		let table = match read.open_table(table) {
			Ok(table) => table,
			Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
			Err(error) => return Err(record_error(&error)),
		};

		table
			.iter()
			.map_err(|error| record_error(&error))?
			.map(|entry| {
				let (_, value) = entry.map_err(|error| record_error(&error))?;
				decode(value.value()).map_err(|error| record_error(&error))
			})
			.collect()
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
			for table in RECORD_TABLES {
				let _ = write.open_table(table).unwrap().remove(&id);
			}
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
			for table in RECORD_TABLES {
				use redb::TableHandle as _;

				write
					.open_table(table)
					.map_err(|error| resource_clear_error(&format!("open the {} table", table.name()), error))?
					.retain(|_, _| false)
					.map_err(|error| resource_clear_error(&format!("clear the {} table", table.name()), error))?;
			}
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
	fn list(&self) -> impl std::future::Future<Output = Result<Vec<SerializableResource>, String>> {
		r#async::future(async {
			let read = self.begin_read().map_err(|error| resource_list_error(&error))?;
			let table = read
				.open_table(RESOURCES_TABLE)
				.map_err(|error| resource_list_error(&error))?;
			let entries = table.iter().map_err(|error| resource_list_error(&error))?;
			let mut resources = Vec::with_capacity(entries.size_hint().0);

			for entry in entries {
				let (_, metadata) = entry.map_err(|error| resource_list_error(&error))?;
				resources.push(crate::from_slice(metadata.value()).map_err(|error| resource_list_error(&error))?);
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
			if page.items.is_empty() && query.cursor.is_none() {
				if self.opened_empty {
					log::warn!(
						"Query for {} resources matched nothing because the resource store was empty when opened. The most likely cause is a development run on a fresh store, which bakes resources only on first request; later runs will find them.",
						query.class
					);
				} else {
					log::info!("Query for {} resources matched nothing.", query.class);
				}
			}
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

	fn read_provenance(&self, id: asset::ResourceId<'_>) -> Result<Option<Provenance>, String> {
		let read = self.begin_read().map_err(|error| provenance_error("read", &error))?;
		let table = match read.open_table(RESOURCE_PROVENANCE_TABLE) {
			Ok(table) => table,
			// A store written before any bake recorded provenance has no table yet.
			Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
			Err(error) => return Err(provenance_error("read", &error)),
		};
		let Some(record) = table
			.get(&ResourceId::from(id.as_ref()))
			.map_err(|error| provenance_error("read", &error))?
		else {
			return Ok(None);
		};

		crate::from_slice::<(String, Provenance)>(record.value())
			.map(|(_, provenance)| Some(provenance))
			.map_err(|error| provenance_error("read", &error))
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
			let Some(trace) = table.get(&id).map_err(|_| "Failed to read resource trace".to_string())? else {
				return Ok(Vec::new());
			};

			crate::from_slice::<(String, Vec<ResourceTraceItem>)>(trace.value())
				.map(|(_, items)| items)
				.map_err(|_| "Failed to deserialize resource trace".to_string())
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

	async fn delete<'a>(&'a self, id: asset::ResourceId<'a>) -> Result<(), String> {
		// The delete commits durably, which also makes every deferred commit before it durable.
		self.sync_payloads().await?;
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

	fn settings_fingerprint(&self) -> u64 {
		use std::hash::Hasher as _;

		// Texture compression decides the encoding of every direct texture's payload. The storage mode only decides
		// where a payload lives, and a store can't change it.
		let mut hasher = utils::hash::FxHasher::with_seed(961961961961961);
		hasher.write(self.image_compression.as_bytes());
		hasher.finish()
	}

	fn write_provenance(&self, id: asset::ResourceId<'_>, provenance: &Provenance) -> Result<(), String> {
		let mut write = match &self.db {
			RedbDatabase::Writable(db) => db.begin_write().map_err(|error| provenance_error("write", &error))?,
			RedbDatabase::ReadOnly(_) => {
				return Err(
					"Cannot write provenance to a read-only resources database. The most likely cause is that the backend was opened with `ReDBStorageBackend::open_read_only`. Open it with `ReDBStorageBackend::new_writable`."
						.to_string(),
				);
			}
		};
		// Provenance is made durable with the resources of the same bake by `persist`.
		write
			.set_durability(redb::Durability::None)
			.map_err(|error| provenance_error("write", &error))?;
		let key = ResourceId::from(id.as_ref());
		{
			let mut table = write
				.open_table(RESOURCE_PROVENANCE_TABLE)
				.map_err(|error| provenance_error("write", &error))?;
			if provenance.is_empty() {
				table.remove(&key).map_err(|error| provenance_error("write", &error))?;
			} else {
				// The ID is kept with the record because a bake that didn't finish has no resource record to name it.
				let serialized =
					crate::to_vec(&(id.to_string(), provenance.clone())).map_err(|error| provenance_error("write", &error))?;
				table
					.insert(&key, serialized.as_slice())
					.map_err(|error| provenance_error("write", &error))?;
			}
		}

		write.commit().map_err(|error| provenance_error("write", &error))
	}

	#[cfg(debug_assertions)]
	fn replace_trace(&self, id: asset::ResourceId<'_>, items: &[ResourceTraceItem]) -> Result<(), String> {
		let mut write = match &self.db {
			RedbDatabase::Writable(db) => db
				.begin_write()
				.map_err(|_| "Failed to begin resource trace write".to_string())?,
			RedbDatabase::ReadOnly(_) => {
				return Err("Cannot write traces to a read-only resources database".to_string());
			}
		};
		// Traces are made durable with the resources of the same bake by `persist`.
		write
			.set_durability(redb::Durability::None)
			.map_err(|_| "Failed to defer resource trace durability".to_string())?;
		let key = ResourceId::from(id.as_ref());
		{
			let mut table = write
				.open_table(RESOURCE_TRACES_TABLE)
				.map_err(|_| "Failed to open resource traces table".to_string())?;
			if items.is_empty() {
				table.remove(&key).map_err(|_| "Failed to clear resource trace".to_string())?;
			} else {
				// The ID is kept with the trace because a failed bake has no resource record to name it.
				let serialized = crate::to_vec(&(id.to_string(), items.to_vec()))
					.map_err(|_| "Failed to serialize resource trace".to_string())?;
				table
					.insert(&key, serialized.as_slice())
					.map_err(|_| "Failed to store resource trace".to_string())?;
			}
		}

		write.commit().map_err(|_| "Failed to commit resource trace".to_string())
	}

	/// Flushes every deferred commit to disk with one durable commit.
	///
	/// The payloads of those commits are synced first, so the flush never publishes a record whose payload could be
	/// missing after a crash.
	async fn persist(&self) -> Result<(), String> {
		let RedbDatabase::Writable(db) = &self.db else {
			return Ok(());
		};
		self.sync_payloads().await?;
		let write = db.begin_write().map_err(|error| {
			format!("Failed to persist baked resources. The resource database could not begin a write. Error: {error}")
		})?;
		write.commit().map_err(|error| {
			format!(
				"Failed to persist baked resources. The most likely cause is that the disk is full or unavailable. Error: {error}"
			)
		})
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
				let stored_size = match encoding {
					ResourcePayloadEncoding::MetalIoLz4 => output.persist_compressed_file(&destination, encoding)?,
					ResourcePayloadEncoding::Raw | ResourcePayloadEncoding::CpuLz4 => {
						output.persist_staged_file(&destination)?;
						prepared_stored_size
					}
				};
				self.unsynced_payloads.lock().unwrap().push(destination);
				stored_size
			}
			ResourceStorageMode::Packed => {
				if packed_range.is_none() || encoding.is_gpu_backed() {
					return Err(());
				}
				output.finish_reserved_file()?;
				prepared_stored_size
			}
		};
		// The references are written in the same transaction as the record, so the two never disagree.
		#[cfg(debug_assertions)]
		let references = (!resource.references().is_empty())
			.then(|| crate::to_vec(&(resource.id().to_string(), resource.references().to_vec())))
			.transpose()
			.map_err(|_| ())?;
		let resource = resource.into_serializable(hash, size, stored_size, encoding);
		let serialized_resource = crate::to_vec_in(&resource, allocator).map_err(|_| ())?;
		let packed_allocator = self.packed_allocator.as_ref();
		let mut allocator_state = packed_allocator.map(|allocator| allocator.lock_state());

		let mut write = match &self.db {
			RedbDatabase::Writable(db) => db.begin_write().map_err(|_| ())?,
			RedbDatabase::ReadOnly(_) => return Err(()),
		};
		// A durable commit per resource costs a full disk flush each; `persist` makes a whole bake durable at once.
		// The record stays consistent after a crash either way, because redb rolls back to the last durable commit.
		write.set_durability(redb::Durability::None).map_err(|_| ())?;

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

			#[cfg(debug_assertions)]
			{
				let mut references_table = write.open_table(RESOURCE_REFERENCES_TABLE).map_err(|_| ())?;
				match &references {
					Some(references) => {
						references_table.insert(&resource_id, references.as_slice()).map_err(|_| ())?;
					}
					None => {
						references_table.remove(&resource_id).map_err(|_| ())?;
					}
				}
			}
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

/// The `StorageUsage` struct describes the disk space a resource store occupies, so tools can find space that no
/// resource uses.
///
/// Read it with [`ReDBStorageBackend::usage`]. The default is the usage of an empty files-mode store.
#[derive(Clone, Debug, Default)]
pub struct StorageUsage {
	/// The size of the resources database in bytes.
	pub database_size: u64,
	/// Every payload file in the directory. A files-mode store writes one per payload.
	pub payload_files: Vec<PayloadFile>,
	/// The number and total size of staging files that interrupted writes left behind.
	pub staging_files: (usize, u64),
	/// The pack's size and free space in packed mode, or `None` in files mode.
	pub pack: Option<PackedFragmentation>,
}

/// The `PayloadFile` struct names one payload file by what it stores, so a tool can tell whether any resource still
/// records it.
///
/// A resource records the file when its UID, [`SerializableResource::hash`], and
/// [`SerializableResource::encoding`] all match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadFile {
	/// The UID of the resource the file was written for.
	pub uid: ResourceId,
	/// The hash of the decoded payload.
	pub hash: u64,
	/// The payload encoding.
	pub encoding: ResourcePayloadEncoding,
	/// The file size in bytes.
	pub size: u64,
}

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
	/// Returns the persisted name, which is also the BELD `--storage-mode` value.
	fn as_str(self) -> &'static str {
		match self {
			Self::Files => "files",
			Self::Packed => "packed",
		}
	}

	fn as_bytes(self) -> &'static [u8] {
		self.as_str().as_bytes()
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
	/// Returns the GPU image compression this build can store and load: Metal I/O LZ4 on macOS with GPU processing,
	/// and none elsewhere. [`ResourceStorageSettings`] rejects any other choice the build can't serve.
	pub fn native() -> Self {
		if cfg!(all(target_os = "macos", feature = "gpu-processing")) {
			Self::MetalIoLz4
		} else {
			Self::Disabled
		}
	}

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
			_ => Err("the persisted image compression is not recognized".to_string()),
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
fn read_packed_ranges(read: &redb::ReadTransaction) -> Result<Vec<PackedRange>, String> {
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
				ResourceId(key)
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
///
/// The error is a complete message for [`ResourceStoreOpenError::Incompatible`].
fn validate_resource_management_signature(base_path: &Path) -> Result<(), String> {
	match read_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE) {
		Some(signature) if signature == RESOURCE_MANAGEMENT_CODE_HASH => Ok(()),
		Some(signature) => Err(format!(
			"Its resource-management signature '{signature}' does not match this engine's signature '{RESOURCE_MANAGEMENT_CODE_HASH}'. The most likely cause is that a different engine revision baked it. Bake the store again with this revision's BELD. See {}.",
			crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
		)),
		None => Err(format!(
			"Its resource-management signature marker '{RESOURCE_MANAGEMENT_SIGNATURE_FILE}' is missing. The most likely cause is that the directory is not a baked resource store, or its bake did not finish. Bake the store with BELD. See {}.",
			crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
		)),
	}
}

/// Writes one cache-owner signature beside the resource database.
fn write_resource_cache_signature(
	base_path: &Path,
	signature_file: &str,
	signature: &str,
) -> Result<(), ResourceStoreOpenError> {
	std::fs::write(base_path.join(signature_file), signature).map_err(|source| ResourceStoreOpenError::Io {
		path: base_path.to_path_buf(),
		action: "write the resource-management signature",
		source,
	})
}

/// Removes every persisted value after a cache owner reports an incompatible signature.
fn reset_resource_cache(base_path: &Path) -> Result<(), ResourceStoreOpenError> {
	std::fs::remove_dir_all(base_path).map_err(|source| ResourceStoreOpenError::Io {
		path: base_path.to_path_buf(),
		action: "delete the stale resources directory",
		source,
	})?;
	create_resources_directory(base_path)
}

fn create_resources_directory(base_path: &Path) -> Result<(), ResourceStoreOpenError> {
	std::fs::create_dir_all(base_path).map_err(|source| ResourceStoreOpenError::Io {
		path: base_path.to_path_buf(),
		action: "create the resources directory",
		source,
	})
}

/// Synchronizes the resource-management implementation marker shared by every database opener.
///
/// A missing or changed signature deletes the directory, because archived values from another schema cannot be read.
/// A matching signature leaves every file in place, so a corrupt database is reported by the open that follows.
fn sync_resource_management_signature(base_path: &Path) -> Result<(), ResourceStoreOpenError> {
	create_resources_directory(base_path)?;

	let stored_signature = read_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE);

	if stored_signature.as_deref() == Some(RESOURCE_MANAGEMENT_CODE_HASH) {
		return Ok(());
	}

	if let Some(stored_signature) = stored_signature {
		log::info!(
			"Deleting resources at '{}' because the resource-management signature changed from '{}' to '{}'.",
			base_path.display(),
			stored_signature,
			RESOURCE_MANAGEMENT_CODE_HASH
		);
		reset_resource_cache(base_path)?;
	} else if base_path.join(RESOURCES_DATABASE_FILE).exists() {
		log::info!(
			"Deleting resources at '{}' because the resource-management signature marker is missing.",
			base_path.display()
		);
		reset_resource_cache(base_path)?;
	}

	write_resource_cache_signature(base_path, RESOURCE_MANAGEMENT_SIGNATURE_FILE, RESOURCE_MANAGEMENT_CODE_HASH)
}

/// Sorts a redb failure during an open into the cause a caller can act on.
///
/// `action` completes "could not ..." for file-system failures, such as "open 'resources.db'".
fn store_open_error(base_path: &Path, action: &'static str, error: impl Into<redb::Error>) -> ResourceStoreOpenError {
	let path = base_path.to_path_buf();
	match error.into() {
		redb::Error::DatabaseAlreadyOpen => ResourceStoreOpenError::InUse { path },
		// redb reports a file without a valid header, such as one another tool wrote, as invalid data.
		redb::Error::Io(source) if source.kind() == std::io::ErrorKind::InvalidData => ResourceStoreOpenError::Corrupt {
			path,
			detail: source.to_string(),
		},
		redb::Error::Io(source) => ResourceStoreOpenError::Io { path, action, source },
		// Damaged pages, an old file format, a refused repair, and missing or mistyped tables all mean that this
		// file cannot serve as the store's database.
		error => ResourceStoreOpenError::Corrupt {
			path,
			detail: error.to_string(),
		},
	}
}

/// Creates a consistent storage error for one REDB clear step.
/// Builds the error for a provenance record that couldn't be read or written.
fn provenance_error(action: &str, error: &dyn std::fmt::Display) -> String {
	format!(
		"Failed to {action} resource provenance. The most likely cause is a corrupt or unwritable resources database. Delete the resources directory and bake again. Error: {error}"
	)
}

fn resource_list_error(error: &dyn std::fmt::Display) -> String {
	format!(
		"Failed to list resources. The most likely cause is a corrupt resources database. Delete the resources directory and bake again. Error: {error}"
	)
}

fn resource_clear_error(action: &str, error: impl std::fmt::Display) -> String {
	format!(
		"Failed to {action}. The most likely cause is that the resources database is corrupt or cannot be updated. Error: {error}"
	)
}

fn resource_payload_path(base_path: &Path, key: [u8; 16], hash: u64, encoding: ResourcePayloadEncoding) -> std::path::PathBuf {
	base_path.join(format!("{}-{hash:016x}-{}", ResourceId(key), encoding.as_str()))
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

		if file_type.is_file() && parse_payload_name(name).is_some() {
			payload_paths.push(entry.path());
		}
	}

	Ok(payload_paths)
}

/// Reads the resource UID, payload hash, and encoding from a payload file name written by [`resource_payload_path`].
fn parse_payload_name(name: &str) -> Option<(ResourceId, u64, ResourcePayloadEncoding)> {
	let (resource_id, remainder) = name.split_once('-')?;
	let (hash, encoding) = remainder.split_once('-')?;
	if hash.len() != 16 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
		return None;
	}
	let encoding = [
		ResourcePayloadEncoding::Raw,
		ResourcePayloadEncoding::CpuLz4,
		ResourcePayloadEncoding::MetalIoLz4,
	]
	.into_iter()
	.find(|candidate| candidate.as_str() == encoding)?;

	Some((
		ResourceId::from_uid_hex(resource_id)?,
		u64::from_str_radix(hash, 16).ok()?,
		encoding,
	))
}

/// Selects ordinary textures while environment images retain their existing multi-image CPU upload path.
fn is_direct_texture(class: &str, metadata: &[u8]) -> bool {
	class == "Image"
		&& crate::from_slice::<crate::resources::image::Image>(metadata)
			.is_ok_and(|image| image.ibl.is_none() && !image.cpu_readable)
}

/// Encodes the class and fixed-width UID directly into the persisted index key.
fn class_index_key(class: &str, key: [u8; 16]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(class.len() + 1 + 32);
	write!(bytes, "{class}\0{}", ResourceId(key)).expect("Writing a resource index to a Vec cannot fail");
	bytes
}

/// Encodes the property fields and UID without allocating an intermediate hexadecimal string.
fn property_index_key(class: &str, property: &str, value: &str, key: [u8; 16]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(class.len() + property.len() + value.len() + 3 + 32);
	write!(bytes, "{class}\0{property}\0{value}\0{}", ResourceId(key)).expect("Writing a resource index to a Vec cannot fail");
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
#[cfg(debug_assertions)]
const RESOURCE_REFERENCES_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resource-references");
/// What the store remembers about how each resource ID came to be stored: its ID and its [`Provenance`].
const RESOURCE_PROVENANCE_TABLE: redb::TableDefinition<[u8; 16], &[u8]> = redb::TableDefinition::new("resource-provenance");
/// The tables besides the resources table that hold at most one record per resource UID and lose it when the resource
/// is deleted.
#[cfg(debug_assertions)]
const RECORD_TABLES: [redb::TableDefinition<[u8; 16], &[u8]>; 3] =
	[RESOURCE_PROVENANCE_TABLE, RESOURCE_TRACES_TABLE, RESOURCE_REFERENCES_TABLE];
#[cfg(not(debug_assertions))]
const RECORD_TABLES: [redb::TableDefinition<[u8; 16], &[u8]>; 1] = [RESOURCE_PROVENANCE_TABLE];

const RESOURCE_MANAGEMENT_CODE_HASH: &str = env!("RESOURCE_MANAGEMENT_CODE_HASH");
const RESOURCE_MANAGEMENT_SIGNATURE_FILE: &str = ".resource-management-version";
const STORAGE_MODE_KEY: &str = "payload-storage-mode";
const IMAGE_COMPRESSION_KEY: &str = "image-compression";
const PACKED_RESOURCES_FILE: &str = "resources.pack";
const RESOURCES_DATABASE_FILE: &str = "resources.db";
const STAGED_RESOURCE_FILE_PREFIX: &str = ".resource-write";
const BAKING_APP_RESOURCES_DOCS_PATH: &str = "use/resources/bake";
const RESOURCE_STORE_RECOVERY_DOCS_PATH: &str = "use/resources/bake#fix-a-resource-store-that-doesnt-open";

#[cfg(test)]
mod tests {
	use super::{
		PACKED_RESOURCES_FILE, RESOURCE_MANAGEMENT_CODE_HASH, RESOURCE_MANAGEMENT_SIGNATURE_FILE, ReDBStorageBackend,
		ResourceStorageMode, STAGED_RESOURCE_FILE_PREFIX, resource_payload_path, validate_resource_management_signature,
	};
	use crate::{
		Model, ProcessedAsset, SerializableResource,
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

	/// Creates a fresh, empty directory under the system temp directory.
	///
	/// `create_dir` fails when the path already exists, so retrying with a new
	/// timestamp suffix gives each caller its own directory without shared counters.
	fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
		loop {
			let nanos = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos();
			let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
			match std::fs::create_dir(&path) {
				Ok(()) => return path,
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
				Err(error) => {
					panic!("Failed to create a test directory: {error}. The most likely cause is an unwritable temp directory.")
				}
			}
		}
	}

	fn backend_with_mode(storage_mode: ResourceStorageMode) -> ReDBStorageBackend {
		ReDBStorageBackend::new_writable_with_mode(unique_temp_dir("byte-engine-redb-tests"), storage_mode).unwrap()
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
			let resources = backend.list().await.unwrap();
			assert_eq!(
				resources.iter().map(SerializableResource::id).collect::<Vec<_>>(),
				["materials/rebuilt"]
			);

			std::fs::remove_dir_all(&backend.base_path).unwrap();
		}
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
			cpu_readable: false,
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

		backend.delete(id).await.unwrap();
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
		let resources_path = unique_temp_dir("byte-engine-read-only-signature-tests");
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

	fn material(group: &str, tag: &str) -> MockMaterialModel {
		MockMaterialModel {
			group: group.into(),
			tag: tag.into(),
		}
	}

	/// Drains every page of `query` with `limit` results per page, asserting no page exceeds it.
	async fn query_all_pages(backend: &ReDBStorageBackend, query: Query, limit: usize) -> Vec<String> {
		let mut ids = Vec::new();
		let mut cursor = None;
		loop {
			let mut page_query = query.clone().limit(limit);
			if let Some(cursor) = cursor.take() {
				page_query = page_query.cursor(cursor);
			}
			let (page, next) = query_ids(backend, page_query).await;
			assert!(page.len() <= limit);
			ids.extend(page);
			match next {
				Some(next) => cursor = Some(next),
				None => return ids,
			}
		}
	}

	#[crate::r#async::test]
	async fn query_pages_stay_complete_when_later_predicates_filter_index_entries() {
		let backend = backend();
		for index in 0..8 {
			let tag = if index % 2 == 0 { "hero" } else { "prop" };
			store_mock(&backend, &format!("materials/{index}"), material("opaque", tag)).await;
		}

		let mut ids = query_all_pages(
			&backend,
			Query::new("MockMaterial").eq("group", "opaque").eq("tag", "hero"),
			2,
		)
		.await;
		ids.sort();

		assert_eq!(ids, vec!["materials/0", "materials/2", "materials/4", "materials/6"]);
	}

	#[crate::r#async::test]
	async fn query_with_exactly_limit_matches_returns_no_cursor() {
		let backend = backend();
		store_mock(&backend, "materials/a", material("opaque", "hero")).await;
		store_mock(&backend, "materials/b", material("opaque", "hero")).await;

		let (ids, cursor) = query_ids(&backend, Query::new("MockMaterial").limit(2)).await;

		assert_eq!(ids.len(), 2);
		assert!(cursor.is_none());
	}

	#[crate::r#async::test]
	async fn query_does_not_match_values_that_share_a_prefix() {
		let backend = backend();
		store_mock(&backend, "materials/a", material("opaque", "hero")).await;
		store_mock(&backend, "materials/b", material("opaque", "heroic")).await;

		let (hero, _) = query_ids(&backend, Query::new("MockMaterial").eq("tag", "hero")).await;
		let (her, _) = query_ids(&backend, Query::new("MockMaterial").eq("tag", "her")).await;

		assert_eq!(hero, vec!["materials/a"]);
		assert!(her.is_empty());
	}

	#[crate::r#async::test]
	async fn query_does_not_match_classes_that_share_a_prefix() {
		#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
		struct MockMaterialExtraModel {
			group: String,
		}

		impl Model for MockMaterialExtraModel {
			fn get_class() -> &'static str {
				"MockMaterialExtra"
			}

			fn queryable_properties(&self, _: &str) -> Vec<crate::QueryableProperty> {
				vec![crate::QueryableProperty {
					name: "group".to_string(),
					value: crate::QueryableValue::String(self.group.clone()),
				}]
			}
		}

		let backend = backend();
		store_mock(&backend, "materials/a", material("opaque", "hero")).await;
		store_mock(&backend, "extras/a", MockMaterialExtraModel { group: "opaque".into() }).await;

		let (by_class, _) = query_ids(&backend, Query::new("MockMaterial")).await;
		let (by_property, _) = query_ids(&backend, Query::new("MockMaterial").eq("group", "opaque")).await;

		assert_eq!(by_class, vec!["materials/a"]);
		assert_eq!(by_property, vec!["materials/a"]);
	}

	#[crate::r#async::test]
	async fn query_after_overwrite_matches_only_the_latest_properties() {
		let backend = backend();
		store_mock(&backend, "materials/a", material("opaque", "hero")).await;
		store_mock(&backend, "materials/a", material("opaque", "prop")).await;

		let (old, _) = query_ids(&backend, Query::new("MockMaterial").eq("tag", "hero")).await;
		let (new, _) = query_ids(&backend, Query::new("MockMaterial").eq("tag", "prop")).await;
		let (all, _) = query_ids(&backend, Query::new("MockMaterial")).await;

		assert!(old.is_empty());
		assert_eq!(new, vec!["materials/a"]);
		assert_eq!(all, vec!["materials/a"]);
	}

	#[crate::r#async::test]
	async fn packed_stores_answer_class_and_property_queries() {
		let backend = backend_with_mode(ResourceStorageMode::Packed);
		store_mock(&backend, "materials/a", material("opaque", "hero")).await;
		store_mock(&backend, "materials/b", material("transparent", "hero")).await;

		let mut all = query_all_pages(&backend, Query::new("MockMaterial"), 1).await;
		all.sort();
		let (opaque, _) = query_ids(&backend, Query::new("MockMaterial").eq("group", "opaque")).await;

		assert_eq!(all, vec!["materials/a", "materials/b"]);
		assert_eq!(opaque, vec!["materials/a"]);
	}

	fn variant_for(render_model: &str) -> crate::resources::material::VariantModel {
		use crate::resources::material::{MaterialModel, RenderModel, VariantModel};

		let material = MaterialModel {
			double_sided: false,
			alpha_mode: crate::types::AlphaMode::Opaque,
			shaders: Vec::new(),
			model: RenderModel {
				name: render_model.to_string(),
				pass: "MaterialEvaluation".to_string(),
			},
			parameters: Vec::new(),
			program: crate::resources::material::MaterialProgram {
				graph: String::new(),
				parameters: Vec::new(),
				identity: 0,
				reads_backdrop: false,
				translucent: false,
			},
		};
		VariantModel {
			material: crate::ReferenceModel::new("materials/shared.material", 0, 0, &material, None),
			variables: Vec::new(),
			alpha_mode: crate::types::AlphaMode::Opaque,
			double_sided: false,
		}
	}

	#[cfg(debug_assertions)]
	#[crate::r#async::test]
	async fn references_are_stored_with_the_resource_and_removed_with_it() {
		use crate::resources::{
			material::{ValueModel, VariantVariableModel},
			pipeline::{Pipeline, PipelineKind},
		};

		let backend = backend();
		let mut variant = variant_for("Visibility");
		// Two variables name the same image, which the references list once.
		for name in ["albedo", "albedo-again"] {
			variant.variables.push(VariantVariableModel {
				name: name.to_string(),
				r#type: "Texture2D".to_string(),
				value: ValueModel::Image(crate::ReferenceModel::new_serialized(
					"textures/albedo.png",
					0,
					0,
					Vec::new(),
					None,
				)),
				dynamic: false,
			});
		}
		let compute = |shader: &str| Pipeline {
			name: "compute".to_string(),
			kind: PipelineKind::Compute {
				shader: shader.to_string(),
				push_constants: Vec::new(),
			},
		};

		store_mock(&backend, "materials/hero.variant", variant).await;
		store_mock(&backend, "pipelines/replaced.pipeline", compute("shaders/first.besl")).await;
		store_mock(&backend, "pipelines/replaced.pipeline", compute("shaders/second.besl")).await;
		store_mock(&backend, "pipelines/kept.pipeline", compute("shaders/kept.besl")).await;

		let mut references = backend.references().unwrap();
		references.sort();
		assert_eq!(
			references,
			[
				(
					"materials/hero.variant".to_string(),
					vec!["materials/shared.material".to_string(), "textures/albedo.png".to_string()]
				),
				("pipelines/kept.pipeline".to_string(), vec!["shaders/kept.besl".to_string()]),
				(
					"pipelines/replaced.pipeline".to_string(),
					vec!["shaders/second.besl".to_string()]
				),
			]
		);

		backend
			.delete(crate::asset::ResourceId::new("pipelines/replaced.pipeline"))
			.await
			.unwrap();
		assert_eq!(backend.references().unwrap().len(), 2);

		backend.clear().await.unwrap();
		assert!(backend.references().unwrap().is_empty());
	}

	#[crate::r#async::test]
	async fn variant_query_ids_match_the_material_render_model() {
		let backend = backend();
		store_mock(&backend, "materials/visibility.variant", variant_for("Visibility")).await;
		store_mock(&backend, "materials/other.variant", variant_for("Forward")).await;
		let resource_manager = crate::ResourceManager::new(backend);

		let page = resource_manager
			.query_ids(
				Query::new("Variant")
					.eq("render-model", "Visibility")
					.eq("render-pass", "MaterialEvaluation"),
			)
			.await
			.unwrap();

		assert_eq!(page.items, vec!["materials/visibility.variant"]);
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
		assert_eq!(backend.traces().unwrap(), [("broken.asset".to_string(), items.clone())]);
		assert!(backend.read(id).await.is_none());
		assert!(backend.list().await.unwrap().is_empty());

		backend.delete(id).await.unwrap();

		assert!(backend.read_trace(id).await.unwrap().is_empty());
		assert!(backend.traces().unwrap().is_empty());
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
		backend.delete(crate::asset::ResourceId::new("materials/a")).await.unwrap();

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

/// How many payload syncs a flush runs at once. Each sync opens and closes its file, and running several hides those
/// round trips, while the bound keeps the blocking pool's threads free for other work.
const PAYLOAD_SYNCS_IN_FLIGHT: usize = 16;

/// Writes one payload file's data to the drive.
///
/// Compio syncs with a plain `fsync` on Apple platforms, where the standard library's `sync_data` also flushes the
/// whole drive cache, once per payload. The durable commit that follows flushes that cache once for every payload.
async fn sync_payload(path: &std::path::Path) -> std::io::Result<()> {
	let file = match compio::fs::File::open(path).await {
		Ok(file) => file,
		// The resource was replaced or deleted after it was published, so no record names this file anymore.
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
		Err(error) => return Err(error),
	};
	file.sync_data().await?;
	file.close().await
}

use std::{
	future::Future,
	io::Write as _,
	path::Path,
	sync::{
		Mutex,
		atomic::{AtomicU64, Ordering},
	},
};

use redb::{ReadableDatabase as _, ReadableTable, ReadableTableMetadata as _};
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
	asset::Provenance,
	r#async::{self, BoxedFuture, File as AsyncFile},
	resource::{
		ResourceCompressionPolicy, ResourceId, ResourcePayloadEncoding, ResourceReaderBacking, reader::StoredResourceReader,
		resource_handler::MultiResourceReader,
	},
};
