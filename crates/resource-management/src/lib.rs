//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(closure_lifetime_binder)]
#![feature(stmt_expr_attributes)]
#![feature(path_file_prefix, path_add_extension)]
#![feature(map_try_insert)]
#![feature(future_join)]
#![feature(once_cell_try)]
#![feature(ascii_char)]

use std::{any::Any, hash::Hasher};
use serde::{Deserialize, Serialize};

use asset::ResourceId;

pub mod asset;
pub mod resource;

pub mod model;
pub mod solver;
pub mod stream;
pub mod reference;

pub mod types;

pub mod resources;

pub mod file_tracker;

pub mod shader_generator;

pub mod glsl_shader_generator;
pub mod spirv_shader_generator;

pub mod glsl;

pub use resource::resource_manager::ResourceManager;
pub use asset::asset_handler::AssetHandler;

pub use model::Model;
pub use resource::Resource;
pub use solver::Solver;
pub use stream::Stream;
pub use reference::Reference;
pub use reference::ReferenceModel;

pub(crate) type DataStorage = Vec<u8>;
pub(crate) use pot::from_slice;

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

/// This is the struct resource handlers should return when processing a resource.
#[derive(Debug, Clone)]
pub struct ProcessedAsset {
    /// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
    id: String,
    /// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
    class: String,
    /// List of resources that this resource depends on.
    // required_resources: Vec<ProcessedResources>,
    /// The resource data.
    // resource: Data,
	resource: DataStorage,
    streams: Option<Vec<StreamDescription>>,
}

impl ProcessedAsset {
    pub fn new<T: Model + serde::Serialize>(id: ResourceId<'_>, resource: T) -> Self {
        ProcessedAsset {
            id: id.to_string(),
            class: T::get_class().to_string(),
            resource: pot::to_vec(&resource).unwrap(),
            streams: None,
        }
    }

    pub fn new_with_serialized(id: &str, class: &str, resource: DataStorage) -> Self {
        ProcessedAsset {
            id: id.to_string(),
            class: class.to_string(),
            resource,
            streams: None,
        }
    }

    pub fn with_streams(mut self, streams: Vec<StreamDescription>) -> Self {
        self.streams = Some(streams);
        self
    }
}

impl<'a, T: Resource + Serialize + Clone> From<Reference<T>> for ProcessedAsset {
    fn from(value: Reference<T>) -> Self {
        ProcessedAsset {
            id: value.id,
            class: value.resource.get_class().to_string(),
            resource: pot::to_vec(&value.resource).unwrap(),
            streams: None,
        }
    }
}

impl From<SerializableResource> for ProcessedAsset {
    fn from(value: SerializableResource) -> Self {
        ProcessedAsset {
            id: value.id,
            class: value.class,
            resource: value.resource.clone(),
            streams: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamDescription {
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
    name: String,
    /// The subresource size.
    size: usize,
    /// The subresource offset.
    offset: usize,
}

impl StreamDescription {
    pub fn new(name: &str, size: usize, offset: usize) -> Self {
        StreamDescription {
            name: name.to_string(),
            size,
            offset,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SerializableResource {
    /// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
    id: String,
    hash: u64,
    /// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
    class: String,
    size: usize,
	resource: DataStorage,
	streams: Option<Vec<StreamDescription>>,
}

impl SerializableResource {
    pub fn new(id: String, hash: u64, class: String, size: usize, resource: DataStorage, streams: Option<Vec<StreamDescription>>) -> Self {
        SerializableResource {
            id,
            hash,
            class,
            size,
            resource,
			streams,
        }
    }
}

impl <M: Model> Into<ReferenceModel<M>> for SerializableResource {
	fn into(self) -> ReferenceModel<M> {
		ReferenceModel::new_serialized(&self.id, self.hash, self.size, self.resource, self.streams)
	}
}

/// Enumaration for all the possible results of a resource load fails.
#[derive(Debug)]
pub enum LoadResults {
    /// No resource could be resolved for the given path.
    ResourceNotFound,
    /// The resource could not be loaded.
    LoadFailed,
    /// The resource could not be found in cache.
    CacheFileNotFound,
    /// The resource type is not supported.
    UnsuportedResourceType,
    /// No read target was set for the resource.
    NoReadTarget,
}

pub trait Description: Any + Send + Sync {
    // type Resource: Resource;
    fn get_resource_class() -> &'static str
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
	/// Path to the assets folder for the tests.
	pub const ASSETS_PATH: &str = "../../assets";

	/// Path to the resources folder for the tests.
	/// This is rarely used, but is here for completeness.
	/// Most of the time the resources are written to memory backed storage or to a /tmp folder.
	pub const RESOURCES_PATH: &str = "../../resources";
}
