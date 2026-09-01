/// The `AssetManager` struct selects asset handlers and bakes source assets into resource storage.
///
/// Register each source format with [`Self::add_asset_handler`], then install the
/// manager on a [`crate::ResourceManager`] for debug loading or call
/// [`Self::bake`] from an explicit baking workflow.
/// See the [assets guide](/docs/develop/resource-management/assets)
/// for supported source families and processing behavior.
pub struct AssetManager {
	state: Arc<AssetManagerState>,
}

/// The `AssetManagerState` struct keeps shared bake state independent from the worker runtimes that execute requests.
pub(crate) struct AssetManagerState {
	asset_handlers: Vec<Box<dyn DynAssetHandler>>,
	storage_backend: Box<dyn DynStorageBackend>,
	resource_storage_backend: Arc<dyn DynResourceStorageBackend>,
	in_flight_bakes: Arc<Mutex<HashMap<String, announcement::Announcement<Result<(), LoadMessages>>>>>,
	bake_memory_budget: Option<Arc<BakeMemoryBudget>>,
	dispatcher: compio::dispatcher::Dispatcher,
	self_weak: std::sync::OnceLock<std::sync::Weak<AssetManagerState>>, // TODO: what is this?
	#[cfg(debug_assertions)]
	resource_trace: ResourceTrace,
	#[cfg(debug_assertions)]
	hot_reload: Mutex<HotReloadState>,
}

impl AssetManager {
	/// Creates an asset manager over source-asset and destination-resource storage.
	///
	/// Next, register all required formats with [`Self::add_asset_handler`] before
	/// installing the manager or starting a bake.
	pub fn new<AS, RS>(storage_backend: AS, resource_storage_backend: RS) -> AssetManager
	where
		AS: StorageBackend + 'static,
		RS: ResourceStorageBackend + 'static,
	{
		Self::new_shared(storage_backend, Arc::new(resource_storage_backend))
	}

