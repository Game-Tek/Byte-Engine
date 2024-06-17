use std::ops::Deref;

use crate::{asset::read_asset_from_source, DbStorageBackend, Description, ProcessedAsset, Model, Resource, Solver, StorageBackend, Reference, ReferenceModel};

use super::{asset_handler::AssetHandler, BEADType};

pub struct AssetManager {
	read_base_path: std::path::PathBuf,
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
}

impl AssetManager {
	pub fn new(read_base_path: std::path::PathBuf) -> AssetManager {
		if let Err(error) = std::fs::create_dir_all(assets_path()) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create assets directory"),
			}
		}

		Self::new_with_path_and_storage_backend("assets".into(), DbStorageBackend::new(&read_base_path))
	}

	pub fn new_with_path_and_storage_backend<SB: StorageBackend>(read_base_path: std::path::PathBuf, storage_backend: SB) -> AssetManager {
		AssetManager {
			read_base_path,
			asset_handlers: Vec::new(),
			storage_backend: Box::new(storage_backend),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.deref()
	}

	#[cfg(test)]
	pub fn get_test_storage_backend(&self) -> &DbStorageBackend {
		self.storage_backend.deref().downcast_ref::<DbStorageBackend>().expect("Storage backend is not a TestStorageBackend")
	}

	/// Load a source asset from a JSON asset description.
	pub async fn bake<'a>(&self, id: &str) -> Result<(), LoadMessages> {
		let storage_backend = self.get_storage_backend();

		let start_time = std::time::Instant::now();

		// TODO: check hash
		if let Some(_) = self.storage_backend.read(id).await {
			return Ok(());
		}

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, storage_backend.deref(), id,));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);

			for load_result in load_results {
				if let Err(error) = load_result {
					log::error!("Failed to load asset: {}", error);
				}
			}

			return Err(LoadMessages::NoAssetHandler);
		}

		log::trace!("Baked '{}' resource in {:#?}", id, start_time.elapsed());

		Ok(())
	}
	
	/// Generates a resource from a loaded asset.
	/// Does nothing if the resource already exists (with a matching hash).
	pub async fn load<'a, M: Model + for <'de> serde::Deserialize<'de>>(&self, id: &str) -> Result<ReferenceModel<M>, LoadMessages> {
		let storage_backend = &self.storage_backend;

		let start_time = std::time::Instant::now();

		// Try to load the resource from the storage backend.
		if let Some((r, _)) = self.storage_backend.read(id).await { // TODO: check hash
			let r: ReferenceModel<M> = r.into();
			return Ok(r)
		}

		// The resource was not found in the storage backend, so we need to load it from the source.

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, storage_backend.deref(), id,));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);
			return Err(LoadMessages::NoAssetHandler);
		}

		// We tried resolving the asset. Now try to load it from the storage backend.
		if let Some((r, _)) = self.storage_backend.read(id).await { // TODO: check hash
			log::trace!("Baked '{}' resource in {:#?}", id, start_time.elapsed());
			let r: ReferenceModel<M> = r.into();
			return Ok(r)
		}

		Err(LoadMessages::NoAsset)
	}

	/// Generates a resource from a description and data.
	/// Does nothing if the resource already exists (with a matching hash).
	pub async fn produce<'a, D: Description>(&self, id: &str, resource_type: &str, description: &D, data: &[u8]) -> ProcessedAsset {
		let asset_handler = self.asset_handlers.iter().find(|asset_handler| asset_handler.can_handle(resource_type)).expect("No asset handler found for class");

		// TODO: check hash
		if let Some((r, _)) = self.storage_backend.read(id).await {
			return r.into();
		}

		let (resource, buffer) = match asset_handler.produce(id, description, data).await {
			Ok(x) => x,
			Err(error) => {
				log::error!("Failed to produce resource: {}", error);
				panic!("Failed to produce resource");
			}
		};
		
		log::trace!("Baked '{}' resource", id);

		self.storage_backend.store(&resource, &buffer).await.unwrap();

		resource
	}
}

fn assets_path() -> std::path::PathBuf {
	if cfg!(test) {
		std::path::PathBuf::from("../assets/")
	} else {
		std::path::PathBuf::from("assets/")
	}
}

#[cfg(test)]
pub mod tests {
	use super::*;

	struct TestAssetHandler {

	}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	struct TestDescription {}

	impl AssetHandler for TestAssetHandler {
		fn load<'a>(&'a self, _: &'a AssetManager, _ : &'a dyn StorageBackend, id: &'a str,) -> utils::SendSyncBoxedFuture<'a, Result<(), String>> {
			let res = if id == "example" {
				Ok(())
			} else {
				Err("Failed to load".to_string())
			};

			Box::pin(async move { res })
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		AssetManager::new(std::path::PathBuf::from("../assets"),)
	}
	
	#[test]
	fn test_new() {
		let _ = new_testing_asset_manager();
	}

	#[test]
	fn test_add_asset_manager() {
		let mut asset_manager = AssetManager::new(std::path::PathBuf::from("../assets"),);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_with_asset_manager() {
		let mut asset_manager = AssetManager::new(std::path::PathBuf::from("../assets"),);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let asset_manager = AssetManager::new(std::path::PathBuf::from("../assets"),);

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let asset_manager = AssetManager::new(std::path::PathBuf::from("../assets"),);

		let _ = json::parse(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}