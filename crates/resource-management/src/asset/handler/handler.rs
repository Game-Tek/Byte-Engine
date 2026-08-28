/// The `AssetHandler` trait provides source-format extensions for asset baking.
///
/// See the [assets guide](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management/assets)
/// before implementing a new source-format handler.
pub trait AssetHandler {
	fn can_handle(&self, r#type: &str) -> bool;

	/// Returns whether recursive asset discovery should include a source handled by this implementation.
	fn should_discover(&self, _id: ResourceId<'_>, _has_sidecar: bool) -> bool {
		true
	}

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> impl Future<Output = Result<(), LoadErrors>>;
}

pub trait DynAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]

pub enum LoadErrors {
	AssetDoesNotExist,
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
			Self::AssetDoesNotExist => "The source asset does not exist.",
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
			Self::AssetDoesNotExist | Self::AssetCouldNotBeRead => {
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
pub(crate) struct TrackingStorageBackend<'a> {
	inner: &'a dyn asset::DynStorageBackend,
	dependencies: &'a Mutex<Vec<AssetDependency>>,
}

impl<'a> TrackingStorageBackend<'a> {
	/// Creates a source backend that records stable versions after successful reads.
	pub(crate) fn new(inner: &'a dyn asset::DynStorageBackend, dependencies: &'a Mutex<Vec<AssetDependency>>) -> Self {
		Self { inner, dependencies }
	}

	/// Records the latest observed version once when handlers resolve the same source repeatedly.
	fn record(&self, id: ResourceId<'_>, version: AssetVersion) {
		let dependency = AssetDependency::new(id, version);

		let mut dependencies = self.dependencies.lock();

		if let Some(existing) = dependencies.iter_mut().find(|existing| existing.id() == dependency.id()) {
			*existing = dependency;
		} else {
			dependencies.push(dependency);
		}
	}

	/// Resolves one source and rejects a result if its filesystem identity changed during the read.
	fn resolve_tracked<'b>(
		&'b self,
		url: ResourceId<'b>,
		allocator: Option<&'b dyn Allocator>,
	) -> crate::r#async::BoxedFuture<'b, Result<(AssetStorageBytes<'b>, Option<BEADType>, String), ()>> {
		crate::r#async::future(async move {
			let before = self.inner.version(url).await?;

			let resolved = match allocator {
				Some(allocator) => self.inner.resolve_in(url, allocator).await?,
				None => self.inner.resolve(url).await?,
			};

			let after = self.inner.version(url).await?;

			if before != after {
				log::warn!(
					"Asset changed while it was being read. The most likely cause is that '{}' was saved during the bake; retry the request.",
					url.as_ref()
				);

				return Err(());
			}

			self.record(url, after);

			Ok(resolved)
		})
	}
}

impl asset::StorageBackend for TrackingStorageBackend<'_> {
	fn directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.inner.directory_accessible(path)
	}

	fn resolve<'a>(
		&'a self,
		url: ResourceId<'a>,
	) -> impl std::future::Future<Output = Result<(AssetStorageBytes<'a>, Option<BEADType>, String), ()>> + 'a {
		self.resolve_tracked(url, None)
	}

	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> impl std::future::Future<Output = Result<(AssetStorageBytes<'a>, Option<BEADType>, String), ()>> + 'a {
		self.resolve_tracked(url, Some(allocator))
	}

	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl std::future::Future<Output = Result<AssetVersion, ()>> + 'a {
		self.inner.version(url)
	}
}

/// The `BakeContext` struct provides format handlers with the shared facilities used during one asset bake.
#[derive(Clone, Copy)]

pub struct BakeContext<'a> {
	asset_manager: &'a AssetManagerState,
	resource_storage_backend: &'a dyn resource::DynStorageBackend,
	asset_storage_backend: &'a dyn asset::DynStorageBackend,
	asset_dependencies: &'a Mutex<Vec<AssetDependency>>,
	allocator: &'a BakeAllocator,
	primary_id: ResourceId<'a>,
	primary_stored: &'a Cell<bool>,
	#[cfg(debug_assertions)]
	resource_trace: &'a ResourceTrace,
}

impl<'a> BakeContext<'a> {
	pub(in crate::asset) fn new(
		asset_manager: &'a AssetManagerState,
		resource_storage_backend: &'a dyn resource::DynStorageBackend,
		asset_storage_backend: &'a dyn asset::DynStorageBackend,
		asset_dependencies: &'a Mutex<Vec<AssetDependency>>,
		allocator: &'a BakeAllocator,
		primary_id: ResourceId<'a>,
		primary_stored: &'a Cell<bool>,
		#[cfg(debug_assertions)] resource_trace: &'a ResourceTrace,
	) -> Self {
		Self {
			asset_manager,
			resource_storage_backend,
			asset_storage_backend,
			asset_dependencies,
			allocator,
			primary_id,
			primary_stored,
			#[cfg(debug_assertions)]
			resource_trace,
		}
	}

