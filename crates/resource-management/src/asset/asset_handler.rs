use crate::{resource, asset, ProcessedAsset};

use super::{asset_manager::AssetManager, ResourceId};

#[derive(Debug)]
pub enum LoadErrors {
    AssetDoesNotExist,
    FailedToProcess,
    AssetCouldNotBeLoaded,
    UnsupportedType,
}

/// An asset handler is responsible for loading assets of a certain type from a url.
pub trait AssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>,) -> Result<Box<dyn Asset>, LoadErrors>;

	fn produce<'a>(&'a self, _: ResourceId<'a>, _: &'a dyn crate::Description, _: Box<[u8]>) -> Result<(ProcessedAsset, Box<[u8]>), String> {
		unimplemented!()
	}
}

/// This trait represents an asset, and will exist during the processing of an asset.
pub trait Asset: Send + Sync {
    fn requested_assets(&self) -> Vec<String>;
    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>) -> Result<(), String>;
}
