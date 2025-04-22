use std::{collections::{hash_map::Entry, HashMap}, ops::Deref};
use utils::sync::{Mutex , OnceCell, RwLock, Arc};

use crate::{asset::ResourceId, resource, Description, Model, ProcessedAsset, ReferenceModel};

use super::{asset_handler::{Asset, AssetHandler}, FileStorageBackend, StorageBackend};

pub struct AssetManager {
	asset_handlers: Vec<Box<dyn AssetHandler>>,
	asset_storage_backend: Box<dyn StorageBackend>,
	resource_storage_backend: Box<dyn resource::StorageBackend>,
	asset_loaders: Mutex<HashMap<String, Arc<OnceCell<Box<dyn Asset>>>>>,
	loading_assets: RwLock<HashMap<String, Arc<OnceCell<()>>>>,
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
	pub fn new(source_path: std::path::PathBuf, destination_path: std::path::PathBuf) -> AssetManager {
		if let Err(error) = std::fs::create_dir_all(&source_path) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create assets directory"),
			}
		}

		Self::new_with_storage_backends(FileStorageBackend::new(source_path), resource::DbStorageBackend::new(destination_path))
	}

	pub fn new_with_storage_backends<ASB: StorageBackend + 'static, RSB: resource::StorageBackend>(asset_storage_backend: ASB, resource_storage_backend: RSB) -> AssetManager {
		AssetManager {
			asset_handlers: Vec::new(),
			asset_storage_backend: Box::new(asset_storage_backend),
			resource_storage_backend: Box::new(resource_storage_backend),
			asset_loaders: Mutex::new(HashMap::new()),
			loading_assets: RwLock::new(HashMap::new()),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	pub fn get_asset_storage_backend(&self) -> &dyn StorageBackend {
		self.asset_storage_backend.deref()
	}

	pub fn get_resource_storage_backend(&self) -> &dyn resource::StorageBackend {
		self.resource_storage_backend.deref()
	}

	/// Load a source asset from a JSON asset description.
	pub fn bake<'a>(&self, id: &str) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		// Try to load the resource from the storage backend.
		if let Some(_) = self.resource_storage_backend.read(id) { // TODO: check hash
			log::debug!("Cache hit for '{:#?}'", id);
			return Ok(());
		}

		let asset_handler = match self.asset_handlers.iter().find(|handler| handler.can_handle(id.get_extension())) {
            Some(handler) => handler,
            None => {
                log::warn!("No asset handler found for asset: {:#?}", id);
                return Err(LoadMessages::NoAssetHandler);
            }
        };

		let start_time = std::time::Instant::now();

		let asset = match asset_handler.load(self, self.resource_storage_backend.as_ref(), self.asset_storage_backend.as_ref(), id) {
            Ok(asset) => asset,
            Err(error) => {
                log::error!("Failed to load asset: {:#?}", error);
                return Err(LoadMessages::NoAsset);
            }
        };

		let dependencies = asset.requested_assets();

		log::trace!("Baking '{:#?}' asset{}{}{}", id, if dependencies.is_empty() { "" } else { " => [" }, dependencies.join(", "), if dependencies.is_empty() { "" } else { "]" });

		// for dependency in dependencies {
        //     self.bake(&dependency).await?;
        // }

		let bake_result = asset.load(self, self.resource_storage_backend.as_ref(), self.asset_storage_backend.as_ref(), id);

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
	pub fn load<'a, M: Model + for <'de> serde::Deserialize<'de>>(&self, id: &str) -> Result<ReferenceModel<M>, LoadMessages> {
		let id = ResourceId::new(id);

		// Try to load the resource from the storage backend.
		if let Some((r, _)) = self.resource_storage_backend.read(id) { // TODO: check hash
			log::debug!("Cache hit for '{:#?}'", id);
			let r: ReferenceModel<M> = r.into();
			return Ok(r)
		}

		// Look for asset loader in map. If it is there, wait for it to finish loading. If it is not there, load it.
		let asset_loader = {
    		let mut assets_loaders = self.asset_loaders.lock();

            match assets_loaders.entry(id.get_base().to_string()) {
                Entry::Occupied(entry) => {
                    entry.get().clone()
                }
                Entry::Vacant(entry) => { // Asset loader was not found, flag as requested
                    let asset_loader = Arc::new(OnceCell::new());
                    entry.insert(asset_loader.clone());
                    asset_loader
                }
            }
		};

		let asset_loader = asset_loader.get_or_try_init(|| {
		    let asset_handler = match self.asset_handlers.iter().find(|handler| handler.can_handle(id.get_extension())) {
                Some(handler) => handler,
                None => {
                    log::warn!("No asset handler found for asset: {:#?}", id);
                    return Err(LoadMessages::NoAssetHandler);
                }
            };

            let asset_loader = match asset_handler.load(self, self.resource_storage_backend.as_ref(), self.asset_storage_backend.as_ref(), id) {
                Ok(asset) => asset,
                Err(error) => {
                    log::error!("Failed to load asset: {:#?}", error);
                    return Err(LoadMessages::NoAsset);
                }
            };

            Ok(asset_loader)
		})?;

        // If asset is already baked, return it.
        if let Some((r, _)) = self.resource_storage_backend.read(id) { // TODO: check hash
 			log::debug!("Cache hit for {:#?}", id);
 			let r: ReferenceModel<M> = r.into();
 			return Ok(r)
  		}

        let mut lock = self.loading_assets.write();

		let lock = match lock.entry(id.to_string()) {
			Entry::Occupied(entry) => entry.get().clone(),
			Entry::Vacant(entry) => {
				let asset_mutex = entry.insert(Arc::new(OnceCell::new())).clone();
				asset_mutex
			}
		};

        let start_time = std::time::Instant::now();

        // Asset is not baked, load/bake it.
        let _ = lock.get_or_try_init(|| {
            let _ = asset_loader.requested_assets();
            let r = asset_loader.load(self, self.resource_storage_backend.as_ref(), self.asset_storage_backend.as_ref(), id).map_err(|r| LoadMessages::FailedToBake { asset: id.to_string(), error: r });
            log::trace!("Baked '{:#?}' asset in {:#?}", id, start_time.elapsed());
            r
        })?;

        // Asset is now baked, return it.
        if let Some((r, _)) = self.resource_storage_backend.read(id) { // TODO: check hash
 			let r: ReferenceModel<M> = r.into();
 			return Ok(r)
  		}

		Err(LoadMessages::FailedToBake { asset: id.to_string(), error: format!("{:#?}", id) })
	}

	/// Generates a resource from a description and data.
	/// Does nothing if the resource already exists (with a matching hash).
	pub fn produce<'a, D: Description>(&self, id: ResourceId<'_>, resource_type: &str, description: &D, data: Box<[u8]>) -> ProcessedAsset {
		let asset_handler = self.asset_handlers.iter().find(|asset_handler| asset_handler.can_handle(resource_type)).expect("No asset handler found for class");

		// TODO: check hash
		if let Some((r, _)) = self.resource_storage_backend.read(id) {
			log::debug!("Cache hit for '{:#?}'", id);
			return r.into();
		}

		let start_time = std::time::Instant::now();

		let (resource, buffer) = match asset_handler.produce(id, description, data) {
			Ok(x) => x,
			Err(error) => {
				log::error!("Failed to produce resource: {}", error);
				panic!("Failed to produce resource");
			}
		};

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		self.resource_storage_backend.store(&resource, &buffer).unwrap();

		resource
	}
}