	/// Adds an informational item to this resource's development trace and terminal log.
	pub fn info(&self, message: impl fmt::Display) {
		#[cfg(debug_assertions)]
		{
			let message = message.to_string();

			log::info!("{message}");

			self.resource_trace.record(self.primary_id, ResourceTraceLevel::Info, message);
		}

		#[cfg(not(debug_assertions))]

		log::info!("{message}");
	}

	/// Adds a warning item to this resource's development trace and terminal log.
	pub fn warn(&self, message: impl fmt::Display) {
		#[cfg(debug_assertions)]
		{
			let message = message.to_string();

			log::warn!("{message}");

			self.resource_trace.record(self.primary_id, ResourceTraceLevel::Warn, message);
		}

		#[cfg(not(debug_assertions))]

		log::warn!("{message}");
	}

	/// Adds an error item to this resource's development trace and terminal log.
	///
	/// The item remains available when the handler returns an error and does not
	/// store the requested resource.
	pub fn error(&self, message: impl fmt::Display) {
		#[cfg(debug_assertions)]
		{
			let message = message.to_string();

			log::error!("{message}");

			self.resource_trace
				.record(self.primary_id, ResourceTraceLevel::Error, message);
		}

		#[cfg(not(debug_assertions))]

		log::error!("{message}");
	}

	/// Returns the resource type used to select and validate a handler.
	pub fn resource_type<'b>(&'b self, id: ResourceId<'b>) -> Option<&'b str> {
		self.resource_storage_backend.get_type(id)
	}

