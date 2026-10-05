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

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> impl Future<Output = Result<(), LoadErrors>>;
}

/// The `DynAssetHandler` trait lets the asset manager keep handlers of different types in one list.
///
/// Every [`AssetHandler`] implements it; register handlers with
/// [`AssetManager::add_asset_handler`](crate::asset::manager::AssetManager::add_asset_handler).
pub trait DynAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

impl<T: AssetHandler + Send + Sync> DynAssetHandler for T {
	fn can_handle(&self, r#type: &str) -> bool {
		AssetHandler::can_handle(self, r#type)
	}

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool {
		AssetHandler::should_discover(self, id, has_sidecar)
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
}

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
		}
	}

	/// Returns the recovery step most likely to resolve this asset loading failure.
	pub(crate) const fn fix(&self) -> &'static str {
		match self {
			Self::AssetCouldNotBeRead => {
				"Check the asset ID and configured assets directory. Engine asset IDs start with 'byte-engine/'."
			}
			Self::FailedToProcess | Self::AssetCouldNotBeLoaded => {
				"Check the source asset and its dependencies for invalid or unsupported data."
			}
			Self::UnsupportedType => "Use a supported asset type or register an asset handler for it.",
			Self::FailedToStore => "Check that the resource destination is writable, then retry.",
			Self::PrimaryResourceIdMismatch => "Store the primary resource under the requested asset ID.",
			Self::PrimaryResourceNotStored => "Make the asset handler store its primary resource before returning.",
		}
	}
}

/// The `TrackingStorageBackend` struct records every source resolved during one asset bake.
///
/// It records a source's version only after a successful read whose version matched on both sides of the read.
pub(crate) struct TrackingStorageBackend<'a> {
	pub(in crate::asset) inner: &'a dyn asset::DynStorageBackend,
	/// The provenance attached to every resource the bake stores, kept sorted by source ID so it persists
	/// deterministically.
	pub(in crate::asset) dependencies: &'a Mutex<Vec<AssetDependency>>,
}

impl TrackingStorageBackend<'_> {
	/// Runs `read` between two version checks of `id` and records its version, rejecting the result when `id` changed
	/// during the read.
	async fn tracked<T, F: Future<Output = Result<T, ()>>>(
		&self,
		id: ResourceId<'_>,
		read: impl FnOnce() -> F,
	) -> Result<T, ()> {
		let before = self.inner.version(id).await?;
		let resolved = read().await?;
		let after = self.inner.version(id).await?;

		if before != after {
			log::warn!(
				"Asset changed while it was being read. The most likely cause is that '{}' was saved during the bake; retry the request.",
				id.as_ref()
			);

			return Err(());
		}

		// Records the latest observed version once when handlers resolve the same source repeatedly.
		upsert_dependency(&mut self.dependencies.lock(), AssetDependency::new(id, after));

		Ok(resolved)
	}
}

impl asset::StorageBackend for TrackingStorageBackend<'_> {
	fn directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.inner.directory_accessible(path)
	}

	async fn resolve<'a>(&'a self, url: ResourceId<'a>) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(url, || self.inner.resolve(url)).await
	}

	async fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(url, || self.inner.resolve_in(url, allocator)).await
	}

	async fn load_sidecar<'a>(&'a self, url: ResourceId<'a>) -> Result<Option<BEADType>, ()> {
		// Track absence as well as content, so creating a sidecar invalidates the baked resource.
		let path = format!("{}.bead", url.get_base().as_ref());
		self.tracked(ResourceId::new(&path), || self.inner.load_sidecar(url)).await
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
}

impl<'a> BakeContext<'a> {
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

