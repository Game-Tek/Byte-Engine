use std::ops::Deref;

use crate::{asset::read_asset_from_source, DbStorageBackend, Description, GenericResourceSerialization, Model, Resource, Solver, StorageBackend, Reference, ReferenceModel};

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

	/// Load a source asset from a JSON asset description.
	pub async fn load<'a>(&self, id: &str) -> Result<(), LoadMessages> {
		let storage_backend = self.get_storage_backend();

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, storage_backend.deref(), id, None));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
	}
	
	pub async fn load_typed_resource<'a, R: Resource + Clone, M: Model + Clone + for <'de> serde::Deserialize<'de>>(&self, id: &str) -> Result<Reference<R>, LoadMessages> where ReferenceModel<M>: Solver<'a, Reference<R>> {
		let storage_backend = &self.storage_backend;

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, storage_backend.deref(), id, None));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);
			return Err(LoadMessages::NoAssetHandler);
		}

		let meta_resource = load_results.iter().find(|load_result| { load_result.is_ok() }).ok_or(LoadMessages::NoAsset)?.clone().unwrap().unwrap();

		let resource: ReferenceModel<M> = meta_resource.try_into().or(Err(LoadMessages::IO))?;
		let resource = resource.solve(storage_backend.deref()).or_else(|_| {
			log::error!("Failed to solve resource {}", id);
			Err(LoadMessages::IO)
		})?;

		Ok(resource.into())
	}

	pub async fn produce<'a, D: Description>(&self, id: &str, resource_type: &str, description: &D, data: &[u8]) -> GenericResourceSerialization {
		let asset_handler = self.asset_handlers.iter().find(|asset_handler| asset_handler.can_handle(resource_type)).expect("No asset handler found for class");

		let (resource, buffer) = match asset_handler.produce(id, description, data).await {
			Ok(x) => x,
			Err(error) => {
				log::error!("Failed to produce resource: {}", error);
				panic!("Failed to produce resource");
			}
		};
		
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
	use polodb_core::bson;

	use crate::{asset::tests::TestStorageBackend, GenericResourceSerialization};

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
		fn load<'a>(&'a self, _: &'a AssetManager, _ : &'a dyn StorageBackend, id: &'a str, _: Option<&'a json::JsonValue>) -> utils::SendSyncBoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
			let res = if id == "example" {
				Ok(Some(GenericResourceSerialization::new_with_serialized("id", "TestAsset", bson::Bson::Null)))
			} else {
				Err("Failed to load".to_string())
			};

			Box::pin(async move { res })
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		AssetManager::new_with_path_and_storage_backend(std::path::PathBuf::from("../assets"), TestStorageBackend::new(),)
	}
	
	#[test]
	fn test_new() {
		let _ = new_testing_asset_manager();
	}

	#[test]
	fn test_add_asset_manager() {
		let mut asset_manager = AssetManager::new_with_path_and_storage_backend(std::path::PathBuf::from("../assets"), TestStorageBackend::new(),);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_with_asset_manager() {
		let mut asset_manager = AssetManager::new_with_path_and_storage_backend(std::path::PathBuf::from("../assets"), TestStorageBackend::new(),);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let asset_manager = AssetManager::new_with_path_and_storage_backend(std::path::PathBuf::from("../assets"), TestStorageBackend::new(),);

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let asset_manager = AssetManager::new_with_path_and_storage_backend(std::path::PathBuf::from("../assets"), TestStorageBackend::new(),);

		let _ = json::parse(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}