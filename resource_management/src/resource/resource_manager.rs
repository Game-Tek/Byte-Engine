use crate::{asset::asset_manager::AssetManager, GenericResourceResponse, LoadResults, Reference, ReferenceModel, Resource, Solver, StorageBackend};

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
}

impl From<polodb_core::Error> for LoadResults {
	fn from(error: polodb_core::Error) -> Self {
		match error {
			_ => LoadResults::LoadFailed
		}
	}
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new() -> Self {
		if let Err(error) = std::fs::create_dir_all(std::path::Path::new("resources")) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create resources directory"),
			}
		}

		ResourceManager {
			// storage_backend: Box::new(DbStorageBackend::new(std::path::Path::new("resources"))),
			asset_manager: None,
		}
	}

	pub fn new_with_storage_backend<T: StorageBackend + 'static>(storage_backend: T) -> Self {
		ResourceManager {
			// storage_backend: Box::new(storage_backend),
			asset_manager: None,
		}
	}

	pub fn set_asset_manager(&mut self, asset_manager: AssetManager) {
		{ self.asset_manager = Some(asset_manager); }
	}

	fn get_storage_backend(&self) -> &dyn StorageBackend {
		if let Some(asset_manager) = &self.asset_manager {
			asset_manager.get_storage_backend()
		} else {
			panic!("No asset manager set");
			// self.storage_backend.deref()
		}
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load()` which will load the binary data.
	pub async fn request<'s, 'a, 'b, T: Resource + 'a>(&'s self, id: &'b str) -> Option<Reference<T>> where ReferenceModel<T::Model>: Solver<'a, Reference<T>>, GenericResourceResponse: TryInto<ReferenceModel<T::Model>> {
		// Try to load the resource from cache or source.
		if let Some(asset_manager) = &self.asset_manager {
			asset_manager.bake(id).await.ok()?;
		}

		let (resource, _) = if let Some(x) = self.get_storage_backend().read(id).await {
			x	
		} else {
			return None;
		};

		let reference_model: ReferenceModel<T::Model> = resource.try_into().ok()?;
		let reference: Reference<T> = reference_model.solve(self.get_storage_backend()).await.unwrap();
		reference.into()
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

	use super::*;

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

	// #[test]
	// fn get() {
	// 	let storage_backend = TestStorageBackend::new();

	// 	smol::block_on(storage_backend.store(&GenericResourceSerialization::new("test", ()), &[])).expect("Failed to store resource");

	// 	let mut resource_manager = ResourceManager::new_with_storage_backend(storage_backend);

	// 	resource_manager.add_resource_handler(MyResourceHandler::new());

	// 	smol::block_on(resource_manager.get("test")).unwrap();
	// }

	// #[test]
	// fn request() {
	// 	let storage_backend = TestStorageBackend::new();

	// 	smol::block_on(storage_backend.store(&GenericResourceSerialization::new("test", ()), &[])).expect("Failed to store resource");

	// 	let mut resource_manager = ResourceManager::new_with_storage_backend(storage_backend);

	// 	resource_manager.add_resource_handler(MyResourceHandler::new());

	// 	let request = smol::block_on(resource_manager.request("test")).unwrap();

	// 	let request = LoadResourceRequest::new(request);

	// 	let load = smol::block_on(resource_manager.load(request)).expect("Failed to load resource");

	// 	assert_eq!(load.get_buffer(), None);
	// }
}