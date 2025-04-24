use crate::{asset::ResourceId, resource::StorageBackend as ResourceStorageBackend, Description, Model, ProcessedAsset, ReferenceModel};

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
	pub fn bake<'a>(&self, id: &str, resource_storage_backend: &dyn ResourceStorageBackend) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		let asset_handler = match self.asset_handlers.iter().find(|handler| handler.can_handle(id.get_extension())) {
            Some(handler) => handler,
            None => {
                log::warn!("No asset handler found for asset: {:#?}", id);
                return Err(LoadMessages::NoAssetHandler);
            }
        };

		let start_time = std::time::Instant::now();

		let asset = match asset_handler.load(self, resource_storage_backend, self.storage_backend.as_ref(), id) {
            Ok(asset) => asset,
            Err(error) => {
                log::error!("Failed to load asset: {:#?}", error);
                return Err(LoadMessages::NoAsset);
            }
        };

		let dependencies = asset.requested_assets();

		log::trace!("Baking '{:#?}' asset{}{}{}", id, if dependencies.is_empty() { "" } else { " => [" }, dependencies.join(", "), if dependencies.is_empty() { "" } else { "]" });

		let bake_result = asset.load(self, resource_storage_backend, self.storage_backend.as_ref(), id);

		match bake_result {
            Ok(_) => {
		        log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());
            },
            Err(error) => {
                log::error!("Failed to bake asset: {:#?}", error);
                return Err(LoadMessages::NoAsset);
            }
        }

		Ok(())
	}

	/// Generates a resource from a loaded asset.
	/// Does nothing if the resource already exists (with a matching hash).
	pub fn load<'a, M: Model + for <'de> serde::Deserialize<'de>>(&self, id: &str, resource_storage_backend: &dyn ResourceStorageBackend) -> Result<ReferenceModel<M>, LoadMessages> {
		let id = ResourceId::new(id);

		let asset_handler = match self.asset_handlers.iter().find(|handler| handler.can_handle(id.get_extension())) {
			Some(handler) => handler,
			None => {
				log::warn!("No asset handler found for asset: {:#?}", id);
				return Err(LoadMessages::NoAssetHandler);
			}
		};

		let asset_loader = match asset_handler.load(self, resource_storage_backend, self.storage_backend.as_ref(), id) {
			Ok(asset) => asset,
			Err(error) => {
				log::error!("Failed to load asset: {:#?}", error);
				return Err(LoadMessages::NoAsset);
			}
		};

		asset_loader.load(self, resource_storage_backend, self.storage_backend.as_ref(), id).or_else(|error| {
			log::error!("Failed to load asset: {:#?}", error);
			Err(LoadMessages::NoAsset)
		})?;

        if let Some(result) = resource_storage_backend.read(id) {
			let (resource, _) = result;
			let resource: ReferenceModel<M> = resource.into();
			return Ok(resource);
		}

		Err(LoadMessages::FailedToBake { asset: id.to_string(), error: format!("{:#?}", id) })
	}

	/// Generates a resource from a description and data.
	/// Does nothing if the resource already exists (with a matching hash).
	pub fn produce<'a, D: Description>(&self, id: ResourceId<'_>, resource_type: &str, description: &D, data: Box<[u8]>, resource_storage_backend: &dyn ResourceStorageBackend) -> ProcessedAsset {
		let asset_handler = self.asset_handlers.iter().find(|asset_handler| asset_handler.can_handle(resource_type)).expect("No asset handler found for class");

		let start_time = std::time::Instant::now();

		let (resource, buffer) = match asset_handler.produce(id, description, data) {
			Ok(x) => x,
			Err(error) => {
				log::error!("Failed to produce resource: {}", error);
				panic!("Failed to produce resource");
			}
		};

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		resource_storage_backend.store(&resource, &buffer).unwrap();

		resource
	}
}

#[cfg(test)]
pub mod tests {
	use utils::json;

	use crate::asset::{self, asset_handler::{Asset, LoadErrors}, storage_backend::tests::TestStorageBackend};

	use super::*;

	struct TestAsset {}

	impl Asset for TestAsset {
		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn ResourceStorageBackend, _: &'a dyn asset::StorageBackend, _: ResourceId<'a>) -> Result<(), String> {
            Ok(())
		}
		fn requested_assets(&self) -> Vec<String> {
		    vec!["example".to_string()]
		}
    }

	struct TestAssetHandler {

	}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	struct TestDescription {}

	impl AssetHandler for TestAssetHandler {
	    fn can_handle(&self, id: &str) -> bool {
            id == "example"
        }

		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn ResourceStorageBackend, _: &'a dyn StorageBackend, id: ResourceId<'a>,) -> Result<Box<dyn Asset>, LoadErrors> {
			let res = if id.get_base().as_ref() == "example" {
				Ok(Box::new(TestAsset {}) as Box<dyn Asset>)
			} else {
				Err(LoadErrors::AssetCouldNotBeLoaded)
			};

			res
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
		let mut asset_manager = AssetManager::new(storage_backend);

		let _: json::Value = json::from_str(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);

		let _: json::Value = json::from_str(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}
