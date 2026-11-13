/// The `AssetHandler` trait provides source types for asset baking.
///
/// See the [assets guide](/docs/develop/resource-management/assets)
/// before implementing a new source-format handler.
pub trait AssetHandler {
	fn can_handle(&self, r#type: &str) -> bool;

	/// Returns whether recursive asset discovery should include a source handled by this implementation.
	fn should_discover(&self, _id: ResourceId<'_>, _has_sidecar: bool) -> bool {
		true
	}

	/// Returns a fingerprint of the handler's configuration: every option set on it that changes what it bakes, such
	/// as whether a model handler generates lightmap UVs.
	///
	/// The asset manager records it as a setting of every bake the handler runs, so reconfiguring the handler makes its
	/// resources stale. A handler without options keeps the default.
	fn identity(&self) -> u64 {
		0
	}

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> impl Future<Output = Result<(), LoadErrors>>;
}

/// The `DynAssetHandler` trait lets the asset manager keep handlers of different types in one list.
///
/// Every [`AssetHandler`] implements it; register handlers with
/// [`AssetManager::add_asset_handler`](crate::asset::manager::AssetManager::add_asset_handler).
pub trait DynAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn identity(&self) -> u64;

	/// Returns the name of the setting that holds [`Self::identity`], which names the handler's type.
	fn setting_name(&self) -> String;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

impl<T: AssetHandler + Send + Sync> DynAssetHandler for T {
	fn can_handle(&self, r#type: &str) -> bool {
		AssetHandler::can_handle(self, r#type)
	}

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool {
		AssetHandler::should_discover(self, id, has_sidecar)
	}

	fn identity(&self) -> u64 {
		AssetHandler::identity(self)
	}

	fn setting_name(&self) -> String {
		format!("handler:{}", std::any::type_name::<T>())
	}

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>> {
		Box::pin(AssetHandler::bake(self, context, url))
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadErrors {
	FailedToProcess,
	AssetCouldNotBeRead,
	AssetCouldNotBeLoaded,
	UnsupportedType,
	FailedToStore,
	PrimaryResourceIdMismatch,
	PrimaryResourceNotStored,
	/// A dependency matches an excluded pattern, so it is never baked.
	DependencyExcluded,
	/// The asset is baked from a resource that is baked from the asset.
	DependencyCycle,
}

// Only debug builds bake assets and record why a bake failed.
#[cfg(debug_assertions)]
impl LoadErrors {
	/// Returns the developer-facing cause for this asset loading failure.
	pub(crate) const fn message(&self) -> &'static str {
		match self {
			Self::FailedToProcess => "The source asset could not be processed.",
			Self::AssetCouldNotBeRead => "The source asset could not be read.",
			Self::AssetCouldNotBeLoaded => "The source asset could not be loaded.",
			Self::UnsupportedType => "The source asset type is unsupported.",
			Self::FailedToStore => "The baked resource could not be stored.",
			Self::PrimaryResourceIdMismatch => "The asset handler stored the primary resource under a different ID.",
			Self::PrimaryResourceNotStored => "The asset handler did not store the primary resource.",
			Self::DependencyExcluded => "A dependency of the source asset matches `assets.exclude`, so it is never baked.",
			Self::DependencyCycle => "The asset depends on a resource that depends on the asset.",
		}
	}

	/// Returns the recovery step most likely to resolve this asset loading failure.
	pub(crate) const fn fix(&self) -> &'static str {
		match self {
			Self::AssetCouldNotBeRead => {
				"Check the asset ID and configured assets directory. Engine asset IDs start with 'byte_engine/'."
			}
			Self::FailedToProcess | Self::AssetCouldNotBeLoaded => {
				"Check the source asset and its dependencies for invalid or unsupported data."
			}
			Self::UnsupportedType => "Use a supported asset type or register an asset handler for it.",
			Self::FailedToStore => "Check that the resource destination is writable, then retry.",
			Self::PrimaryResourceIdMismatch => "Store the primary resource under the requested asset ID.",
			Self::PrimaryResourceNotStored => "Make the asset handler store its primary resource before returning.",
			Self::DependencyExcluded => {
				"Stop using the excluded dependency, which the warning before this error names, or remove its pattern from `assets.exclude`."
			}
			Self::DependencyCycle => {
				"Remove one reference of the cycle, which the warning before this error names, such as a material that names itself as its parent."
			}
		}
	}
}

/// The `TrackingStorageBackend` struct records every source resolved during one asset bake.
///
/// It records a source's version only after a successful read whose version matched on both sides of the read.
pub(crate) struct TrackingStorageBackend<'a> {
	pub(in crate::asset) inner: &'a dyn asset::DynStorageBackend,
	/// The inputs the bake observed so far, kept sorted by key so they persist deterministically. Shared, because a
	/// resource manager a handler takes from its context records what it reads after the context is gone from view.
	pub(in crate::asset) inputs: &'a Arc<Mutex<Vec<BakeInput>>>,
}

impl TrackingStorageBackend<'_> {
	/// Runs `read` between two version checks of `id` and records its version, rejecting the result when `id` changed
	/// during the read.
	///
	/// `content` returns the bytes `read` produced, when it produced the file's bytes, so the recorded version carries
	/// their hash and a later change of the modification time alone doesn't make the resource stale.
	///
	/// A read that fails because `id` doesn't exist records `id` as missing, so creating it later rebakes the resource.
	/// Handlers that look for optional files, such as the two files a BESL module may live in, rely on this.
	async fn tracked<T, F: Future<Output = Result<T, ()>>>(
		&self,
		id: ResourceId<'_>,
		read: impl FnOnce() -> F,
		content: impl for<'r> FnOnce(&'r T) -> Option<&'r [u8]>,
	) -> Result<T, ()> {
		let before = self.inner.version(id).await?;
		let resolved = match read().await {
			Ok(resolved) => resolved,
			Err(()) => {
				if before == AssetVersion::missing() {
					self.record(id, before);
				}
				return Err(());
			}
		};
		let after = self.inner.version(id).await?;

		if before != after {
			log::warn!(
				"Asset changed while it was being read. The most likely cause is that '{}' was saved during the bake; retry the request.",
				id.as_ref()
			);

			return Err(());
		}

		// Records the latest observed version once when handlers resolve the same source repeatedly.
		let version = match content(&resolved) {
			Some(bytes) => after.with_content(bytes),
			None => after,
		};
		self.record(id, version);

		Ok(resolved)
	}

	/// Records `version` as what the bake observed of the source file `id` names, whatever fragment `id` carries.
	pub(in crate::asset) fn record(&self, id: ResourceId<'_>, version: AssetVersion) {
		upsert_input(
			&mut self.inputs.lock(),
			BakeInput::Source {
				id: id.get_base().as_ref().to_string(),
				version,
			},
		);
	}
}

