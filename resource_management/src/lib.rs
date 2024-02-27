//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use std::{borrow::Cow, hash::Hasher};

use polodb_core::bson;
use resource::resource_handler::{FileResourceReader, ReadTargets, ResourceReader};
use smol::io::AsyncWriteExt;

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
	resource: bson::Bson,
}

impl GenericResourceSerialization {
	pub fn new<T: Resource + serde::Serialize>(url: String, resource: T) -> Self {
		GenericResourceSerialization {
			url,
			required_resources: Vec::new(),
			class: resource.get_class().to_string(),
			resource: polodb_core::bson::to_bson(&resource).unwrap(),
		}
	}

	pub fn required_resources(mut self, required_resources: &[ProcessedResources]) -> Self {
		self.required_resources = required_resources.to_vec();
		self
	}
}

#[derive()]
pub struct GenericResourceResponse<'a> {
	/// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
	url: String,
	/// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
	class: String,
	size: usize,
	/// The resource data.
	resource: bson::Bson,
	read_target: Option<ReadTargets<'a>>,
}

impl <'a> GenericResourceResponse<'a> {
	pub fn new(url: String, class: String, size: usize, resource: bson::Bson,) -> Self {
		GenericResourceResponse {
			url,
			class,
			size,
			resource,
			read_target: None,
		}
	}

	pub fn set_box_buffer(&mut self, buffer: Box<[u8]>) {
		self.read_target = Some(ReadTargets::Box(buffer));
	}

	pub fn set_streams(&mut self, streams: &'a mut [Stream<'a>]) {
		self.read_target = Some(ReadTargets::Streams(streams));
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
	id: u64,
	url: String,
	size: u64,
	hash: u64,
	class: String,
}

impl ResourceRequest {
	pub fn new(resource: GenericResourceResponse) -> Self {
		ResourceRequest {
			id: 0,
			url: resource.url,
			size: resource.size as u64,
			hash: 0,
			class: resource.class,
		}
	}
	
	pub fn resource(&self) -> &dyn Resource {
		todo!()
	}
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
			resource_request: ResourceRequest {
				id: 0,
				url: resource_request.url,
				size: resource_request.size as u64,
				hash: 0,
				class: resource_request.class,
			},
			streams: Lox::None,
		}
	}

	pub fn id(&self) -> &str {
		&self.resource_request.url
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

pub struct ResourceResponse<'a> {
	id: u64,
	url: String,
	size: u64,
	offset: u64,
	hash: u64,
	class: String,
	resource: Box<dyn Resource>,
	required_resources: Vec<String>,
	read_target: Option<ReadTargets<'a>>,
}

impl <'a> ResourceResponse<'a> {
	pub fn new<T: Resource>(r: GenericResourceResponse<'a>, resource: T) -> Self {
		ResourceResponse {
			id: 0,
			url: r.url,
			size: 0,
			offset: 0,
			hash: 0,
			class: r.class,
			resource: Box::new(resource),
			required_resources: Vec::new(),
			read_target: r.read_target,
		}
	}

	pub fn resource(&self) -> &dyn Resource {
		self.resource.as_ref()
	}

	pub fn get_stream(&self, name: &str) -> Option<&[u8]> {
		match &self.read_target {
			Some(ReadTargets::Streams(streams)) => {
				for stream in streams.iter() {
					if stream.name == name {
						return Some(stream.buffer);
					}
				}

				None
			}
			_ => None,
		}
	}

	pub fn get_buffer(&self) -> Option<&[u8]> {
		match &self.read_target {
			Some(ReadTargets::Box(buffer)) => Some(buffer),
			Some(ReadTargets::Buffer(buffer)) => Some(buffer),
			_ => None,
		}
	}

	pub fn id(&self) -> u64 {
		self.id
	}

	pub fn hash(&self) -> u64 {
		self.hash
	}