	/// Creates an asset manager that shares an existing destination resource store.
	///
	/// Next, register all required formats with [`Self::add_asset_handler`] before
	/// installing the manager or starting a bake.
	pub fn new_shared<AS: StorageBackend + 'static>(
		storage_backend: AS,
		resource_storage_backend: Arc<dyn DynResourceStorageBackend>,
	) -> AssetManager {
		#[cfg(test)]
		let worker_count = std::num::NonZeroUsize::new(2).unwrap();

		#[cfg(not(test))]
		let worker_count = std::thread::available_parallelism()
			.unwrap_or(std::num::NonZeroUsize::MIN)
			.min(std::num::NonZeroUsize::new(16).unwrap());

		// Start the shared worker pool with the manager so the first bake does not pay its setup cost.
		let dispatcher = compio::dispatcher::Dispatcher::builder()
			.worker_threads(worker_count)
			.thread_names(|worker_index| format!("Asset Worker {worker_index}"))
			.build()
			.expect(
				"Failed to start asset workers. The most likely cause is that the platform I/O driver or worker threads could not be initialized.",
			);

		Self {
			state: Arc::new(AssetManagerState {
				asset_handlers: Vec::with_capacity(8),
				storage_backend: Box::new(storage_backend),
				resource_storage_backend,
				in_flight_bakes: Arc::new(Mutex::new(HashMap::with_capacity(32))),
				bake_memory_budget: None,
				dispatcher,
				self_weak: std::sync::OnceLock::new(),
				#[cfg(debug_assertions)]
				resource_trace: ResourceTrace::default(),
				#[cfg(debug_assertions)]
				hot_reload: Mutex::new(HotReloadState::default()),
			}),
		}
	}

	/// Sets the soft memory budget shared by concurrent asset bakes.
	///
	/// The manager reserves capacity for each independent bake tree and charges retained arena growth
	/// to this budget. Active work can exceed the budget so it can finish and release memory without deadlocking.
	/// Configure the budget before calling [`Self::bake`] or sharing the manager.
	pub fn set_bake_memory_budget(&mut self, byte_budget: NonZeroUsize) {
		Arc::get_mut(&mut self.state)
			.expect("The bake memory budget must be configured before the asset manager starts processing requests.")
			.bake_memory_budget = Some(Arc::new(BakeMemoryBudget::new(byte_budget.get())));
	}

	/// Registers a handler for one family of source assets.
	///
	/// After all handlers are registered, install this manager on a
	/// [`crate::ResourceManager`] in a debug build or call [`Self::bake`].
	pub fn add_asset_handler<T: AssetHandler + Send + Sync + 'static>(&mut self, asset_handler: T) {
		struct AssetHandlerWrapper<T: AssetHandler + Send + Sync>(T);

		impl<T: AssetHandler + Send + Sync> DynAssetHandler for AssetHandlerWrapper<T> {
			fn can_handle(&self, r#type: &str) -> bool {
				self.0.can_handle(r#type)
			}

			fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool {
				self.0.should_discover(id, has_sidecar)
			}

			fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>> {
				Box::pin(self.0.bake(context, url))
			}
		}

		Arc::get_mut(&mut self.state)
			.expect("Asset handlers must be registered before the asset manager starts processing requests.")
			.asset_handlers
			.push(Box::new(AssetHandlerWrapper(asset_handler)));
	}

	pub fn get_storage_backend(&self) -> &dyn DynStorageBackend {
		self.state.storage_backend.as_ref()
	}

	/// Reports whether a source directory can be read when the storage backend exposes paths.
	#[cfg(debug_assertions)]
	pub(crate) fn source_directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.state.storage_backend.directory_accessible(path)
	}

	/// Returns whether this manager writes to the same shared store as a resource manager.
	#[cfg(debug_assertions)]
	pub(crate) fn uses_resource_storage(&self, storage: &Arc<dyn DynResourceStorageBackend>) -> bool {
		Arc::ptr_eq(&self.state.resource_storage_backend, storage)
	}

	/// Returns the development trace populated by this manager's latest resource bakes.
	///
	/// Next, call [`ResourceTrace::items`] with a requested resource ID.
	#[cfg(debug_assertions)]
	pub fn resource_trace(&self) -> &ResourceTrace {
		&self.state.resource_trace
	}

	/// Returns whether a registered asset handler can bake the given source ID.
	pub fn supports(&self, id: &str) -> bool {
		let id = ResourceId::new(id);

		self.state
			.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_asset_type()))
	}

	/// Returns whether recursive discovery should include the given supported source asset.
	pub fn should_discover(&self, id: &str, has_sidecar: bool) -> bool {
		let id = ResourceId::new(id);

		self.state
			.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_asset_type()) && handler.should_discover(id, has_sidecar))
	}

	/// Returns the discoverable source IDs supported by the registered asset handlers.
	///
	/// Next, pass each ID to [`AssetManager::bake`] to produce its resource output.
	pub async fn discover(&self) -> Result<Vec<String>, String> {
		let mut ids = self
			.state
			.storage_backend
			.discover()
			.await?
			.into_iter()
			.filter(|source| self.should_discover(source.id(), source.has_sidecar()))
			.map(crate::asset::AssetSource::into_id)
			.collect::<Vec<_>>();

		ids.sort_unstable();

		Ok(ids)
	}

	/// Bakes the asset at `id` without checking for an existing stored resource.
	///
	/// Next, await [`crate::ResourceManager::request`] for the stored output or
	/// inspect it through the storage backend.
	pub async fn bake(&self, id: &str) -> Result<(), LoadMessages> {
		self.dispatch_bake(id, false).await
	}

	/// Returns the stored asset, or bakes it when it is missing or its recorded source versions changed.
	pub async fn bake_if_not_exists<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadMessages> {
		self.bake_if_not_exists_serialized(id).await.map(Into::into)
	}

	/// Returns the stored resource after ensuring its complete source provenance is current.
	pub(crate) async fn bake_if_not_exists_serialized(&self, id: &str) -> Result<crate::SerializableResource, LoadMessages> {
		self.dispatch_bake(id, true).await?;

		self.state
			.resource_storage_backend
			.read(ResourceId::new(id))
			.await
			.map(|(resource, _)| resource)
			.ok_or(LoadMessages::NoAsset)
	}

	/// Adds a requested root resource to the development dependency index.
	#[cfg(debug_assertions)]
	pub(crate) fn track_resource(&self, resource: &crate::SerializableResource) {
		self.state.track_resource(resource);
	}

	/// Starts recursive debounced watching when the source backend exposes a local root.
	#[cfg(debug_assertions)]
	pub(crate) fn start_watching(&self, updates: Arc<crate::resource::resource_manager::ResourceUpdateBroadcaster>) {
		let Some(root) = self.state.storage_backend.watch_root() else {
			return;
		};

		let _ = self.state.self_weak.set(Arc::downgrade(&self.state));

		let weak = Arc::downgrade(&self.state);

		let watched_root = root.clone();

		let mut watcher = match notify_debouncer_full::new_debouncer(
			std::time::Duration::from_millis(250),
			None,
			move |result: notify_debouncer_full::DebounceEventResult| match result {
				Ok(events) => {
					let Some(state) = weak.upgrade() else { return };
					let mut sources = std::collections::HashSet::new();

					for path in events.iter().flat_map(|event| event.paths.iter()) {
						let Some((id, sidecar_source)) = source_ids_from_path(&watched_root, path) else {
							continue;
						};

						sources.insert(id);

						if let Some(sidecar_source) = sidecar_source {
							sources.insert(sidecar_source);
						}
					}

					state.reload_sources(sources);
				}
				Err(errors) => log::warn!(
					"Asset watching reported an error. The most likely cause is that the development asset directory became inaccessible: {errors:?}"
				),
			},
		) {
			Ok(watcher) => watcher,
			Err(error) => {
				log::warn!(
					"Asset watching could not start. The most likely cause is that the platform watcher is unavailable: {error}"
				);

				return;
			}
		};

		if let Err(error) = watcher.watch(&root, notify_debouncer_full::notify::RecursiveMode::Recursive) {
			log::warn!(
				"Asset watching could not watch '{}'. The most likely cause is that the directory is inaccessible: {error}",
				root.display()
			);

			return;
		}

		let mut hot_reload = self.state.hot_reload.lock();

		hot_reload.updates = Some(updates);

		hot_reload.watcher = Some(watcher);
	}

	/// Runs one owned bake request on the asset worker pool.
	async fn dispatch_bake(&self, id: &str, only_when_stale: bool) -> Result<(), LoadMessages> {
		let _ = self.state.self_weak.set(Arc::downgrade(&self.state));

		self.state.dispatch_bake(id, only_when_stale).await
	}
}

enum InFlightBakeRole {
	Leader(announcement::Announcer<Result<(), LoadMessages>>),
	Follower(announcement::Listener<Result<(), LoadMessages>>),
}

/// The `InFlightBakeCleanup` struct removes a leader entry when its future completes, is canceled, or unwinds.
struct InFlightBakeCleanup {
	registry: Arc<Mutex<HashMap<String, announcement::Announcement<Result<(), LoadMessages>>>>>,
	id: String,
}

impl InFlightBakeCleanup {
	fn new(registry: &Arc<Mutex<HashMap<String, announcement::Announcement<Result<(), LoadMessages>>>>>, id: &str) -> Self {
		Self {
			registry: Arc::clone(registry),
			id: id.to_owned(),
		}
	}
}

impl Drop for InFlightBakeCleanup {
	fn drop(&mut self) {
		self.registry.lock().remove(&self.id);
	}
}

#[cfg(debug_assertions)]
struct HotReloadState {
	watcher: Option<
		notify_debouncer_full::Debouncer<
			notify_debouncer_full::notify::RecommendedWatcher,
			notify_debouncer_full::RecommendedCache,
		>,
	>,
	resources_by_source: HashMap<String, std::collections::HashSet<String>>,
	sources_by_resource: HashMap<String, Vec<String>>,
	in_flight: std::collections::HashSet<String>,
	pending: std::collections::HashSet<String>,
	updates: Option<Arc<crate::resource::resource_manager::ResourceUpdateBroadcaster>>,
}

#[cfg(debug_assertions)]
impl Default for HotReloadState {
	fn default() -> Self {
		Self {
			watcher: None,
			resources_by_source: HashMap::new(),
			sources_by_resource: HashMap::new(),
			in_flight: std::collections::HashSet::new(),
			pending: std::collections::HashSet::new(),
			updates: None,
		}
	}
}