#[cfg(test)]
pub mod tests {
	use utils::json;

	use crate::asset::{self, asset_handler::{Asset, LoadErrors}};

	use super::*;

	struct TestAsset {}

	impl Asset for TestAsset {
		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn resource::StorageBackend, _: &'a dyn asset::StorageBackend, _: ResourceId<'a>) -> Result<(), String> {
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

		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn resource::StorageBackend, _: &'a dyn StorageBackend, id: ResourceId<'a>,) -> Result<Box<dyn Asset>, LoadErrors> {
			let res = if id.get_base().as_ref() == "example" {
				Ok(Box::new(TestAsset {}) as Box<dyn Asset>)
			} else {
				Err(LoadErrors::AssetCouldNotBeLoaded)
			};

			res
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		AssetManager::new("../assets".into(), "../assets".into())
	}

	#[test]
	fn test_new() {
		let _ = new_testing_asset_manager();
	}

	#[test]
	fn test_add_asset_manager() {
		let mut asset_manager = AssetManager::new("../assets".into(), "../assets".into());

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_with_asset_manager() {
		let mut asset_manager = AssetManager::new("../assets".into(), "../assets".into());

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let _: json::Value = json::from_str(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let _ = AssetManager::new("../assets".into(), "../assets".into());

		let _: json::Value = json::from_str(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let _ = AssetManager::new("../assets".into(), "../assets".into());

		let _: json::Value = json::from_str(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}
