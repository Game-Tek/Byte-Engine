use crate::{asset::{AssetResolver, StorageBackend}, GenericResourceSerialization};

use super::asset_handler::AssetHandler;

struct AssetManager {
	asset_handlers: Vec<Box<dyn AssetHandler>>,
}

/// Enumeration of the possible messages that can be returned when loading an asset.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadMessages {
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

		let mut args = std::env::args();

		let mut memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		if cfg!(test) { // If we are running tests we want to use memory database. This way we can run tests in parallel.
			memory_only = true;
		}

		let db_res = if !memory_only {
			polodb_core::Database::open_file(resolve_internal_path(std::path::Path::new("assets.db")))
		} else {
			log::info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file(resolve_internal_path(std::path::Path::new("assets.db"))).unwrap();

				log::warn!("Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file(resolve_internal_path(std::path::Path::new("assets.db")));

				match db_res {
					Ok(db) => db,
					Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
						Ok(db) => {
							log::error!("Could not create database file, using memory database instead.");
							db
						},
						Err(_) => panic!("Could not create database"),
					}
				}
			}
		};

		AssetManager {
			asset_handlers: Vec::new(),
		}
	}

	pub fn add_asset_handler<T: AssetHandler + 'static>(&mut self, asset_handler: T) {
		self.asset_handlers.push(Box::new(asset_handler));
	}

	/// Load a source asset from a JSON asset description.
	pub async fn load(&self, json: &json::JsonValue) -> Result<(), LoadMessages> {
		let url = json["url"].as_str().ok_or(LoadMessages::NoURL)?; // Source asset url

		struct MyAssetResolver {}

		impl AssetResolver for MyAssetResolver {
		}

		let asset_resolver = MyAssetResolver {};

		struct MyStorageBackend {}

		impl StorageBackend for MyStorageBackend {
			fn store(&self, _: GenericResourceSerialization, _: &[u8]) -> Result<(), ()> {
				Ok(())
			}

			fn read(&self, _: &str) -> Result<(GenericResourceSerialization, Box<[u8]>), ()> {
				todo!()
			}
		}

		let storage_backend = MyStorageBackend {};

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(&asset_resolver, &storage_backend, url, &json));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { if let Some(Ok(_)) = load_result { true } else { false } });

		if !asset_handler_found {
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
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
	if cfg!(test) {
		std::path::PathBuf::from("../assets/").join(path)
	} else {
		std::path::PathBuf::from("assets/").join(path)
	}
}

#[cfg(test)]
mod tests {
	use smol::future::FutureExt;

	use crate::asset::StorageBackend;

use super::*;

	struct TestAssetHandler {

	}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	impl AssetHandler for TestAssetHandler {
		fn load<'a>(&'a self, _: &'a dyn AssetResolver, _ : &'a dyn StorageBackend, url: &'a str, _: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
			let res = if url == "http://example.com" {
				Some(Ok(()))
			} else {
				None
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
	fn test_load_with_asset_manager() {
		let mut asset_manager = AssetManager::new();

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);

		let json = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		assert_eq!(smol::block_on(asset_manager.load(&json)), Ok(()));
	}

	#[test]
	fn test_load_no_asset_handler() {
		let asset_manager = AssetManager::new();

		let json = json::parse(r#"{"url": "http://example.com"}"#).unwrap();

		assert_eq!(smol::block_on(asset_manager.load(&json)), Err(LoadMessages::NoAssetHandler));
	}

	#[test]
	fn test_load_no_asset_url() {
		let asset_manager = AssetManager::new();

		let json = json::parse(r#"{}"#).unwrap();

		assert_eq!(smol::block_on(asset_manager.load(&json)), Err(LoadMessages::NoURL));
	}
}