	/// Loads optional BEAD settings and records their presence or absence as a bake dependency.
	///
	/// Call this only when the handler consumes sidecar settings, then read the source with [`Self::resolve`].
	pub async fn load_sidecar(&self, id: ResourceId<'_>) -> Result<Option<BEADType>, LoadErrors> {
		self.asset_storage_backend
			.load_sidecar(id)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Bakes a referenced source asset on the shared worker pool when necessary and returns its stored model.
	///
	/// Concurrent calls from one handler bake on separate workers, each in its own arena.
	pub async fn bake_dependency<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadErrors> {
		Ok(self.bake_dependencies(&[id.to_owned()], 1).await?.remove(0))
	}

	/// Bakes independent dependencies on the shared worker pool while bounding active requests.
	///
	/// Results preserve the input order. Each completed request returns its already-read
	/// resource so provenance does not require another serialized storage pass.
	pub async fn bake_dependencies<M: Model>(
		&self,
		ids: &[String],
		max_concurrency: usize,
	) -> Result<Vec<ReferenceModel<M>>, LoadErrors> {
		use utils::r#async::StreamExt as _;

		let requests = ids.iter().enumerate().map(|(index, id)| async move {
			self.asset_manager
				.dispatch_bake_in_scope(
					id,
					true,
					crate::asset::manager::BakeOrigin::Dependency(self.allocator.memory_scope().cloned()),
				)
				.await
				.map_err(dependency_load_error)?;

			let Some((resource, _)) = self.asset_manager.resource_storage_backend.read(ResourceId::new(id)).await else {
				return Err(LoadErrors::FailedToProcess);
			};

			Ok((index, resource))
		});

		let mut completed = utils::r#async::stream::iter(requests)
			.buffer_unordered(max_concurrency.max(1))
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect::<Result<Vec<_>, _>>()?;

		completed.sort_unstable_by_key(|(index, _)| *index);

		let mut dependencies = Vec::with_capacity(completed.len());

		for (_, resource) in completed {
			self.inherit_dependency_provenance(&resource);

			dependencies.push(resource.into());
		}

		Ok(dependencies)
	}

	/// Adds one stored dependency's transitive source versions to the parent bake.
	fn inherit_dependency_provenance(&self, resource: &SerializableResource) {
		let mut dependencies = self.asset_storage_backend.dependencies.lock();

		for dependency in resource.asset_dependencies() {
			upsert_dependency(&mut dependencies, dependency.clone());
		}
	}

	/// Returns a stored resource that the handler may reference instead of producing it again.
	///
	/// Use it only for content-addressed IDs, where the ID changes whenever anything that affects the content
	/// changes. Generated material shaders use it with
	/// [`PreparedBeslShader::cache_key`](crate::asset::handler::implementations::besl::PreparedBeslShader::cache_key).
	/// Resources baked before the cutoff set by
	/// [`AssetManager::rebuild_resources_baked_before`](crate::asset::manager::AssetManager::rebuild_resources_baked_before)
	/// are not returned, so a forced rebuild regenerates them.
	pub(crate) async fn reusable_resource(&self, id: ResourceId<'_>) -> Option<SerializableResource> {
		let (resource, _) = self.asset_manager.resource_storage_backend.read(id).await?;

		(!self.asset_manager.precedes_rebuild_cutoff(&resource)).then_some(resource)
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
		let resource = resource.with_asset_dependencies(self.asset_storage_backend.dependencies.lock().clone());
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
		let resource = resource.with_asset_dependencies(self.asset_storage_backend.dependencies.lock().clone());

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
		let transaction =
			resource::storage_backend::write_complete_owned_resource(data, storage.cpu_compression_policy(&resource), |size| {
				storage.begin_resource(id, size)
			})
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

	/// Records that the requested primary resource is stored when `stored` is that resource.
	fn mark_if_primary(&self, stored: SerializableResource) -> SerializableResource {
		if stored.id() == self.primary_id.as_ref() {
			self.primary_stored.set(true);
		}

		stored
	}

	pub(crate) fn asset_storage_backend(&self) -> &'a dyn asset::DynStorageBackend {
		self.asset_storage_backend
	}

	/// Returns the allocator shared by source resolution, processing, and resource storage for this bake.
	pub fn allocator(&self) -> &'a dyn Allocator {
		self.allocator
	}
}

/// Replaces the recorded version of a source already in `dependencies`, or inserts it so the list stays sorted by ID.
fn upsert_dependency(dependencies: &mut Vec<AssetDependency>, dependency: AssetDependency) {
	match dependencies.binary_search_by(|existing| existing.id().cmp(dependency.id())) {
		Ok(index) => dependencies[index] = dependency,
		Err(index) => dependencies.insert(index, dependency),
	}
}

/// Maps a failed dependency bake to the error its parent handler returns.
fn dependency_load_error(error: crate::asset::manager::LoadMessages) -> LoadErrors {
	match error {
		crate::asset::manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
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
	storage_backend::{AssetDependency, AssetVersion},
};
use crate::{Model, ProcessedAsset, ReferenceModel, SerializableResource, asset, r#async::BoxedFuture, resource};
