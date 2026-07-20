use super::{
	storage_backend::{Query, QueryError, QueryPage},
	StorageBackend,
};
#[cfg(debug_assertions)]
use crate::asset::asset_manager::AssetManager;
use crate::{asset::ResourceId, Model, Reference, ReferenceModel, Resource, SerializableResource, Solver};

/// The `ResourceManager` struct provides typed resource loading and caching across storage backends.
///
/// Debug builds can use an asset manager to bake missing source assets on demand.
/// Release builds load only resources that already exist in the configured backend.
///
/// File-system paths are relative to the assets directory.
pub struct ResourceManager {
	#[cfg(debug_assertions)]
	asset_manager: std::sync::OnceLock<AssetManager>,

	storage_backend: Box<dyn StorageBackend>,
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new<SB: StorageBackend>(storage_backend: SB) -> Self {
		ResourceManager {
			#[cfg(debug_assertions)]
			asset_manager: std::sync::OnceLock::new(),
			storage_backend: Box::new(storage_backend),
		}
	}

	/// Installs an asset manager that can bake missing assets on demand in debug builds.
	///
	/// # Panics
	///
	/// Panics when asset management was already installed on this resource manager.
	#[cfg(debug_assertions)]
	pub fn set_asset_manager(&self, asset_manager: AssetManager) {
		assert!(
			self.try_set_asset_manager(asset_manager).is_ok(),
			"Failed to set up resource manager. The most likely cause is that asset management was installed more than once."
		);
	}

	/// Attempts to install the development asset manager without replacing an existing one.
	#[cfg(debug_assertions)]
	pub fn try_set_asset_manager(&self, asset_manager: AssetManager) -> Result<(), AssetManager> {
		self.asset_manager.set(asset_manager)
	}

	fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.as_ref()
	}

	/// Loads resource metadata and dependencies, then returns a deferred binary-data [`Reference`].
	///
	/// Use [`Reference::load`](crate::Reference::load) to load the binary data into
	/// caller-provided memory or reader-owned storage.
	pub fn request<'s, 'a, 'b, T: Resource + 'a>(&'s self, id: &'b str) -> Result<Reference<T>, &'static str>
	where
		ReferenceModel<T::Model>: Solver<'a, Reference<T>>,
		SerializableResource: TryInto<ReferenceModel<T::Model>>,
	{
		let storage_backend = self.get_storage_backend();

		let reference_model: ReferenceModel<T::Model> = if let Some(result) = storage_backend.read(ResourceId::new(id)) {
			let (resource, _) = result;
			resource.into()
		} else {
			#[cfg(debug_assertions)]
			{
				let Some(asset_manager) = self.asset_manager.get() else {
					return Err("Resource does not exist and an asset manager is not available");
				};
				let runtime = compio::runtime::Runtime::new().unwrap();

				runtime
					.block_on(asset_manager.bake_if_not_exists(id, storage_backend))
					.map_err(|_| "Failed to load asset. The asset manager could not bake the resource.")?
			}
			#[cfg(not(debug_assertions))]
			{
				return Err("Resource does not exist in the baked release resource store");
			}
		};

		let reference: Reference<T> = reference_model
			.solve(self.get_storage_backend())
			.map_err(Into::<&'static str>::into)?;

		Ok(reference)
	}

	pub fn query<'a, T: Resource + 'a>(&'a self, query: Query) -> Result<QueryPage<Reference<T>>, QueryError>
	where
		ReferenceModel<T::Model>: Solver<'a, Reference<T>>,
		SerializableResource: Into<ReferenceModel<T::Model>>,
	{
		let page = self.get_storage_backend().query(Query {
			class: T::Model::get_class().to_string(),
			..query
		})?;

		Ok(QueryPage {
			items: page
				.items
				.into_iter()
				.map(|e| {
					let r: ReferenceModel<T::Model> = e.0.into();
					let r: Reference<T> = r.solve(self.get_storage_backend()).unwrap();
					r
				})
				.collect(),
			cursor: page.cursor,
		})
	}
}

#[cfg(all(test, debug_assertions))]
mod debug_tests {
	use std::sync::Arc;

	use super::ResourceManager;
	use crate::{
		asset::{asset_manager::AssetManager, storage_backend::tests::TestStorageBackend as AssetTestStorageBackend},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
	};

	#[test]
	fn asset_management_can_be_installed_after_the_resource_manager_is_shared() {
		let resource_manager = Arc::new(ResourceManager::new(ResourceTestStorageBackend::new()));
		let renderer_reference = Arc::downgrade(&resource_manager);

		resource_manager.set_asset_manager(AssetManager::new(AssetTestStorageBackend::new()));
		assert!(renderer_reference.upgrade().is_some());
		assert!(resource_manager
			.try_set_asset_manager(AssetManager::new(AssetTestStorageBackend::new()))
			.is_err());
	}
}

#[cfg(all(test, not(debug_assertions)))]
mod release_tests {
	use super::ResourceManager;
	use crate::{resource::storage_backend::tests::TestStorageBackend, resources::material::Shader};

	#[test]
	fn missing_release_resource_fails_without_running_asset_processors() {
		let resource_manager = ResourceManager::new(TestStorageBackend::new());
		let result = resource_manager.request::<Shader>("missing/render-pass.besl");

		assert!(matches!(
			result,
			Err("Resource does not exist in the baked release resource store")
		));
	}
}
