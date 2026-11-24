//! Early resource-load requests for scene pipelines.

#![expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md")]

use std::sync::Arc;

pub use resource_management::resource::storage_backend::Query;

/// The `Resource` enum asks scene pipelines to start loading resources before any entity uses them.
///
/// Create it through the world scope's creation factory ([`crate::core::message_bus::MessageScope::factory`]),
/// for example `world.create(Resource::new("meshes/Box.gltf"))`. Each installed
/// scene pipeline reads the stored class of every named resource and starts the
/// matching load during the renderer's next `update`, so later entities that use
/// the resource adopt the resident copy. Meshes also load their materials,
/// textures, and material pipelines.
///
/// Install the scene pipeline before creating resources; creations without a
/// listening pipeline are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
	/// One resource by ID.
	// Shared, not owned: the bus clones this ID into every listener, and the render thread, loader tasks, and inspector
	// each keep it in long-lived maps. A numeric key cannot replace the string, since debug builds bake from it on demand.
	Id(Arc<str>),
	/// Every stored resource that matches an indexed metadata query.
	///
	/// The query runs on the pipeline's loader, so it never blocks the caller.
	/// Development builds only match resources that were already baked.
	Query(Query),
}

impl Resource {
	/// Names one resource to load.
	pub fn new(id: impl Into<Arc<str>>) -> Self {
		Self::Id(id.into())
	}

	/// Loads every resource that matches `query`.
	pub fn query(query: impl Into<Query>) -> Self {
		Self::Query(query.into())
	}
}

impl From<&str> for Resource {
	fn from(id: &str) -> Self {
		Self::Id(id.into())
	}
}

impl From<Query> for Resource {
	fn from(query: Query) -> Self {
		Self::Query(query)
	}
}
