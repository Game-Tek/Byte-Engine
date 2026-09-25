//! Early resource-load requests for scene pipelines.

pub use resource_management::resource::storage_backend::Query;

/// The `Resource` enum asks scene pipelines to start loading resources before any entity uses them.
///
/// Create it through [`crate::gameplay::world::DefaultWorld`]'s creation factory,
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
	Id(&'static str),
	/// Every stored resource that matches an indexed metadata query.
	///
	/// The query runs on the pipeline's loader, so it never blocks the caller.
	/// Development builds only match resources that were already baked.
	Query(Query),
}

impl Resource {
	/// Names one resource to load.
	pub fn new(id: &'static str) -> Self {
		Self::Id(id)
	}

	/// Loads every resource that matches `query`.
	pub fn query(query: impl Into<Query>) -> Self {
		Self::Query(query.into())
	}
}

impl From<&'static str> for Resource {
	fn from(id: &'static str) -> Self {
		Self::Id(id)
	}
}

impl From<Query> for Resource {
	fn from(query: Query) -> Self {
		Self::Query(query)
	}
}
