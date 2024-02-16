use std::{hash::{Hash, Hasher}, io::Read, ops::Deref, pin::Pin};
use futures::future::join_all;
use polodb_core::bson::oid::ObjectId;
use smol::{fs::File, io::{AsyncReadExt, AsyncWriteExt}};
use crate::{GenericResourceSerialization, LoadRequest, LoadResourceRequest, LoadResults, Lox, ProcessedResources, Request, Resource, ResourceRequest, ResourceResponse, Response};

use super::resource_handler::ResourceHandler;

/// Resource manager.
/// Handles loading assets or resources from different origins (network, local, etc.).
/// It also handles caching of resources.
/// 
/// Resource can be sourced from the local filesystem or from the network.
/// When in a debug build it will lazily load resources from source and cache them.
/// When in a release build it will exclusively load resources from cache.
/// 
/// If accessing the filesystem paths will be relative to the assets directory, and assets should omit the extension.
/// 
/// The stored resource document is like the following:
/// ```json
/// { "_id":"OId" , "id": 01234, "path": "../..", "class": "X", "size": 0, "resource": { ... }, "hash": 0 }
/// ```
pub struct ResourceManager {
	db: polodb_core::Database,
	resource_handlers: Vec<Box<dyn ResourceHandler + Send>>,
	deserializers: std::collections::HashMap<&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>>,
}