impl asset::StorageBackend for TrackingStorageBackend<'_> {
	fn directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.inner.directory_accessible(path)
	}

	async fn resolve<'a>(&'a self, url: ResourceId<'a>) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(url, || self.inner.resolve(url), |(bytes, _)| Some(bytes.as_slice()))
			.await
	}

	async fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(
			url,
			|| self.inner.resolve_in(url, allocator),
			|(bytes, _)| Some(bytes.as_slice()),
		)
		.await
	}

	async fn load_bead<'a>(&'a self, id: ResourceId<'a>) -> Result<Option<BEADType>, ()> {
		// Track absence as well as content, so creating a sidecar or directory default invalidates the baked resource.
		self.tracked(id, || self.inner.load_bead(id), |_| None).await
	}

	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl std::future::Future<Output = Result<AssetVersion, ()>> + 'a {
		self.inner.version(url)
	}
}

/// The `BakeContext` struct provides format handlers with the shared facilities used during one asset bake.
#[derive(Clone, Copy)]
pub struct BakeContext<'a> {
	pub(in crate::asset) asset_manager: &'a Arc<AssetManagerState>,
	pub(in crate::asset) asset_storage_backend: &'a TrackingStorageBackend<'a>,
	pub(in crate::asset) allocator: &'a BakeAllocator,
	pub(in crate::asset) primary_id: ResourceId<'a>,
	pub(in crate::asset) primary_stored: &'a Cell<bool>,
	/// The IDs of the resources this bake stored besides the primary one, sorted.
	pub(in crate::asset) outputs: &'a Mutex<Vec<String>>,
	/// The IDs waiting on this bake, outermost first, ending with the primary ID. A dependency of this bake never
	/// waits on one of them.
	pub(in crate::asset) ancestors: &'a [String],
	/// Whether this bake belongs to a forced rebuild, so its dependencies are rebuilt too.
	pub(in crate::asset) forced: bool,
}

