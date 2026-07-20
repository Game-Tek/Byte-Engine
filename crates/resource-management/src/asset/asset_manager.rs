use std::{
	alloc::{Allocator, Global},
	cell::Cell,
};

use super::{
	asset_handler::{AssetHandler, BakeContext},
	StorageBackend,
};
use crate::{
	asset::{self, asset_handler::LoadErrors, ResourceId},
	r#async::BoxedFuture,
	resource::{self, StorageBackend as ResourceStorageBackend},
	Model, ProcessedAsset, ReferenceModel,
};

trait AbstractAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;
	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

/// The `AssetManager` struct selects asset handlers and bakes source assets into resource storage.
///
/// Register each source format with [`Self::add_asset_handler`], then install the
/// manager on a [`crate::ResourceManager`] for debug loading or call
/// [`Self::bake`] from an explicit baking workflow.
pub struct AssetManager {
	asset_handlers: Vec<Box<dyn AbstractAssetHandler>>,
	storage_backend: Box<dyn StorageBackend>,
}

/// The `LoadMessages` enum identifies failures while an asset is loaded, baked, or stored.
#[derive(Debug, PartialEq, Eq)]
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
}

impl AssetManager {
	/// Creates an asset manager over the source-asset storage backend.
	///
	/// Next, register all required formats with [`Self::add_asset_handler`] before
	/// installing the manager or starting a bake.
	pub fn new<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
		Self {
			asset_handlers: Vec::with_capacity(8),
			storage_backend: Box::new(storage_backend),
		}
	}

	/// Registers a handler for one family of source assets.
	///
	/// After all handlers are registered, install this manager on a
	/// [`crate::ResourceManager`] in a debug build or call [`Self::bake`].
	pub fn add_asset_handler<T: AssetHandler + Send + Sync + 'static>(&mut self, asset_handler: T) {
		struct AssetHandlerWrapper<T: AssetHandler + Send + Sync>(T);

		impl<T: AssetHandler + Send + Sync> AbstractAssetHandler for AssetHandlerWrapper<T> {
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

		self.asset_handlers.push(Box::new(AssetHandlerWrapper(asset_handler)));
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.as_ref()
	}

	/// Returns whether a registered asset handler can bake the given source ID.
	pub fn supports(&self, id: &str) -> bool {
		let id = ResourceId::new(id);
		self.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_extension()))
	}

	/// Returns whether recursive discovery should include the given supported source asset.
	pub fn should_discover(&self, id: &str, has_sidecar: bool) -> bool {
		let id = ResourceId::new(id);
		self.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_extension()) && handler.should_discover(id, has_sidecar))
	}

	/// Bakes the asset at `id` without checking for an existing stored resource.
	///
	/// Next, request the stored output through [`crate::ResourceManager::request`]
	/// or inspect it through the storage backend.
	pub async fn bake<'a>(&self, id: &str, resource_storage_backend: &dyn ResourceStorageBackend) -> Result<(), LoadMessages> {
		self.bake_in(id, resource_storage_backend, &Global).await
	}

	/// Bakes an asset while using the provided allocator for generation-time buffers.
	pub async fn bake_in<'a>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
		allocator: &dyn Allocator,
	) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		let asset_handler = match self
			.asset_handlers
			.iter()
			.find(|handler| handler.can_handle(id.get_extension()))
		{
			Some(handler) => handler,
			None => {
				log::warn!("No asset handler found for asset: {:#?}", id);
				return Err(LoadMessages::NoAssetHandler);
			}
		};

		let start_time = std::time::Instant::now();

		// The shared flag enforces the primary-write contract without rereading potentially expensive storage.
		let primary_stored = Cell::new(false);
		let context = BakeContext::new(
			self,
			resource_storage_backend,
			self.storage_backend.as_ref(),
			allocator,
			id,
			&primary_stored,
		);
		match asset_handler.bake(context, id).await {
			Ok(()) if primary_stored.get() => {}
			Ok(()) => {
				return Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error: LoadErrors::PrimaryResourceNotStored,
				});
			}
			Err(LoadErrors::FailedToStore) => {
				return Err(LoadMessages::FailedToStore {
					asset: id.to_string(),
					error: format!(
						"Failed to store asset {:#?}. The resource storage backend likely rejected the primary resource write.",
						id
					),
				});
			}
			Err(error) => {
				log::error!("Failed to bake asset: {:#?}", error);
				return Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error,
				});
			}
		}

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		Ok(())
	}

	/// Returns the stored asset, or bakes it when no resource with a matching hash exists.
	pub async fn bake_if_not_exists<'a, M: Model>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
	) -> Result<ReferenceModel<M>, LoadMessages> {
		self.bake_if_not_exists_in(id, resource_storage_backend, &Global).await
	}

	/// Bakes an asset with the provided allocator if the resource does not already exist.
	pub async fn bake_if_not_exists_in<'a, M: Model>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
		allocator: &dyn Allocator,
	) -> Result<ReferenceModel<M>, LoadMessages> {
		let id = ResourceId::new(id);

		if resource_storage_backend.read(id).is_none() {
			self.bake_in(id.as_ref(), resource_storage_backend, allocator).await?;
		}

		if let Some(result) = resource_storage_backend.read(id) {
			let (resource, _) = result;
			let resource: ReferenceModel<M> = resource.into();
			return Ok(resource);
		}

		Err(LoadMessages::NoAsset)
	}
}

