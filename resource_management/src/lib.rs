//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(async_closure)]
#![feature(closure_lifetime_binder)]
#![feature(stmt_expr_attributes)]
#![feature(path_file_prefix)]
#![feature(trait_upcasting)]

use std::{any::Any, hash::Hasher};

use polodb_core::bson;
use resource::resource_handler::{FileResourceReader, ReadTargets, ResourceReader};
use serde::{ser::SerializeStruct, Serialize};
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
	id: String,
	/// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
	class: String,
	/// List of resources that this resource depends on.
	// required_resources: Vec<ProcessedResources>,
	/// The resource data.
	resource: bson::Bson,
}

impl GenericResourceSerialization {
	pub fn new<T: Resource + serde::Serialize>(id: &str, resource: T) -> Self {
		GenericResourceSerialization {
			id: id.to_string(),
			// required_resources: Vec::new(),
			class: resource.get_class().to_string(),
			resource: polodb_core::bson::to_bson(&resource).unwrap(),
		}
	}

	pub fn new_with_serialized(id: &str, class: &str, resource: bson::Bson) -> Self {
		GenericResourceSerialization {
			id: id.to_string(),
			// required_resources: Vec::new(),
			class: class.to_string(),
			resource,
		}
	}


	// pub fn required_resources(mut self, required_resources: &[ProcessedResources]) -> Self {
	// 	self.required_resources = required_resources.to_vec();
	// 	self
	// }
}

impl <T: Model + for <'de> serde::Deserialize<'de>> From<GenericResourceSerialization> for TypedResourceModel<T> {
	fn from(value: GenericResourceSerialization) -> Self {
		TypedResourceModel::new(&value.id, 0) // TODO: hash
	}
}

impl <T: Resource + Serialize + Clone> From<TypedResource<T>> for GenericResourceSerialization {
	fn from(value: TypedResource<T>) -> Self {
		GenericResourceSerialization {
			id: value.id,
			class: value.resource.get_class().to_string(),
			resource: polodb_core::bson::to_bson(&value.resource).unwrap(),
		}
	}
}

#[derive()]
pub struct GenericResourceResponse<'a> {
	/// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
	id: String,
	hash: u64,
	/// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
	class: String,
	size: usize,
	/// The resource data.
	resource: bson::Bson,
	read_target: Option<ReadTargets<'a>>,
}

impl <'a> GenericResourceResponse<'a> {
	pub fn new(id: &str, hash: u64, class: String, size: usize, resource: bson::Bson,) -> Self {
		GenericResourceResponse {
			id: id.to_string(),
			hash,
			class,
			size,
			resource,
			read_target: None,
		}
	}

	pub fn set_box_buffer(&mut self, buffer: Box<[u8]>) {
		self.read_target = Some(ReadTargets::Box(buffer));
	}

	pub fn set_streams(&mut self, streams: Vec<Stream<'a>>) {
		self.read_target = Some(ReadTargets::Streams(streams));
	}
}

pub trait Model {
	fn get_class() -> &'static str;
}

#[derive(Debug, serde::Deserialize)]
pub struct TypedResourceModel<T: Model> {
	pub id: String,
	hash: u64,
	class: String,
	#[serde(skip)]
	phantom: std::marker::PhantomData<T>,
}

impl <T: Model> TypedResourceModel<T> {
	pub fn new(id: &str, hash: u64) -> Self {
		TypedResourceModel {
			id: id.to_string(),
			hash,
			class: T::get_class().to_string(),
			phantom: std::marker::PhantomData,
		}
	}
}

impl <'a, T: Model + ResourceStore<'a>> From<T> for TypedResourceModel<T> {
	fn from(value: T) -> Self {
		TypedResourceModel {
			id: value.id().to_string(),
			hash: 0,
			class: value.class().to_string(),
			phantom: std::marker::PhantomData,
		}
	}
}

#[derive(Debug, Clone)]
pub struct TypedResource<T: Resource + Clone> {
	pub id: String,
	hash: u64,
	resource: T,
	pub buffer: Option<Box<[u8]>>,
}

