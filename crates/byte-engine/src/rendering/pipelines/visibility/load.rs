//! This module implements loading of resources for the Visibility pipeline.

/// The shared elements for a loading couple.
trait LoadBase {
	/// The handle to a resource. A resource is anything that can be loaded by the pipeline.
	type ResourceHandle: Hash;
	/// The requirements for a resource.
	/// These are the memory requirements for the resource.
	type ResourceRequirements;
}

/// The requesting counter part of the loading couple.
trait LoadClient: LoadBase {
	fn request(&self, resource: Self::ResourceHandle);
}

/// The serving/worker part of the loading couple.
trait LoadServer: LoadBase {}

use std::hash::Hash;

use resource_management::ResourceManager;

use crate::{
	core::EntityHandle,
	rendering::renderable::mesh::{MeshKey, MeshSource},
};