/// The `LoadMessages` enum identifies failures while an asset is loaded, baked, or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadMessages {
	/// The asset was not found in the storage backend.
	NoAsset,
	/// An I/O operation failed while loading the asset.
	IO,
	/// The asset description does not contain a URL.
	NoURL,
	/// No asset handler was found for the asset.
	NoAssetHandler,
	/// The asset or one of its dependencies could not be baked or loaded.
	FailedToBake { asset: String, error: LoadErrors },
	/// The asset could not be stored in the resource storage backend.
	FailedToStore { asset: String, error: String },
	/// No asset worker was available to execute the request.
	ExecutionUnavailable,
}

impl AssetManagerState {
	/// Replaces one root resource's entries in the inverse source dependency index.
	#[cfg(debug_assertions)]
	fn track_resource(&self, resource: &crate::SerializableResource) {
		let mut hot_reload = self.hot_reload.lock();

		if let Some(previous) = hot_reload.sources_by_resource.remove(resource.id()) {
			for source in previous {
				if let Some(resources) = hot_reload.resources_by_source.get_mut(&source) {
					resources.remove(resource.id());

					if resources.is_empty() {
						hot_reload.resources_by_source.remove(&source);
					}
				}
			}
		}

		let sources = resource
			.asset_dependencies()
			.iter()
			.map(|dependency| dependency.id().to_string())
			.collect::<Vec<_>>();

		for source in &sources {
			hot_reload
				.resources_by_source
				.entry(source.clone())
				.or_default()
				.insert(resource.id().to_string());
		}

		hot_reload.sources_by_resource.insert(resource.id().to_string(), sources);
	}

	/// Schedules one rebake for every tracked root affected by a debounced source batch.
	#[cfg(debug_assertions)]
	fn reload_sources(self: &Arc<Self>, sources: std::collections::HashSet<String>) {
		let roots = {
			let mut hot_reload = self.hot_reload.lock();

			let roots = sources
				.iter()
				.flat_map(|source| hot_reload.resources_by_source.get(source).into_iter().flatten())
				.cloned()
				.collect::<std::collections::HashSet<_>>();

			roots
				.into_iter()
				.filter(|root| {
					if hot_reload.in_flight.insert(root.clone()) {
						true
					} else {
						hot_reload.pending.insert(root.clone());

						false
					}
				})
				.collect::<Vec<_>>()
		};

		for root in roots {
			let state = Arc::clone(self);

			let root_for_error = root.clone();

			if self
				.dispatcher
				.dispatch(move || async move { state.reload_resource(root).await })
				.is_err()
			{
				self.hot_reload.lock().in_flight.remove(&root_for_error);
			}
		}
	}

	/// Rebakes one affected root and publishes it only after replacement storage succeeds.
	#[cfg(debug_assertions)]
	async fn reload_resource(self: Arc<Self>, id: String) {
		let stale = match self.resource_storage_backend.read(ResourceId::new(&id)).await {
			Some((resource, _)) => self.resource_is_stale(&resource).await,
			None => true,
		};

		let result = if stale {
			let allocator = BakeAllocator::new(self.bake_memory_budget.as_ref()).await;

			// Enter the shared bake registry so one source referenced by several changed roots is rebuilt once.
			self.ensure_baked_in(&id, &allocator).await
		} else {
			Ok(())
		};

		if stale
			&& result.is_ok()
			&& let Some((resource, _)) = self.resource_storage_backend.read(ResourceId::new(&id)).await
		{
			self.track_resource(&resource);

			let update = crate::resource::resource_manager::ResourceUpdate::new(id.clone(), resource.class().to_string());

			if let Some(updates) = self.hot_reload.lock().updates.clone() {
				updates.send(update);
			}
		}

		let retry = {
			let mut hot_reload = self.hot_reload.lock();

			hot_reload.in_flight.remove(&id);

			hot_reload.pending.remove(&id)
		};

		if retry {
			let state = Arc::clone(&self);

			let retry_id = id.clone();

			self.hot_reload.lock().in_flight.insert(id.clone());

			if self
				.dispatcher
				.dispatch(move || async move { state.reload_resource(retry_id).await })
				.is_err()
			{
				self.hot_reload.lock().in_flight.remove(&id);
			}
		}
	}

	/// Runs one owned bake request on the asset worker pool.
	pub(crate) async fn dispatch_bake(&self, id: &str, only_when_stale: bool) -> Result<(), LoadMessages> {
		self.dispatch_bake_in_scope(id, only_when_stale, None).await
	}

	/// Runs one dependency request in its root bake's memory scope.
	pub(super) async fn dispatch_bake_in_scope(
		&self,
		id: &str,
		only_when_stale: bool,
		memory_scope: Option<Arc<BakeMemoryScope>>,
	) -> Result<(), LoadMessages> {
		let state = self
			.self_weak
			.get()
			.and_then(std::sync::Weak::upgrade)
			.ok_or(LoadMessages::ExecutionUnavailable)?;

		if let Some(notification) = self.bake_listener(id) {
			return notification.listen().await.map_err(|_| LoadMessages::ExecutionUnavailable)?;
		}

		// Independent roots wait before becoming leaders. This lets an admitted parent claim and run a dependency
		// instead of following a root that cannot start until the parent releases memory.
		let memory_scope = match (memory_scope, &self.bake_memory_budget) {
			(Some(memory_scope), _) => Some(memory_scope),
			(None, Some(memory_budget)) => Some(memory_budget.acquire().await),
			(None, None) => None,
		};

		let notification = match self.register_bake(id) {
			InFlightBakeRole::Leader(notification) => notification,
			InFlightBakeRole::Follower(notification) => {
				drop(memory_scope);

				return notification.listen().await.map_err(|_| LoadMessages::ExecutionUnavailable)?;
			}
		};

		let id = id.to_owned();

		let registry_cleanup = InFlightBakeCleanup::new(&self.in_flight_bakes, &id);

		let task = self
			.dispatcher
			.dispatch(move || async move {
				// The future owns cleanup before its first poll, so queued cancellation cannot leave a closed registry entry.
				let _registry_cleanup = registry_cleanup;

				// Dependencies inherit their admitted root scope and use separate arenas without another admission wait.
				let allocator = BakeAllocator::in_scope(memory_scope);

				let result = if only_when_stale {
					state.ensure_baked_uncoalesced(&id, &allocator).await
				} else {
					state.bake_uncoalesced(&id, &allocator).await
				};

				let _ = notification.announce(result.clone());

				result
			})
			.map_err(|_| LoadMessages::ExecutionUnavailable)?;

		task.await.map_err(|_| LoadMessages::ExecutionUnavailable)?
	}

