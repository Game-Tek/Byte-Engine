//! Intercepts the intermediate products a bake builds on its way to a resource, such as the BESL source of a shader.
//!
//! A bake normally keeps its intermediate products to itself: a material's program becomes a compiled shader without
//! its BESL ever leaving memory. Install a [`BakeInterceptor`] with
//! [`AssetManager::set_interceptor`](crate::asset::manager::AssetManager::set_interceptor) to receive them, for example
//! to see exactly what a renderer's program generator built. `beld bake --dry-run --intercept` is built on it.
//!
//! # Artifact names
//!
//! Every product is an artifact: bytes with a name. The name is the ID of the resource the product belongs to, then a
//! `.` and the usual file extension of the product's format. The BESL a material's shader compiles from is
//! `wood.bema#shader.besl`, and a glTF container's generated shaders are `mesh.gltf#shaders/<key>.besl`. The set of
//! names is open: a handler emits a new kind of product by naming it with
//! [`BakeContext::intercept`](crate::asset::handler::BakeContext::intercept), with no list to extend. A caller selects
//! products with a pattern over their names, such as `*.besl` for one format or `mesh.gltf#*` for one asset.
//!
//! Products come from the bake that builds the resource, not from a separate path, so they are exactly what the bake
//! used. Products of a resource the bake reuses from the store are not built again, so a caller that needs every one
//! bakes into an empty store, as a dry run does.

/// The `BakeInterceptor` trait lets a caller receive the intermediate products of every bake an asset manager runs.
///
/// Install one with [`AssetManager::set_interceptor`](crate::asset::manager::AssetManager::set_interceptor) before
/// baking. Bakes run concurrently, so [`Self::receive`] can be called from several workers at once and in any order;
/// sort what it collects by name when the order matters. See the [module documentation](self) for how products are
/// named.
pub trait BakeInterceptor: Send + Sync {
	/// Returns whether the caller wants the product named `name`. The bake builds a product only when this returns
	/// `true`, so a product nobody asks for costs nothing.
	fn wants(&self, name: &str) -> bool;

	/// Receives the product named `name`, after [`Self::wants`] accepted it. Both are handed over, so keeping them
	/// costs no copy.
	fn receive(&self, name: String, content: Vec<u8>);
}
