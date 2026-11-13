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
	/// The image pipeline every handler that produces images shares.
	pub(in crate::asset) image_processor: ImageProcessor,
	/// The renderer's generator every handler that bakes materials turns their BEMX graphs into programs with.
	pub(in crate::asset) program_generator: Option<Box<dyn ProgramGenerator>>,
	/// The renderer's adaptation every standalone BESL program goes through before it links.
	pub(in crate::asset) program_transform: Option<Box<dyn ProgramTransform>>,
	/// The compiler every handler that bakes shaders compiles them with.
	pub(in crate::asset) shader_compiler: Box<dyn ShaderCompiler>,
	/// The caller that receives the intermediate products a bake emits, if any asked for them.
	pub(in crate::asset) interceptor: Option<Arc<dyn BakeInterceptor>>,
	storage_backend: Box<dyn DynStorageBackend>,
	pub(in crate::asset) resource_storage_backend: Arc<dyn DynResourceStorageBackend>,
	in_flight_bakes: Arc<Mutex<HashMap<String, announcement::Announcement<Result<BakeOutcome, LoadMessages>>>>>,
	bake_memory_budget: Option<Arc<BakeMemoryBudget>>,
	/// The bakes running and ended, reported to the reporter set with [`AssetManager::set_bake_reporter`].
	bakes: BakeTracker,
	/// The root bakes callers are waiting on, whether admitted yet or not. The last one to finish makes what they
	/// stored durable with one flush; see [`RootInFlight`].
	roots_in_flight: AtomicUsize,
	/// Whether a root stored anything since the last flush, so a bake that reused every resource doesn't flush.
	unflushed: AtomicBool,
	/// The fingerprint of every declared setting, by name. See [`AssetManager::declare_setting`].
	settings: HashMap<String, u64>,
	/// Serializes the records of which bake stores which resource, which several bakes can change at once.
	provenance_lock: Mutex<()>,
	/// Counts the times source files may have changed. A freshness check holds until the count moves.
	epoch: AtomicU64,
	/// Whether sources hold still for this manager's whole life, so no request discards earlier checks.
	verify_once: bool,
	/// Counts the resources stored and removed. A freshness check of a bake that ran a query holds until the count
	/// moves, because what the query matches may have changed.
	store_generation: AtomicU64,
	/// The epoch each ID was last made current in and how that went, so a resource many others use is checked once.
	/// A check that depends on what the store holds also keeps the store generation it was made at.
	checked: Mutex<HashMap<String, (u64, Option<u64>, Result<(), LoadMessages>)>>,
	/// The IDs of the resources this manager stored, so a forced rebuild builds each only once.
	stored: Mutex<std::collections::HashSet<String>>,
	/// The source IDs the asset storage listed, and the epoch it listed them in.
	sources: Mutex<Option<(u64, Arc<Vec<String>>)>>,
	/// The patterns of asset IDs that are never baked, from the project's `assets.exclude`.
	excluded: Vec<Glob>,
	/// Whether discovery lists an asset, from [`AssetManager::set_discovery_filter`]. Without one it lists every asset.
	discovery_filter: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
	/// The resources assembled from what the store holds, which a full bake bakes last. See
	/// [`AssetManager::set_assembled_resources`].
	assembled_resources: Vec<String>,
	dispatcher: compio::dispatcher::Dispatcher,
	#[cfg(debug_assertions)]
	pub(in crate::asset) resource_trace: ResourceTrace,
	#[cfg(debug_assertions)]
	hot_reload: Mutex<HotReloadState>,
}

/// The `BakeOrigin` enum tells a dispatched bake whether it answers a caller directly or runs inside a parent bake.
#[derive(Clone)]
pub(crate) enum BakeOrigin {
	/// A bake requested through [`AssetManager`]; it waits for memory admission and makes its outputs durable.
	Root,
	/// A root the manager checks again after source files changed. It is a [`Self::Root`], except that the change that
	/// asked for it already discarded the earlier freshness checks, once for every root it checks. Only development
	/// builds watch sources.
	#[cfg_attr(not(debug_assertions), allow(dead_code))]
	Recheck,
	/// A dependency of a running bake or freshness check; it shares the parent's memory scope and leaves durability
	/// to the root. `ancestors` are the IDs waiting on it, outermost first, which it never waits on in turn.
	Dependency {
		memory_scope: Option<Arc<BakeMemoryScope>>,
		ancestors: Vec<String>,
	},
}

impl BakeOrigin {
	/// Returns whether `id` is waiting on this bake, so this bake must not wait on `id`.
	fn waits_on(&self, id: &str) -> bool {
		matches!(self, BakeOrigin::Dependency { ancestors, .. } if ancestors.iter().any(|ancestor| ancestor == id))
	}
}

/// The `BakeMode` enum tells a dispatched bake how much the stored resource counts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BakeMode {
	/// Bake only when the resource is missing or an input of its latest bake changed.
	IfStale,
	/// Bake the resource whatever its record says, and its dependencies only when stale.
	Always,
	/// Bake the resource and everything it is baked from, each once per manager.
	Forced,
}

/// The `RootInFlight` struct counts one root bake as in flight for as long as its caller waits on it.
///
/// A durable flush waits for the disk, and on some systems for the whole drive cache, so the manager flushes once
/// when the last root a caller requested finishes, rather than once per root, so concurrent bakes don't stall behind
/// flushes of payloads other roots are still writing.
struct RootInFlight<'a> {
	state: &'a Arc<AssetManagerState>,
	/// Whether the root left through [`Self::leave`], whose caller waits for the flush.
	left: bool,
}

impl<'a> RootInFlight<'a> {
	fn enter(state: &'a Arc<AssetManagerState>) -> Self {
		state.roots_in_flight.fetch_add(1, Ordering::AcqRel);
		Self { state, left: false }
	}

	/// Leaves, and returns the flush to wait for when this root left last with stored resources unflushed.
	fn leave(mut self) -> Option<utils::r#async::oneshot::Receiver<()>> {
		self.left = true;
		self.flush_if_last()
	}

	fn flush_if_last(&self) -> Option<utils::r#async::oneshot::Receiver<()>> {
		// A root marks its output before it leaves, so the root that leaves last sees every mark.
		let last = self.state.roots_in_flight.fetch_sub(1, Ordering::AcqRel) == 1;
		if last && self.state.unflushed.swap(false, Ordering::AcqRel) {
			self.state.flush()
		} else {
			None
		}
	}
}

impl Drop for RootInFlight<'_> {
	fn drop(&mut self) {
		// A root whose caller stopped waiting can't wait for the flush either, so the flush runs with nobody waiting.
		if !self.left {
			drop(self.flush_if_last());
		}
	}
}