impl<'a> BakeContext<'a> {
	/// Adds a detail item to this resource's development trace, and to the terminal log at the debug level, which
	/// most runs leave hidden.
	///
	/// Use it for what a bake did as expected and only someone studying that bake wants to read. The trace keeps it as
	/// an informational item.
	pub fn debug(&self, message: impl fmt::Display) {
		self.log(log::Level::Debug, message);
	}

	/// Adds an informational item to this resource's development trace and terminal log.
	pub fn info(&self, message: impl fmt::Display) {
		self.log(log::Level::Info, message);
	}

	/// Adds a warning item to this resource's development trace and terminal log.
	pub fn warn(&self, message: impl fmt::Display) {
		self.log(log::Level::Warn, message);
	}

	/// Adds an error item to this resource's development trace and terminal log.
	///
	/// The item remains available when the handler returns an error and does not
	/// store the requested resource.
	pub fn error(&self, message: impl fmt::Display) {
		self.log(log::Level::Error, message);
	}

	/// Writes one message to the terminal log and, in development builds, to this resource's trace.
	fn log(&self, level: log::Level, message: impl fmt::Display) {
		#[cfg(debug_assertions)]
		let message = message.to_string();

		log::log!(level, "{message}");

		#[cfg(debug_assertions)]
		self.asset_manager.resource_trace.record(
			self.primary_id,
			match level {
				log::Level::Error => ResourceTraceLevel::Error,
				log::Level::Warn => ResourceTraceLevel::Warn,
				log::Level::Info | log::Level::Debug | log::Level::Trace => ResourceTraceLevel::Info,
			},
			message,
		);
	}

