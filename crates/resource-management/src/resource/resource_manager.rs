use crate::{asset::asset_manager::AssetManager, Reference, ReferenceModel, Resource, SerializableResource, Solver};

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

	storage_backend: Option<Box<dyn StorageBackend>>,
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new(resources_path: std::path::PathBuf) -> Self {
		if let Err(error) = std::fs::create_dir_all(&resources_path) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create resources directory"),
			}
		}

		ResourceManager {
			asset_manager: None,
			storage_backend: None,
		}
	}

	pub fn new_with_storage_backend<SB: StorageBackend + 'static>(storage_backend: SB) -> Self {
		ResourceManager {
			storage_backend: Some(Box::new(storage_backend)),
			asset_manager: None,
		}
	}

	pub fn set_asset_manager(&mut self, asset_manager: AssetManager) {
		self.asset_manager = Some(asset_manager);
	}

	fn get_storage_backend(&self) -> Option<&dyn StorageBackend> {
		self.storage_backend.as_ref().map(|s| s.as_ref())
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load()` which will load the binary data.
	pub fn request<'s, 'a, 'b, T: Resource + 'a>(&'s self, id: &'b str) -> Option<Reference<T>> where ReferenceModel<T::Model>: Solver<'a, Reference<T>>, SerializableResource: TryInto<ReferenceModel<T::Model>> {
		// Try to load the resource from cache or source.
		let reference_model: ReferenceModel<T::Model> = if let Some(asset_manager) = &self.asset_manager {
			asset_manager.load(id).ok()?
		} else {
            return None;
        };

		let reference: Reference<T> = reference_model.solve(self.get_storage_backend()?).ok()?;

		reference.into()
	}

	pub fn query<'a, T: Resource + 'a>(&'a self) -> Vec<Reference<T>> where ReferenceModel<T::Model>: Solver<'a, Reference<T>>, SerializableResource: Into<ReferenceModel<T::Model>> {
		self.get_storage_backend().expect("Need storage backend").query(Query::new().classes(&["Material"])).unwrap().into_iter().map(|e| { let r: ReferenceModel<T::Model> = e.0.into(); let r: Reference<T> = r.solve(self.get_storage_backend().expect("Need storage backend")).unwrap(); r }).collect()
	}

	// /// Tries to load a resource from cache or source.\
	// /// This is a more advanced version of get() as it allows to load resources that depend on other resources.\
	// ///
	// /// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	// /// If the resource is in cache but it's data cannot be parsed, it will return None.
	// /// Return is a tuple containing the resource description and it's associated binary data.\
	// /// The requested resource will always the last one in the array. With the previous resources being the ones it depends on. This way when iterating the array forward the dependencies will be loaded first.
	// pub async fn get<'s, 'a, T: Resource + 'a>(&'s self, mut resource: Reference<'a, T>) -> Option<Reference<'a, T>> where ReferenceModel<T::Model>: Solver<'a, Reference<'a, T>>, Reference<'a, T>: Loader<'b>, 'a: 'b {
	// 	// let reference: Reference<T> = resource.load().await.ok()?;
	// 	resource.load().await.ok()?;
	// 	resource.into()
	// }
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
