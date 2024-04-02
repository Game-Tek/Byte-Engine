use smol::{io::AsyncReadExt, stream::StreamExt};

use crate::{asset::{read_asset_from_source, AssetResolver}, DbStorageBackend, Resource, StorageBackend, TypedResource};

use super::asset_handler::AssetHandler;

pub struct AssetManager {
	asset_handlers: Vec<Box<dyn AssetHandler>>,
	storage_backend: DbStorageBackend,
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

		AssetManager {
			asset_handlers: Vec::new(),
			storage_backend: DbStorageBackend::new(&resolve_internal_path(std::path::Path::new("assets.db"))),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	/// Load a source asset from a JSON asset description.
	pub async fn load(&self, id: &str) -> Result<(), LoadMessages> {
		let mut dir = smol::fs::read_dir(assets_path()).await.or_else(|_| Err(LoadMessages::IO))?;

		let entry = dir.find(|e| 
			e.as_ref().unwrap().file_name().to_str().unwrap().contains(id) && e.as_ref().unwrap().path().extension().unwrap() == "json"
		).await.ok_or(LoadMessages::NoAsset)?.or_else(|_| Err(LoadMessages::NoAsset))?;

		let mut asset_resolver = smol::fs::File::open(entry.path()).await.or_else(|_| Err(LoadMessages::IO))?;

		let mut json_string = String::with_capacity(1024);

		asset_resolver.read_to_string(&mut json_string).await.or_else(|_| Err(LoadMessages::IO))?;

		let json = json::parse(&json_string).or_else(|_| Err(LoadMessages::IO))?;

		let url = json["url"].as_str().ok_or(LoadMessages::NoURL)?; // Source asset url

		struct MyAssetResolver {}

		impl AssetResolver for MyAssetResolver {
			fn resolve<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Vec<u8>, String)>> + Send + 'a>> {
				Box::pin(async move {
					read_asset_from_source(url, Some(&assets_path())).await.ok()
				})
			}
		}

		let asset_resolver = MyAssetResolver {};

		let storage_backend = &self.storage_backend;

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, &asset_resolver, storage_backend, id, &json));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", url);
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		&self.storage_backend
	}
	
	pub async fn load_typed_resource<T: Resource>(&self, id: &str) -> Result<TypedResource<T>, LoadMessages> {
		let mut dir = smol::fs::read_dir(assets_path()).await.or_else(|_| Err(LoadMessages::IO))?;

		let entry = dir.find(|e| 
			e.as_ref().unwrap().file_name().to_str().unwrap().contains(id) && e.as_ref().unwrap().path().extension().unwrap() == "json"
		).await.ok_or(LoadMessages::NoAsset)?.or_else(|_| Err(LoadMessages::NoAsset))?;

		let mut asset_resolver = smol::fs::File::open(entry.path()).await.or_else(|_| Err(LoadMessages::IO))?;

		let mut json_string = String::with_capacity(1024);

		asset_resolver.read_to_string(&mut json_string).await.or_else(|_| Err(LoadMessages::IO))?;

		let json = json::parse(&json_string).or_else(|_| Err(LoadMessages::IO))?;

		let url = json["url"].as_str().ok_or(LoadMessages::NoURL)?; // Source asset url

		struct MyAssetResolver {}

		impl AssetResolver for MyAssetResolver {
			fn resolve<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Vec<u8>, String)>> + Send + 'a>> {
				Box::pin(async move {
					read_asset_from_source(url, Some(&assets_path())).await.ok()
				})
			}
		}

		let asset_resolver = MyAssetResolver {};

		let storage_backend = &self.storage_backend;

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(self, &asset_resolver, storage_backend, id, &json));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { load_result.is_ok() });

		if !asset_handler_found {
			log::warn!("No asset handler found for asset: {}", url);
			return Err(LoadMessages::NoAssetHandler);
		}

		todo!("Implement loading typed resources");

		// Ok(())
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

	use crate::{GenericResourceResponse, GenericResourceSerialization};

use super::*;

	struct TestAssetHandler {

	}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	impl AssetHandler for TestAssetHandler {
		fn load<'a>(&'a self, _: &'a AssetManager, _: &'a dyn AssetResolver, _ : &'a dyn StorageBackend, id: &'a str, _: &'a json::JsonValue) -> utils::BoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
			let res = if id == "example" {
				Ok(Some(GenericResourceSerialization::new_with_serialized("id", "TestAsset", bson::Bson::Null)))
			} else {
				Err("Failed to load".to_string())
			};

			async move { res }.boxed()
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

		let json = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Ok(()));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_handler() {
		let asset_manager = AssetManager::new();

		let json = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	#[ignore = "Need to solve DI"]
	fn test_load_no_asset_url() {
		let asset_manager = AssetManager::new();

		let json = json::parse(r#"{}"#).unwrap();

		// assert_eq!(smol::block_on(asset_manager.load("example", &json)), Err(LoadMessages::NoURL));
	}
}