	/// Resolves only the requested source bytes with the bake allocator.
	pub async fn resolve<'b>(&'b self, id: ResourceId<'b>) -> Result<(AssetStorageBytes<'b>, String), LoadErrors> {
		self.asset_storage_backend
			.resolve_in(id, self.allocator)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Loads optional BEAD settings, the directory `.bead` defaults merged under the source's sidecar, and records the
	/// presence or absence of each file as a bake dependency.
	///
	/// Call this only when the handler consumes sidecar settings, then read the source with [`Self::resolve`].
	pub async fn load_sidecar(&self, id: ResourceId<'_>) -> Result<Option<BEADType>, LoadErrors> {
		self.asset_storage_backend
			.load_sidecar(id)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Returns whether the source asset `id` exists, without reading it, and records its version as a bake dependency.
	///
	/// Use it to choose among optional files: an absent file is recorded as missing, so creating it later makes the
	/// resource stale, and a present one rebakes the resource when it changes.
	pub async fn source_exists(&self, id: ResourceId<'_>) -> Result<bool, LoadErrors> {
		let version = self
			.asset_storage_backend
			.inner
			.version(id)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)?;
		let exists = version != AssetVersion::missing();
		self.asset_storage_backend.record(id, version);
		Ok(exists)
	}

	/// Returns the IDs of every stored resource `query` matches, sorted, after making each of them current.
	///
	/// The matches are recorded as an input of this bake, so the resource goes stale when a match appears, disappears,
	/// or bakes to a different result. Resources of [excluded](crate::asset::manager::AssetManager::set_excluded_assets)
	/// assets don't match.
	///
	/// Next, read the matches with [`Self::bake_dependencies`].
	pub async fn query_resources(&self, query: crate::resource::storage_backend::Query) -> Result<Vec<String>, LoadErrors> {
		let (ids, fingerprint) = self
			.asset_manager
			.query_matches(&query, self.dependency_origin())
			.await
			.map_err(|_| LoadErrors::FailedToProcess)?;
		upsert_input(
			&mut self.asset_storage_backend.inputs.lock(),
			BakeInput::query(&query, fingerprint),
		);
		Ok(ids)
	}

	/// Returns the fingerprint of the setting `name`, and records the setting as an input of this bake, so the
	/// resource goes stale when the declared value changes.
	///
	/// Call it for everything that shapes the bake and isn't a file or a stored resource, such as the identity of a
	/// service the handler bakes with. Declare the value with
	/// [`AssetManager::declare_setting`](crate::asset::manager::AssetManager::declare_setting). A name nobody declared
	/// has the fingerprint `0`.
	pub fn setting(&self, name: &str) -> u64 {
		let fingerprint = self.asset_manager.setting(name);
		upsert_input(
			&mut self.asset_storage_backend.inputs.lock(),
			BakeInput::Setting {
				name: name.to_string(),
				fingerprint,
			},
		);
		fingerprint
	}

	/// Bakes a referenced source asset on the shared worker pool when necessary and returns its stored model.
	///
	/// Concurrent calls from one handler bake on separate workers, each in its own arena.
	pub async fn bake_dependency<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadErrors> {
		Ok(self.bake_dependencies(&[id.to_owned()], 1).await?.remove(0))
	}

	/// Bakes independent dependencies on the shared worker pool while bounding active requests.
	///
	/// Results preserve the input order. Each dependency is recorded as an input of this bake with the fingerprint of
	/// the resource it read, so this resource goes stale when the dependency bakes to a different result. A dependency
	/// that fails is recorded as missing before the error returns, so a handler that carries on without it bakes again
	/// once the dependency works.
	pub async fn bake_dependencies<M: Model>(
		&self,
		ids: &[String],
		max_concurrency: usize,
	) -> Result<Vec<ReferenceModel<M>>, LoadErrors> {
		use utils::r#async::StreamExt as _;

		let mode = if self.forced {
			crate::asset::manager::BakeMode::Forced
		} else {
			crate::asset::manager::BakeMode::IfStale
		};

		let requests = ids.iter().map(|id| async move {
			// A resource this bake stored itself is read as stored: asking for it would wait on this very bake.
			let own = id == self.primary_id.as_ref() || self.outputs.lock().binary_search(id).is_ok();
			if !own
				&& let Err(error) = self
					.asset_manager
					.dispatch_bake_in_scope(id, mode, self.dependency_origin())
					.await
			{
				self.record_dependency(id, MISSING_RESOURCE);
				return Err(dependency_load_error(error));
			}

			let Some((resource, _)) = self.asset_manager.resource_storage_backend.read(ResourceId::new(id)).await else {
				self.record_dependency(id, MISSING_RESOURCE);
				return Err(LoadErrors::FailedToProcess);
			};
			// What the bake reads back of its own resources is dropped from its inputs when it finishes.
			self.record_dependency(id, resource_fingerprint(&resource));

			Ok(resource.into())
		});

		// Buffered, not unordered, so the results keep the order of `ids`.
		utils::r#async::stream::iter(requests)
			.buffered(max_concurrency.max(1))
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect()
	}

	/// Records the stored resource `id`, whose fingerprint is `fingerprint`, as an input of this bake.
	fn record_dependency(&self, id: &str, fingerprint: u64) {
		upsert_input(
			&mut self.asset_storage_backend.inputs.lock(),
			BakeInput::Resource {
				id: id.to_string(),
				fingerprint,
			},
		);
	}

	/// Returns the origin of the bakes this bake asks for: they share its memory scope and never ask it to bake
	/// itself.
	fn dependency_origin(&self) -> crate::asset::manager::BakeOrigin {
		crate::asset::manager::BakeOrigin::Dependency {
			memory_scope: self.allocator.memory_scope().cloned(),
			ancestors: self.ancestors.to_vec(),
		}
	}

	/// Solves a dependency [`Self::bake_dependency`] or [`Self::bake_dependencies`] returned into its runtime resource,
	/// for a handler that reads what it baked, like a motion database that samples its clips.
	///
	/// Fails when the stored resource is of another class than `M`'s, or doesn't solve.
	///
	/// Every stored resource the solve reads, such as the skeleton of a clip, is recorded as an input of this bake.
	pub async fn solve<M: StoredModel>(&self, dependency: ReferenceModel<M>) -> Result<Reference<M::Resource>, LoadErrors> {
		let storage = self.recording_storage();
		let storage: &dyn crate::resource::DynReadStorageBackend = &storage;
		let (stored, reader) = storage.read(dependency.id()).await.ok_or(LoadErrors::FailedToProcess)?;
		if stored.class() != M::get_class() {
			return Err(LoadErrors::FailedToProcess);
		}
		M::solve_stored(stored, reader, storage)
			.await
			.map_err(|_| LoadErrors::FailedToProcess)
	}

	/// Returns a resource manager over the resources this bake stores, for a handler that reads its baked dependencies
	/// through the runtime's own loading code, like a lightmap bake that loads the meshes and materials the renderer
	/// draws.
	///
	/// Bake each dependency with [`Self::bake_dependency`] first: the manager only reads what is already stored, and
	/// can't store anything. Every resource it reads is recorded as an input of this bake, so the resource goes stale
	/// when one of them bakes to a different result.
	pub fn resource_manager(&self) -> crate::ResourceManager {
		crate::ResourceManager::new(self.recording_storage())
	}

	/// Returns a reader of the resource store that records every resource read through it as an input of this bake.
	fn recording_storage(&self) -> RecordingStorage {
		RecordingStorage {
			inner: self.asset_manager.resource_storage_backend.clone(),
			inputs: Arc::clone(self.asset_storage_backend.inputs),
		}
	}

	/// Returns the IDs of the source assets `pattern` matches, in ascending order, for a handler that selects sources
	/// by pattern, like a motion database's clips.
	///
	/// `pattern` is a [`Glob`](utils::Glob). The matches are recorded as an input of this bake, so the resource goes
	/// stale when a matching source appears or disappears.
	pub async fn source_ids(&self, pattern: &str) -> Result<Vec<String>, LoadErrors> {
		let (ids, fingerprint) = self
			.asset_manager
			.source_listing(pattern)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)?;
		upsert_input(
			&mut self.asset_storage_backend.inputs.lock(),
			BakeInput::Listing {
				pattern: pattern.to_string(),
				fingerprint,
			},
		);
		Ok(ids)
	}

	/// Returns a stored resource that the handler may reference instead of producing it again, and records it as an
	/// output of this bake.
	///
	/// Use it only for content-addressed IDs, where the ID changes whenever anything that affects the content
	/// changes. Generated material shaders use it with
	/// [`PreparedBeslShader::cache_key`](crate::asset::handler::implementations::besl::PreparedBeslShader::cache_key).
	/// A forced rebuild only reuses what it stored itself, so it regenerates everything once.
	pub(crate) async fn reusable_resource(&self, id: ResourceId<'_>) -> Option<SerializableResource> {
		if self.forced && !self.asset_manager.stored_in_process(id.as_ref()) {
			return None;
		}
		let (resource, _) = self.asset_manager.resource_storage_backend.read(id).await?;
		self.record_output(resource.id());

		Some(resource)
	}

	/// Reserves exact resource storage before a processor starts writing its payload.
	///
	/// This incremental authoring path always stores bytes uncompressed. Use
	/// [`Self::store_resource`] when the complete payload is available.
	///
	/// Write exactly `size` bytes through [`resource::ResourceTransaction::write_all`], then pass the
	/// transaction to [`Self::commit_primary`] or [`Self::commit_resource`].
	pub async fn begin_resource(
		&self,
		id: ResourceId<'_>,
		size: usize,
	) -> Result<resource::ResourceTransaction<'_>, LoadErrors> {
		self.asset_manager
			.resource_storage_backend
			.begin_resource(id, size)
			.await
			.map_err(|_| LoadErrors::FailedToStore)
	}

	/// Commits the requested primary resource after its transaction has written the declared payload.
	pub async fn commit_primary(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.commit_resource(transaction, resource).await.map(|_| ())
	}

	/// Commits a resource and records it as primary when its ID matches the current bake.
	///
	/// Generated dependencies use this path too; parent resources reference the returned metadata.
	pub async fn commit_resource(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<SerializableResource, LoadErrors> {
		let stored = transaction
			.commit(resource, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		Ok(self.mark_if_primary(stored))
	}

	/// Stores the requested resource after all of its generated dependencies are ready.
	pub async fn store_primary(&self, resource: ProcessedAsset, data: &[u8]) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.store_resource(resource, data).await.map(|_| ())
	}

	/// Stores an owned primary payload and moves large buffers directly into asynchronous file writes.
	pub async fn store_primary_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.store_resource_owned(resource, data).await.map(|_| ())
	}

	/// Stores a resource and records it as the requested primary when its ID matches the current bake.
	///
	/// Generated dependencies use this path too; parent resources reference the returned metadata.
	pub async fn store_resource(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		let stored = self
			.asset_manager
			.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		Ok(self.mark_if_primary(stored))
	}

	/// Stores an owned payload and records it as primary when its ID matches the current bake.
	pub async fn store_resource_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<SerializableResource, LoadErrors> {
		let id = ResourceId::new(resource.id());
		let storage = self.asset_manager.resource_storage_backend.as_ref();
		let transaction = resource::storage_backend::write_complete_owned_resource(
			data,
			resource.streams.as_deref(),
			storage.cpu_compression_policy(&resource),
			|size| storage.begin_resource(id, size),
		)
		.await
		.map_err(|_| LoadErrors::FailedToStore)?;

		self.commit_resource(transaction, resource).await
	}

	/// Rejects a primary store whose resource ID differs from the requested asset.
	fn ensure_primary(&self, resource: &ProcessedAsset) -> Result<(), LoadErrors> {
		if resource.id() == self.primary_id.as_ref() {
			Ok(())
		} else {
			Err(LoadErrors::PrimaryResourceIdMismatch)
		}
	}

	/// Records that the requested primary resource is stored when `stored` is that resource, and any other resource as
	/// an output of this bake.
	fn mark_if_primary(&self, stored: SerializableResource) -> SerializableResource {
		if stored.id() == self.primary_id.as_ref() {
			self.primary_stored.set(true);
		} else {
			self.record_output(stored.id());
		}
		// The store decides how an image's payload is encoded, so its options are an input of every bake that stores one.
		if stored.class() == "Image" {
			self.setting(STORE_SETTING);
		}
		self.asset_manager.mark_stored_in_process(stored.id());

		stored
	}

	/// Records `id` as a resource this bake stored besides the primary one, keeping the list sorted.
	fn record_output(&self, id: &str) {
		if id == self.primary_id.as_ref() {
			return;
		}
		let mut outputs = self.outputs.lock();
		if let Err(index) = outputs.binary_search_by(|existing| existing.as_str().cmp(id)) {
			outputs.insert(index, id.to_string());
		}
	}

	pub(crate) fn asset_storage_backend(&self) -> &'a dyn asset::DynStorageBackend {
		self.asset_storage_backend
	}

	/// Returns the image processor shared by every handler that produces images.
	///
	/// Decode pixels into an [`ImageSource`](crate::processors::image::ImageSource), then call
	/// [`ImageProcessor::process_in`] instead of building an image resource by hand, so every source format gets the
	/// same mips, compression, and packing.
	pub fn image_processor(&self) -> &'a ImageProcessor {
		&self.asset_manager.image_processor
	}

	/// Returns the allocator shared by source resolution, processing, and resource storage for this bake.
	pub fn allocator(&self) -> &'a dyn Allocator {
		self.allocator
	}

	/// Returns the renderer's program generator, which every handler that bakes materials shares.
	///
	/// Reports the error and fails when the manager has none. Next, build the material's program with
	/// [`ProgramGenerator::generate`].
	pub fn program_generator(&self) -> Result<&'a dyn ProgramGenerator, LoadErrors> {
		// The generator shapes every program it builds, so its identity is an input of each bake that takes it.
		self.setting(PROGRAM_GENERATOR_SETTING);
		self.asset_manager.program_generator.as_deref().ok_or_else(|| {
			self.error(
				"Material generation is unavailable. The most likely cause is an asset manager configured without the renderer's program generator. Install one with `AssetManager::set_program_generator`, as `configure_default_baking` does.",
			);
			LoadErrors::FailedToProcess
		})
	}

	/// Returns the renderer's transform for standalone BESL programs, or `None` when the manager has none.
	pub fn program_transform(&self) -> Option<&'a dyn ProgramTransform> {
		// Recorded even without a transform, so installing one later makes the shader stale.
		self.setting(PROGRAM_TRANSFORM_SETTING);
		self.asset_manager.program_transform.as_deref()
	}

	/// Emits an intermediate product of this bake to the manager's
	/// [`BakeInterceptor`](crate::asset::intercept::BakeInterceptor), if one wants `name`.
	///
	/// `name` is the ID of the resource the product belongs to, a `.`, and the usual file extension of its format, such
	/// as `wood.bema#shader.besl`; see [`crate::asset::intercept`]. `produce` runs only when an interceptor wants the
	/// product, so emitting one is free for a bake nobody intercepts.
	pub fn intercept(&self, name: impl fmt::Display, produce: impl FnOnce() -> Vec<u8>) {
		let Some(interceptor) = &self.asset_manager.interceptor else {
			return;
		};
		let name = name.to_string();
		if interceptor.wants(&name) {
			interceptor.receive(name, produce());
		}
	}

	/// Links and lowers `program` without compiling it, for shaders whose resource ID hashes their lowered source.
	///
	/// While an interceptor is installed, the result keeps the program's BESL source, which [`Self::compile_prepared`]
	/// emits once the ID is known. Next, name the shader, then call [`Self::compile_prepared`].
	pub(crate) fn prepare_shader(
		&self,
		program: besl::parser::Node<'_>,
		settings: &ShaderGenerationSettings,
	) -> Result<PreparedBeslShader, String> {
		// Linking and lowering are BESL's, so a change of its code is an input of every shader.
		self.setting(SHADER_CODE_SETTING);
		let source = self.asset_manager.interceptor.is_some().then(|| besl::write(&program));
		let mut prepared = prepare_besl_shader(program, settings)?;
		prepared.source = source;
		Ok(prepared)
	}

	/// Compiles `shader` with the manager's shader compiler into the shader `id`. Every shader a bake compiles goes
	/// through here.
	///
	/// Before compiling, it emits the shader's BESL source as the `<id>.besl` product, and the platform source it lowered
	/// to as `<id>.msl`, `<id>.hlsl`, or `<id>.glsl`, in the platform's shading language. After compiling, it emits the
	/// binary as `<id>.metallib` or `<id>.spv`, so tools that inspect compiled shaders, such as pipeline statistics, read
	/// exactly what the bake stored.
	pub(crate) async fn compile_prepared(
		&self,
		id: &str,
		mut shader: PreparedBeslShader,
	) -> Result<(Shader, Box<[u8]>), String> {
		if let Some(source) = shader.source.take() {
			self.intercept(format_args!("{id}.besl"), || source.into_bytes());
		}
		let language = PlatformShaderLanguage::current_platform();
		self.intercept(format_args!("{id}.{}", language.extension()), || {
			shader.platform_source().as_bytes().to_vec()
		});
		// The fingerprint is awaited before the inputs are locked, so the lock is never held across the wait.
		let compiler = self.asset_manager.shader_compiler_fingerprint().await;
		upsert_input(
			&mut self.asset_storage_backend.inputs.lock(),
			BakeInput::Setting {
				name: SHADER_COMPILER_SETTING.to_string(),
				fingerprint: compiler,
			},
		);
		let (shader, binary) = self.asset_manager.shader_compiler.compile(shader, id).await?;
		if let Some(extension) = language.binary_extension() {
			self.intercept(format_args!("{id}.{extension}"), || binary.to_vec());
		}
		Ok((shader, binary))
	}

	/// Prepares and compiles `program` into the shader named by `settings`, for shaders whose ID is known up front.
	pub(crate) async fn compile_shader(
		&self,
		program: besl::parser::Node<'_>,
		settings: ShaderGenerationSettings,
	) -> Result<(Shader, Box<[u8]>), String> {
		let shader = self.prepare_shader(program, &settings)?;
		self.compile_prepared(&settings.name, shader).await
	}
}

