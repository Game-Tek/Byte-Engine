use crate::{Description, GenericResourceSerialization, Resource, StorageBackend};

use super::{asset_manager::AssetManager, AssetResolver};

/// An asset handler is responsible for loading assets of a certain type from a url.
pub trait AssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		false
	}

	/// Load an asset from a url.
	/// # Arguments
	/// * `id` - The id of the asset.
	/// * `json` - The JSON asset description.
	/// 	## Example
	/// 	```json
	/// 	{
	/// 		"url": "/path/to/asset",
	/// 	}
	/// 	```
	/// # Returns
	/// Returns Some(...) if the asset was managed by this handler, None otherwise.
	/// Returns Some(Ok(...)) if the asset was loaded successfully, Some(Err(...)) otherwise.
	fn load<'a>(&'a self, asset_manager: &'a AssetManager, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, url: &'a str, json: Option<&'a json::JsonValue>) -> utils::BoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>>;

	fn produce<'a>(&'a self, description: &'a dyn crate::Description, data: &'a [u8]) -> utils::BoxedFuture<'a, Result<(Box<dyn Resource>, Box<[u8]>), String>>;
}