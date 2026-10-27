use crate::ProcessedAsset;

/// The `ResourceValidator` trait lets a project reject baked resources that break its own rules, such as a size limit.
///
/// The asset manager calls [`Self::validate`] for every resource a bake stores, including generated dependencies,
/// before the resource is published to resource storage. A rejection fails the bake of that resource and of every
/// resource that depends on it, and nothing is stored for it.
///
/// Register a validator with
/// [`AssetManager::add_resource_validator`](crate::asset::manager::AssetManager::add_resource_validator).
pub trait ResourceValidator: Send + Sync {
	/// Returns why `resource` must not be stored: what happened, the most likely cause, and the fix.
	///
	/// `payload_size` is the size of the payload in bytes, before storage compression.
	fn validate(&self, resource: &ProcessedAsset, payload_size: usize) -> Result<(), String>;
}
