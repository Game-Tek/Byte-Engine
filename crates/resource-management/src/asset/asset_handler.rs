use crate::{asset, r#async::BoxedFuture, resource, ProcessedAsset};

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

	fn bake<'a>(
		&'a self,
		asset_manager: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>>;
}