/// The `BakeOutcome` enum reports whether a freshness-checked bake wrote anything.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BakeOutcome {
	/// The stored resource was current and reused.
	Current,
	/// The stored resource was reused, and is current if the bake waiting on this check finds itself current: the
	/// resource, or one it is baked from, is stored by that bake, whose own freshness is still being decided.
	Provisional,
	/// The resource was baked and stored.
	Baked,
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
				image_processor: ImageProcessor::default(),
				program_generator: None,
				program_transform: None,
				shader_compiler: Box::new(PlatformShaderCompilerAdapter),
				interceptor: None,
				storage_backend: Box::new(storage_backend),
				resource_storage_backend,
				in_flight_bakes: Arc::new(Mutex::new(HashMap::with_capacity_and_hasher(32, Default::default()))),
				bake_memory_budget: None,
				bakes: BakeTracker::default(),
				roots_in_flight: AtomicUsize::new(0),
				unflushed: AtomicBool::new(false),
				settings: HashMap::from_iter([(SHADER_CODE_SETTING.to_owned(), shader_code_fingerprint())]),
				provenance_lock: Mutex::new(()),
				epoch: AtomicU64::new(0),
				verify_once: false,
				store_generation: AtomicU64::new(0),
				checked: Mutex::new(HashMap::default()),
				stored: Mutex::new(Default::default()),
				sources: Mutex::new(None),
				excluded: Vec::new(),
				discovery_filter: None,
				assembled_resources: Vec::new(),
				dispatcher,
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
		self.configurable_state().bake_memory_budget = Some(Arc::new(BakeMemoryBudget::new(byte_budget.get())));
	}

	/// Reports this manager's bakes to `reporter`: its [`BakeState`](super::bake_state::BakeState) each time a bake starts or ends.
	///
	/// Set it before calling [`Self::bake`] or sharing the manager. The engine sets one in debug builds, which turns
	/// bakes into the `bakes` [debug data stream](/docs/reference/resource-management#bake-stream).
	pub fn set_bake_reporter(&mut self, reporter: Arc<dyn BakeReporter>) {
		self.configurable_state().bakes = BakeTracker::new(reporter);
	}

	/// Declares the fingerprint of a setting that shapes bakes and isn't a file or a stored resource, such as the
	/// identity of a bake service or a store option.
	///
	/// A bake that reads the setting through [`BakeContext::setting`] records the fingerprint, so its resources go stale
	/// when a later manager declares another one. Declare every setting before calling [`Self::bake`] or sharing the
	/// manager. The installers of bake services, such as [`Self::set_program_generator`], declare the service's identity
	/// themselves.
	pub fn declare_setting(&mut self, name: &str, fingerprint: u64) {
		self.configurable_state().settings.insert(name.to_owned(), fingerprint);
	}

	/// Promises that no source file changes while this manager lives, so each resource is checked for freshness once
	/// however many roots use it.
	///
	/// A tool that bakes a project in one run, such as BELD, calls it. A manager that serves requests while sources
	/// are edited must not.
	pub fn verify_once(&mut self) {
		self.configurable_state().verify_once = true;
	}

	/// Sets the patterns of asset IDs that are never baked, usually the project's `assets.exclude`.
	///
	/// Every bake request for a matching ID fails with [`LoadMessages::Excluded`], including a dependency of another
	/// asset. [`Self::discover`] still lists matching assets, so a caller can check them against its own selection
	/// with [`Self::is_excluded`] before baking. Configure it before calling [`Self::bake`] or sharing the manager.
	pub fn set_excluded_assets(&mut self, patterns: Vec<Glob>) {
		self.configurable_state().excluded = patterns;
	}

	/// Sets which assets [`Self::discover`] lists: those for which `filter` returns `true`.
	///
	/// A renderer installs one when its configuration leaves out a feature, so a full bake skips the engine assets
	/// only that feature uses. Unlike [`Self::set_excluded_assets`], an asset the filter rejects still bakes when it is
	/// asked for by ID or another asset needs it. Configure it before calling [`Self::bake`] or sharing the manager.
	pub fn set_discovery_filter(&mut self, filter: impl Fn(&str) -> bool + Send + Sync + 'static) {
		self.configurable_state().discovery_filter = Some(Box::new(filter));
	}

	/// Sets the resources assembled from what the store holds, such as shaders gathered from every baked material.
	///
	/// A tool that bakes a whole project, such as BELD, bakes them with [`Self::bake_if_stale`] after every other bake,
	/// so what they gather is stored. Each one's handler must record the set it gathers as an input, so a resource whose
	/// set didn't change stays current. Configure them before calling [`Self::bake`] or sharing the manager.
	pub fn set_assembled_resources(&mut self, ids: Vec<String>) {
		self.configurable_state().assembled_resources = ids;
	}

	/// Returns the resources set with [`Self::set_assembled_resources`], which a full bake bakes last.
	pub fn assembled_resources(&self) -> &[String] {
		&self.state.assembled_resources
	}

	/// Returns whether a pattern set with [`Self::set_excluded_assets`] matches the asset ID.
	pub fn is_excluded(&self, id: &str) -> bool {
		self.state.is_excluded(id)
	}

	/// Registers a handler for one family of source assets.
	///
	/// After all handlers are registered, install this manager on a
	/// [`crate::ResourceManager`] in a debug build or call [`Self::bake`].
	pub fn add_asset_handler<T: AssetHandler + Send + Sync + 'static>(&mut self, asset_handler: T) {
		let asset_handler: Box<dyn DynAssetHandler> = Box::new(asset_handler);
		// The handler's options are an input of every bake it runs.
		self.declare_setting(&asset_handler.setting_name(), asset_handler.identity());
		self.configurable_state().asset_handlers.push(asset_handler);
	}

	/// Replaces the image processor every handler that produces images bakes with.
	///
	/// A new manager processes images on the CPU. Install a processor with GPU backends here once, instead of on each
	/// image-producing handler, so every source format bakes images the same way.
	pub fn set_image_processor(&mut self, image_processor: ImageProcessor) {
		self.configurable_state().image_processor = image_processor;
	}

	/// Installs the generator every material handler turns BEMX graphs into shader programs with.
	///
	/// Install the renderer's generator here once, so `.bema`, glTF, and FBX materials all target it with one instance.
	/// Without one, materials fail to bake.
	pub fn set_program_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.declare_setting(PROGRAM_GENERATOR_SETTING, generator.identity());
		self.configurable_state().program_generator = Some(Box::new(generator));
	}

	/// Installs the transform every standalone BESL program goes through after its modules attach and before it
	/// links.
	pub fn set_program_transform<T: ProgramTransform + 'static>(&mut self, transform: T) {
		self.declare_setting(PROGRAM_TRANSFORM_SETTING, transform.identity());
		self.configurable_state().program_transform = Some(Box::new(transform));
	}

	/// Replaces the platform compiler every shader bakes with, so tests can bake shaders without a platform toolchain.
	#[cfg(test)]
	pub(crate) fn set_shader_compiler<C: ShaderCompiler + 'static>(&mut self, compiler: C) {
		self.configurable_state().shader_compiler = Box::new(compiler);
	}

	/// Installs the interceptor that receives the intermediate products bakes emit, such as the BESL source of every
	/// shader.
	///
	/// See [`BakeInterceptor`] for how products are named. A bake without an interceptor emits nothing and pays only a
	/// branch per product.
	pub fn set_interceptor(&mut self, interceptor: Arc<dyn BakeInterceptor>) {
		self.configurable_state().interceptor = Some(interceptor);
	}

	/// Returns the state to configure, which only exists while no bake shares it.
	fn configurable_state(&mut self) -> &mut AssetManagerState {
		Arc::get_mut(&mut self.state).expect("The asset manager must be configured before it starts processing requests.")
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
		let asset_type = ResourceId::new(id).get_asset_type();

		self.state.asset_handlers.iter().any(|handler| handler.can_handle(asset_type))
	}

	/// Returns whether recursive discovery should include the given supported source asset.
	pub fn should_discover(&self, id: &str, has_sidecar: bool) -> bool {
		if let Some(filter) = &self.state.discovery_filter
			&& !filter(id)
		{
			return false;
		}

		let id = ResourceId::new(id);
		let asset_type = id.get_asset_type();

		self.state
			.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(asset_type) && handler.should_discover(id, has_sidecar))
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
	/// Its dependencies are still reused when current. To rebuild them too, call [`Self::rebuild`].
	///
	/// Next, await [`crate::ResourceManager::request`] for the stored output or
	/// inspect it through the storage backend.
	pub async fn bake(&self, id: &str) -> Result<(), LoadMessages> {
		self.state
			.dispatch_bake_in_scope(id, BakeMode::Always, BakeOrigin::Root)
			.await
			.map(|_| ())
	}

	/// Bakes the asset at `id` only when its resource is missing or an input of its latest bake changed.
	///
	/// The inputs are everything the bake observed: source files, the resources it was baked from, queries, source
	/// listings, and declared settings. The [freshness reference](/docs/reference/resource-management#freshness) lists
	/// them.
	///
	/// Next, await [`crate::ResourceManager::request`] for the stored output.
	pub async fn bake_if_stale(&self, id: &str) -> Result<(), LoadMessages> {
		self.state
			.dispatch_bake_in_scope(id, BakeMode::IfStale, BakeOrigin::Root)
			.await
			.map(|_| ())
	}

	/// Bakes the asset at `id` and every resource it is baked from, whatever the store holds.
	///
	/// Each resource is rebuilt once per manager, so roots that share dependencies don't rebuild them twice. Nothing
	/// outside the root's dependencies is rebuilt.
	///
	/// Next, await [`crate::ResourceManager::request`] for the stored output.
	pub async fn rebuild(&self, id: &str) -> Result<(), LoadMessages> {
		self.state
			.dispatch_bake_in_scope(id, BakeMode::Forced, BakeOrigin::Root)
			.await
			.map(|_| ())
	}

	/// Returns the stored asset, after baking it when it is missing or stale; see [`Self::bake_if_stale`].
	pub async fn bake_if_not_exists<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadMessages> {
		self.bake_if_not_exists_serialized(id).await.map(Into::into)
	}

	/// Returns the stored resource after ensuring its complete source provenance is current.
	pub(crate) async fn bake_if_not_exists_serialized(&self, id: &str) -> Result<crate::SerializableResource, LoadMessages> {
		self.bake_if_stale(id).await?;

		self.state
			.resource_storage_backend
			.read(ResourceId::new(id))
			.await
			.map(|(resource, _)| resource)
			.ok_or(LoadMessages::NoAsset)
	}

	/// Adds a requested root resource to the ones checked again when source files change.
	#[cfg(debug_assertions)]
	pub(crate) fn track_resource(&self, id: &str) {
		let mut hot_reload = self.state.hot_reload.lock();

		if !hot_reload.tracked.contains(id) {
			hot_reload.tracked.insert(id.to_owned());
		}
	}

	/// Starts recursive debounced watching when the source backend exposes a local root.
	#[cfg(debug_assertions)]
	pub(crate) fn start_watching(&self, updates: Arc<crate::resource::resource_manager::ResourceUpdateBroadcaster>) {
		let roots = self.state.storage_backend.watch_roots();
		if roots.is_empty() {
			return;
		}

		let weak = Arc::downgrade(&self.state);

		let mut watcher = match notify_debouncer_full::new_debouncer(
			std::time::Duration::from_millis(250),
			None,
			move |result: notify_debouncer_full::DebounceEventResult| match result {
				Ok(_) => {
					let Some(state) = weak.upgrade() else { return };

					state.reload_changed();
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

		for root in &roots {
			if let Err(error) = watcher.watch(root, notify_debouncer_full::notify::RecursiveMode::Recursive) {
				log::warn!(
					"Asset watching could not watch '{}'. The most likely cause is that the directory is inaccessible: {error}",
					root.display()
				);

				return;
			}
		}

		let mut hot_reload = self.state.hot_reload.lock();

		hot_reload.updates = Some(updates);

		hot_reload.watcher = Some(watcher);
	}
}

enum InFlightBakeRole {
	/// The caller runs the bake, announces its result, and drops the cleanup guard once it finishes.
	Leader(
		announcement::Announcer<Result<BakeOutcome, LoadMessages>>,
		InFlightBakeCleanup,
	),
	Follower(announcement::Listener<Result<BakeOutcome, LoadMessages>>),
}

/// The `InFlightBakeCleanup` struct removes a leader entry when its future completes, is canceled, or unwinds.
struct InFlightBakeCleanup {
	registry: Arc<Mutex<HashMap<String, announcement::Announcement<Result<BakeOutcome, LoadMessages>>>>>,
	id: String,
}

impl Drop for InFlightBakeCleanup {
	fn drop(&mut self) {
		self.registry.lock().remove(&self.id);
	}
}

#[cfg(debug_assertions)]
#[derive(Default)]
struct HotReloadState {
	watcher: Option<
		notify_debouncer_full::Debouncer<
			notify_debouncer_full::notify::RecommendedWatcher,
			notify_debouncer_full::RecommendedCache,
		>,
	>,
	/// The roots requested so far, each checked again when source files change.
	tracked: std::collections::HashSet<String>,
	/// Each root being checked; a root missing from the map is idle.
	reloads: HashMap<String, RootReload>,
	updates: Option<Arc<crate::resource::resource_manager::ResourceUpdateBroadcaster>>,
}

/// The `RootReload` enum is the phase of one root's hot-reload rebake.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RootReload {
	/// One rebake runs and no source changed since it started.
	Running,
	/// A source changed while the rebake ran, so another rebake follows it.
	Rerun,
}

/// The `LoadMessages` enum identifies failures while an asset is loaded, baked, or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadMessages {
	/// The asset was not found in the storage backend.
	NoAsset,
	/// No asset handler was found for the asset.
	NoAssetHandler,
	/// The asset or one of its dependencies could not be baked or loaded.
	FailedToBake { asset: String, error: LoadErrors },
	/// The asset matches an excluded pattern, so it is never baked. See [`AssetManager::set_excluded_assets`].
	Excluded { asset: String },
	/// The asset could not be stored in the resource storage backend.
	FailedToStore { asset: String, error: String },
	/// No asset worker was available to execute the request.
	ExecutionUnavailable,
}

impl AssetManagerState {
	/// Marks every tracked root for one freshness check after source files changed.
	///
	/// A watcher batch only says that something changed. Which resources that makes stale is decided by the same
	/// check every request runs, so no path has to be matched to an asset.
	#[cfg(debug_assertions)]
	fn reload_changed(self: &Arc<Self>) {
		// The checks made before the change no longer hold.
		self.epoch.fetch_add(1, Ordering::AcqRel);

		let roots = {
			let mut hot_reload = self.hot_reload.lock();
			let hot_reload = &mut *hot_reload;

			hot_reload
				.tracked
				.iter()
				.filter(|root| {
					// A root already being checked is checked again once it finishes instead of twice at once.
					if let Some(reload) = hot_reload.reloads.get_mut(*root) {
						*reload = RootReload::Rerun;

						false
					} else {
						hot_reload.reloads.insert((*root).clone(), RootReload::Running);

						true
					}
				})
				.cloned()
				.collect::<Vec<_>>()
		};

		for root in roots {
			self.dispatch_reload(root);
		}
	}

