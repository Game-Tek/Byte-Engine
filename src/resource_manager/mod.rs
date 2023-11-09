//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

pub mod resource_manager;

pub mod resource_handler;
pub mod texture_resource_handler;
pub mod mesh_resource_handler;
pub mod material_resource_handler;

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

/// This is the struct resource handlers should return when processing a resource.
#[derive(Debug, Clone)]
pub struct GenericResourceSerialization {
	/// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
	url: String,
	/// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
	class: String,
	/// List of resources that this resource depends on.
	required_resources: Vec<ProcessedResources>,
	/// The resource data.
	resource: polodb_core::bson::Document,
}

impl GenericResourceSerialization {
	pub fn new<T: Resource + serde::Serialize>(url: String, resource: T) -> Self {
		GenericResourceSerialization {
			url,
			required_resources: Vec::new(),
			class: resource.get_class().to_string(),
			resource: polodb_core::bson::to_document(&resource).unwrap(),
		}
	}

	pub fn required_resources(mut self, required_resources: &[ProcessedResources]) -> Self {
		self.required_resources = required_resources.to_vec();
		self
	}
}

#[derive(Debug, Clone)]
pub enum ProcessedResources {
	Generated((GenericResourceSerialization, Vec<u8>)),
	Ref(String),
}

pub struct Stream<'a> {
	/// The slice of the buffer to load the resource binary data into.
	pub buffer: &'a mut [u8],
	/// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
	pub name: String,
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
}

/// Struct that describes a resource request.
pub struct ResourceRequest {
	_id: polodb_core::bson::oid::ObjectId,
	pub id: u64,
	pub	url: String,
	pub size: u64,
	pub hash: u64,
	pub class: String,
	pub resource: Box<dyn std::any::Any>,
	pub required_resources: Vec<String>,
}

pub struct ResourceResponse {
	pub id: u64,
	pub	url: String,
	pub size: u64,
	pub offset: u64,
	pub hash: u64,
	pub class: String,
	pub resource: Box<dyn std::any::Any>,
	pub required_resources: Vec<String>,
}

/// Trait that defines a resource.
pub trait Resource {
	/// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
	/// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// Is needed by the deserialize function.
	fn get_class(&self) -> &'static str;
}

#[derive(Debug, Clone)]
pub struct SerializedResourceDocument(polodb_core::bson::Document);

pub struct Request {
	pub resources: Vec<ResourceRequest>,
}

pub struct Response {
	pub resources: Vec<ResourceResponse>,
}

/// Options for loading a resource.
pub struct OptionResource<'a> {
	/// The resource to apply this option to.
	pub url: String,
	/// The buffers to load the resource binary data into.
	pub streams: Vec<Stream<'a>>,
}

/// Represents the options for performing a bundled/batch resource load.
pub struct Options<'a> {
	pub resources: Vec<OptionResource<'a>>,
}