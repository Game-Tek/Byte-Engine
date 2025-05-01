use crate::{asset::{asset_manager::AssetManager, ResourceId}, Reference, ReferenceModel, Resource, SerializableResource, Solver};

use super::{storage_backend::Query, StorageBackend};

/// Resource manager.
/// Handles loading assets or resources from different origins (network, local, etc.).
/// It also handles caching of resources.
///
/// Resource can be sourced from the local filesystem or from the network.
/// When in a debug build it will lazily load resources from source and cache them.
/// When in a release build it will exclusively load resources from cache.
///
/// If accessing the filesystem paths will be relative to the assets directory, and assets should omit the extension.
pub struct ResourceManager {
	// #[cfg(debug_assertions)]
	asset_manager: Option<AssetManager>,

	storage_backend: Box<dyn StorageBackend>,
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new<SB: StorageBackend>(storage_backend: SB) -> Self {
		ResourceManager {
			asset_manager: None,
			storage_backend: Box::new(storage_backend),
		}
	}

	/// Provide an asset manager to process assets on demand.
	/// This is useful for loading assets during testing/development without having to bake them.
	pub fn set_asset_manager(&mut self, asset_manager: AssetManager) {
		self.asset_manager = Some(asset_manager);
	}

	fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.as_ref()
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load()` which will load the binary data.
	pub fn request<'s, 'a, 'b, T: Resource + 'a>(&'s self, id: &'b str) -> Result<Reference<T>, &'static str> where ReferenceModel<T::Model>: Solver<'a, Reference<T>>, SerializableResource: TryInto<ReferenceModel<T::Model>> {
		let storage_backend = self.get_storage_backend();

		let reference_model: ReferenceModel<T::Model> = if let Some(result) = storage_backend.read(ResourceId::new(id)) {
			let (resource, _) = result;
			resource.into()
		} else if let Some(asset_manager) = &self.asset_manager {
			asset_manager.load(id, storage_backend).map_err(|_| "Failed to load asset")?
		} else {
            return Err("Resource does not exists and an asset manager is not available");
        };

		let reference: Reference<T> = reference_model.solve(self.get_storage_backend()).map_err(|_| "Failed to solve reference")?;

		Ok(reference)
	}

	pub fn query<'a, T: Resource + 'a>(&'a self) -> Vec<Reference<T>> where ReferenceModel<T::Model>: Solver<'a, Reference<T>>, SerializableResource: Into<ReferenceModel<T::Model>> {
		self.get_storage_backend().query(Query::new().classes(&["Material"])).unwrap().into_iter().map(|e| { let r: ReferenceModel<T::Model> = e.0.into(); let r: Reference<T> = r.solve(self.get_storage_backend()).unwrap(); r }).collect()
	}
}

#[cfg(test)]
mod tests {
	use crate::{Model, Resource};

	struct MyResourceHandler {}

	impl MyResourceHandler {
		pub fn new() -> Self {
			MyResourceHandler {}
		}
	}

	impl Resource for () {
		fn get_class(&self) -> &'static str {
			"MyResource"
		}

		type Model = ();
	}

	impl Model for () {
		fn get_class() -> &'static str {
			"MyResource"
		}
	}
}