	/// Checks `root` on a worker, and returns the root to idle when no worker accepts the job.
	#[cfg(debug_assertions)]
	fn dispatch_reload(self: &Arc<Self>, root: String) {
		let state = Arc::clone(self);

		let id = root.clone();

		if self
			.dispatcher
			.dispatch(move || async move { state.reload_resource(id).await })
			.is_err()
		{
			self.hot_reload.lock().reloads.remove(&root);
		}
	}

	/// Makes one tracked root current and publishes it only when that stored it again.
	#[cfg(debug_assertions)]
	async fn reload_resource(self: Arc<Self>, id: String) {
		// Enter the shared bake registry so a resource several changed roots use is rebuilt once.
		let rebaked = matches!(
			self.dispatch_bake_in_scope(&id, BakeMode::IfStale, BakeOrigin::Recheck).await,
			Ok(BakeOutcome::Baked)
		);

		if rebaked && let Some((resource, _)) = self.resource_storage_backend.read(ResourceId::new(&id)).await {
			let update = crate::resource::resource_manager::ResourceUpdate::new(id.clone(), resource.class().to_string());

			if let Some(updates) = self.hot_reload.lock().updates.clone() {
				updates.send(update);
			}
		}

		// The root stays running across the rerun's dispatch, so a change in between marks it again instead of starting a
		// second check.
		let rerun = {
			let mut hot_reload = self.hot_reload.lock();

			match hot_reload.reloads.get(&id) {
				Some(RootReload::Rerun) => {
					hot_reload.reloads.insert(id.clone(), RootReload::Running);

					true
				}
				Some(RootReload::Running) | None => {
					hot_reload.reloads.remove(&id);

					false
				}
			}
		};

		if rerun {
			self.dispatch_reload(id);
		}
	}

	/// Runs a bake with an explicit owner, inheriting the root memory scope for dependencies.
	pub(super) async fn dispatch_bake_in_scope(
		self: &Arc<Self>,
		id: &str,
		mode: BakeMode,
		origin: BakeOrigin,
	) -> Result<BakeOutcome, LoadMessages> {
		// An excluded asset fails before any freshness check, so even a stored resource is never rebaked or reused.
		if self.is_excluded(id) {
			let message = format!(
				"Asset '{id}' is excluded from baking. The most likely cause is that a `assets.exclude` pattern matches it while an application or another asset still uses it. Stop using the asset, or remove its pattern from `assets.exclude`. See {}.",
				online_docs_url(EXCLUDED_ASSETS_DOCS_PATH)
			);

			log::warn!("{message}");

			#[cfg(debug_assertions)]
			{
				let id = ResourceId::new(id);

				self.resource_trace.clear(id);

				self.resource_trace.record(id, ResourceTraceLevel::Error, message);

				self.persist_resource_trace(id);
			}

			return Err(LoadMessages::Excluded { asset: id.to_owned() });
		}

		// Waiting on a bake that waits on this one would never end.
		if origin.waits_on(id) {
			log::warn!(
				"Asset '{id}' depends on itself through another asset. The most likely cause is a reference that leads back to it, such as a material that names itself as its parent. Remove one reference of the cycle."
			);

			return Err(LoadMessages::FailedToBake {
				asset: id.to_owned(),
				error: LoadErrors::DependencyCycle,
			});
		}

		// Bind the listener first so the registry lock is released before waiting on the leader.
		let listener = self.in_flight_bakes.lock().get(id).map(announcement::Announcement::listener);

		if let Some(notification) = listener {
			return notification.listen().await.map_err(|_| LoadMessages::ExecutionUnavailable)?;
		}

		let is_root = matches!(origin, BakeOrigin::Root | BakeOrigin::Recheck);

		// Without a caller's promise that sources hold still, each root request starts from fresh checks. A running
		// watcher is no such promise: it only brings a check forward, so the next request finds a change it missed.
		if matches!(origin, BakeOrigin::Root) && !self.verify_once {
			self.epoch.fetch_add(1, Ordering::AcqRel);
		}

		// Counting starts before admission, so roots still waiting for memory keep the flush from running early.
		let root = is_root.then(|| RootInFlight::enter(self));
		let result = self.admit_and_bake(id, mode, origin).await;

		// The root that leaves last returns once the flush is done, so its caller finds every stored resource durable.
		if let Some(flushed) = root.and_then(RootInFlight::leave) {
			let _ = flushed.await;
		}

		result
	}

	/// Admits a bake and runs it as the leader, or waits for the leader that already runs it.
	async fn admit_and_bake(
		self: &Arc<Self>,
		id: &str,
		mode: BakeMode,
		origin: BakeOrigin,
	) -> Result<BakeOutcome, LoadMessages> {
		let is_root = matches!(origin, BakeOrigin::Root | BakeOrigin::Recheck);

		// Independent roots wait before becoming leaders. This lets an admitted parent claim and run a dependency
		// instead of following a root that cannot start until the parent releases memory.
		let (memory_scope, mut ancestors) = match (origin, &self.bake_memory_budget) {
			(BakeOrigin::Dependency { memory_scope, ancestors }, _) => (memory_scope, ancestors),
			(BakeOrigin::Root | BakeOrigin::Recheck, Some(memory_budget)) => (Some(memory_budget.acquire().await), Vec::new()),
			(BakeOrigin::Root | BakeOrigin::Recheck, None) => (None, Vec::new()),
		};
		// Everything this bake asks for must not wait on it.
		ancestors.push(id.to_owned());

		let (notification, registry_cleanup) = match self.register_bake(id) {
			InFlightBakeRole::Leader(notification, registry_cleanup) => (notification, registry_cleanup),
			InFlightBakeRole::Follower(notification) => {
				drop(memory_scope);

				return notification.listen().await.map_err(|_| LoadMessages::ExecutionUnavailable)?;
			}
		};

		let id = id.to_owned();

		// Only the leader needs an owned reference for the queued worker future.
		let state = Arc::clone(self);
		let task = self
			.dispatcher
			.dispatch(move || async move {
				// The future owns cleanup before its first poll, so queued cancellation cannot leave a closed registry entry.
				let _registry_cleanup = registry_cleanup;

				// Dependencies inherit their admitted root scope and use separate arenas without another admission wait.
				let allocator = BakeAllocator::in_scope(memory_scope);

				let result = state.ensure_baked_uncoalesced(&id, mode, &ancestors, &allocator).await;

				// A root that reused a current resource wrote nothing, so it doesn't ask for the flush.
				if is_root && matches!(result, Ok(BakeOutcome::Baked) | Err(_)) {
					state.unflushed.store(true, Ordering::Release);
				}

				let _ = notification.announce(result.clone());

				result
			})
			.map_err(|_| LoadMessages::ExecutionUnavailable)?;

		task.await.map_err(|_| LoadMessages::ExecutionUnavailable)?
	}

