use utils::Glob;

use crate::asset::{StorageBackend, manager::AssetManager};
use crate::parameters::{Parameter, Parameters};

/// Creates the asset manager BELD bakes with, with the bake services and asset handlers `configure` installs for the
/// application's `parameters`, and the `assets.exclude` patterns of `parameters`.
///
/// `configure` is the function the application's debug runtime also configures its manager with, so both bake the same
/// resources with the same services.
pub fn get_asset_manager<AS, RS>(
	storage_backend: AS,
	resource_storage_backend: RS,
	parameters: &[Parameter],
	configure: impl FnOnce(&mut AssetManager, &[Parameter]),
) -> AssetManager
where
	AS: StorageBackend + 'static,
	RS: crate::resource::StorageBackend + 'static,
{
	let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend);
	configure(&mut asset_manager, parameters);
	asset_manager.set_excluded_assets(globs(parameters.scope("assets"), "exclude"));
	asset_manager
}

/// Returns the glob patterns of the array parameter `name`, such as `resources.roots`.
pub fn globs(parameters: impl Parameters, name: &str) -> Vec<Glob> {
	parameters
		.get_elements(name)
		.map(|pattern| Glob::new(pattern.value()))
		.collect()
}
