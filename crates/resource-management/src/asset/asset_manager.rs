use crate::{asset::ResourceId, resource::StorageBackend as ResourceStorageBackend, Model, ReferenceModel};

use super::{asset_handler::AssetHandler, StorageBackend};

pub struct AssetManager {
	asset_handlers: Vec<Box<dyn AssetHandler>>,
	storage_backend: Box<dyn StorageBackend>,
}

/// Enumeration of the possible messages that can be returned when loading an asset.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadMessages {
	NoAsset,
	IO,
	/// The URL was missing in the asset JSON.
	NoURL,
	/// No asset handler was found for the asset.
	NoAssetHandler,
	FailedToBake {
		asset: String,
		error: String,
	},
}

impl AssetManager {
	pub fn new<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
		Self {
			asset_handlers: Vec::with_capacity(8),
			storage_backend: Box::new(storage_backend),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.as_ref()
	}

	/// Load a source asset from a JSON asset description.
	pub async fn bake<'a>(&self, id: &str, resource_storage_backend: &dyn ResourceStorageBackend) -> Result<(), LoadMessages> {
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

		let (resource, buffer) = match asset_handler
			.bake(self, resource_storage_backend, self.storage_backend.as_ref(), id)
			.await
		{
			Ok(baked_asset) => baked_asset,
			Err(error) => {
				log::error!("Failed to bake asset: {:#?}", error);
				return Err(LoadMessages::NoAsset);
			}
		};

		if let Err(_) = resource_storage_backend.store(&resource, &buffer) {
			return Err(LoadMessages::FailedToBake {
				asset: id.to_string(),
				error: "Failed to store baked resource".to_string(),
			});
		}

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		Ok(())
	}

	/// Generates a resource from a loaded asset.
	/// Does nothing if the resource already exists (with a matching hash).
	pub async fn load<'a, M: Model + for<'de> serde::Deserialize<'de>>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
	) -> Result<ReferenceModel<M>, LoadMessages> {
		let id = ResourceId::new(id);

		if resource_storage_backend.read(id).is_none() {
			self.bake(id.as_ref(), resource_storage_backend).await?;
		}

		if let Some(result) = resource_storage_backend.read(id) {
			let (resource, _) = result;
			let resource: ReferenceModel<M> = resource.into();
			return Ok(resource);
		}

		Err(LoadMessages::FailedToBake {
			asset: id.to_string(),
			error: format!("{:#?}", id),
		})
	}
}

#[cfg(test)]
pub mod tests {
	use utils::json;

	use crate::{
		asset::{asset_handler::LoadErrors, storage_backend::tests::TestStorageBackend},
		r#async::BoxedFuture,
		Model, ProcessedAsset,
	};

	use super::*;

	#[derive(serde::Serialize, serde::Deserialize)]
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
			id == "example"
		}

		fn bake<'a>(
			&'a self,
			_: &'a AssetManager,
			_: &'a dyn ResourceStorageBackend,
			_: &'a dyn StorageBackend,
			id: ResourceId<'a>,
		) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
			Box::pin(async move {
				if id.get_base().as_ref() == "example" {
					Ok((ProcessedAsset::new(id, TestResource {}), Vec::new().into_boxed_slice()))
				} else {
					Err(LoadErrors::AssetCouldNotBeLoaded)
				}
			})
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
	#[ignore = "Need to solve DI"]
	fn test_load_with_asset_manager() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let _: json::Value = json::from_str(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let storage_backend = TestStorageBackend::new();
		let _asset_manager = AssetManager::new(storage_backend);

		let _: json::Value = json::from_str(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let storage_backend = TestStorageBackend::new();
		let _asset_manager = AssetManager::new(storage_backend);

		let _: json::Value = json::from_str(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}