	/// Makes every resource stored so far durable with one flush, instead of one flush per stored resource.
	///
	/// The flush runs on an asset worker, because it does file I/O through the workers' runtime while a root's caller
	/// may wait on any executor. Await the returned receiver to wait for it; it's `None` once the workers stopped.
	/// A failure leaves this bake's resources usable in this process; they may only be lost if the process then crashes.
	/// A crash before the flush loses only resources that the next bake rebuilds, because storage rolls back to its
	/// last durable state.
	fn flush(self: &Arc<Self>) -> Option<utils::r#async::oneshot::Receiver<()>> {
		let state = Arc::clone(self);
		self.dispatcher
			.dispatch(move || async move {
				if let Err(error) = state.resource_storage_backend.persist().await {
					log::warn!("{error}");
				}
			})
			.ok()
	}

	/// Returns whether an excluded pattern matches the asset ID, or the asset a fragment of it belongs to.
	pub(super) fn is_excluded(&self, id: &str) -> bool {
		let base = asset::resource_id::get_base(id).unwrap_or(id);

		self.excluded
			.iter()
			.any(|pattern| pattern.matches(id) || pattern.matches(base))
	}

	/// Returns the fingerprint of the setting `name`: the store's own for [`STORE_SETTING`], else the declared one, or
	/// `0` when nobody declared it.
	pub(in crate::asset) fn setting(&self, name: &str) -> u64 {
		if name == STORE_SETTING {
			return self.resource_storage_backend.settings_fingerprint();
		}

		self.settings.get(name).copied().unwrap_or(0)
	}

	/// Returns the fingerprint of the toolchain shaders compile with, or `0` when it can't be identified.
	///
	/// It isn't declared like other settings because identifying the toolchain may run it, which a bake that compiles
	/// no shader shouldn't wait for.
	pub(in crate::asset) async fn shader_compiler_fingerprint(&self) -> u64 {
		use std::hash::Hasher as _;

		let Ok(identity) = self.shader_compiler.identity().await else {
			return 0;
		};
		let mut hasher = crate::asset::provenance::fingerprint_hasher();
		hasher.write(identity.as_bytes());

		// Zero is reserved for an unidentified toolchain.
		hasher.finish().max(1)
	}

	/// Returns whether this manager stored the resource `id` since it was created.
	pub(in crate::asset) fn stored_in_process(&self, id: &str) -> bool {
		self.stored.lock().contains(id)
	}

	/// Records that this manager stored the resource `id`, so a forced rebuild builds it only once.
	pub(in crate::asset) fn mark_stored_in_process(&self, id: &str) {
		self.stored.lock().insert(id.to_owned());
		self.store_generation.fetch_add(1, Ordering::AcqRel);
	}

	/// Returns the IDs of the stored resources `query` matches, sorted, and the fingerprint of the match set, after
	/// making each match current.
	///
	/// A match that fails to bake stays in the list with the fingerprint of a missing resource. A match its bake no
	/// longer stores, and a resource of an excluded asset, are left out.
	pub(in crate::asset) async fn query_matches(
		self: &Arc<Self>,
		query: &crate::resource::storage_backend::Query,
		origin: BakeOrigin,
	) -> Result<(Vec<String>, u64), ()> {
		use utils::r#async::StreamExt as _;

		let page = self.resource_storage_backend.query(query.clone()).await.map_err(|_| ())?;
		let mut ids = page
			.items
			.into_iter()
			.map(|(stored, _)| stored.id().to_owned())
			// The bake that asks never matches itself: making itself current would wait on its own bake.
			.filter(|id| !self.is_excluded(id) && !origin.waits_on(id))
			.collect::<Vec<_>>();
		ids.sort_unstable();

		let checks = ids.into_iter().map(|id| {
			let origin = origin.clone();

			async move {
				// The dispatch is boxed because a freshness check that runs a query re-enters the dispatcher.
				let baked = Box::pin(self.dispatch_bake_in_scope(&id, BakeMode::IfStale, origin)).await;
				let stored = self.resource_storage_backend.read(ResourceId::new(&id)).await;

				match (baked, stored) {
					(Ok(_), Some((resource, _))) => Some((id, resource_fingerprint(&resource))),
					(Ok(_), None) => None,
					(Err(_), _) => Some((id, MISSING_RESOURCE)),
				}
			}
		});

		let matches = utils::r#async::stream::iter(checks)
			.buffered(INPUT_CHECK_CONCURRENCY)
			.filter_map(|entry| async move { entry })
			.collect::<Vec<_>>()
			.await;

		let fingerprint = set_fingerprint(matches.iter().map(|(id, fingerprint)| (id.as_str(), *fingerprint)));

		Ok((matches.into_iter().map(|(id, _)| id).collect(), fingerprint))
	}

	/// Returns the IDs of the source assets the glob `pattern` matches, sorted, and the fingerprint of that list.
	pub(in crate::asset) async fn source_listing(&self, pattern: &str) -> Result<(Vec<String>, u64), ()> {
		let epoch = self.epoch.load(Ordering::Acquire);

		let cached = self
			.sources
			.lock()
			.as_ref()
			.filter(|(listed_at, _)| *listed_at == epoch)
			.map(|(_, sources)| Arc::clone(sources));

		// Walking the asset directory is costly, so every listing between two source changes shares one walk.
		let sources = match cached {
			Some(sources) => sources,
			None => {
				let mut sources = self
					.storage_backend
					.discover()
					.await
					.map_err(|_| ())?
					.into_iter()
					.map(crate::asset::AssetSource::into_id)
					.collect::<Vec<_>>();
				sources.sort_unstable();

				let sources = Arc::new(sources);
				*self.sources.lock() = Some((epoch, Arc::clone(&sources)));

				sources
			}
		};

		let pattern = Glob::new(pattern);
		let ids = sources
			.iter()
			.filter(|source| pattern.matches(source))
			.cloned()
			.collect::<Vec<_>>();
		let fingerprint = set_fingerprint(ids.iter().map(|id| (id.as_str(), 0)));

		Ok((ids, fingerprint))
	}

	/// Registers one requested resource before it is submitted to a worker.
	fn register_bake(&self, id: &str) -> InFlightBakeRole {
		let mut registry = self.in_flight_bakes.lock();

		match registry.entry(id.to_owned()) {
			Occupied(entry) => InFlightBakeRole::Follower(entry.get().listener()),
			Vacant(entry) => {
				let (announcer, announcement) = announcement::Announcement::new();

				entry.insert(announcement);

				let registry_cleanup = InFlightBakeCleanup {
					registry: Arc::clone(&self.in_flight_bakes),
					id: id.to_owned(),
				};

				InFlightBakeRole::Leader(announcer, registry_cleanup)
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

	/// Reads what the store remembers about how `id` came to be stored. A record that can't be read counts as none,
	/// so the resource bakes again.
	fn read_provenance(&self, id: &str) -> Option<Provenance> {
		match self.resource_storage_backend.read_provenance(ResourceId::new(id)) {
			Ok(provenance) => provenance,
			Err(error) => {
				log::warn!("{error}");

				None
			}
		}
	}

	/// Writes what the store remembers about how `id` came to be stored.
	fn write_provenance(&self, id: &str, provenance: &Provenance) {
		if let Err(error) = self
			.resource_storage_backend
			.write_provenance(ResourceId::new(id), provenance)
		{
			log::warn!("{error}");
		}
	}

	/// Marks the bake of `id` as unfinished before its handler runs, so a bake that fails or is interrupted after
	/// storing some of its resources leaves `id` stale instead of a mix that counts as current.
	fn begin_provenance(&self, id: &str) {
		let _guard = self.provenance_lock.lock();

		if let Some(mut provenance) = self.read_provenance(id)
			&& provenance.inputs.take().is_some()
		{
			self.write_provenance(id, &provenance);
		}
	}

	/// Records the finished bake of `id`: what it observed and which other resources it stored. Returns the resources
	/// the previous bake of `id` stored that no bake stores anymore, for the caller to remove.
	fn commit_provenance(&self, id: &str, inputs: Vec<BakeInput>, outputs: Vec<String>) -> Vec<String> {
		// Several bakes can store the same resource, such as a skeleton a mesh and its clips share, so their records of
		// who stores it change under one lock.
		let _guard = self.provenance_lock.lock();

		let previous = self.read_provenance(id).unwrap_or_default();
		let mut orphans = Vec::new();

		for output in outputs.iter().filter(|output| !previous.outputs.contains(output)) {
			let mut record = self.read_provenance(output).unwrap_or_default();

			if let Err(index) = record.producers.binary_search_by(|producer| producer.as_str().cmp(id)) {
				record.producers.insert(index, id.to_owned());

				self.write_provenance(output, &record);
			}
		}

		for output in previous.outputs.iter().filter(|output| !outputs.contains(output)) {
			let mut record = self.read_provenance(output).unwrap_or_default();

			record.producers.retain(|producer| producer != id);

			// A resource that was also baked by request keeps its own record and stays.
			if record.producers.is_empty() && record.inputs.is_none() {
				orphans.push(output.clone());
			} else {
				self.write_provenance(output, &record);
			}
		}

		self.write_provenance(
			id,
			&Provenance {
				inputs: Some(inputs),
				outputs,
				producers: previous.producers,
			},
		);

		orphans
	}

	/// Runs one asset handler invocation without consulting the in-flight registry.
	///
	/// Call this method directly when no coalescing is desired.
	async fn bake_uncoalesced(
		self: &Arc<Self>,
		id: &str,
		forced: bool,
		ancestors: &[String],
		allocator: &BakeAllocator,
	) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		#[cfg(debug_assertions)]
		{
			self.resource_trace.clear(id);

			self.persist_resource_trace(id);
		}

		let asset_type = id.get_asset_type();

		let Some(asset_handler) = self.asset_handlers.iter().find(|handler| handler.can_handle(asset_type)) else {
			let message = format!(
				"No asset handler found for '{}'. The most likely cause is an unsupported file extension or missing handler registration. See {}.",
				id.as_ref(),
				online_docs_url(ASSETS_DOCS_PATH)
			);

			log::warn!("{message}");

			#[cfg(debug_assertions)]
			{
				self.resource_trace.record(id, ResourceTraceLevel::Error, message);

				self.persist_resource_trace(id);
			}

			return Err(LoadMessages::NoAssetHandler);
		};

		let start_time = std::time::Instant::now();
		// Every return from here on is the end of a bake; the run records it as failed unless it succeeded below.
		// `ancestors` ends with `id`; the rest are the IDs waiting on this bake.
		let run = self.bakes.begin(id.as_ref(), &ancestors[..ancestors.len().saturating_sub(1)]);

		self.begin_provenance(id.as_ref());

		// The shared flag enforces the primary-write contract without rereading potentially expensive storage.
		// TODO: replace the flag when storage can confirm the primary write without a second read.
		let primary_stored = Cell::new(false);

		// Everything the handler observes during this invocation becomes the provenance of what it stores.
		let inputs = Arc::new(Mutex::new(Vec::new()));
		let outputs = Mutex::new(Vec::new());

		let tracking_storage_backend = TrackingStorageBackend {
			inner: self.storage_backend.as_ref(),
			inputs: &inputs,
		};

		let context = BakeContext {
			asset_manager: self,
			asset_storage_backend: &tracking_storage_backend,
			allocator,
			primary_id: id,
			primary_stored: &primary_stored,
			outputs: &outputs,
			ancestors,
			forced,
		};

		context.setting(&asset_handler.setting_name());

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

		let outputs = outputs.into_inner();
		let mut inputs = std::mem::take(&mut *inputs.lock());
		// What the bake read back of its own resources is no input: it is the result.
		inputs.retain(
			|input| !matches!(input, BakeInput::Resource { id: read, .. } if read == id.as_ref() || outputs.contains(read)),
		);

		// The record is written last, so only a bake that stored everything counts as current.
		let orphans = self.commit_provenance(id.as_ref(), inputs, outputs);

		for orphan in orphans {
			log::info!(
				"Removing resource '{orphan}'. The latest bake of '{}' no longer stores it.",
				id.as_ref()
			);

			if let Err(error) = self.resource_storage_backend.delete(ResourceId::new(&orphan)).await {
				log::warn!("{error}");
			}
			self.store_generation.fetch_add(1, Ordering::AcqRel);
		}

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());
		run.succeeded();

		Ok(())
	}

	/// Makes `id` current, baking what the mode and the freshness check call for, without consulting the in-flight
	/// registry.
	///
	/// `ancestors` are the IDs waiting on this one, outermost first and ending with `id`.
	async fn ensure_baked_uncoalesced(
		self: &Arc<Self>,
		id: &str,
		mode: BakeMode,
		ancestors: &[String],
		allocator: &BakeAllocator,
	) -> Result<BakeOutcome, LoadMessages> {
		let epoch = self.epoch.load(Ordering::Acquire);

		// A forced rebuild builds each resource once per process, then treats it like any other.
		let forced = mode == BakeMode::Forced && !self.stored_in_process(id);
		let check = mode == BakeMode::IfStale || (mode == BakeMode::Forced && !forced);

		if check
			&& let Some((checked_at, store_generation, result)) = self.checked.lock().get(id)
			&& *checked_at == epoch
			&& store_generation.is_none_or(|generation| generation == self.store_generation.load(Ordering::Acquire))
		{
			return result.clone().map(|()| BakeOutcome::Current);
		}

		let result = self
			.ensure_baked_unchecked(id, mode, forced, check, ancestors, allocator)
			.await;

		// A provisional answer depends on a bake that hasn't decided its own freshness, so it isn't remembered.
		if matches!(result, Ok(BakeOutcome::Provisional)) {
			return result;
		}

		// A bake that ran a query observed the store, so its check only holds while the store holds the same resources.
		// The generation is read after the bake, which counts what the bake itself stored.
		let observes_store = self
			.read_provenance(id)
			.as_ref()
			.and_then(Provenance::inputs)
			.is_some_and(|inputs| inputs.iter().any(|input| matches!(input, BakeInput::Query { .. })));
		let store_generation = observes_store.then(|| self.store_generation.load(Ordering::Acquire));

		self.checked
			.lock()
			.insert(id.to_owned(), (epoch, store_generation, result.clone().map(|_| ())));

		result
	}

	/// Does the work of [`Self::ensure_baked_uncoalesced`] once no earlier check of `id` in this epoch answers it.
	async fn ensure_baked_unchecked(
		self: &Arc<Self>,
		id: &str,
		mode: BakeMode,
		forced: bool,
		check: bool,
		ancestors: &[String],
		allocator: &BakeAllocator,
	) -> Result<BakeOutcome, LoadMessages> {
		let provenance = self.read_provenance(id);
		let origin = || BakeOrigin::Dependency {
			memory_scope: allocator.memory_scope().cloned(),
			ancestors: ancestors.to_vec(),
		};

		// A resource that only another ID's bake stores is made current by that bake: its own ID may not be enough to
		// rebuild it, such as a material a model takes from one of its detail-level files.
		if let Some(provenance) = &provenance
			&& provenance.inputs.is_none()
			&& !provenance.producers.is_empty()
		{
			let stored = async || self.resource_storage_backend.read(ResourceId::new(id)).await.is_some();

			match provenance.producers.iter().find(|producer| !ancestors.contains(producer)) {
				Some(producer) => {
					// The dispatch is boxed because the producer's bake re-enters this function for its own dependencies.
					let outcome = Box::pin(self.dispatch_bake_in_scope(producer, mode, origin())).await?;

					if stored().await {
						return Ok(outcome);
					}
				}
				// Every producer waits on this check, so what it stored stands until its own freshness is decided. If it
				// then bakes, it stores the resource again and asks for whatever is baked from it again.
				None if stored().await => return Ok(BakeOutcome::Provisional),
				None => {}
			}
			// No producer stores the resource anymore, so only a bake of the ID itself can.
		}

		if check
			&& let Some(inputs) = provenance.as_ref().and_then(Provenance::inputs)
			&& self.resource_storage_backend.read(ResourceId::new(id)).await.is_some()
		{
			match self.first_changed_input(inputs, origin).await {
				Ok(outcome) => return Ok(outcome),
				Err(changed) => log::info!(
					"Re-baking stale asset '{id}'. Its input '{changed}' changed since the stored resource was produced."
				),
			}
		}

		self.bake_uncoalesced(id, forced, ancestors, allocator)
			.await
			.map(|()| BakeOutcome::Baked)
	}

	/// Returns how the inputs of a bake stand: current, provisionally current, or the key of the first input found to
	/// have changed. `origin` builds the origin of the checks that make the resources an input names current.
	async fn first_changed_input(
		self: &Arc<Self>,
		inputs: &[BakeInput],
		origin: impl Fn() -> BakeOrigin,
	) -> Result<BakeOutcome, String> {
		use utils::r#async::StreamExt as _;

		let checks = inputs.iter().map(|input| {
			let origin = origin();

			async move { (input, self.input_changed(input, origin).await) }
		});

		// Inputs are independent; bound the pressure on metadata and workers, and stop after the first change.
		let mut checks = std::pin::pin!(utils::r#async::stream::iter(checks).buffer_unordered(INPUT_CHECK_CONCURRENCY));

		let mut outcome = BakeOutcome::Current;
		while let Some((input, check)) = checks.next().await {
			match check {
				InputCheck::Unchanged => {}
				InputCheck::Provisional => outcome = BakeOutcome::Provisional,
				InputCheck::Changed => return Err(input.key()),
			}
		}

		Ok(outcome)
	}

	/// Returns whether one recorded input still has the fingerprint the bake recorded.
	///
	/// Checking an input that names stored resources first makes those resources current, which may bake them.
	async fn input_changed(self: &Arc<Self>, input: &BakeInput, origin: BakeOrigin) -> InputCheck {
		let changed = match input {
			BakeInput::Source { id, version } => source_changed(id, version, self.storage_backend.as_ref()).await,
			BakeInput::Resource { id, fingerprint } => {
				// The dispatch is boxed because the dependency's own check re-enters this function.
				let baked = Box::pin(self.dispatch_bake_in_scope(id, BakeMode::IfStale, origin)).await;
				let current = match baked {
					Ok(_) => self
						.resource_storage_backend
						.read(ResourceId::new(id))
						.await
						.map_or(MISSING_RESOURCE, |(resource, _)| resource_fingerprint(&resource)),
					Err(_) => MISSING_RESOURCE,
				};

				if current == *fingerprint && baked == Ok(BakeOutcome::Provisional) {
					return InputCheck::Provisional;
				}

				current != *fingerprint
			}
			BakeInput::Query {
				class,
				predicates,
				fingerprint,
			} => {
				let query = BakeInput::recorded_query(class, predicates);

				!matches!(self.query_matches(&query, origin).await, Ok((_, current)) if current == *fingerprint)
			}
			BakeInput::Listing { pattern, fingerprint } => {
				!matches!(self.source_listing(pattern).await, Ok((_, current)) if current == *fingerprint)
			}
			BakeInput::Setting { name, fingerprint } if name == SHADER_COMPILER_SETTING => {
				self.shader_compiler_fingerprint().await != *fingerprint
			}
			BakeInput::Setting { name, fingerprint } => self.setting(name) != *fingerprint,
		};

		if changed { InputCheck::Changed } else { InputCheck::Unchanged }
	}
}

/// The `InputCheck` enum says how one recorded input of a bake stands now.
enum InputCheck {
	Unchanged,
	/// Unchanged if the bake waiting on this check finds itself current; see [`BakeOutcome::Provisional`].
	Provisional,
	Changed,
}

/// Returns the IDs of the source files a bake recorded that changed or no longer exist.
///
/// A resource is stale when any input of its bake changed; this checks the source files among them, which a tool can
/// do without the bake's settings. Pass the provenance of the resource, from
/// [`ReadStorageBackend::read_provenance`](crate::resource::ReadStorageBackend::read_provenance), and the asset storage
/// backend of the source directory it was baked from, such as a
/// [`FileStorageBackend`](crate::asset::FileStorageBackend). The IDs keep the order of [`Provenance::inputs`].
pub async fn stale_sources<'a>(provenance: &'a Provenance, storage_backend: &dyn DynStorageBackend) -> Vec<&'a str> {
	use utils::r#async::StreamExt as _;

	let checks = provenance
		.inputs()
		.unwrap_or_default()
		.iter()
		.filter_map(|input| Some((input.source_id()?, input.source_version()?)))
		.map(|(id, version)| async move { source_changed(id, version, storage_backend).await.then_some(id) });

	utils::r#async::stream::iter(checks)
		.buffered(INPUT_CHECK_CONCURRENCY)
		.filter_map(|stale| async move { stale })
		.collect()
		.await
}

