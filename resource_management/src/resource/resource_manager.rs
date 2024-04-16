use std::ops::Deref;

use crate::{asset::asset_manager::{self, AssetManager}, DbStorageBackend, LoadResourceRequest, LoadResults, ResourceRequest, ResourceResponse, StorageBackend};
use super::resource_handler::ResourceHandler;

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
	// storage_backend: Box<dyn StorageBackend>,
	resource_handlers: Vec<Box<dyn ResourceHandler + Send>>,

	#[cfg(debug_assertions)]
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

		// let mut args = std::env::args();

		// let mut memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		ResourceManager {
			// storage_backend: Box::new(DbStorageBackend::new(std::path::Path::new("resources"))),
			resource_handlers: Vec::with_capacity(8),

			#[cfg(debug_assertions)]
			asset_manager: None,
		}
	}

	pub fn new_with_storage_backend<T: StorageBackend + 'static>(storage_backend: T) -> Self {
		ResourceManager {
			// storage_backend: Box::new(storage_backend),
			resource_handlers: Vec::with_capacity(8),

			#[cfg(debug_assertions)]
			asset_manager: None,
		}
	}

	pub fn set_asset_manager(&mut self, asset_manager: AssetManager) {
		#[cfg(debug_assertions)]
		self.asset_manager = Some(asset_manager);
	}

	pub fn add_resource_handler<T>(&mut self, resource_handler: T) where T: ResourceHandler + Send + 'static {
		self.resource_handlers.push(Box::new(resource_handler));
	}

	fn get_storage_backend(&self) -> &dyn StorageBackend {
		if let Some(asset_manager) = &self.asset_manager {
			asset_manager.get_storage_backend()
		} else {
			panic!("No asset manager set");
			// self.storage_backend.deref()
		}
	}

	/// Tries to load a resource from cache or source.\
	/// This is a more advanced version of get() as it allows to load resources that depend on other resources.\
	/// 
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	/// If the resource is in cache but it's data cannot be parsed, it will return None.
	/// Return is a tuple containing the resource description and it's associated binary data.\
	/// The requested resource will always the last one in the array. With the previous resources being the ones it depends on. This way when iterating the array forward the dependencies will be loaded first.
	pub async fn get<'s, 'a, 'b>(&'s self, id: &'a str) -> Option<ResourceResponse<'a>> where 'b: 'a {
		let load = {
			let (resource, reader) = if let Some(x) = self.get_storage_backend().read(id).await {
				x	
			} else {
				if let Some(asset_manager) = &self.asset_manager {	
					asset_manager.load(id).await.ok()?;	
					self.get_storage_backend().read(id).await?
				} else {
					return None;
				}
			};

			self.resource_handlers.iter().find(|rh| rh.get_handled_resource_classes().contains(&resource.class.as_str()))?.read(resource, Some(reader), self.get_storage_backend()).await?
		};

		Some(load)
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load()` which will load the binary data.
	pub async fn request(&self, id: &str) -> Option<ResourceRequest> {
		let (resource, _) = if let Some(x) = self.get_storage_backend().read(id).await {
			x	
		} else {
			if let Some(asset_manager) = &self.asset_manager {
				asset_manager.load(id).await.ok()?;
				self.get_storage_backend().read(id).await?
			} else {
				return None;
			}
		};

		let p = self.resource_handlers.iter().find(|rh| rh.get_handled_resource_classes().contains(&resource.class.as_str()))?.read(resource, None, self.get_storage_backend()).await?;

		Some(ResourceRequest::new(p))
	}

	/// Loads the resource binary data from cache.\
	/// If a buffer range is provided it will load the data into the buffer.\
	/// If no buffer range is provided it will return the data in a vector.
	/// 
	/// If a buffer is not provided for a resurce in the options parameters it will be either be loaded into the provided buffer or returned in a vector.
	pub async fn load<'a, 's>(&'s self, request: LoadResourceRequest<'a>) -> Option<ResourceResponse<'a>> {
		let (mut resource, reader) = self.get_storage_backend().read(request.id()).await?;

		let resource_handler = self.resource_handlers.iter().find(|rh| rh.get_handled_resource_classes().contains(&resource.class.as_str()))?;

		resource.read_target = request.streams;

		let load = resource_handler.read(resource, Some(reader), self.get_storage_backend()).await?;

		Some(load)
	}
}

#[cfg(test)]
mod tests {
	use crate::{asset::tests::TestStorageBackend, resource::resource_handler::ResourceReader, GenericResourceResponse, GenericResourceSerialization, Resource};

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
	}

	impl ResourceHandler for MyResourceHandler {
		fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
			&["MyResource"]
		}

		fn read<'s, 'a, 'b>(&'s self, r: GenericResourceResponse<'a>, _: Option<Box<dyn ResourceReader>>, _: &'b dyn StorageBackend) -> utils::BoxedFuture<'b, Option<ResourceResponse<'a>>> where 'a: 'b{
			Box::pin(async move {
				Some(ResourceResponse::new(r, ()))
			})
		}
	}

	#[test]
	fn get() {
		let storage_backend = TestStorageBackend::new();

		smol::block_on(storage_backend.store(&GenericResourceSerialization::new("test", ()), &[])).expect("Failed to store resource");

		let mut resource_manager = ResourceManager::new_with_storage_backend(storage_backend);

		resource_manager.add_resource_handler(MyResourceHandler::new());

		smol::block_on(resource_manager.get("test")).unwrap();
	}

	#[test]
	fn request() {
		let storage_backend = TestStorageBackend::new();

		smol::block_on(storage_backend.store(&GenericResourceSerialization::new("test", ()), &[])).expect("Failed to store resource");

		let mut resource_manager = ResourceManager::new_with_storage_backend(storage_backend);

		resource_manager.add_resource_handler(MyResourceHandler::new());

		let request = smol::block_on(resource_manager.request("test")).unwrap();

		let request = LoadResourceRequest::new(request);

		let load = smol::block_on(resource_manager.load(request)).expect("Failed to load resource");

		assert_eq!(load.get_buffer(), None);
	}
}