/// The `RecordingStorage` struct lets a handler read stored resources through the runtime's own loading code while
/// the bake records each one it reads as an input.
///
/// A handler that loads a mesh the way the renderer does also reads the mesh's materials and their images. Those reads
/// don't go through [`BakeContext::bake_dependency`], so this reader records them. It stores nothing: every write
/// fails.
struct RecordingStorage {
	inner: Arc<dyn resource::DynStorageBackend>,
	inputs: Arc<Mutex<Vec<BakeInput>>>,
}

impl RecordingStorage {
	/// Records the stored resource as an input, with the fingerprint of what was read.
	fn record(&self, stored: &SerializableResource) {
		upsert_input(
			&mut self.inputs.lock(),
			BakeInput::Resource {
				id: stored.id().to_string(),
				fingerprint: resource_fingerprint(stored),
			},
		);
	}
}

impl resource::ReadStorageBackend for RecordingStorage {
	fn list(&self) -> impl Future<Output = Result<Vec<SerializableResource>, String>> {
		self.inner.list()
	}

	fn read<'a>(
		&'a self,
		id: ResourceId<'a>,
	) -> impl Future<Output = Option<(SerializableResource, resource::resource_handler::MultiResourceReader)>> + 'a {
		async move {
			let read = self.inner.read(id).await?;
			self.record(&read.0);
			Some(read)
		}
	}

	fn query(
		&self,
		query: resource::storage_backend::Query,
	) -> impl Future<
		Output = Result<
			resource::storage_backend::QueryPage<(SerializableResource, resource::resource_handler::MultiResourceReader)>,
			resource::storage_backend::QueryError,
		>,
	> {
		async move {
			let page = self.inner.query(query).await?;
			for (stored, _) in &page.items {
				self.record(stored);
			}
			Ok(page)
		}
	}

	fn read_provenance(&self, id: ResourceId<'_>) -> Result<Option<crate::asset::Provenance>, String> {
		self.inner.read_provenance(id)
	}
}