/// Returns whether the source file `id` differs from the `recorded` version of it.
///
/// The file's content is only read when its metadata changed and its size didn't.
async fn source_changed(id: &str, recorded: &AssetVersion, storage_backend: &dyn DynStorageBackend) -> bool {
	let id = ResourceId::new(id);

	let Ok(current) = storage_backend.version(id).await else {
		return true;
	};

	match recorded.compare(&current) {
		VersionChange::Unchanged => false,
		VersionChange::Changed => true,
		VersionChange::ContentDecides(recorded) => storage_backend.content_hash(id).await != Ok(recorded),
	}
}

/// Returns the fingerprint of the BESL code this crate was built with.
fn shader_code_fingerprint() -> u64 {
	use std::hash::Hasher as _;

	let mut hasher = crate::asset::provenance::fingerprint_hasher();
	hasher.write(env!("BESL_CODE_HASH").as_bytes());
	hasher.finish()
}

/// Bounds how many inputs one freshness check examines at once.
const INPUT_CHECK_CONCURRENCY: usize = 8;

const ASSETS_DOCS_PATH: &str = "reference/assets";
const EXCLUDED_ASSETS_DOCS_PATH: &str = "reference/assets#excluded-assets";

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
		resource::{ReadStorageBackend, storage_backend::MemoryStorageBackend},
	};

	#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct TestResource {}

	impl Model for TestResource {
		fn get_class() -> &'static str {
			"TestResource"
		}
	}

	struct TestAssetHandler;
	struct CompoundBeadAssetHandler;

	/// Builds a manager with [`TestAssetHandler`] over `storage` and returns the resource store it writes to.
	fn test_asset_manager(storage: TestStorageBackend) -> (AssetManager, MemoryStorageBackend) {
		let resource_storage = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage, resource_storage.clone());
		asset_manager.add_asset_handler(TestAssetHandler);
		(asset_manager, resource_storage)
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
			context.load_sidecar(id).await?;

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
		resource_storage: MemoryStorageBackend,
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

		let (started, started_listeners): (Vec<_>, Vec<_>) = (0..8)
			.map(|_| {
				let (announcer, announcement) = announcement::Announcement::new();

				(Mutex::new(Some(announcer)), announcement.listener())
			})
			.unzip();

		let started = Arc::new(started);

		let (release, release_announcement) = announcement::Announcement::new();

		let mut manager = AssetManager::new(TestStorageBackend::new(), MemoryStorageBackend::new());

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
			let (source, asset_type) = context.resolve(id).await?;

			if asset_type != "environment.bead" {
				return Err(LoadErrors::UnsupportedType);
			}

			context.store_primary(ProcessedAsset::new(id, TestResource {}), &source).await
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		AssetManager::new(TestStorageBackend::new(), MemoryStorageBackend::new())
	}

	/// Releases the blocked test handler once `started` reports that the awaited invocation began.
	async fn release_once_started(started: &announcement::Listener<()>, release: announcement::Announcer<()>) {
		started.listen().await.expect("the awaited invocation should start");

		release.announce(()).expect("release should be announced once");
	}

	#[r#async::test]
	async fn discovery_filters_backend_sources_through_registered_handlers_and_sorts_ids() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("z-last.test", b"");
		storage_backend.add_file("nested/a-first.test", b"");
		storage_backend.add_file("nested/a-first.test.bead", b"{}");
		storage_backend.add_file("ignored.unknown", b"");

		let (asset_manager, _) = test_asset_manager(storage_backend);

		assert_eq!(
			asset_manager.discover().await.unwrap(),
			["nested/a-first.test", "z-last.test"]
		);
	}

	#[r#async::test]
	async fn excluded_assets_never_bake() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("kept.test", b"");
		storage_backend.add_file("wip/draft.test", b"");

		let (mut asset_manager, resource_storage) = test_asset_manager(storage_backend);
		asset_manager.set_excluded_assets(vec![Glob::new("WIP/*")]);

		assert!(asset_manager.is_excluded("wip/draft.test") && !asset_manager.is_excluded("kept.test"));
		assert!(matches!(
			asset_manager.bake("wip/draft.test").await,
			Err(LoadMessages::Excluded { asset }) if asset == "wip/draft.test"
		));
		assert!(resource_storage.read(ResourceId::new("wip/draft.test")).await.is_none());
	}

	#[r#async::test]
	async fn an_asset_that_needs_an_excluded_asset_fails_to_bake() {
		let asset_storage = TestStorageBackend::new();
		asset_storage.add_file("parent.test", b"parent");
		asset_storage.add_file("child.test", b"child");

		let resource_storage = MemoryStorageBackend::new();
		let (mut asset_manager, _) = versioned_asset_manager(asset_storage, resource_storage.clone());
		asset_manager.set_excluded_assets(vec![Glob::new("child.*")]);

		assert!(matches!(
			asset_manager.bake("parent.test").await,
			Err(LoadMessages::FailedToBake {
				error: LoadErrors::DependencyExcluded,
				..
			})
		));
		assert!(resource_storage.read(ResourceId::new("child.test")).await.is_none());
	}

	#[r#async::test]
	async fn excluding_an_asset_stops_reusing_its_stored_resource() {
		let asset_storage = TestStorageBackend::new();
		asset_storage.add_file("kept.test", b"kept");

		let resource_storage = MemoryStorageBackend::new();
		let (asset_manager, _) = versioned_asset_manager(asset_storage.clone(), resource_storage.clone());
		asset_manager
			.bake("kept.test")
			.await
			.expect("the asset should bake before it is excluded");

		let (mut asset_manager, invocations) = versioned_asset_manager(asset_storage, resource_storage);
		asset_manager.set_excluded_assets(vec![Glob::new("kept.test")]);

		assert!(matches!(
			asset_manager.bake_if_stale("kept.test").await,
			Err(LoadMessages::Excluded { .. })
		));
		assert_eq!(invocations.load(Ordering::SeqCst), 0);
	}

	#[r#async::test]
	async fn compound_bead_sources_are_discoverable_and_dispatch_without_claiming_sidecars() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("studio.environment.bead", b"environment");
		storage_backend.add_file("studio.environment.bead.bead", b"{ invalid: true }");
		storage_backend.add_file("studio.exr.bead", b"{ image: true }");

		let resource_storage = MemoryStorageBackend::new();
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
	#[r#async::test]
	async fn tracked_transitive_sources_publish_the_requested_root_after_rebaking() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("parent.test", b"parent");

		asset_storage.add_file("child.test", b"child");

		let resource_storage = MemoryStorageBackend::new();

		let (manager, _) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		let updates = Arc::new(crate::resource::resource_manager::ResourceUpdateBroadcaster::default());

		let listener = updates.listener();

		manager.state.hot_reload.lock().updates = Some(updates);

		let resource = manager.bake_if_not_exists_serialized("parent.test").await.unwrap();

		manager.track_resource(resource.id());

		asset_storage.add_file("child.test", b"changed child");

		// A watcher batch discards the earlier checks once, then checks each tracked root.
		manager.state.epoch.fetch_add(1, Ordering::AcqRel);
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

		let resource_storage = MemoryStorageBackend::new();

		let (manager, _) = versioned_asset_manager(asset_storage, resource_storage);

		let updates = Arc::new(crate::resource::resource_manager::ResourceUpdateBroadcaster::default());

		let listener = updates.listener();

		manager.state.hot_reload.lock().updates = Some(updates);

		let resource = manager.bake_if_not_exists_serialized("stable.test").await.unwrap();

		manager.track_resource(resource.id());

		manager.state.clone().reload_resource("stable.test".to_string()).await;

		assert_eq!(listener.read(), None);
	}

	#[r#async::test]
	async fn lazy_baking_reuses_fresh_assets_and_rebakes_changed_sources_and_sidecars() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("versioned.test", b"first source");

		let resource_storage = MemoryStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage.clone());

		let bake = async |expectation: &str| {
			asset_manager
				.bake_if_not_exists::<TestResource>("versioned.test")
				.await
				.expect(expectation);
		};

		let stored_hash = async || {
			let (resource, _) = resource_storage
				.read(ResourceId::new("versioned.test"))
				.await
				.expect("the resource should be stored");
			resource.hash()
		};

		bake("initial source should bake").await;

		let first_hash = stored_hash().await;

		bake("unchanged source should be reused").await;

		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("versioned.test", b"changed source bytes");

		bake("changed source should rebake").await;

		let changed_hash = stored_hash().await;

		assert_ne!(first_hash, changed_hash);
		assert_eq!(invocations.load(Ordering::SeqCst), 2);

		asset_storage.add_file("versioned.test.bead", br#"{ purpose: "first" }"#);

		bake("new sidecar should rebake").await;

		asset_storage.add_file("versioned.test.bead", br#"{ purpose: "changed" }"#);

		bake("changed sidecar should rebake").await;

		assert_eq!(stored_hash().await, changed_hash);
		assert_eq!(invocations.load(Ordering::SeqCst), 4);

		asset_storage.remove_file("versioned.test.bead");
		bake("removing the requested sidecar should rebake").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 5);
		bake("an unchanged absent sidecar should stay fresh").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 5);

		asset_storage.add_file(".bead", br#"{ purpose: "directory" }"#);
		bake("a new directory default should rebake").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 6);
		asset_storage.add_file(".bead", br#"{ purpose: "changed directory" }"#);
		bake("a changed directory default should rebake").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 7);
		bake("an unchanged directory default should stay fresh").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 7);
	}

	#[r#async::test]
	async fn lazy_baking_tracks_directly_resolved_source_dependencies() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("external.test", b"root");

		asset_storage.add_file("external.bin", b"first dependency");

		let resource_storage = MemoryStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		let bake = async |expectation: &str| {
			asset_manager
				.bake_if_not_exists::<TestResource>("external.test")
				.await
				.expect(expectation);
		};

		bake("asset with external source should bake").await;

		bake("unchanged external source should be reused").await;

		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("external.bin.bead", b"invalid unrequested settings");
		bake("unrequested dependency settings must leave the resource fresh").await;
		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("external.bin", b"changed dependency");

		bake("changed external source should rebake its owner").await;

		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn lazy_baking_propagates_transitive_asset_versions_to_parent_resources() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("parent.test", b"parent");

		asset_storage.add_file("child.test", b"first child");

		let resource_storage = MemoryStorageBackend::new();

		let (asset_manager, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage);

		let bake = async |expectation: &str| {
			asset_manager
				.bake_if_not_exists::<TestResource>("parent.test")
				.await
				.expect(expectation);
		};

		bake("parent and child should bake").await;

		bake("unchanged dependency graph should be reused").await;

		assert_eq!(invocations.load(Ordering::SeqCst), 2);

		asset_storage.add_file("child.test", b"changed child");

		bake("changed child should rebake the child and parent").await;

		assert_eq!(invocations.load(Ordering::SeqCst), 4);
	}

	#[r#async::test]
	async fn a_forced_rebuild_rebakes_current_dependencies_once() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("parent.test", b"parent");

		asset_storage.add_file("child.test", b"child");

		let resource_storage = MemoryStorageBackend::new();

		let (first_run, invocations) = versioned_asset_manager(asset_storage.clone(), resource_storage.clone());

		first_run
			.bake_if_stale("parent.test")
			.await
			.expect("parent and child should bake");

		assert_eq!(invocations.load(Ordering::SeqCst), 2);

		let (forced_run, invocations) = versioned_asset_manager(asset_storage, resource_storage);

		forced_run.rebuild("parent.test").await.expect("parent should rebuild");
		forced_run
			.rebuild("child.test")
			.await
			.expect("child should already be rebuilt");

		assert_eq!(
			invocations.load(Ordering::SeqCst),
			2,
			"the parent and its child should each rebuild exactly once"
		);
	}

	#[r#async::test]
	async fn stale_resource_is_not_reused_after_its_source_is_removed() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("removed.test", b"source");

		let resource_storage = MemoryStorageBackend::new();

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
	async fn stale_sources_names_only_the_changed_and_removed_sources() {
		let asset_storage = TestStorageBackend::new();

		asset_storage.add_file("external.test", b"source");
		asset_storage.add_file("external.bin", b"payload");
		asset_storage.add_file("removed.test", b"source");

		let resource_storage = MemoryStorageBackend::new();
		let (asset_manager, _) = versioned_asset_manager(asset_storage.clone(), resource_storage.clone());

		for id in ["external.test", "removed.test"] {
			asset_manager
				.bake_if_not_exists::<TestResource>(id)
				.await
				.expect("fixture source should bake");
		}

		let external = resource_storage
			.read_provenance(ResourceId::new("external.test"))
			.unwrap()
			.unwrap();
		let removed = resource_storage
			.read_provenance(ResourceId::new("removed.test"))
			.unwrap()
			.unwrap();

		assert!(stale_sources(&external, &asset_storage).await.is_empty());

		asset_storage.add_file("external.bin", b"edited payload");
		asset_storage.remove_file("removed.test");

		assert_eq!(stale_sources(&external, &asset_storage).await, ["external.bin"]);
		assert_eq!(stale_sources(&removed, &asset_storage).await, ["removed.test"]);
	}

	/// The `ScriptedAssetHandler` struct bakes `.script` sources whose lines say what the bake does, so a test builds
	/// any bake graph from source files: `dep <id>` bakes a dependency, `try <id>` bakes one and carries on when it
	/// fails, `setting <name>` reads a setting, `query` gathers every stored `TestVariant`, `list <pattern>` lists
	/// sources, `load <id>` reads a stored resource through the bake's resource manager, `out <fragment>` stores
	/// another resource, `variant` stores the primary as a `TestVariant`,
	/// `payload <text>` sets the stored bytes, and `fail` fails the bake. A line that starts with `at <fragment>` only
	/// runs for the ID with that fragment, or with `at -` for the ID without one. Other lines do nothing.
	struct ScriptedAssetHandler {
		/// How many times each ID was baked.
		invocations: Arc<Mutex<HashMap<String, usize>>>,
	}

	#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct TestVariant {}

	impl Model for TestVariant {
		fn get_class() -> &'static str {
			"TestVariant"
		}
	}

	impl AssetHandler for ScriptedAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "script"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			*self.invocations.lock().entry(id.to_string()).or_default() += 1;

			let base = id.get_base();
			let (source, ..) = context.resolve(ResourceId::new(base.as_ref())).await?;
			let script = String::from_utf8(source.to_vec()).map_err(|_| LoadErrors::FailedToProcess)?;
			let mut payload = String::new();
			let mut variant = false;

			let fragment = id.get_fragment();
			let fragment = fragment.as_ref().map_or("-", |fragment| fragment.as_ref());

			for line in script.lines() {
				let line = match line.strip_prefix("at ").and_then(|rest| rest.split_once(' ')) {
					Some((at, _)) if at != fragment => continue,
					Some((_, rest)) => rest,
					None => line,
				};
				let (command, argument) = line.split_once(' ').unwrap_or((line, ""));
				match command {
					"dep" => {
						context.bake_dependency::<TestResource>(argument).await?;
					}
					"try" => {
						let _ = context.bake_dependency::<TestResource>(argument).await;
					}
					"setting" => {
						context.setting(argument);
					}
					"query" => {
						let ids = context
							.query_resources(crate::resource::storage_backend::Query::new("TestVariant"))
							.await?;
						context.bake_dependencies::<TestVariant>(&ids, 4).await?;
						payload.push_str(&ids.join(","));
					}
					"list" => {
						payload.push_str(&context.source_ids(argument).await?.join(","));
					}
					"load" => {
						// The request fails to solve a test resource as audio, after it read the resource.
						let _ = context
							.resource_manager()
							.request::<crate::resources::audio::Audio>(argument)
							.await;
					}
					"out" => {
						let output = format!("{}#{argument}", base.as_ref());
						context
							.store_resource(
								ProcessedAsset::new(ResourceId::new(&output), TestResource {}),
								argument.as_bytes(),
							)
							.await?;
					}
					"variant" => variant = true,
					"payload" => payload.push_str(argument),
					"fail" => return Err(LoadErrors::FailedToProcess),
					_ => {}
				}
			}

			let asset = if variant {
				ProcessedAsset::new(id, TestVariant {})
			} else {
				ProcessedAsset::new(id, TestResource {})
			};
			context.store_primary(asset, payload.as_bytes()).await
		}
	}

	/// The `Scripted` struct is a manager with a [`ScriptedAssetHandler`] and the stores and bake counts a test
	/// reads back.
	struct Scripted {
		manager: AssetManager,
		invocations: Arc<Mutex<HashMap<String, usize>>>,
		sources: TestStorageBackend,
		resources: MemoryStorageBackend,
	}

	impl Scripted {
		/// Builds a manager over `sources` and `resources`, which another manager may share, after `configure`.
		fn over(
			sources: TestStorageBackend,
			resources: MemoryStorageBackend,
			configure: impl FnOnce(&mut AssetManager),
		) -> Self {
			let invocations = Arc::new(Mutex::new(HashMap::default()));
			let mut manager = AssetManager::new(sources.clone(), resources.clone());
			manager.add_asset_handler(ScriptedAssetHandler {
				invocations: Arc::clone(&invocations),
			});
			configure(&mut manager);
			Scripted {
				manager,
				invocations,
				sources,
				resources,
			}
		}

		fn new() -> Self {
			Self::over(TestStorageBackend::new(), MemoryStorageBackend::new(), |_| {})
		}

		/// Returns how many times `id` was baked.
		fn bakes(&self, id: &str) -> usize {
			self.invocations.lock().get(id).copied().unwrap_or(0)
		}

		async fn stored(&self, id: &str) -> bool {
			self.resources.read(ResourceId::new(id)).await.is_some()
		}
	}

	#[r#async::test]
	async fn a_dependency_baked_again_to_the_same_result_leaves_its_parent_current() {
		let scripted = Scripted::new();
		scripted.sources.add_file("parent.script", b"dep child.script");
		scripted.sources.add_file("child.script", b"payload same\nfirst");

		scripted.manager.bake_if_stale("parent.script").await.unwrap();

		scripted.sources.add_file("child.script", b"payload same\nsecond");
		scripted.manager.bake_if_stale("parent.script").await.unwrap();

		assert_eq!(scripted.bakes("child.script"), 2, "the edited child should bake again");
		assert_eq!(scripted.bakes("parent.script"), 1, "the child's result didn't change");

		scripted.sources.add_file("child.script", b"payload other");
		scripted.manager.bake_if_stale("parent.script").await.unwrap();

		assert_eq!(scripted.bakes("parent.script"), 2, "the child's result changed");
	}

	#[r#async::test]
	async fn a_changed_setting_rebakes_only_the_resources_that_read_it() {
		let (sources, resources) = (TestStorageBackend::new(), MemoryStorageBackend::new());
		sources.add_file("reader.script", b"setting quality");
		sources.add_file("other.script", b"payload other");

		let run = async |quality: u64| {
			let scripted = Scripted::over(sources.clone(), resources.clone(), |manager| {
				manager.declare_setting("quality", quality);
			});
			scripted.manager.bake_if_stale("reader.script").await.unwrap();
			scripted.manager.bake_if_stale("other.script").await.unwrap();
			(scripted.bakes("reader.script"), scripted.bakes("other.script"))
		};

		assert_eq!(run(1).await, (1, 1));
		assert_eq!(run(1).await, (0, 0), "the same setting should leave both current");
		assert_eq!(run(2).await, (1, 0), "only the reader should bake again");
	}

	#[r#async::test]
	async fn a_setting_nobody_declares_anymore_rebakes_its_readers() {
		let (sources, resources) = (TestStorageBackend::new(), MemoryStorageBackend::new());
		sources.add_file("reader.script", b"setting quality");

		let declared = Scripted::over(sources.clone(), resources.clone(), |manager| {
			manager.declare_setting("quality", 1);
		});
		declared.manager.bake_if_stale("reader.script").await.unwrap();

		let undeclared = Scripted::over(sources, resources, |_| {});
		undeclared.manager.bake_if_stale("reader.script").await.unwrap();

		assert_eq!(undeclared.bakes("reader.script"), 1);
	}

	#[r#async::test]
	async fn a_resource_another_bake_stores_is_made_current_by_that_bake() {
		let scripted = Scripted::new();
		scripted.sources.add_file("container.script", b"out first");

		scripted.manager.bake_if_stale("container.script").await.unwrap();
		scripted.manager.bake_if_stale("container.script#first").await.unwrap();

		assert_eq!(scripted.bakes("container.script"), 1);
		assert_eq!(
			scripted.bakes("container.script#first"),
			0,
			"the fragment is never baked on its own"
		);

		scripted.sources.add_file("container.script", b"out first\nedited");
		scripted.manager.bake_if_stale("container.script#first").await.unwrap();

		assert_eq!(scripted.bakes("container.script"), 2, "the stale container should bake again");
		assert_eq!(scripted.bakes("container.script#first"), 0);
	}

	#[r#async::test]
	async fn a_resource_its_bake_no_longer_stores_is_removed() {
		let scripted = Scripted::new();
		scripted.sources.add_file("container.script", b"out first\nout second");

		scripted.manager.bake_if_stale("container.script").await.unwrap();
		assert!(scripted.stored("container.script#second").await);

		scripted.sources.add_file("container.script", b"out first");
		scripted.manager.bake_if_stale("container.script").await.unwrap();

		assert!(scripted.stored("container.script#first").await);
		assert!(!scripted.stored("container.script#second").await);
	}

	#[r#async::test]
	async fn a_resource_two_bakes_store_stays_until_neither_stores_it() {
		let scripted = Scripted::new();
		scripted.sources.add_file("shared.script", b"out skeleton");
		// Both IDs name the same source, as a model and one of its clips do, and both store its skeleton.
		scripted.manager.bake_if_stale("shared.script").await.unwrap();
		scripted.manager.bake_if_stale("shared.script#clip").await.unwrap();

		scripted.sources.add_file("shared.script", b"edited");
		scripted.manager.bake_if_stale("shared.script").await.unwrap();
		assert!(
			scripted.stored("shared.script#skeleton").await,
			"the clip's bake still stores it"
		);

		scripted.manager.bake_if_stale("shared.script#clip").await.unwrap();
		assert!(!scripted.stored("shared.script#skeleton").await);
	}

	#[r#async::test]
	async fn a_bake_checks_a_dependency_that_reads_what_the_bake_itself_stores() {
		let scripted = Scripted::new();
		scripted.sources.add_file("container.script", b"at - out shared");
		scripted.manager.bake_if_stale("container.script").await.unwrap();

		// The container now bakes a part of itself, and the part reads a resource only the container stores.
		scripted.sources.add_file(
			"container.script",
			b"at - out shared\nat - dep container.script#part\nat part dep container.script#shared",
		);
		scripted.manager.bake_if_stale("container.script").await.unwrap();
		assert_eq!(scripted.bakes("container.script"), 2);
		assert_eq!(scripted.bakes("container.script#part"), 1);

		// Checking the container checks the part, which must not wait for the container to check what it stored.
		scripted.manager.bake_if_stale("container.script").await.unwrap();
		assert_eq!(scripted.bakes("container.script"), 2, "nothing changed");
		assert_eq!(scripted.bakes("container.script#shared"), 0, "only the container stores it");
	}

	#[r#async::test]
	async fn assets_baked_from_each_other_fail_instead_of_waiting_forever() {
		let scripted = Scripted::new();
		scripted.sources.add_file("first.script", b"dep second.script");
		scripted.sources.add_file("second.script", b"dep first.script");

		assert_eq!(
			scripted.manager.bake_if_stale("first.script").await,
			Err(LoadMessages::FailedToBake {
				asset: "first.script".to_string(),
				error: LoadErrors::DependencyCycle,
			})
		);
	}

	#[r#async::test]
	async fn a_resource_read_through_the_bakes_resource_manager_is_an_input() {
		let scripted = Scripted::new();
		scripted.sources.add_file("reader.script", b"load read.script");
		scripted.sources.add_file("read.script", b"payload first");

		scripted.manager.bake_if_stale("read.script").await.unwrap();
		scripted.manager.bake_if_stale("reader.script").await.unwrap();
		scripted.manager.bake_if_stale("reader.script").await.unwrap();
		assert_eq!(scripted.bakes("reader.script"), 1);

		scripted.sources.add_file("read.script", b"payload second");
		scripted.manager.bake_if_stale("reader.script").await.unwrap();

		assert_eq!(
			scripted.bakes("read.script"),
			2,
			"checking the reader makes what it read current"
		);
		assert_eq!(scripted.bakes("reader.script"), 2);
	}

	#[r#async::test]
	async fn a_failed_bake_leaves_the_asset_stale_even_once_its_sources_are_restored() {
		let scripted = Scripted::new();
		scripted.sources.add_file("asset.script", b"out first");
		scripted.manager.bake_if_stale("asset.script").await.unwrap();

		scripted.sources.add_file("asset.script", b"out other\nfail");
		assert!(scripted.manager.bake_if_stale("asset.script").await.is_err());

		// The failed bake stored `other`, so the store no longer holds what the first bake recorded.
		scripted.sources.add_file("asset.script", b"out first");
		scripted.manager.bake_if_stale("asset.script").await.unwrap();

		assert_eq!(scripted.bakes("asset.script"), 3);
	}

	#[r#async::test]
	async fn a_dependency_that_failed_rebakes_its_parent_once_it_works() {
		let scripted = Scripted::new();
		scripted.sources.add_file("parent.script", b"try child.script");
		scripted.sources.add_file("child.script", b"fail");

		scripted.manager.bake_if_stale("parent.script").await.unwrap();
		scripted.manager.bake_if_stale("parent.script").await.unwrap();
		assert_eq!(
			scripted.bakes("parent.script"),
			1,
			"a dependency that still fails changes nothing"
		);

		scripted.sources.add_file("child.script", b"payload fixed");
		scripted.manager.bake_if_stale("parent.script").await.unwrap();

		assert_eq!(scripted.bakes("parent.script"), 2);
	}

	#[r#async::test]
	async fn a_forced_rebuild_bakes_the_root_and_what_it_is_baked_from_and_nothing_else() {
		let (sources, resources) = (TestStorageBackend::new(), MemoryStorageBackend::new());
		sources.add_file("root.script", b"dep shared.script");
		sources.add_file("shared.script", b"payload shared");
		sources.add_file("unrelated.script", b"payload unrelated");

		let first = Scripted::over(sources.clone(), resources.clone(), |_| {});
		for id in ["root.script", "unrelated.script"] {
			first.manager.bake_if_stale(id).await.unwrap();
		}

		let forced = Scripted::over(sources, resources, |_| {});
		forced.manager.rebuild("root.script").await.unwrap();
		forced.manager.bake_if_stale("unrelated.script").await.unwrap();

		assert_eq!(forced.bakes("root.script"), 1);
		assert_eq!(forced.bakes("shared.script"), 1);
		assert_eq!(forced.bakes("unrelated.script"), 0);
	}

	#[r#async::test]
	async fn a_query_bake_goes_stale_when_a_match_appears_changes_or_disappears() {
		let scripted = Scripted::new();
		scripted.sources.add_file("gather.script", b"query");
		scripted.sources.add_file("a.script", b"variant\npayload a");

		scripted.manager.bake_if_stale("a.script").await.unwrap();
		scripted.manager.bake_if_stale("gather.script").await.unwrap();
		scripted.manager.bake_if_stale("gather.script").await.unwrap();
		assert_eq!(scripted.bakes("gather.script"), 1, "an unchanged match set is current");

		scripted.sources.add_file("b.script", b"variant\npayload b");
		scripted.manager.bake_if_stale("b.script").await.unwrap();
		scripted.manager.bake_if_stale("gather.script").await.unwrap();
		assert_eq!(scripted.bakes("gather.script"), 2, "a new match");

		// The gathering bake makes its matches current, so the edit reaches it without a request for the match.
		scripted.sources.add_file("a.script", b"variant\npayload edited");
		scripted.manager.bake_if_stale("gather.script").await.unwrap();
		assert_eq!(scripted.bakes("a.script"), 2);
		assert_eq!(scripted.bakes("gather.script"), 3, "a changed match");

		crate::resource::WriteStorageBackend::delete(&scripted.resources, ResourceId::new("b.script"))
			.await
			.unwrap();
		scripted.manager.bake_if_stale("gather.script").await.unwrap();
		assert_eq!(scripted.bakes("gather.script"), 4, "a removed match");
	}

	#[r#async::test]
	async fn a_listing_bake_goes_stale_when_a_matching_source_appears() {
		let scripted = Scripted::new();
		scripted.sources.add_file("database.script", b"list clips/*");
		scripted.sources.add_file("clips/walk.script", b"");

		scripted.manager.bake_if_stale("database.script").await.unwrap();

		scripted.sources.add_file("other/run.script", b"");
		scripted.manager.bake_if_stale("database.script").await.unwrap();
		assert_eq!(
			scripted.bakes("database.script"),
			1,
			"a source the pattern doesn't match changes nothing"
		);

		scripted.sources.add_file("clips/run.script", b"");
		scripted.manager.bake_if_stale("database.script").await.unwrap();
		assert_eq!(scripted.bakes("database.script"), 2);
	}

	#[r#async::test]
	async fn a_file_whose_modification_time_changed_but_not_its_content_stays_current() {
		let directory = std::env::temp_dir().join(format!("byte-engine-freshness-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&directory);
		std::fs::create_dir_all(&directory).unwrap();
		let source = directory.join("asset.script");
		std::fs::write(&source, b"payload first").unwrap();

		let invocations = Arc::new(Mutex::new(HashMap::default()));
		let mut manager = AssetManager::new(
			crate::asset::FileStorageBackend::new(directory.clone()),
			MemoryStorageBackend::new(),
		);
		manager.add_asset_handler(ScriptedAssetHandler {
			invocations: Arc::clone(&invocations),
		});
		let bakes = || invocations.lock().get("asset.script").copied().unwrap_or(0);
		let touch = |seconds: u64| {
			let modified = std::time::UNIX_EPOCH + Duration::from_secs(seconds);
			std::fs::File::options()
				.write(true)
				.open(&source)
				.unwrap()
				.set_modified(modified)
				.unwrap();
		};

		touch(1_000_000);
		manager.bake_if_stale("asset.script").await.unwrap();

		touch(2_000_000);
		manager.bake_if_stale("asset.script").await.unwrap();
		assert_eq!(bakes(), 1, "the same content with a new modification time is unchanged");

		// The same size, so only the content tells the edit apart.
		std::fs::write(&source, b"payload other").unwrap();
		touch(3_000_000);
		manager.bake_if_stale("asset.script").await.unwrap();
		assert_eq!(bakes(), 2);

		let _ = std::fs::remove_dir_all(&directory);
	}

	#[r#async::test]
	async fn concurrent_bakes_for_one_asset_and_store_share_one_invocation() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, false);

		let (_, first, second, third) = std::future::join!(
			release_once_started(&started[0], release),
			asset_manager.bake("coalesced.test"),
			asset_manager.bake("coalesced.test"),
			asset_manager.bake("coalesced.test"),
		)
		.await;

		assert_eq!((first, second, third), (Ok(()), Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 1);
	}

	#[r#async::test]
	async fn concurrent_failures_are_shared_but_later_bakes_retry() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(true, false);

		let (_, first, follower) = std::future::join!(
			release_once_started(&started[0], release),
			asset_manager.bake("failed.test"),
			asset_manager.bake("failed.test"),
		)
		.await;

		assert_eq!(first, follower);
		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		let retry = asset_manager.bake("failed.test").await;

		assert_eq!(retry, first);
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn completed_explicit_bake_is_not_memoized() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, true);

		let (_, first) =
			std::future::join!(release_once_started(&started[0], release), asset_manager.bake("repeat.test")).await;

		assert_eq!(first, Ok(()));
		assert_eq!(asset_manager.bake("repeat.test").await, Ok(()));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn different_assets_run_independently() {
		let (asset_manager, invocations, _, started, release) = coordinating_asset_manager(false, false);

		let (_, first, second) = std::future::join!(
			release_once_started(&started[1], release),
			asset_manager.bake("first.test"),
			asset_manager.bake("second.test"),
		)
		.await;

		assert_eq!((first, second), (Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn admitted_parent_can_claim_a_dependency_before_a_memory_waiting_root() {
		let (started, started_announcement) = announcement::Announcement::new();

		let (release, release_announcement) = announcement::Announcement::new();

		let parent_started_for_root = started_announcement.listener();

		let parent_started_for_release = started_announcement.listener();

		let mut asset_manager = AssetManager::new(TestStorageBackend::new(), MemoryStorageBackend::new());

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

		let (parent, competing_root, ()) = compio::time::timeout(Duration::from_secs(1), async {
			std::future::join!(parent, competing_root, release_parent).await
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

		let (_, result) =
			std::future::join!(release_once_started(&started[1], release), asset_manager.bake("parent.batch")).await;

		assert_eq!(result, Ok(()));
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn handler_trace_keeps_ordered_info_and_warning_items_for_a_baked_resource() {
		let (asset_manager, resource_storage_backend) = test_asset_manager(TestStorageBackend::new());

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
		let (asset_manager, resource_storage_backend) = test_asset_manager(TestStorageBackend::new());

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
	async fn bakes_that_store_or_fail_reach_the_bake_reporter() {
		let reported = Arc::new(crate::asset::bake_state::CapturedBakes::default());

		let asset_storage = TestStorageBackend::new();
		asset_storage.add_file("kept.test", b"kept");
		let (mut stores, _) = versioned_asset_manager(asset_storage, MemoryStorageBackend::new());
		stores.set_bake_reporter(reported.clone());
		stores.bake("kept.test").await.expect("the asset bakes");
		{
			let states = reported.0.lock();
			assert_eq!(states.first().map(|state| state.baking[0].id.as_str()), Some("kept.test"));
			let last = states.last().expect("a report");
			assert!(last.baking.is_empty());
			assert_eq!((last.baked, last.failed), (1, 0));
			assert_eq!(last.finished[0].id, "kept.test");
			assert!(last.finished[0].ok);
		}

		let (mut fails, _) = test_asset_manager(TestStorageBackend::new());
		fails.set_bake_reporter(reported.clone());
		assert!(fails.bake("failed.test").await.is_err());
		let last = reported.0.lock().last().cloned().expect("a report");
		assert_eq!((last.baked, last.failed), (0, 1));
		assert_eq!(last.finished[0].id, "failed.test");
		assert!(!last.finished[0].ok);
	}

	#[r#async::test]
	async fn successful_handler_must_store_the_requested_primary_resource() {
		let (asset_manager, _) = test_asset_manager(TestStorageBackend::new());

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
		let (asset_manager, resource_storage_backend) = test_asset_manager(TestStorageBackend::new());

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
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
	},
};

use announcement;
use utils::{Glob, hash::HashMap, sync::Mutex};

#[cfg(debug_assertions)]
use super::resource_trace::{ResourceTrace, ResourceTraceLevel};
use super::bake_state::{BakeReporter, BakeTracker};
use super::{
	StorageBackend,
	bake_memory::{BakeAllocator, BakeMemoryBudget, BakeMemoryScope},
	handler::{AssetHandler, BakeContext, TrackingStorageBackend},
};
use crate::{
	Model, ProcessedAsset, ReferenceModel,
	asset::{
		self, DynStorageBackend, ResourceId,
		handler::{
			DynAssetHandler, LoadErrors, PROGRAM_GENERATOR_SETTING, PROGRAM_TRANSFORM_SETTING, SHADER_CODE_SETTING,
			SHADER_COMPILER_SETTING, STORE_SETTING,
			implementations::{
				ProgramGenerator, ProgramTransform,
				besl::{PlatformShaderCompilerAdapter, ShaderCompiler},
			},
		},
		intercept::BakeInterceptor,
		provenance::{BakeInput, MISSING_RESOURCE, Provenance, resource_fingerprint, set_fingerprint},
		storage_backend::{AssetVersion, VersionChange},
	},
	online_docs_url,
	processors::image::ImageProcessor,
	resource::{self, DynStorageBackend as DynResourceStorageBackend, StorageBackend as ResourceStorageBackend},
};