impl From<polodb_core::Error> for LoadResults {
	fn from(error: polodb_core::Error) -> Self {
		match error {
			_ => LoadResults::LoadFailed
		}
	}
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new() -> Self {
		if let Err(error) = std::fs::create_dir_all(Self::resolve_resource_path(std::path::Path::new(""))) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create resources directory"),
			}
		}

		let mut args = std::env::args();

		let mut memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		if cfg!(test) { // If we are running tests we want to use memory database. This way we can run tests in parallel.
			memory_only = true;
		}

		let db_res = if !memory_only {
			polodb_core::Database::open_file(Self::resolve_resource_path(std::path::Path::new("resources.db")))
		} else {
			log::info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file(Self::resolve_resource_path(std::path::Path::new("resources.db"))).unwrap();

				log::warn!("Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file(Self::resolve_resource_path(std::path::Path::new("resources.db")));

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

		ResourceManager {
			db,
			resource_handlers: Vec::with_capacity(8),
			deserializers: std::collections::HashMap::with_capacity(8),
		}
	}

	pub fn add_resource_handler<T>(&mut self, resource_handler: T) where T: ResourceHandler + Send + 'static {
		for deserializer in resource_handler.get_deserializers() {
			self.deserializers.insert(deserializer.0, deserializer.1);
		}

		self.resource_handlers.push(Box::new(resource_handler));
	}

	/// Tries to load a resource from cache or source.\
	/// This is a more advanced version of get() as it allows to load resources that depend on other resources.\
	/// 
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	/// If the resource is in cache but it's data cannot be parsed, it will return None.
	/// Return is a tuple containing the resource description and it's associated binary data.\
	/// The requested resource will always the last one in the array. With the previous resources being the ones it depends on. This way when iterating the array forward the dependencies will be loaded first.
	pub async fn get(&self, path: &str) -> Option<(Response, Vec<u8>)> {
		todo!();

		// let request = self.load_from_cache_or_source(path).await?;

		// let size = request.resources.iter().map(|r| r.size).sum::<u64>() as usize;

		// let mut buffer = Vec::with_capacity(size);

		// unsafe { buffer.set_len(size); }

		// let mut a = utils::BufferAllocator::new(&mut buffer);

		// let request = request.resources.into_iter().map(|r| { let size = r.size as usize; LoadResourceRequest::new(r).buffer(a.take(size)) }).collect::<Vec<_>>();
		
		// let response = self.load_data_from_cache(LoadRequest::new(request),).await.ok()?;

		// Some((response, buffer))
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load_resource()` which will load the binary data.
	pub async fn request_resource(&self, path: &str) -> Option<Request> {
		// let request = self.load_from_cache_or_source(path).await?;
		// Some(request)
		todo!()
	}

	/// Loads the resource binary data from cache.\
	/// If a buffer range is provided it will load the data into the buffer.\
	/// If no buffer range is provided it will return the data in a vector.
	/// 
	/// If a buffer is not provided for a resurce in the options parameters it will be either be loaded into the provided buffer or returned in a vector.
	/// 
	/// Options: Let's you specify how to load the resources.
	/// ```json
	/// { "resources": [{ "path": "../..", "buffer":{ "index": 0, "offset": 0 } }]}
	/// ```
	pub async fn load_resource<'a>(&self, request: LoadRequest<'a>) -> Result<(Response, Option<Vec<u8>>), LoadResults> {
		let response = self.load_data_from_cache(request).await?;
		Ok((response, None))
	}

	/// Stores the asset as a resource.
	/// Returns the resource document.
	async fn write_resource_to_cache(&self, resource_package: &(GenericResourceSerialization, Vec<u8>)) -> Option<polodb_core::bson::Document> {
		let mut resource_document = polodb_core::bson::Document::new();

		let mut hasher = std::collections::hash_map::DefaultHasher::new();

		resource_document.insert("id", hasher.finish() as i64);
		resource_document.insert("size", resource_package.1.len() as i64);

		resource_document.insert("url", resource_package.0.url.clone());
		resource_package.0.url.hash(&mut hasher);

		resource_document.insert("class", resource_package.0.class.clone());

		let mut required_resources_json = polodb_core::bson::Array::new();

		for required_resources in &resource_package.0.required_resources { // TODO: make new type that gives a guarantee that these resources have been loaded
			match required_resources {
				ProcessedResources::Generated(g) => {
					required_resources_json.push(polodb_core::bson::Bson::String(g.0.url.clone()));
				},
				ProcessedResources::Reference(r) => {
					required_resources_json.push(polodb_core::bson::Bson::String(r.clone()));
				}
			}
		}

		resource_document.insert("required_resources", required_resources_json);

		let json_resource = resource_package.0.resource.clone();

		if let None = resource_document.get("hash") {
			let mut hasher = std::collections::hash_map::DefaultHasher::new();

			std::hash::Hasher::write(&mut hasher, resource_package.1.as_slice()); // Hash binary data

			std::hash::Hasher::write(&mut hasher, &polodb_core::bson::to_vec(&json_resource).unwrap()); // Hash resource metadata, since changing the resources description must also change the hash. (For caching purposes)

			resource_document.insert("hash", hasher.finish() as i64);
		}

		resource_document.insert("resource", json_resource);

		log::debug!("Generated resource: {:#?}", &resource_document);

		let insert_result = self.db.collection::<polodb_core::bson::Document>("resources").insert_one(&resource_document).ok()?;

		let resource_id = insert_result.inserted_id.as_object_id()?;

		let resource_path = Self::resolve_resource_path(std::path::Path::new(&resource_id.to_string()));

		let mut file = smol::fs::File::create(resource_path).await.ok()?;

		file.write_all(resource_package.1.as_slice()).await.ok()?;
		file.flush().await.ok()?; // Must flush to ensure the file is written to disk, or else reads can cause failures

		resource_document.insert("_id", resource_id);

		Some(resource_document)
	}

	/// Tries to load a resource from cache.\
	/// If the resource cannot be found/loaded or if it's become stale it will return None.
	async fn load_data_from_cache<'a>(&self, request: LoadRequest<'a>) -> Result<Response, LoadResults> {
		todo!();
		// let offset = 0usize;

		// let resources = request.resources.into_iter().map(|resource_container| { // Build responses			
		// 	let response = ResourceResponse {
		// 		id: resource_container.resource_request.id,
		// 		url: resource_container.resource_request.url,
		// 		size: resource_container.resource_request.size,
		// 		offset: offset as u64,
		// 		hash: resource_container.resource_request.hash,
		// 		class: resource_container.resource_request.class,
		// 		resource: resource_container.resource_request.resource,
		// 		required_resources: resource_container.resource_request.required_resources,
		// 	};
			
		// 	(resource_container.resource_request._id, resource_container.streams, response)
		// }).map(async move |(db_resource_id, slice, response)| { // Load resources
		// 	let native_db_resource_id = db_resource_id.to_string();
	
		// 	let mut file = match File::open(Self::resolve_resource_path(std::path::Path::new(&native_db_resource_id))).await {
		// 		Ok(it) => it,
		// 		Err(reason) => {
		// 			match reason { // TODO: handle specific errors
		// 				_ => return Err(LoadResults::CacheFileNotFound),
		// 			}
		// 		}
		// 	};

		// 	match slice {
		// 		Lox::None => {}
		// 		Lox::Buffer(buffer) => {
		// 			match file.read_exact(buffer).await {
		// 				Ok(_) => {},
		// 				Err(_) => {
		// 					return Err(LoadResults::LoadFailed);
		// 				}
		// 			}
		// 		}
		// 		Lox::Streams(mut streams) => {
		// 			if let Some(resource_handler) = self.resource_handlers.iter().find(|h| h.can_handle_type(response.class.as_str())) {
		// 				resource_handler.read(response.resource.deref(), &mut file, &mut streams).await;
		// 			} else {
		// 				log::warn!("No resource handler could handle resource: {}", response.url);
		// 			}
		// 		}
		// 	}

		// 	Ok(response)
		// }).collect::<Vec<_>>();

		// let resources = join_all(resources).await.into_iter().collect::<Result<Vec<_>, _>>()?;

		// return Ok(Response { resources });
	}

	fn resolve_resource_path(path: &std::path::Path) -> std::path::PathBuf {
		if cfg!(test) {
			std::env::temp_dir().join("resources").join(path)
		} else {
			std::path::PathBuf::from("resources/").join(path)
		}
	}
}

// TODO: test resource caching

#[cfg(test)]
mod tests {
	// TODO: test resource load order

	use super::*;
}