impl <T: Resource + Clone> Serialize for TypedResource<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {
		let mut state = serializer.serialize_struct("TypedDocument", 3)?;

		state.serialize_field("id", &self.id)?;
		state.serialize_field("hash", &self.hash)?;
		state.serialize_field("class", &self.resource().get_class())?;

		state.end()
	}
}

impl <T: Resource + Clone> TypedResource<T> {
	pub fn new(id: &str, hash: u64, resource: T) -> Self {
		TypedResource {
			id: id.to_string(),
			hash,
			resource,
			buffer: None,
		}
	}

	pub fn new_with_buffer(id: &str, hash: u64, resource: T, buffer: Box<[u8]>) -> Self {
		TypedResource {
			id: id.to_string(),
			hash,
			resource,
			buffer: Some(buffer),
		}
	}

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn hash(&self) -> u64 {
		self.hash
	}

	pub fn resource(&self) -> &T {
		&self.resource
	}

	pub fn get_buffer(&self) -> Option<&[u8]> {
		self.buffer.as_ref().map(|b| &**b)
	}
}

impl <'a, M: Model + for<'de> serde::Deserialize<'de>> Into<TypedResourceModel<M>> for GenericResourceResponse<'a> {
	fn into(self) -> TypedResourceModel<M> {
		TypedResourceModel::new(&self.id, 0) // TODO: hash
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
	id: String,
	size: u64,
	hash: u64,
	class: String,
	resource: Box<dyn Resource>,
}

impl ResourceRequest {
	pub fn new(metadata: ResourceResponse) -> Self {
		ResourceRequest {
			id: metadata.id,
			size: metadata.size as u64,
			hash: 0,
			class: metadata.class,
			resource: metadata.resource,
		}
	}
	
	pub fn resource(&self) -> &dyn Resource {
		self.resource.as_ref()
	}
}

pub struct LoadResourceRequest<'a> {
	/// The resource to load.
	resource_request: ResourceRequest,
	/// The buffers to load the resource binary data into.
	pub streams: Option<ReadTargets<'a>>,
}

impl <'a> LoadResourceRequest<'a> {
	pub fn new(resource_request: ResourceRequest) -> Self {
		LoadResourceRequest {
			resource_request: ResourceRequest {
				id: resource_request.id,
				size: resource_request.size as u64,
				hash: 0,
				class: resource_request.class,
				resource: resource_request.resource,
			},
			streams: None,
		}
	}

	pub fn id(&self) -> &str {
		&self.resource_request.id
	}

	pub fn streams(mut self, streams: Vec<Stream<'a>>) -> Self {
		self.streams = Some(ReadTargets::Streams(streams));
		self
	}

	pub fn buffer(mut self, buffer: &'a mut [u8]) -> Self {
		self.streams = Some(ReadTargets::Buffer(buffer));
		self
	}
}

pub struct ResourceResponse<'a> {
	id: String,
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
			id: r.id,
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

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn hash(&self) -> u64 {
		self.hash
	}
}

impl <T: Resource + Clone> From<TypedResource<T>> for ResourceResponse<'_> {
	fn from(value: TypedResource<T>) -> Self {
		ResourceResponse {
			id: value.id,
			size: 0,
			offset: 0,
			hash: value.hash,
			class: value.resource.get_class().to_string(),
			resource: Box::new(value.resource),
			required_resources: Vec::new(),
			read_target: value.buffer.map(|b| ReadTargets::Box(b)),
		}
	}
}

/// Trait that defines a resource.
pub trait Resource: downcast_rs::Downcast + Send + Sync {
	/// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
	/// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// Is needed by the deserialize function.
	fn get_class(&self) -> &'static str;
}

downcast_rs::impl_downcast!(Resource);

pub trait ResourceStore<'a> {
	fn id(&self) -> &str;
	fn resource(&self) -> &dyn Resource;
	fn get_buffer(&self) -> Option<&[u8]>;
	fn class(&self) -> &str;
}

