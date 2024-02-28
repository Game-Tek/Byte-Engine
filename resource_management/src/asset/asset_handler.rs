use crate::StorageBackend;

use super::AssetResolver;

/// An asset handler is responsible for loading assets of a certain type from a url.
pub trait AssetHandler {
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
	fn load<'a>(&'a self, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, id: &'a str, json: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>>;
}