	/// Resolves source bytes and their optional BEAD description with the bake allocator.
	pub async fn resolve<'b>(
		&'b self,
		id: ResourceId<'b>,
	) -> Result<(AssetStorageBytes<'b>, Option<BEADType>, String), LoadErrors> {
		self.asset_storage_backend
			.resolve_in(id, self.allocator)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Bakes a referenced source asset when necessary and returns its stored model.
	pub async fn bake_dependency<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadErrors> {
		let resource = self
			.asset_manager
			.bake_if_not_exists_in(id, self.allocator)
			.await
			.map_err(|error| match error {
				crate::asset::manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
				_ => LoadErrors::FailedToProcess,
			})?;

		self.inherit_dependency_provenance(&resource);

		Ok(resource.into())
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

		let max_concurrency = max_concurrency.max(1);

		let requests = ids.iter().enumerate().map(|(index, id)| async move {
			self.asset_manager
				.dispatch_bake_in_scope(id, true, self.allocator.memory_scope().cloned())
				.await
				.map_err(|error| match error {
					crate::asset::manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
					_ => LoadErrors::FailedToProcess,
				})?;

			let Some((resource, _)) = self.resource_storage_backend.read(ResourceId::new(id)).await else {
				return Err(LoadErrors::FailedToProcess);
			};

			Ok((index, resource))
		});

		let completed = utils::r#async::stream::iter(requests)
			.buffer_unordered(max_concurrency)
			.collect::<Vec<_>>()
			.await;

		let mut completed = completed.into_iter().collect::<Result<Vec<_>, _>>()?;

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
		let mut dependencies = self.asset_dependencies.lock();

		for dependency in resource.asset_dependencies() {
			if let Some(existing) = dependencies.iter_mut().find(|existing| existing.id() == dependency.id()) {
				*existing = dependency.clone();
			} else {
				dependencies.push(dependency.clone());
			}
		}
	}

	/// Reserves exact resource storage before a processor starts writing its payload.
	///
	/// This incremental authoring path always stores bytes uncompressed. Use
	/// [`Self::store_resource`] when the complete payload is available.
	///
	/// Write exactly `size` bytes through [`resource::ResourceTransaction::write_all`], then pass the
	/// transaction to [`Self::commit_primary`], [`Self::commit_resource`], or
	/// [`Self::commit_generated`].
	pub async fn begin_resource(
		&self,
		id: ResourceId<'_>,
		size: usize,
	) -> Result<resource::ResourceTransaction<'_>, LoadErrors> {
		self.resource_storage_backend
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
		if resource.id() != self.primary_id.as_ref() {
			return Err(LoadErrors::PrimaryResourceIdMismatch);
		}

		self.commit_resource(transaction, resource).await.map(|_| ())
	}

	/// Commits a resource and records it as primary when its ID matches the current bake.
	pub async fn commit_resource(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<SerializableResource, LoadErrors> {
		let is_primary = resource.id() == self.primary_id.as_ref();
		let resource = resource.with_asset_dependencies(self.sorted_asset_dependencies());
		let resource = transaction
			.commit(resource, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		if is_primary {
			self.primary_stored.set(true);
		}

		Ok(resource)
	}

	/// Commits a generated dependency without marking the requested primary as stored.
	pub async fn commit_generated(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<SerializableResource, LoadErrors> {
		let resource = resource.with_asset_dependencies(self.sorted_asset_dependencies());

		transaction
			.commit(resource, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)
	}

	/// Stores the requested resource after all of its generated dependencies are ready.
	pub async fn store_primary(&self, resource: ProcessedAsset, data: &[u8]) -> Result<(), LoadErrors> {
		if resource.id != self.primary_id.as_ref() {
			return Err(LoadErrors::PrimaryResourceIdMismatch);
		}

		self.store_resource(resource, data).await.map(|_| ())
	}

	/// Stores an owned primary payload and moves large buffers directly into asynchronous file writes.
	pub async fn store_primary_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<(), LoadErrors> {
		if resource.id != self.primary_id.as_ref() {
			return Err(LoadErrors::PrimaryResourceIdMismatch);
		}

		self.store_resource_owned(resource, data).await.map(|_| ())
	}

	/// Stores a resource and records it as the requested primary when its ID matches the current bake.
	pub async fn store_resource(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		let is_primary = resource.id == self.primary_id.as_ref();

		let resource = resource.with_asset_dependencies(self.sorted_asset_dependencies());

		let resource = self
			.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		if is_primary {
			self.primary_stored.set(true);
		}

		Ok(resource)
	}

	/// Stores an owned payload and records it as primary when its ID matches the current bake.
	pub async fn store_resource_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<SerializableResource, LoadErrors> {
		let transaction = write_complete_owned_resource(self.resource_storage_backend, &resource, data).await?;

		self.commit_resource(transaction, resource).await
	}

	/// Stores a generated dependency and returns the serialized metadata used by parent resources.
	pub async fn store_generated(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		let resource = resource.with_asset_dependencies(self.sorted_asset_dependencies());

		self.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)
	}

	/// Stores an owned generated dependency without marking the requested primary as stored.
	pub async fn store_generated_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<SerializableResource, LoadErrors> {
		let transaction = write_complete_owned_resource(self.resource_storage_backend, &resource, data).await?;

		self.commit_generated(transaction, resource).await
	}

	/// Returns deterministic source provenance for persisted resource metadata.
	fn sorted_asset_dependencies(&self) -> Vec<AssetDependency> {
		let mut dependencies = self.asset_dependencies.lock().clone();

		dependencies.sort_by(|left, right| left.id().cmp(right.id()));

		dependencies
	}

	pub(crate) fn asset_storage_backend(&self) -> &'a dyn asset::DynStorageBackend {
		self.asset_storage_backend
	}

	/// Returns the allocator shared by source resolution, processing, and resource storage for this bake.
	pub fn allocator(&self) -> &'a dyn Allocator {
		self.allocator
	}
}

/// Writes a complete owned payload after applying the backend and per-resource CPU compression policy.
async fn write_complete_owned_resource<'a, T: compio::buf::IoBuf>(
	storage: &'a dyn resource::DynWriteStorageBackend,
	resource: &ProcessedAsset,
	data: T,
) -> Result<resource::ResourceTransaction<'a>, LoadErrors> {
	let id = ResourceId::new(resource.id());
	resource::storage_backend::write_complete_owned_resource(data, storage.cpu_compression_policy(resource), |size| {
		storage.begin_resource(id, size)
	})
	.await
	.map_err(|_| LoadErrors::FailedToStore)
}

use std::{alloc::Allocator, cell::Cell, fmt, future::Future};

use utils::sync::Mutex;

#[cfg(debug_assertions)]
use crate::asset::resource_trace::{ResourceTrace, ResourceTraceLevel};
use crate::asset::{
	AssetStorageBytes, BEADType, ResourceId,
	bake_memory::BakeAllocator,
	manager::AssetManagerState,
	storage_backend::{AssetDependency, AssetVersion},
};
use crate::{Model, ProcessedAsset, ReferenceModel, SerializableResource, asset, r#async::BoxedFuture, resource};