impl <'a, T: Resource + Clone> ResourceStore<'a> for TypedResource<T> {
	fn id(&self) -> &str {
		self.id()
	}
	
	fn class(&self) -> &str {
		self.resource().get_class()
	}
	fn resource(&self) -> &dyn Resource {
		self.resource()
	}

	fn get_buffer(&self) -> Option<&[u8]> {
		self.get_buffer()
	}
}

impl <'a> ResourceStore<'a> for ResourceResponse<'a> {
	fn id(&self) -> &str {
		self.id()
	}

	fn class(&self) -> &str {
		&self.class
	}

	fn resource(&self) -> &dyn Resource {
		self.resource()
	}

	fn get_buffer(&self) -> Option<&[u8]> {
		self.get_buffer()
	}
}

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
		TypedResourceDocument::new(value.id, value.class, value.resource)
	}
}

enum SolveErrors {
	DeserializationFailed(String),
	StorageError,
}

trait Solver<'de, T> where Self: serde::Deserialize<'de> {
	fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<T, SolveErrors>;
}

// impl <T: Resource, 'de> Solver<'de> for T where T: Clone 

pub trait StorageBackend: Sync + Send + downcast_rs::Downcast {
	fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>>;
	fn delete<'a>(&'a self, id: &'a str) -> utils::BoxedFuture<'a, Result<(), String>>;
	fn store<'a>(&'a self, resource: &'a GenericResourceSerialization, data: &'a [u8]) -> utils::SendSyncBoxedFuture<'a, Result<(), ()>>;
	fn read<'s, 'a, 'b>(&'s self, id: &'b str) -> utils::BoxedFuture<'a, Option<(GenericResourceResponse<'a>, Box<dyn ResourceReader>)>>;
	fn sync<'s, 'a>(&'s self, _: &'a dyn StorageBackend) -> utils::BoxedFuture<'a, ()> {
		Box::pin(async move {})
	}
}

downcast_rs::impl_downcast!(StorageBackend);

pub struct DbStorageBackend {
	db: polodb_core::Database,
	base_path: std::path::PathBuf,
}

impl DbStorageBackend {
	pub fn new(base_path: &std::path::Path) -> Self {
		let mut memory_only = false;

		if cfg!(test) { // If we are running tests we want to use memory database. This way we can run tests in parallel.
			memory_only = true;
		}

		let db_res = if !memory_only {
			polodb_core::Database::open_file(base_path.join("resources.db"))
		} else {
			log::info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// // Delete file and try again
				// std::fs::remove_file(path).unwrap();

				// log::warn!("Database file was corrupted, deleting and trying again.");

				// let db_res = polodb_core::Database::open_file(path);

				// match db_res {
				// 	Ok(db) => db,
				// 	Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
				// 		Ok(db) => {
				// 			log::error!("Could not create database file, using memory database instead.");
				// 			db
				// 		},
				// 		Err(_) => panic!("Could not create database"),
				// 	}
				// }
				panic!("Could not create database")
			}
		};

		// db.collection::<bson::Document>("resources").create_index(polodb_core::IndexModel{ keys: bson::doc! { "id": 1 }, options: Some(polodb_core::IndexOptions{ name: None, unique: Some(true) }) });

		DbStorageBackend {
			db,
			base_path: base_path.to_path_buf(),
		}
	}
}

impl StorageBackend for DbStorageBackend {
	fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>> {
		Box::pin(async move {
			let mut resources = Vec::new();

			let cursor = self.db.collection::<bson::Document>("resources").find(None); // polodb_core can sometimes return deleted records, so don't worry about it

			for doc in cursor.unwrap() {
				let doc = doc.unwrap();
				resources.push(doc.get_str("id").unwrap().to_string());
			}

			Ok(resources)
		})
	}