	pub fn url(&self) -> &str {
		&self.url
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

pub struct Response<'a> {
	pub resources: Vec<ResourceResponse<'a>>,
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

pub struct TypedResourceDocument {
	url: String,
	class: String,
	resource: bson::Bson,
}

impl TypedResourceDocument {
	pub fn new(url: String, class: String, resource: bson::Bson) -> Self {
		TypedResourceDocument {
			url,
			class,
			resource,
		}
	}
}

impl From<GenericResourceSerialization> for TypedResourceDocument {
	fn from(value: GenericResourceSerialization) -> Self {
		TypedResourceDocument::new(value.url, value.class, value.resource)
	}
}

pub trait StorageBackend: Sync + Send {
	fn store<'a>(&'a self, resource: GenericResourceSerialization, data: &'a [u8]) -> utils::BoxedFuture<'a, Result<(), ()>>;
	fn read<'s, 'a, 'b>(&'s self, id: &'b str) -> utils::BoxedFuture<'a, Option<(GenericResourceResponse<'a>, Box<dyn ResourceReader>)>>;
}

struct DbStorageBackend {
	db: polodb_core::Database,
}

impl DbStorageBackend {
	pub fn new(path: &std::path::Path) -> Self {
		let mut memory_only = false;

		if cfg!(test) { // If we are running tests we want to use memory database. This way we can run tests in parallel.
			memory_only = true;
		}

		let db_res = if !memory_only {
			polodb_core::Database::open_file(path)
		} else {
			log::info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file(path).unwrap();

				log::warn!("Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file(path);

				match db_res {
					Ok(db) => db,
					Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
						Ok(db) => {
							log::error!("Could not create database file, using memory database instead.");
							db
						},
						Err(_) => panic!("Could not create database"),
					}
				}
			}
		};

		DbStorageBackend {
			db,
		}
	}

	fn resolve_resource_path(path: &std::path::Path) -> std::path::PathBuf {
		if cfg!(test) {
			std::env::temp_dir().join("resources").join(path)
		} else {
			std::path::PathBuf::from("resources/").join(path)
		}
	}
}

impl StorageBackend for DbStorageBackend {
	fn read<'s, 'a, 'b>(&'s self, id: &'b str) -> utils::BoxedFuture<'a, Option<(GenericResourceResponse<'a>, Box<dyn ResourceReader>)>> {
		let resource_document = self.db.collection::<bson::Document>("resources").find_one(bson::doc! { "_id": id }).ok();
		let id = id.to_string();

		Box::pin(async move {
			let resource: GenericResourceResponse<'a> = {
				let resource_document = resource_document??;

				let class = resource_document.get_str("class").ok()?.to_string();
				let size = resource_document.get_i64("size").ok()? as usize;
				let resource = resource_document.get("resource")?.clone();

				GenericResourceResponse::new(id.clone(), class, size, resource)
			};

			let resource_reader = FileResourceReader::new(smol::fs::File::open(Self::resolve_resource_path(std::path::Path::new(&id))).await.ok()?);	
	
			Some((resource, Box::new(resource_reader) as Box<dyn ResourceReader>))
		})
	}

	fn store<'a>(&'a self, resource: GenericResourceSerialization, data: &'a [u8]) -> utils::BoxedFuture<'a, Result<(), ()>> {
		Box::pin(async move {
			let mut resource_document = bson::Document::new();
	
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
	
			let size = 0usize;
			let url = "";
			let class = "";
	
			resource_document.insert("id", hasher.finish() as i64);
			resource_document.insert("size", size as i64);
	
			resource_document.insert("url", url);
	
			// resource_package.0.url.hash(&mut hasher);
	
			resource_document.insert("class", class);
	
			let mut required_resources_json = bson::Array::new();
	
			resource_document.insert("required_resources", required_resources_json);
	
			let json_resource = resource.resource.clone();
	
			if let None = resource_document.get("hash") {
				let mut hasher = std::collections::hash_map::DefaultHasher::new();
	
				std::hash::Hasher::write(&mut hasher, data); // Hash binary data
	
				std::hash::Hasher::write(&mut hasher, &bson::to_vec(&json_resource).unwrap()); // Hash resource metadata, since changing the resources description must also change the hash. (For caching purposes)
	
				resource_document.insert("hash", hasher.finish() as i64);
			}
	
			resource_document.insert("resource", json_resource);
	
			log::debug!("Generated resource: {:#?}", &resource_document);
	
			let insert_result = self.db.collection::<bson::Document>("resources").insert_one(&resource_document).or(Err(()))?;
	
			let resource_id = insert_result.inserted_id.as_object_id().unwrap();
	
			let resource_path = Self::resolve_resource_path(std::path::Path::new(&resource_id.to_string()));

			let mut file = smol::fs::File::create(resource_path).await.or(Err(()))?;
	
			file.write_all(data).await.or(Err(()))?;
			file.flush().await.or(Err(()))?; // Must flush to ensure the file is written to disk, or else reads can cause failures
			resource_document.insert("_id", resource_id);
	
			Ok(())
		})

	}
}