impl resource::WriteStorageBackend for RecordingStorage {
	async fn clear(&self) -> Result<(), String> {
		Err(READ_ONLY_DURING_BAKE.to_string())
	}

	async fn delete<'a>(&'a self, _: ResourceId<'a>) -> Result<(), String> {
		Err(READ_ONLY_DURING_BAKE.to_string())
	}

	fn begin_resource<'a>(
		&'a self,
		_: ResourceId<'_>,
		_: usize,
	) -> impl Future<Output = Result<resource::ResourceTransaction<'a>, ()>> + 'a {
		// Storing goes through the bake context, which records what the bake stored.
		async { Err(()) }
	}

	fn write_provenance(&self, _: ResourceId<'_>, _: &crate::asset::Provenance) -> Result<(), String> {
		Err(READ_ONLY_DURING_BAKE.to_string())
	}
}

impl resource::StorageBackend for RecordingStorage {}

/// The error of a write through the resource manager a handler takes from its [`BakeContext`].
const READ_ONLY_DURING_BAKE: &str = "The resource store is read-only through a bake's resource manager. The most likely cause is a handler that stores or deletes through `BakeContext::resource_manager`. Store resources with `BakeContext::store_resource`.";

/// The setting that holds the identity of the manager's [`ProgramGenerator`].
pub(crate) const PROGRAM_GENERATOR_SETTING: &str = "program-generator";
/// The setting that holds the identity of the manager's [`ProgramTransform`].
pub(crate) const PROGRAM_TRANSFORM_SETTING: &str = "program-transform";
/// The setting that holds the fingerprint of the BESL code every shader links and lowers with.
pub(crate) const SHADER_CODE_SETTING: &str = "shader-code";
/// The setting that holds the fingerprint of the toolchain every shader compiles with.
pub(crate) const SHADER_COMPILER_SETTING: &str = "shader-compiler";
/// The setting that holds the options of the resource store that change a stored payload.
pub(crate) const STORE_SETTING: &str = "store";

