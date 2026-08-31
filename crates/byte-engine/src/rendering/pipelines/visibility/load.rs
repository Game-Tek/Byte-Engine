//! This module implements loading of resources for the Visibility pipeline.

/// The requesting counter part of the loading couple.
struct LoadClient {}

/// The serving/worker part of the loading couple.
struct LoadServer {
	resource_manager: EntityHandle<ResourceManager>,
	resource_factory: ghi::implementation::Factory,
}

impl LoadServer {
	async fn load(&self, request: ResourceRequest) {
		match request {
			ResourceRequest::Mesh { key, source } => {}
			ResourceRequest::Material { id } => {}
			ResourceRequest::Image { key } => {}
			ResourceRequest::Environment { id } => {}
		}
	}
}

#[derive(Clone)]
pub(crate) enum ResourceRequest {
	Mesh { key: MeshKey, source: MeshSource },
	Material { id: String },
	Image { key: String },
	Environment { id: String },
}

pub fn spawn(
	resource_manager: EntityHandle<ResourceManager>,
	resource_factory: ghi::implementation::Factory,
) -> (LoadClient, LoadServer) {
	let server = LoadServer {
		resource_manager,
		resource_factory,
	};

	(LoadClient {}, server)
}

use resource_management::ResourceManager;

use crate::{
	core::EntityHandle,
	rendering::renderable::mesh::{MeshKey, MeshSource},
};