	fn delete<'a>(&'a self, id: &'a str) -> utils::BoxedFuture<'a, Result<(), String>> {
		Box::pin(async move {
			match self.db.collection::<bson::Document>("resources").find_one(bson::doc! { "id": id }) {
				Ok(o) => {
					match o {
						Some(o) => {
							let db_id = o.get_object_id("_id").map_err(|_| "Resource entry does not have '_id' key of type 'ObjectId'".to_string())?;
							let resource_path = self.base_path.join(std::path::Path::new(db_id.to_string().as_str()));

							let file_deletion_result = smol::fs::remove_file(&resource_path).await;

							let doc_deletion_result = match self.db.collection::<bson::Document>("resources").delete_one(bson::doc!{ "_id": db_id }) {
								Ok(o) => {
									if o.deleted_count == 1 { // polodb_core sometimes returns 1 even if the resource was supposed to be deleted
										Ok(())
									} else {
										Err("Resource not found".to_string())
									}
								}
								Err(e) => {
									Err(e.to_string())
								}
							};

							match (file_deletion_result, doc_deletion_result) {
								(Ok(()), Ok(())) => {
									Ok(())
								}
								(Ok(_), Err(_)) => {
									Ok(())
								}
								(Err(e), Ok(_)) => {
									match e.kind() {
										std::io::ErrorKind::NotFound => {
											Ok(())
										}
										_ => {
											Err(e.to_string())
										}
									}
								}
								(Err(_) , Err(_)) => {
									Err("Resource not found".to_string())
								}
							}
						}
						None => {
							Err("Resource not found".to_string())
						}
					}
				}
				Err(e) => {
					Err(e.to_string())
				}
			}
		})
	}

	fn read<'s, 'a, 'b>(&'s self, id: &'b str) -> utils::BoxedFuture<'a, Option<(GenericResourceResponse<'a>, Box<dyn ResourceReader>)>> {
		let resource_document = self.db.collection::<bson::Document>("resources").find_one(bson::doc! { "id": id }).ok();
		let id = id.to_string();
		let base_path = self.base_path.clone();

		Box::pin(async move {
			let resource_document = resource_document??;

			let resource = {
				let hash = resource_document.get_i64("hash").ok()? as u64;
				let class = resource_document.get_str("class").ok()?.to_string();
				let size = resource_document.get_i64("size").ok()? as usize;
				let resource = resource_document.get("resource")?.clone();

				GenericResourceResponse::new(&id, hash, class, size, resource)
			};

			let resource_reader = FileResourceReader::new(smol::fs::File::open(base_path.join(std::path::Path::new(&resource_document.get_object_id("_id").ok()?.to_string()))).await.ok()?);	
	
			Some((resource, Box::new(resource_reader) as Box<dyn ResourceReader>))
		})
	}

	fn store<'a>(&'a self, resource: &'a GenericResourceSerialization, data: &'a [u8]) -> utils::SendSyncBoxedFuture<'a, Result<(), ()>> { // TODO: define schema
		// TODO: skip storage of items with same hash
		Box::pin(async move {
			let mut resource_document = bson::Document::new();
	
			let size = data.len();
			let url = &resource.id;
			let class = &resource.class;
	
			resource_document.insert("id", url);
			resource_document.insert("size", size as i64);
			resource_document.insert("class", class);
	
			let json_resource = resource.resource.clone();
	
			if let None = resource_document.get("hash") {
				let mut hasher = std::collections::hash_map::DefaultHasher::new();
	
				std::hash::Hasher::write(&mut hasher, data); // Hash binary data
	
				std::hash::Hasher::write(&mut hasher, &bson::to_vec(&json_resource).unwrap()); // Hash resource metadata, since changing the resources description must also change the hash. (For caching purposes)
	
				resource_document.insert("hash", hasher.finish() as i64);
			}
	
			resource_document.insert("resource", json_resource);
	
			let insert_result = self.db.collection::<bson::Document>("resources").insert_one(&resource_document).or(Err(()))?;
	
			let resource_id = insert_result.inserted_id.as_object_id().unwrap();
	
			let resource_path = self.base_path.join(std::path::Path::new(&resource_id.to_string()));

			let mut file = smol::fs::File::create(resource_path).await.or(Err(()))?;
	
			file.write_all(data).await.or(Err(()))?;
			file.flush().await.or(Err(()))?; // Must flush to ensure the file is written to disk, or else reads can cause failures
			resource_document.insert("_id", resource_id);
	
			Ok(())
		})
	}

	fn sync<'s, 'a>(&'s self, other: &'a dyn StorageBackend) -> utils::BoxedFuture<'a, ()> {
		if let Some(other) = other.downcast_ref::<DbStorageBackend>() {
			self.db.collection::<bson::Document>("resources").drop();

			other.db.collection::<bson::Document>("resources").find(bson::doc! {}).unwrap().for_each(|doc| {
				self.db.collection::<bson::Document>("resources").insert_one(&doc.unwrap()).unwrap();
			});
		}

		Box::pin(async move {})
	}
}

