use std::{alloc::Allocator, future::Future};

use super::{asset_manager::AssetManager, ResourceId};
use crate::{asset, resource, ProcessedAsset};

#[derive(Debug, PartialEq, Eq)]
pub enum LoadErrors {
	AssetDoesNotExist,
	FailedToProcess,
	AssetCouldNotBeLoaded,
	UnsupportedType,
}

/// The `AssetHandler` trait defines how to load assets of a given type.
pub trait AssetHandler {
	fn can_handle(&self, r#type: &str) -> bool;

	fn bake<'a>(
		&'a self,
		asset_manager: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> impl Future<Output = Result<(ProcessedAsset, Box<[u8]>), LoadErrors>>;
}