/// Maps a failed dependency bake to the error its parent handler returns.
fn dependency_load_error(error: crate::asset::manager::LoadMessages) -> LoadErrors {
	match error {
		crate::asset::manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
		crate::asset::manager::LoadMessages::Excluded { .. } => LoadErrors::DependencyExcluded,
		crate::asset::manager::LoadMessages::FailedToBake {
			error: LoadErrors::DependencyCycle,
			..
		} => LoadErrors::DependencyCycle,
		_ => LoadErrors::FailedToProcess,
	}
}

use std::{alloc::Allocator, cell::Cell, fmt, future::Future, sync::Arc};

use utils::sync::Mutex;

#[cfg(debug_assertions)]
use crate::asset::resource_trace::ResourceTraceLevel;
use crate::asset::{
	AssetStorageBytes, BEADType, ResourceId, StorageBackend as _,
	bake_memory::BakeAllocator,
	manager::AssetManagerState,
	provenance::{BakeInput, MISSING_RESOURCE, resource_fingerprint, upsert_input},
	storage_backend::AssetVersion,
};
use crate::{
	Model, ProcessedAsset, Reference, ReferenceModel, SerializableResource, StoredModel, asset,
	asset::handler::implementations::{
		ProgramGenerator, ProgramTransform,
		besl::{PreparedBeslShader, prepare_besl_shader},
	},
	r#async::BoxedFuture,
	processors::image::ImageProcessor,
	resource,
	resources::material::Shader,
	shader::{besl::backends::platform::PlatformShaderLanguage, generator::ShaderGenerationSettings},
};
