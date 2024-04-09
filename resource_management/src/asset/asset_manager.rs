use std::ops::Deref;

use crate::{asset::{read_asset_from_source, AssetResolver}, DbStorageBackend, Description, Model, Resource, Solver, StorageBackend, TypedResource, TypedResourceModel};

use super::asset_handler::AssetHandler;

struct MyAssetResolver {}

impl AssetResolver for MyAssetResolver {
	fn resolve<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Vec<u8>, String)>> + Send + 'a>> {
		Box::pin(async move {
			read_asset_from_source(url, Some(&assets_path())).await.ok()
		})
	}
}

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
}

impl AssetManager {
	pub fn new() -> AssetManager {
		if let Err(error) = std::fs::create_dir_all(resolve_asset_path(std::path::Path::new(""))) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create assets directory"),
			}
		}

		// let mut args = std::env::args();

		// let mut memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		Self::new_with_storage_backend(DbStorageBackend::new(&resolve_internal_path(std::path::Path::new("assets.db"))))
	}

	pub fn new_with_storage_backend<SB: StorageBackend>(storage_backend: SB) -> AssetManager {
		AssetManager {
			asset_handlers: Vec::new(),
			storage_backend: Box::new(storage_backend),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	/// Load a source asset from a JSON asset description.
	pub async fn load(&self, id: &str) -> Result<(), LoadMessages> {
		let asset_resolver = MyAssetResolver {};

		let storage_backend = &self.storage_backend;

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, &asset_resolver, storage_backend.deref(), id, None));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.deref()
	}
	
	pub async fn load_typed_resource<'a, T: Resource + Model + Clone + for <'de> serde::Deserialize<'de>>(&self, id: &str) -> Result<TypedResource<T>, LoadMessages> where TypedResourceModel<T>: Solver<'a, TypedResource<T>> {
		let asset_resolver = MyAssetResolver {};

		let storage_backend = &self.storage_backend;

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, &asset_resolver, storage_backend.deref(), id, None));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", id);
			return Err(LoadMessages::NoAssetHandler);
		}

		let meta_resource = load_results.iter().find(|load_result| { load_result.is_ok() }).ok_or(LoadMessages::NoAsset)?.clone().unwrap().unwrap();

		let resource: TypedResourceModel<T> = meta_resource.try_into().or(Err(LoadMessages::IO))?;
		let resource = resource.solve(storage_backend.deref()).or_else(|_| {
			log::error!("Failed to solve resource {}", id);
			Err(LoadMessages::IO)
		})?;

		Ok(resource.into())
	}

	pub async fn produce<'a, D: Description, R: Resource + Clone + serde::Serialize>(&self, id: &str, resource_type: &str, description: &D, data: &[u8]) -> TypedResource<R> {
		let asset_handler = self.asset_handlers.iter().find(|asset_handler| asset_handler.can_handle(resource_type)).expect("No asset handler found for class");

		let (resource, buffer) = match asset_handler.produce(description, data).await {
			Ok(x) => x,
			Err(error) => {
				log::error!("Failed to produce resource: {}", error);
				panic!("Failed to produce resource");
			}
		};
		
		let resource = resource.into_any();
		
		if let Ok(resource) = resource.downcast::<R>() {
			let resource = TypedResource::new(id, 0, *resource);

			self.storage_backend.store(&resource.clone().into(), &buffer).await.unwrap();

			resource
		} else {
			panic!("Failed to downcast resource");
		}
	}
}

fn resolve_internal_path(path: &std::path::Path) -> std::path::PathBuf {
	if cfg!(test) {
		std::path::PathBuf::from("../.byte-editor/").join(path)
	} else {
		std::path::PathBuf::from(".byte-editor/").join(path)
	}
}

fn resolve_asset_path(path: &std::path::Path) -> std::path::PathBuf {
	assets_path().join(path)
}

fn assets_path() -> std::path::PathBuf {
	if cfg!(test) {
		std::path::PathBuf::from("../assets/")
	} else {
		std::path::PathBuf::from("assets/")
	}
}

#[cfg(test)]
mod tests {
	use polodb_core::bson;
use smol::future::FutureExt;

	use crate::GenericResourceSerialization;

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
		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn AssetResolver, _ : &'a dyn StorageBackend, id: &'a str, _: Option<&'a json::JsonValue>) -> utils::BoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
			let res = if id == "example" {
				Ok(Some(GenericResourceSerialization::new_with_serialized("id", "TestAsset", bson::Bson::Null)))
			} else {
				Err("Failed to load".to_string())
			};

			async move { res }.boxed()
		}

		fn produce<'a>(&'a self, _: &'a dyn crate::Description, _: &'a [u8]) -> utils::BoxedFuture<'a, Result<(Box<dyn Resource>, Box<[u8]>), String>> {
			unimplemented!()
		}
	}
	
	#[test]
	fn test_new() {
		let _ = AssetManager::new();
	}

	#[test]
	fn test_add_asset_manager() {
		let mut asset_manager = AssetManager::new();

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_with_asset_manager() {
		let mut asset_manager = AssetManager::new();

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let asset_manager = AssetManager::new();

		let _ = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let asset_manager = AssetManager::new();

		let _ = json::parse(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}