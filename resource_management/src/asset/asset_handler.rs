use crate::{GenericResourceResponse, ProcessedAsset, StorageBackend};

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

	fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: ResourceId<'a>,) -> utils::SendBoxedFuture<'a, Result<Box<dyn Asset>, LoadErrors>>;

	fn produce<'a>(&'a self, id: ResourceId<'a>, description: &'a dyn crate::Description, data: Box<[u8]>) -> utils::SendSyncBoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), String>> {
		unimplemented!()
	}
}

/// This trait represents an asset, and will exist during the processing of an asset.
pub trait Asset: Send + Sync {
    fn requested_assets(&self) -> Vec<String>;
    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: ResourceId<'a>) -> utils::SendBoxedFuture<Result<(), String>>;
}
