use std::{alloc::Allocator, cell::Cell, fmt, future::Future};

#[cfg(debug_assertions)]
use super::resource_trace::{ResourceTrace, ResourceTraceLevel};
use super::{asset_manager::AssetManager, AssetStorageBytes, BEADType, ResourceId};
use crate::{asset, resource, Model, ProcessedAsset, ReferenceModel, SerializableResource};

#[derive(Debug, PartialEq, Eq)]
pub enum LoadErrors {
	AssetDoesNotExist,
	FailedToProcess,
	AssetCouldNotBeLoaded,
	UnsupportedType,
	FailedToStore,
	PrimaryResourceIdMismatch,
	PrimaryResourceNotStored,
}

/// The `BakeContext` struct provides format handlers with the shared facilities used during one asset bake.
#[derive(Clone, Copy)]
pub struct BakeContext<'a> {
	asset_manager: &'a AssetManager,
	resource_storage_backend: &'a dyn resource::StorageBackend,
	asset_storage_backend: &'a dyn asset::StorageBackend,
	allocator: &'a dyn Allocator,
	primary_id: ResourceId<'a>,
	primary_stored: &'a Cell<bool>,
	#[cfg(debug_assertions)]
	resource_trace: &'a ResourceTrace,
}

impl<'a> BakeContext<'a> {
	pub(crate) fn new(
		asset_manager: &'a AssetManager,
		resource_storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		allocator: &'a dyn Allocator,
		primary_id: ResourceId<'a>,
		primary_stored: &'a Cell<bool>,
		#[cfg(debug_assertions)] resource_trace: &'a ResourceTrace,
	) -> Self {
		Self {
			asset_manager,
			resource_storage_backend,
			asset_storage_backend,
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
			.map_err(|_| LoadErrors::AssetCouldNotBeLoaded)
	}

	/// Bakes a referenced source asset when necessary and returns its stored model.
	pub async fn bake_dependency<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadErrors> {
		self.asset_manager
			.bake_if_not_exists_in(id, self.resource_storage_backend, self.allocator)
			.await
			.map_err(|error| match error {
				super::asset_manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
				_ => LoadErrors::FailedToProcess,
			})
	}

	/// Stores the requested resource after all of its generated dependencies are ready.
	pub fn store_primary(&self, resource: ProcessedAsset, data: &[u8]) -> Result<(), LoadErrors> {
		if resource.id != self.primary_id.as_ref() {
			return Err(LoadErrors::PrimaryResourceIdMismatch);
		}
		self.store_resource(resource, data).map(|_| ())
	}

	/// Stores a resource and records it as the requested primary when its ID matches the current bake.
	pub fn store_resource(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		let is_primary = resource.id == self.primary_id.as_ref();
		let resource = self
			.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.map_err(|_| LoadErrors::FailedToStore)?;
		if is_primary {
			self.primary_stored.set(true);
		}
		Ok(resource)
	}

	/// Stores a generated dependency and returns the serialized metadata used by parent resources.
	pub fn store_generated(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		self.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.map_err(|_| LoadErrors::FailedToStore)
	}

	pub(crate) fn asset_storage_backend(&self) -> &'a dyn asset::StorageBackend {
		self.asset_storage_backend
	}

	/// Returns the allocator shared by source resolution, processing, and resource storage for this bake.
	pub fn allocator(&self) -> &'a dyn Allocator {
		self.allocator
	}
}

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
