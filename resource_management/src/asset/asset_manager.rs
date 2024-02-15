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

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(url, &json));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { if let Some(Ok(_)) = load_result { true } else { false } });

		if !asset_handler_found {
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use smol::future::FutureExt;

	use super::*;

	struct TestAssetHandler {

	}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	impl AssetHandler for TestAssetHandler {
		fn load(&self, url: &str) -> utils::BoxedFuture<Option<Result<(), String>>> {
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
		let asset_manager = AssetManager::new();
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