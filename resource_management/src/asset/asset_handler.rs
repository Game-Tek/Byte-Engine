use crate::{GenericResourceResponse, ProcessedAsset, StorageBackend};

use super::asset_manager::AssetManager;

/// An asset handler is responsible for loading assets of a certain type from a url.
pub trait AssetHandler: Send + Sync {
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
	fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: &'a str,) -> utils::SendBoxedFuture<'a, Result<(), String>>;

	fn produce<'a>(&'a self, id: &'a str, description: &'a dyn crate::Description, data: &'a [u8]) -> utils::SendSyncBoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), String>> {
		unimplemented!()
	}
}