#[cfg(test)]
pub mod tests {
	use super::*;
	use crate::{
		asset::{asset_handler::LoadErrors, storage_backend::tests::TestStorageBackend},
		r#async::{self, BoxedFuture},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		Model, ProcessedAsset,
	};

	#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct TestResource {}

	impl Model for TestResource {
		fn get_class() -> &'static str {
			"TestResource"
		}
	}

	struct TestAssetHandler {}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	impl AssetHandler for TestAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			match id.get_base().as_ref() {
				"example.test" => context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]),
				"unstored.test" => Ok(()),
				"mismatched.test" => {
					context.store_primary(ProcessedAsset::new(ResourceId::new("other.test"), TestResource {}), &[])
				}
				_ => Err(LoadErrors::AssetCouldNotBeLoaded),
			}
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		let storage_backend = TestStorageBackend::new();
		AssetManager::new(storage_backend)
	}

	#[test]
	fn test_new() {
		let _ = new_testing_asset_manager();
	}

	#[test]
	fn test_add_asset_manager() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	fn asset_manager_reports_support_for_registered_asset_types() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.supports("nested/example.test"));
		assert!(asset_manager.supports("nested/example.test#fragment"));
		assert!(!asset_manager.supports("nested/example.unknown"));
	}

	#[test]
	fn registered_handlers_are_discoverable_by_default() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.should_discover("nested/example.test", false));
		assert!(asset_manager.should_discover("nested/example.test", true));
		assert!(!asset_manager.should_discover("nested/example.unknown", true));
	}

	#[r#async::test]
	async fn test_bake_with_asset_manager() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		asset_manager
			.bake("example.test", &resource_storage_backend)
			.await
			.expect("registered asset handler should bake its resource");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("example.test"))
			.expect("baked resource should be stored");
		assert_eq!(resource.class, "TestResource");
	}

	#[r#async::test]
	async fn test_bake_no_asset_handler() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let asset_manager = AssetManager::new(storage_backend);

		let result = asset_manager.bake("example.unknown", &resource_storage_backend).await;

		assert_eq!(result, Err(LoadMessages::NoAssetHandler));
	}

	#[r#async::test]
	async fn successful_handler_must_store_the_requested_primary_resource() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("unstored.test", &resource_storage_backend).await;

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
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("mismatched.test", &resource_storage_backend).await;

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