	/// Returns a listener when the requested resource is already being baked.
	fn bake_listener(&self, id: &str) -> Option<announcement::Listener<Result<(), LoadMessages>>> {
		self.in_flight_bakes.lock().get(id).map(announcement::Announcement::listener)
	}

	/// Registers one requested resource before it is submitted to a worker.
	fn register_bake(&self, id: &str) -> InFlightBakeRole {
		let mut registry = self.in_flight_bakes.lock();

		match registry.entry(id.to_owned()) {
			Occupied(entry) => InFlightBakeRole::Follower(entry.get().listener()),
			Vacant(entry) => {
				let (announcer, announcement) = announcement::Announcement::new();

				entry.insert(announcement);

				InFlightBakeRole::Leader(announcer)
			}
		}
	}

	/// Copies the latest in-memory trace into development resource storage for external tools.
	#[cfg(debug_assertions)]
	fn persist_resource_trace(&self, id: ResourceId<'_>) {
		if let Err(error) = self
			.resource_storage_backend
			.replace_trace(id, &self.resource_trace.items(id.as_ref()))
		{
			log::warn!(
				"Failed to store the resource trace for '{}'. The most likely cause is that development resource storage is not writable. Error: {}",
				id.as_ref(),
				error
			);
		}
	}

	/// Runs one asset handler invocation without consulting the in-flight registry.
	///
	/// Call this method directly when no coalescing is desired.
	async fn bake_uncoalesced(&self, id: &str, allocator: &BakeAllocator) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		#[cfg(debug_assertions)]
		{
			self.resource_trace.clear(id);

			self.persist_resource_trace(id);
		}

		let asset_handler = match self
			.asset_handlers
			.iter()
			.find(|handler| handler.can_handle(id.get_asset_type()))
		{
			Some(handler) => handler,
			None => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					format!(
						"No asset handler found for '{}'. The most likely cause is an unsupported file extension or missing handler registration. See {}.",
						id.as_ref(),
						online_docs_url(ASSETS_DOCS_PATH)
					),
				);

				#[cfg(debug_assertions)]
				self.persist_resource_trace(id);

				log::warn!(
					"No asset handler found for asset: {:#?}. The most likely cause is an unsupported file extension or missing handler registration. See {}.",
					id,
					online_docs_url(ASSETS_DOCS_PATH)
				);

				return Err(LoadMessages::NoAssetHandler);
			}
		};

		let start_time = std::time::Instant::now();

		// The shared flag enforces the primary-write contract without rereading potentially expensive storage.
		// TODO: replace the flag when storage can confirm the primary write without a second read.
		let primary_stored = Cell::new(false);

		// Every resolution during this handler invocation contributes to the provenance attached to stored outputs.
		let asset_dependencies = Mutex::new(Vec::new());

		let tracking_storage_backend = TrackingStorageBackend::new(self.storage_backend.as_ref(), &asset_dependencies);

		let context = BakeContext::new(
			self,
			self.resource_storage_backend.as_ref(),
			&tracking_storage_backend,
			&asset_dependencies,
			allocator,
			id,
			&primary_stored,
			#[cfg(debug_assertions)]
			&self.resource_trace,
		);

		let result = match asset_handler.bake(context, id).await {
			Ok(()) if primary_stored.get() => Ok(()),
			Ok(()) => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					"The asset handler completed without storing the requested primary resource. The most likely cause is a missing store_primary call."
						.to_string(),
				);

				Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error: LoadErrors::PrimaryResourceNotStored,
				})
			}
			Err(LoadErrors::FailedToStore) => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					"Failed to store the requested resource. The resource storage backend likely rejected the primary resource write."
						.to_string(),
				);

				Err(LoadMessages::FailedToStore {
					asset: id.to_string(),
					error: format!(
						"Failed to store asset {:#?}. The resource storage backend likely rejected the primary resource write.",
						id
					),
				})
			}
			Err(error) => {
				#[cfg(debug_assertions)]
				if !self.resource_trace.has_error(id) {
					self.resource_trace.record(
						id,
						ResourceTraceLevel::Error,
						format!(
							"Could not bake asset '{}'. Cause: {} Fix: {} See {}.",
							id.as_ref(),
							error.message(),
							error.fix(),
							online_docs_url(ASSETS_DOCS_PATH)
						),
					);
				}

				Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error,
				})
			}
		};

		#[cfg(debug_assertions)]
		self.persist_resource_trace(id);

		result?;

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		Ok(())
	}

	/// Bakes an asset with the provided allocator when the resource is missing or stale.
	pub(super) async fn bake_if_not_exists_in(
		&self,
		id: &str,
		allocator: &BakeAllocator,
	) -> Result<crate::SerializableResource, LoadMessages> {
		self.ensure_baked_in(id, allocator).await?;

		if let Some((resource, _)) = self.resource_storage_backend.read(ResourceId::new(id)).await {
			return Ok(resource);
		}

		Err(LoadMessages::NoAsset)
	}

	/// Ensures that the requested resource exists and reflects its current source versions.
	async fn ensure_baked_in(&self, id: &str, allocator: &BakeAllocator) -> Result<(), LoadMessages> {
		match self.register_bake(id) {
			InFlightBakeRole::Leader(notification) => {
				let _registry_cleanup = InFlightBakeCleanup::new(&self.in_flight_bakes, id);

				let result = self.ensure_baked_uncoalesced(id, allocator).await;

				let _ = notification.announce(result.clone());

				result
			}
			InFlightBakeRole::Follower(notification) => {
				notification.listen().await.map_err(|_| LoadMessages::ExecutionUnavailable)?
			}
		}
	}

	/// Checks freshness and runs one bake without consulting the in-flight registry.
	async fn ensure_baked_uncoalesced(&self, id: &str, allocator: &BakeAllocator) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		if let Some((resource, _)) = self.resource_storage_backend.read(id).await {
			if !self.resource_is_stale(&resource).await {
				return Ok(());
			}

			log::info!(
				"Re-baking stale asset '{}'. One or more source files changed after the stored resource was produced.",
				id.as_ref()
			);
		}

		self.bake_uncoalesced(id.as_ref(), allocator).await
	}

	/// Returns whether any source version recorded by a stored resource differs from the current asset backend.
	async fn resource_is_stale(&self, resource: &crate::SerializableResource) -> bool {
		use utils::r#async::StreamExt as _;

		let checks = resource.asset_dependencies().iter().map(|dependency| async move {
			let id = ResourceId::new(dependency.id());
			let current = if dependency.version().tracks_sidecar() {
				self.storage_backend.version(id).await
			} else {
				self.storage_backend.raw_version(id).await
			};

			current.as_ref().ok() != Some(dependency.version())
		});

		// Provenance entries are independent; bound metadata pressure and stop after the first stale source.
		utils::r#async::stream::iter(checks)
			.buffer_unordered(8)
			.any(|stale| async move { stale })
			.await
	}
}

