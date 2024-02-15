/// An asset handler is responsible for loading assets of a certain type from a url.
pub trait AssetHandler {
	/// Load an asset from a url.
	/// # Arguments
	/// * `url` - The absolute url of the source asset.
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
	fn load(&self, url: &str, json: &json::JsonValue) -> utils::BoxedFuture<Option<Result<(), String>>>;
}