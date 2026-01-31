use crate::{ProcessedAsset, asset, r#async::BoxedFuture, resource};

use super::{asset_manager::AssetManager, ResourceId};

#[derive(Debug)]
pub enum LoadErrors {
	AssetDoesNotExist,
	FailedToProcess,
	AssetCouldNotBeLoaded,
	UnsupportedType,
}

/// The `AssetHandler` trait defines how to load assets of a given type.
pub trait AssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn load<'a>(
		&'a self,
		asset_manager: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<Box<dyn Asset>, LoadErrors>>;

	fn produce<'a>(
		&'a self,
		_: ResourceId<'a>,
		_: &'a dyn crate::Description,
		_: Box<[u8]>
	) -> Result<(ProcessedAsset, Box<[u8]>), String> {
		unimplemented!()
	}
}

/// The `Asset` trait defines how to process a loaded asset into resources.
pub trait Asset: Send + Sync {
	fn requested_assets(&self) -> Vec<String>;
	fn load<'a>(
		&'a self,
		asset_manager: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>
	) -> BoxedFuture<'a, Result<(), String>>;
}
