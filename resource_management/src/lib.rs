//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

pub mod asset;
pub mod resource;

pub mod types;

pub mod file_tracker;

pub mod shader_generation;

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
	Reference(String),
}

#[derive(Debug)]
pub struct Stream<'a> {
	/// The slice of the buffer to load the resource binary data into.
	buffer: &'a mut [u8],
	/// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
	name: &'a str,
}

impl <'a> Stream<'a> {
	pub fn new(name: &'a str, buffer: &'a mut [u8]) -> Self {
		Stream {
			buffer,
			name,
		}
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
}

/// Struct that describes a resource request.
pub struct ResourceRequest {
	_id: polodb_core::bson::oid::ObjectId,
	pub id: u64,
	pub	url: String,
	pub size: u64,
	pub hash: u64,
	pub class: String,
	pub resource: Box<dyn Resource>,
	pub required_resources: Vec<String>,
}

pub enum Lox<'a> {
	None,
	Streams(Vec<Stream<'a>>),
	Buffer(&'a mut [u8]),
}

pub struct LoadResourceRequest<'a> {
	/// The resource to load.
	resource_request: ResourceRequest,
	/// The buffers to load the resource binary data into.
	pub streams: Lox<'a>,
}

impl <'a> LoadResourceRequest<'a> {
	pub fn new(resource_request: ResourceRequest) -> Self {
		LoadResourceRequest {
			resource_request,
			streams: Lox::None,
		}
	}

	pub fn streams(mut self, streams: Vec<Stream<'a>>) -> Self {
		self.streams = Lox::Streams(streams);
		self
	}

	pub fn buffer(mut self, buffer: &'a mut [u8]) -> Self {
		self.streams = Lox::Buffer(buffer);
		self
	}
}

pub struct ResourceResponse {
	id: u64,
	url: String,
	size: u64,
	offset: u64,
	hash: u64,
	class: String,
	resource: Box<dyn Resource>,
	required_resources: Vec<String>,
}

impl ResourceResponse {
	pub fn new<T: Resource>(r: &GenericResourceSerialization, resource: T) -> Self {
		ResourceResponse {
			id: 0,
			url: r.url.clone(),
			size: 0,
			offset: 0,
			hash: 0,
			class: r.class.clone(),
			resource: Box::new(resource),
			required_resources: Vec::new(),
		}
	}
}

/// Trait that defines a resource.
pub trait Resource: downcast_rs::Downcast {
	/// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
	/// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// Is needed by the deserialize function.
	fn get_class(&self) -> &'static str;
}

downcast_rs::impl_downcast!(Resource);

#[derive(Debug, Clone)]
pub struct SerializedResourceDocument(polodb_core::bson::Document);

pub struct Request {
	pub resources: Vec<ResourceRequest>,
}

pub struct LoadRequest<'a> {
	pub resources: Vec<LoadResourceRequest<'a>>,
}

impl <'a> LoadRequest<'a> {
	pub fn new(resources: Vec<LoadResourceRequest<'a>>) -> Self {
		LoadRequest {
			resources,
		}
	}
}

pub struct Response {
	pub resources: Vec<ResourceResponse>,
}

/// Options for loading a resource.
#[derive(Debug)]
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

pub trait CreateResource: downcast_rs::Downcast + Send + Sync {
}

downcast_rs::impl_downcast!(CreateResource);

pub struct CreateInfo<'a> {
	pub name: &'a str,
	pub info: Box<dyn CreateResource>,
	pub data: &'a [u8],
}