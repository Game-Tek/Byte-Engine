use crate::asset::AssetResolver;

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

		struct MyAssetResolver {}

		impl AssetResolver for MyAssetResolver {
		}

		let asset_resolver = MyAssetResolver {};

		let asset_handler_loads = self.asset_handlers.iter().map(|asset_handler| asset_handler.load(&asset_resolver, url, &json));

		let load_results = futures::future::join_all(asset_handler_loads).await;

		let asset_handler_found = load_results.iter().any(|load_result| { if let Some(Ok(_)) = load_result { true } else { false } });

		if !asset_handler_found {
			return Err(LoadMessages::NoAssetHandler);
		}

		Ok(())
	}

	// Recursively loads all the resources needed to load the resource at the given url.
	// **Will** load from source and cache the resources if they are not already cached.
	// fn gather<'a>(&'a self, db: &'a polodb_core::Database, url: &'a str) -> Pin<Box<dyn std::future::Future<Output = Option<Vec<polodb_core::bson::Document>>> + 'a>> {
	// 	Box::pin(async move {
	// 		let resource_documents = if let Some(resource_document) = db.collection::<polodb_core::bson::Document>("resources").find_one(polodb_core::bson::doc!{ "url": url }).unwrap() {
	// 			let mut documents = vec![];
				
	// 			if let Some(polodb_core::bson::Bson::Array(required_resources)) = resource_document.get("required_resources") {
	// 				for required_resource in required_resources {
	// 					if let polodb_core::bson::Bson::Document(required_resource) = required_resource {
	// 						let resource_path = required_resource.get("url").unwrap().as_str().unwrap();
	// 						documents.append(&mut self.gather(db, resource_path).await?);
	// 					}

	// 					if let polodb_core::bson::Bson::String(required_resource) = required_resource {
	// 						let resource_path = required_resource.as_str();
	// 						documents.append(&mut self.gather(db, resource_path).await?);
	// 					}
	// 				}
	// 			}

	// 			documents.push(resource_document);

	// 			documents
	// 		} else {
	// 			let mut loaded_resource_documents = Vec::new();

	// 			let asset_type = self.get_url_type(url)?;

	// 			let resource_handlers = self.resource_handlers.iter().filter(|h| h.can_handle_type(&asset_type));

	// 			for resource_handler in resource_handlers {
	// 				let gg = resource_handler.process(self, url,).await.unwrap();

	// 				for g in gg {
	// 					match g {
	// 						ProcessedResources::Generated(g) => {
	// 							for e in &g.0.required_resources {
	// 								match e {
	// 									ProcessedResources::Generated(g) => {
	// 										loaded_resource_documents.push(self.write_resource_to_cache(g,).await?);
	// 									},
	// 									ProcessedResources::Reference(r) => {
	// 										loaded_resource_documents.append(&mut self.gather(db, r).await?);
	// 									}
	// 								}
	// 							}

	// 							loaded_resource_documents.push(self.write_resource_to_cache(&g,).await?);
	// 						},
	// 						ProcessedResources::Reference(r) => {
	// 							loaded_resource_documents.append(&mut self.gather(db, &r).await?);
	// 						}
	// 					}
	// 				}
	// 			}

	// 			if loaded_resource_documents.is_empty() {
	// 				log::warn!("No resource handler could handle resource: {}", url);
	// 			}

	// 			loaded_resource_documents
	// 		};


	// 		Some(resource_documents)
	// 	})
	// }

	// Tries to load a resource from cache.\
	// It also resolves all dependencies.\
	// async fn load_from_cache_or_source(&self, url: &str) -> Option<Request> {
	// 	let resource_descriptions = self.gather(&self.db, url).await.expect("Could not load resource");

	// 	for r in &resource_descriptions {
	// 		log::trace!("Loaded resource: {:#?}", r);
	// 	}

	// 	let request = Request {
	// 		resources: resource_descriptions.iter().map(|r|
	// 			ResourceRequest { 
	// 				_id: r.get_object_id("_id").unwrap(),
	// 				id: r.get_i64("id").unwrap() as u64,
	// 				url: r.get_str("url").unwrap().to_string(),
	// 				size: r.get_i64("size").unwrap() as u64,
	// 				hash: r.get_i64("hash").unwrap() as u64,
	// 				class: r.get_str("class").unwrap().to_string(),
	// 				resource: self.deserializers[r.get_str("class").unwrap()](r.get_document("resource").unwrap()), // TODO: handle errors
	// 				required_resources: if let Ok(rr) = r.get_array("required_resources") { rr.iter().map(|e| e.as_str().unwrap().to_string()).collect() } else { vec![] },
	// 			}
	// 		).collect(),
	// 	};

	// 	Some(request)
	// }
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
		fn load<'a>(&'a self, _: &'a dyn AssetResolver, url: &'a str, _: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
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