/// Converts a watcher path into its raw asset ID and, for BEAD files, its possible sidecar source ID.
#[cfg(debug_assertions)]
fn source_ids_from_path(root: &std::path::Path, path: &std::path::Path) -> Option<(String, Option<String>)> {
	let relative = path.strip_prefix(root).ok()?;

	let id = relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
	let resource_id = ResourceId::new(&id);
	let extension = resource_id.get_extension();
	let sidecar_source = extension
		.eq_ignore_ascii_case("bead")
		.then(|| id[..id.len() - extension.len() - 1].to_string());

	Some((id, sidecar_source))
}

const ASSETS_DOCS_PATH: &str = "develop/resource-management/assets";

#[cfg(test)]
pub mod tests {

	use std::{
		future::Future,
		sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		},
		time::Duration,
	};

	use super::*;
	#[cfg(debug_assertions)]
	use crate::asset::ResourceTraceLevel;
	use crate::{
		Model, ProcessedAsset,
		asset::{handler::LoadErrors, storage_backend::tests::TestStorageBackend},
		r#async::{self, BoxedFuture},
		resource::{ReadStorageBackend, storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend},
	};

	#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct TestResource {}

	impl Model for TestResource {
		fn get_class() -> &'static str {
			"TestResource"
		}
	}

	struct TestAssetHandler {}
	struct CompoundBeadAssetHandler;

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	struct VersionedAssetHandler {
		invocations: Arc<AtomicUsize>,
	}

	impl AssetHandler for VersionedAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			self.invocations.fetch_add(1, Ordering::SeqCst);

			let (source, ..) = context.resolve(id).await?;

			match id.get_base().as_ref() {
				"external.test" => {
					context.resolve(ResourceId::new("external.bin")).await?;
				}
				"parent.test" => {
					context.bake_dependency::<TestResource>("child.test").await?;
				}
				_ => {}
			}

			context.store_primary(ProcessedAsset::new(id, TestResource {}), &source).await
		}
	}

	fn versioned_asset_manager(
		storage: TestStorageBackend,
		resource_storage: ResourceTestStorageBackend,
	) -> (AssetManager, Arc<AtomicUsize>) {
		let invocations = Arc::new(AtomicUsize::new(0));

		let mut manager = AssetManager::new(storage, resource_storage);

		manager.add_asset_handler(VersionedAssetHandler {
			invocations: Arc::clone(&invocations),
		});

		(manager, invocations)
	}

	struct CoordinatingAssetHandler {
		invocations: Arc<AtomicUsize>,
		thread_ids: Arc<Mutex<Vec<std::thread::ThreadId>>>,
		started: Arc<Vec<Mutex<Option<announcement::Announcer<()>>>>>,
		release: announcement::Listener<()>,
		fail: bool,
		block_first_only: bool,
	}

	struct BatchedDependencyAssetHandler;

	struct DelayedDependencyAssetHandler {
		started: Mutex<Option<announcement::Announcer<()>>>,
		release: announcement::Listener<()>,
	}

	impl AssetHandler for DelayedDependencyAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "parent" || id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			if id.get_extension() == "parent" {
				self.started
					.lock()
					.take()
					.expect("the parent should announce one invocation")
					.announce(())
					.expect("the parent-start announcement should remain open");

				self.release
					.listen()
					.await
					.expect("the dependency release announcement should remain open");

				context.bake_dependency::<TestResource>("child.test").await?;
			}

			context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]).await
		}
	}

	impl AssetHandler for BatchedDependencyAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "batch"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			let dependencies = vec!["first.test".to_string(), "second.test".to_string()];

			context.bake_dependencies::<TestResource>(&dependencies, 2).await?;

			context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]).await
		}
	}

	impl AssetHandler for CoordinatingAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			let invocation = self.invocations.fetch_add(1, Ordering::SeqCst);

			self.thread_ids.lock().push(std::thread::current().id());

			self.started[invocation]
				.lock()
				.take()
				.expect("each test invocation should announce once")
				.announce(())
				.expect("test invocation announcement should be open");

			if !self.block_first_only || invocation == 0 {
				self.release
					.listen()
					.await
					.expect("test release announcement should remain open");
			}

			if self.fail {
				Err(LoadErrors::FailedToProcess)
			} else {
				context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]).await
			}
		}
	}

	fn coordinating_asset_manager(
		fail: bool,
		block_first_only: bool,
	) -> (
		AssetManager,
		Arc<AtomicUsize>,
		Arc<Mutex<Vec<std::thread::ThreadId>>>,
		Vec<announcement::Listener<()>>,
		announcement::Announcer<()>,
	) {
		let invocations = Arc::new(AtomicUsize::new(0));

		let thread_ids = Arc::new(Mutex::new(Vec::with_capacity(8)));

		let mut started_announcers = Vec::with_capacity(8);

		let mut started_listeners = Vec::with_capacity(8);

		for _ in 0..8 {
			let (announcer, announcement) = announcement::Announcement::new();

			started_announcers.push(Mutex::new(Some(announcer)));

			started_listeners.push(announcement.listener());
		}

		let started = Arc::new(started_announcers);

		let (release, release_announcement) = announcement::Announcement::new();

		let mut manager = AssetManager::new(TestStorageBackend::new(), ResourceTestStorageBackend::new());

		manager.add_asset_handler(CoordinatingAssetHandler {
			invocations: Arc::clone(&invocations),
			thread_ids: Arc::clone(&thread_ids),
			started: Arc::clone(&started),
			release: release_announcement.listener(),
			fail,
			block_first_only,
		});

		(manager, invocations, thread_ids, started_listeners, release)
	}

	impl AssetHandler for TestAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			match id.get_base().as_ref() {
				"example.test" => context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]).await,
				"messages.test" => {
					context.info("Imported test metadata.");

					context.warn(format_args!("Discarded {} optional test value.", 1));

					context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]).await
				}
				"failed.test" => {
					context
						.error("Test resource is malformed. The most likely cause is the intentionally invalid fixture data.");

					Err(LoadErrors::FailedToProcess)
				}
				"unstored.test" => Ok(()),
				"mismatched.test" => {
					context
						.store_primary(ProcessedAsset::new(ResourceId::new("other.test"), TestResource {}), &[])
						.await
				}
				_ => Err(LoadErrors::AssetCouldNotBeLoaded),
			}
		}
	}

	impl AssetHandler for CompoundBeadAssetHandler {
		fn can_handle(&self, asset_type: &str) -> bool {
			asset_type.eq_ignore_ascii_case("environment.bead")
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			if context.resource_type(id) != Some("environment.bead") {
				return Err(LoadErrors::UnsupportedType);
			}

			let (source, sidecar, asset_type) = context.resolve(id).await?;

			if sidecar.is_some() || asset_type != "environment.bead" {
				return Err(LoadErrors::UnsupportedType);
			}

			context.store_primary(ProcessedAsset::new(id, TestResource {}), &source).await
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		let storage_backend = TestStorageBackend::new();

		AssetManager::new(storage_backend, ResourceTestStorageBackend::new())
	}

	#[test]
	fn asset_manager_reports_support_for_registered_asset_types() {
		let storage_backend = TestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, ResourceTestStorageBackend::new());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.supports("nested/example.test"));
		assert!(asset_manager.supports("nested/example.test#fragment"));
		assert!(!asset_manager.supports("nested/example.unknown"));
		assert!(!asset_manager.supports(""));
		assert!(!asset_manager.supports("#fragment"));
	}

	#[test]
	fn registered_handlers_are_discoverable_by_default() {
		let storage_backend = TestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, ResourceTestStorageBackend::new());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.should_discover("nested/example.test", false));
		assert!(asset_manager.should_discover("nested/example.test", true));
		assert!(!asset_manager.should_discover("nested/example.unknown", true));
	}

	#[r#async::test]
	async fn discovery_filters_backend_sources_through_registered_handlers_and_sorts_ids() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("z-last.test", b"");
		storage_backend.add_file("nested/a-first.test", b"");
		storage_backend.add_file("nested/a-first.test.bead", b"{}");
		storage_backend.add_file("ignored.unknown", b"");

		let mut asset_manager = AssetManager::new(storage_backend, ResourceTestStorageBackend::new());
		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert_eq!(
			asset_manager.discover().await.unwrap(),
			["nested/a-first.test", "z-last.test"]
		);
	}

	#[r#async::test]
	async fn compound_bead_sources_are_discoverable_and_dispatch_without_claiming_sidecars() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("studio.environment.bead", b"environment");
		storage_backend.add_file("studio.environment.bead.bead", b"{ invalid: true }");
		storage_backend.add_file("studio.exr.bead", b"{ image: true }");

		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend, resource_storage.clone());
		asset_manager.add_asset_handler(CompoundBeadAssetHandler);

		assert!(asset_manager.supports("studio.environment.bead"));
		assert!(!asset_manager.supports("studio.exr.bead"));
		assert_eq!(asset_manager.discover().await.unwrap(), ["studio.environment.bead"]);

		asset_manager
			.bake("studio.environment.bead")
			.await
			.expect("the compound BEAD handler must receive the primary declaration");

		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("studio.environment.bead"))
			.expect("the compound BEAD resource must be stored");

		assert_eq!(data.as_ref(), b"environment");
	}

	#[cfg(debug_assertions)]
	#[test]
	fn watcher_paths_preserve_bead_assets_and_also_report_possible_sidecar_sources() {
		let root = std::path::Path::new("/assets");

		assert_eq!(
			source_ids_from_path(root, std::path::Path::new("/assets/rendering/pass.pipeline")),
			Some(("rendering/pass.pipeline".to_string(), None))
		);
		assert_eq!(
			source_ids_from_path(root, std::path::Path::new("/assets/rendering/pass.besl.bead")),
			Some((
				"rendering/pass.besl.bead".to_string(),
				Some("rendering/pass.besl".to_string())
			))
		);
		assert_eq!(
			source_ids_from_path(root, std::path::Path::new("/assets/lighting/studio.environment.BEAD")),
			Some((
				"lighting/studio.environment.BEAD".to_string(),
				Some("lighting/studio.environment".to_string())
			))
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn tracked_transitive_sources_publish_the_requested_root_after_rebaking() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("parent.test", b"parent");

		asset_storage.add_file("child.test", b"child");

		let resource_storage = ResourceTestStorageBackend::new();

		let (manager, _) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		let updates = Arc::new(crate::resource::resource_manager::ResourceUpdateBroadcaster::default());

		let listener = updates.listener();

		manager.state.hot_reload.lock().updates = Some(updates);

		let resource = manager.bake_if_not_exists_serialized("parent.test").await.unwrap();

		manager.track_resource(&resource);

		asset_storage.add_file("child.test", b"changed child");

		manager.state.clone().reload_resource("parent.test".to_string()).await;

		assert_eq!(
			listener.read(),
			Some(crate::resource::ResourceUpdate::new(
				"parent.test".into(),
				"TestResource".into()
			))
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn unchanged_watcher_events_do_not_publish_resource_updates() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("stable.test", b"stable");

		let resource_storage = ResourceTestStorageBackend::new();

		let (manager, _) = versioned_asset_manager(asset_storage, resource_storage);

		let updates = Arc::new(crate::resource::resource_manager::ResourceUpdateBroadcaster::default());

		let listener = updates.listener();

		manager.state.hot_reload.lock().updates = Some(updates);

		let resource = manager.bake_if_not_exists_serialized("stable.test").await.unwrap();

		manager.track_resource(&resource);

		manager.state.clone().reload_resource("stable.test".to_string()).await;

		assert_eq!(listener.read(), None);
	}

	#[r#async::test]
	async fn test_bake_with_asset_manager() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		asset_manager
			.bake("example.test")
			.await
			.expect("registered asset handler should bake its resource");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("example.test"))
			.expect("baked resource should be stored");

		assert_eq!(resource.class, "TestResource");
	}

	#[r#async::test]
	async fn lazy_baking_reuses_fresh_assets_and_rebakes_changed_sources_and_sidecars() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("versioned.test", b"first source");

		let resource_storage = ResourceTestStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage.clone());

		asset_manager
			.bake_if_not_exists::<TestResource>("versioned.test")
			.await
			.expect("initial source should bake");

		let first_hash = resource_storage
			.read(ResourceId::new("versioned.test"))
			.await
			.expect("initial resource should be stored")
			.0
			.hash();

		asset_manager
			.bake_if_not_exists::<TestResource>("versioned.test")
			.await
			.expect("unchanged source should be reused");

		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("versioned.test", b"changed source bytes");

		asset_manager
			.bake_if_not_exists::<TestResource>("versioned.test")
			.await
			.expect("changed source should rebake");

		let changed_hash = resource_storage
			.read(ResourceId::new("versioned.test"))
			.await
			.expect("changed resource should replace the prior value")
			.0
			.hash();

		assert_ne!(first_hash, changed_hash);
		assert_eq!(invocations.load(Ordering::SeqCst), 2);

		asset_storage.add_file("versioned.test.bead", br#"{ purpose: "first" }"#);

		asset_manager
			.bake_if_not_exists::<TestResource>("versioned.test")
			.await
			.expect("new sidecar should rebake");

		asset_storage.add_file("versioned.test.bead", br#"{ purpose: "changed" }"#);

		asset_manager
			.bake_if_not_exists::<TestResource>("versioned.test")
			.await
			.expect("changed sidecar should rebake");

		let sidecar_rebake_hash = resource_storage
			.read(ResourceId::new("versioned.test"))
			.await
			.expect("sidecar rebake should keep the resource")
			.0
			.hash();

		assert_eq!(sidecar_rebake_hash, changed_hash);
		assert_eq!(invocations.load(Ordering::SeqCst), 4);
	}

	#[r#async::test]
	async fn lazy_baking_tracks_directly_resolved_source_dependencies() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("external.test", b"root");

		asset_storage.add_file("external.bin", b"first dependency");

		let resource_storage = ResourceTestStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		asset_manager
			.bake_if_not_exists::<TestResource>("external.test")
			.await
			.expect("asset with external source should bake");

		asset_manager
			.bake_if_not_exists::<TestResource>("external.test")
			.await
			.expect("unchanged external source should be reused");

		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("external.bin", b"changed dependency");

		asset_manager
			.bake_if_not_exists::<TestResource>("external.test")
			.await
			.expect("changed external source should rebake its owner");

		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn lazy_baking_propagates_transitive_asset_versions_to_parent_resources() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("parent.test", b"parent");

		asset_storage.add_file("child.test", b"first child");

		let resource_storage = ResourceTestStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		asset_manager
			.bake_if_not_exists::<TestResource>("parent.test")
			.await
			.expect("parent and child should bake");

		asset_manager
			.bake_if_not_exists::<TestResource>("parent.test")
			.await
			.expect("unchanged dependency graph should be reused");

		assert_eq!(invocations.load(Ordering::SeqCst), 2);

		asset_storage.add_file("child.test", b"changed child");

		asset_manager
			.bake_if_not_exists::<TestResource>("parent.test")
			.await
			.expect("changed child should rebake the child and parent");

		assert_eq!(invocations.load(Ordering::SeqCst), 4);
	}

	#[r#async::test]
	async fn stale_resource_is_not_reused_after_its_source_is_removed() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("removed.test", b"source");

		let resource_storage = ResourceTestStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		asset_manager
			.bake_if_not_exists::<TestResource>("removed.test")
			.await
			.expect("initial source should bake");

		asset_storage.remove_file("removed.test");

		let result = asset_manager.bake_if_not_exists::<TestResource>("removed.test").await;

		assert!(result.is_err());
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn concurrent_bakes_for_one_asset_and_store_share_one_invocation() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, false);

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");

			release.announce(()).expect("release should be announced once");
		};

		let requests = async {
			std::future::join!(
				asset_manager.bake("coalesced.test"),
				asset_manager.bake("coalesced.test"),
				asset_manager.bake("coalesced.test"),
			)
			.await
		};

		let (_, results) = std::future::join!(release_handler, requests).await;

		assert_eq!(results, (Ok(()), Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 1);
	}

	#[r#async::test]
	async fn concurrent_failures_are_shared_but_later_bakes_retry() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(true, false);

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");

			release.announce(()).expect("release should be announced once");
		};

		let requests =
			async { std::future::join!(asset_manager.bake("failed.test"), asset_manager.bake("failed.test"),).await };

		let (_, (first, follower)) = std::future::join!(release_handler, requests).await;

		assert_eq!(first, follower);
		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		let retry = asset_manager.bake("failed.test").await;

		assert_eq!(retry, first);
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn completed_explicit_bake_is_not_memoized() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, true);

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");

			release.announce(()).expect("release should be announced once");
		};

		let (_, first) = std::future::join!(release_handler, asset_manager.bake("repeat.test"),).await;

		assert_eq!(first, Ok(()));
		assert_eq!(asset_manager.bake("repeat.test").await, Ok(()));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn different_assets_run_independently() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, false);

		let release_handler = async {
			started[1].listen().await.expect("two independent invocations should start");

			release.announce(()).expect("release should be announced once");
		};

		let requests = async { std::future::join!(asset_manager.bake("first.test"), asset_manager.bake("second.test"),).await };

		let (_, results) = std::future::join!(release_handler, requests).await;

		assert_eq!(results, (Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn admitted_parent_can_claim_a_dependency_before_a_memory_waiting_root() {
		let (started, started_announcement) = announcement::Announcement::new();

		let (release, release_announcement) = announcement::Announcement::new();

		let parent_started_for_root = started_announcement.listener();

		let parent_started_for_release = started_announcement.listener();

		let mut asset_manager = AssetManager::new(TestStorageBackend::new(), ResourceTestStorageBackend::new());

		asset_manager.set_bake_memory_budget(NonZeroUsize::MIN);

		asset_manager.add_asset_handler(DelayedDependencyAssetHandler {
			started: Mutex::new(Some(started)),
			release: release_announcement.listener(),
		});

		let parent = asset_manager.bake("root.parent");

		let competing_root = async {
			parent_started_for_root.listen().await.unwrap();

			asset_manager.bake("child.test").await
		};

		let release_parent = async {
			parent_started_for_release.listen().await.unwrap();

			compio::time::sleep(Duration::from_millis(10)).await;

			release.announce(()).unwrap();
		};

		let (parent, competing_root) = compio::time::timeout(Duration::from_secs(1), async {
			let ((parent, competing_root), ()) =
				std::future::join!(async { std::future::join!(parent, competing_root).await }, release_parent).await;

			(parent, competing_root)
		})
		.await
		.expect("the admitted parent and competing root should not deadlock");

		assert_eq!(parent, Ok(()));
		assert_eq!(competing_root, Ok(()));
	}

	#[r#async::test]
	async fn batched_dependencies_start_before_parent_continues() {
		let (mut asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, false);

		// A one-byte budget proves child requests inherit the parent's scope instead of waiting behind it.
		asset_manager.set_bake_memory_budget(NonZeroUsize::MIN);

		asset_manager.add_asset_handler(BatchedDependencyAssetHandler);

		let release_handler = async {
			started[1].listen().await.expect("both dependency bakes should start");

			release.announce(()).expect("dependency release should be announced once");
		};

		let (_, result) = std::future::join!(release_handler, asset_manager.bake("parent.batch")).await;

		assert_eq!(result, Ok(()));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn test_bake_no_asset_handler() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let asset_manager = AssetManager::new(storage_backend, resource_storage_backend);

		let result = asset_manager.bake("example.unknown").await;

		assert_eq!(result, Err(LoadMessages::NoAssetHandler));

		#[cfg(debug_assertions)]
		assert_eq!(
			asset_manager.resource_trace().items("example.unknown")[0].level(),
			ResourceTraceLevel::Error
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn handler_trace_keeps_ordered_info_and_warning_items_for_a_baked_resource() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		asset_manager
			.bake("messages.test")
			.await
			.expect("message fixture should bake");

		// A new bake replaces the prior trace instead of accumulating stale messages.
		asset_manager
			.bake("messages.test")
			.await
			.expect("message fixture should rebake");

		let items = asset_manager.resource_trace().items("messages.test");

		assert_eq!(items.len(), 2);
		assert_eq!(items[0].level(), ResourceTraceLevel::Info);
		assert_eq!(items[0].message(), "Imported test metadata.");
		assert_eq!(items[1].level(), ResourceTraceLevel::Warn);
		assert_eq!(items[1].message(), "Discarded 1 optional test value.");
		assert_eq!(
			resource_storage_backend
				.read_trace(ResourceId::new("messages.test"))
				.await
				.unwrap(),
			items
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn handler_error_trace_survives_when_the_resource_bake_fails() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("failed.test").await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "failed.test".to_string(),
				error: LoadErrors::FailedToProcess,
			})
		);
		assert!(
			resource_storage_backend
				.get_resource(ResourceId::new("failed.test"))
				.is_none()
		);

		let items = asset_manager.resource_trace().items("failed.test");

		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);
		assert_eq!(
			items[0].message(),
			"Test resource is malformed. The most likely cause is the intentionally invalid fixture data."
		);
		assert_eq!(
			resource_storage_backend
				.read_trace(ResourceId::new("failed.test"))
				.await
				.unwrap(),
			items
		);
		assert_eq!(asset_manager.resource_trace().resource_ids(), vec!["failed.test"]);
	}

	#[r#async::test]
	async fn successful_handler_must_store_the_requested_primary_resource() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend);

		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("unstored.test").await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "unstored.test".to_string(),
				error: LoadErrors::PrimaryResourceNotStored,
			})
		);
	}

	#[r#async::test]
	async fn handler_cannot_store_a_different_resource_as_the_primary() {
		let storage_backend = TestStorageBackend::new();

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("mismatched.test").await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "mismatched.test".to_string(),
				error: LoadErrors::PrimaryResourceIdMismatch,
			})
		);
		assert!(resource_storage_backend.get_resources().is_empty());
	}
}

use std::{
	cell::Cell,
	collections::hash_map::Entry::{Occupied, Vacant},
	num::NonZeroUsize,
	ops::Deref,
	sync::Arc,
};

use announcement;
use gxhash::HashMapExt;
use utils::{hash::HashMap, sync::Mutex};

#[cfg(debug_assertions)]
use super::resource_trace::{ResourceTrace, ResourceTraceLevel};
use super::{
	StorageBackend,
	bake_memory::{BakeAllocator, BakeMemoryBudget, BakeMemoryScope},
	handler::{AssetHandler, BakeContext, TrackingStorageBackend},
};
use crate::{
	Model, ProcessedAsset, ReferenceModel,
	asset::{
		self, DynStorageBackend, ResourceId,
		handler::{DynAssetHandler, LoadErrors},
	},
	r#async::BoxedFuture,
	online_docs_url,
	resource::{self, DynStorageBackend as DynResourceStorageBackend, StorageBackend as ResourceStorageBackend},
};