pub trait Description: Any + Send + Sync {
	// type Resource: Resource;
	fn get_resource_class() -> &'static str where Self: Sized;
}

#[cfg(test)]
mod tests {
    use polodb_core::bson;
    use serde::Deserialize;

    use crate::{asset::tests::TestStorageBackend, GenericResourceSerialization, Model, Resource, SolveErrors, Solver, StorageBackend, TypedResource, TypedResourceModel};

	#[test]
	fn solve_resources() {
		#[derive(serde::Serialize, Clone)]
		struct Base {
			items: Vec<TypedResource<Item>>,
		}

		#[derive(serde::Deserialize)]
		struct BaseModel {
			items: Vec<TypedResourceModel<Item>>,
		}

		#[derive(serde::Serialize, serde::Deserialize, Clone)]
		struct Item {
			property: String,
		}

		#[derive(serde::Serialize)]
		struct Variant {
			parent: TypedResource<Base>
		}

		impl Resource for Base {
			fn get_class(&self) -> &'static str { "Base" }
		}

		impl Model for BaseModel {
			fn get_class() -> &'static str { "Base" }
		}

		impl Resource for Item {
			fn get_class(&self) -> &'static str { "Item" }
		}

		impl Model for Item {
			fn get_class() -> &'static str { "Item" }
		}

		impl Resource for Variant {
			fn get_class(&self) -> &'static str { "Variant" }
		}

		let storage_backend = TestStorageBackend::new();

		smol::block_on(storage_backend.store(&GenericResourceSerialization::new("item", Item{ property: "hello".to_string() }), &[])).unwrap();
		smol::block_on(storage_backend.store(&GenericResourceSerialization::new("base", Base{ items: vec![TypedResource::new("item", 0, Item{ property: "hello".to_string() })] }), &[])).unwrap();
		smol::block_on(storage_backend.store(&GenericResourceSerialization::new("variant", Variant{ parent: TypedResource::new("base", 0, Base{ items: vec![TypedResource::new("item", 0, Item{ property: "hello".to_string() })] }) }), &[])).unwrap();

		impl <'de> Solver<'de, TypedResource<Base>> for TypedResourceModel<BaseModel> {
			fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Base>, SolveErrors> {
				let (gr, _) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
				println!("{:#?}", gr.resource);
				let resource = BaseModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;
				let base = Base{
					items: resource.items.iter().map(|item| {
						item.solve(storage_backend)
					}).collect::<Result<_, _>>().map_err(|_| SolveErrors::StorageError)?
				};
				Ok(TypedResource::new(&gr.id, 0, base))
			}
		}

		impl <'de> Solver<'de, TypedResource<Item>> for TypedResourceModel<Item> {
			fn solve(&self, storage_backend: &dyn StorageBackend) -> Result<TypedResource<Item>, SolveErrors> {
				let (gr, _) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
				let item = Item::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;
				Ok(TypedResource::new(&gr.id, 0, item))
			}
		}

		let base: TypedResourceModel<BaseModel> = smol::block_on(storage_backend.read("base")).unwrap().0.into();
		let base = base.solve(&storage_backend);
	}
}