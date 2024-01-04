use std::{io::{Read, Write}, hash::{Hasher, Hash}, pin::Pin};
use futures::{AsyncReadExt, AsyncWriteExt};
use log::{info, warn, error, trace, debug};
use smol::fs::File;
use crate::{orchestrator, utils};
use super::{resource_handler, texture_resource_handler, mesh_resource_handler, material_resource_handler, Request, Response, Options, LoadResults, ProcessedResources, ResourceRequest, GenericResourceSerialization, ResourceResponse, Resource, audio_resource_handler, Stream, LoadRequest, LoadResourceRequest, Lox};

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
	resource_handlers: Vec<Box<dyn resource_handler::ResourceHandler + Send>>,
	deserializers: std::collections::HashMap<&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>>,
}

impl orchestrator::Entity for ResourceManager {}
impl orchestrator::System for ResourceManager {}

impl From<polodb_core::Error> for super::LoadResults {
	fn from(error: polodb_core::Error) -> Self {
		match error {
			_ => super::LoadResults::LoadFailed
		}
	}
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new() -> Self {
		if let Err(error) = std::fs::create_dir_all(Self::resolve_asset_path(std::path::Path::new(""))) {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create assets directory"),
			}
		}

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
			info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file(Self::resolve_resource_path(std::path::Path::new("resources.db"))).unwrap();

				warn!("Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file(Self::resolve_resource_path(std::path::Path::new("resources.db")));

				match db_res {
					Ok(db) => db,
					Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
						Ok(db) => {
							error!("Could not create database file, using memory database instead.");
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

	pub fn new_as_system() -> orchestrator::EntityReturn<'static, ResourceManager> {
		orchestrator::EntityReturn::new(Self::new())
	}

	pub fn add_resource_handler<T>(&mut self, resource_handler: T) where T: resource_handler::ResourceHandler + Send + 'static {
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
		let request = self.load_from_cache_or_source(path).await?;

		let size = request.resources.iter().map(|r| r.size).sum::<u64>() as usize;

		let mut buffer = Vec::with_capacity(size);

		unsafe { buffer.set_len(size); }

		let mut a = utils::BufferAllocator::new(&mut buffer);

		let request = request.resources.into_iter().map(|r| { let size = r.size as usize; LoadResourceRequest::new(r).buffer(a.take(size)) }).collect::<Vec<_>>();
		
		let response = self.load_data_from_cache(LoadRequest::new(request),).await.ok()?;

		Some((response, buffer))
	}

	/// Tries to load the information/metadata for a resource (and it's dependencies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load_resource()` which will load the binary data.
	pub async fn request_resource(&self, path: &str) -> Option<Request> {
		let request = self.load_from_cache_or_source(path).await?;
		Some(request)
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

	/// Recursively loads all the resources needed to load the resource at the given url.
	/// **Will** load from source and cache the resources if they are not already cached.
	fn gather<'a>(&'a self, db: &'a polodb_core::Database, url: &'a str) -> Pin<Box<dyn std::future::Future<Output = Option<Vec<polodb_core::bson::Document>>> + 'a>> {
		Box::pin(async move {
			let resource_documents = if let Some(resource_document) = db.collection::<polodb_core::bson::Document>("resources").find_one(polodb_core::bson::doc!{ "url": url }).unwrap() {
				let mut documents = vec![];
				
				if let Some(polodb_core::bson::Bson::Array(required_resources)) = resource_document.get("required_resources") {
					for required_resource in required_resources {
						if let polodb_core::bson::Bson::Document(required_resource) = required_resource {
							let resource_path = required_resource.get("url").unwrap().as_str().unwrap();
							documents.append(&mut self.gather(db, resource_path).await?);
						}

						if let polodb_core::bson::Bson::String(required_resource) = required_resource {
							let resource_path = required_resource.as_str();
							documents.append(&mut self.gather(db, resource_path).await?);
						}
					}
				}

				documents.push(resource_document);

				documents
			} else {
				// let r = self.read_asset_from_source(url).unwrap();

				let mut loaded_resource_documents = Vec::new();

				let asset_type = self.get_url_type(url);

				let resource_handlers = self.resource_handlers.iter().filter(|h| h.can_handle_type(&asset_type));

				for resource_handler in resource_handlers {
					let gg = resource_handler.process(self, url,).await.unwrap();

					for g in gg {
						match g {
							ProcessedResources::Generated(g) => {
								for e in &g.0.required_resources {
									match e {
										ProcessedResources::Generated(g) => {
											loaded_resource_documents.push(self.write_resource_to_cache(g,).await?);
										},
										ProcessedResources::Ref(r) => {
											loaded_resource_documents.append(&mut self.gather(db, r).await?);
										}
									}
								}

								loaded_resource_documents.push(self.write_resource_to_cache(&g,).await?);
							},
							ProcessedResources::Ref(r) => {
								loaded_resource_documents.append(&mut self.gather(db, &r).await?);
							}
						}
					}
				}

				if loaded_resource_documents.is_empty() {
					warn!("No resource handler could handle resource: {}", url);
				}

				loaded_resource_documents
			};


			Some(resource_documents)
		})
	}

	/// Tries to load a resource from cache.\
	/// It also resolves all dependencies.\
	async fn load_from_cache_or_source(&self, url: &str) -> Option<Request> {
		let resource_descriptions = self.gather(&self.db, url).await.expect("Could not load resource");

		for r in &resource_descriptions {
			trace!("Loaded resource: {:#?}", r);
		}

		let request = Request {
			resources: resource_descriptions.iter().map(|r|
				ResourceRequest { 
					_id: r.get_object_id("_id").unwrap(),
					id: r.get_i64("id").unwrap() as u64,
					url: r.get_str("url").unwrap().to_string(),
					size: r.get_i64("size").unwrap() as u64,
					hash: r.get_i64("hash").unwrap() as u64,
					class: r.get_str("class").unwrap().to_string(),
					resource: self.deserializers[r.get_str("class").unwrap()](r.get_document("resource").unwrap()),
					required_resources: if let Ok(rr) = r.get_array("required_resources") { rr.iter().map(|e| e.as_str().unwrap().to_string()).collect() } else { vec![] },
				}
			).collect(),
		};

		Some(request)
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
				ProcessedResources::Ref(r) => {
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

		debug!("Generated resource: {:#?}", &resource_document);

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
		let mut offset = 0usize;

		let resources = request.resources.into_iter().map(|resource_container| { // Build responses			
			let response = ResourceResponse {
				id: resource_container.resource_request.id,
				url: resource_container.resource_request.url,
				size: resource_container.resource_request.size,
				offset: offset as u64,
				hash: resource_container.resource_request.hash,
				class: resource_container.resource_request.class,
				resource: resource_container.resource_request.resource,
				required_resources: resource_container.resource_request.required_resources,
			};
			
			(resource_container.resource_request._id, resource_container.streams, response)
		}).map(async move |(db_resource_id, slice, response)| { // Load resources
			let native_db_resource_id = db_resource_id.to_string();
	
			let mut file = match File::open(Self::resolve_resource_path(std::path::Path::new(&native_db_resource_id))).await {
				Ok(it) => it,
				Err(reason) => {
					match reason { // TODO: handle specific errors
						_ => return Err(LoadResults::CacheFileNotFound),
					}
				}
			};

			match slice {
				Lox::None => {}
				Lox::Buffer(buffer) => {
					if let Err(err) = file.read_exact(buffer).await {
						return Err(LoadResults::LoadFailed);
					}
				}
				Lox::Streams(mut streams) => {
					if let Some(resource_handler) = self.resource_handlers.iter().find(|h| h.can_handle_type(response.class.as_str())) {
						resource_handler.read(&response.resource, &mut file, &mut streams).await;
					} else {
						log::warn!("No resource handler could handle resource: {}", response.url);
					}
				}
			}

			Ok(response)
		}).collect::<Vec<_>>();

		let resources = futures::future::join_all(resources).await.into_iter().collect::<Result<Vec<_>, _>>()?;

		return Ok(Response { resources });
	}

	fn resolve_resource_path(path: &std::path::Path) -> std::path::PathBuf {
		if cfg!(test) {
			std::env::temp_dir().join("resources").join(path)
		} else {
			std::path::PathBuf::from("resources/").join(path)
		}
	}

	fn resolve_asset_path(path: &std::path::Path) -> std::path::PathBuf {
		std::path::PathBuf::from("assets/").join(path)
	}

	/// Loads an asset from source.\
	/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
	/// If the asset is not found it will return None.
	/// ```ignore
	/// let (bytes, format) = ResourceManager::read_asset_from_source("textures/concrete").unwrap(); // Path relative to .../assets
	/// ```
	pub async fn read_asset_from_source(&self, url: &str) -> Result<(Vec<u8>, String), Option<polodb_core::bson::Document>> {
		let resource_origin = if url.starts_with("http://") || url.starts_with("https://") { "network" } else { "local" };
		let mut source_bytes;
		let format;
		match resource_origin {
			"network" => {
				let request = if let Ok(request) = ureq::get(url).call() { request } else { return Err(None); };
				let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return Err(None); };
				format = content_type;

				source_bytes = Vec::new();

				request.into_reader().read_to_end(&mut source_bytes);
			},
			"local" => {
				let path = self.realize_asset_path(url).ok_or(None)?;

				let mut file = smol::fs::File::open(&path).await.unwrap();

				format = path.extension().unwrap().to_str().unwrap().to_string();

				source_bytes = Vec::with_capacity(file.metadata().await.unwrap().len() as usize);

				if let Err(_) = file.read_to_end(&mut source_bytes).await {
					return Err(None);
				}
			},
			_ => {
				// Could not resolve how to get raw resource, return empty bytes
				return Err(None);
			}
		}

		Ok((source_bytes, format))
	}

	pub fn realize_asset_path(&self, url:&str) -> Option<std::path::PathBuf> {
		let path = Self::resolve_asset_path(std::path::Path::new(""));

		let url_as_path = std::path::Path::new(url);

		let url_as_path_parent = url_as_path.parent().or(None)?;

		let path = path.join(url_as_path_parent);

		let path = if let Ok(dir) = std::fs::read_dir(path) {
			let files = dir.filter(|f|
				if let Ok(f) = f {
					let path = if f.path().is_file() { f.path() } else { return false; };
					// Do this to only try loading files that have supported extensions
					// Take this case
					// Suzanne.gltf	Suzanne.bin
					// We want to load Suzanne.gltf and not Suzanne.bin
					let extension = path.extension().unwrap().to_str().unwrap();
					self.resource_handlers.iter().any(|rm| rm.can_handle_type(extension)) && f.path().file_stem().unwrap().eq(url_as_path.file_name().unwrap())
				} else {
					false
				}
			);

			files.last()?.unwrap().path()
		} else { return None; };

		Some(path)
	}

	fn get_url_type(&self, url:&str) -> String {
		let origin = if url.starts_with("http://") || url.starts_with("https://") {
			"network".to_string()
		} else {
			"local".to_string()
		};
	
		match origin.as_str() {
			"network" => {
				let request = if let Ok(request) = ureq::get(url).call() { request } else { return "unknown".to_string(); };
				let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return "unknown".to_string(); };
				content_type
			},
			"local" => {
				let path = self.realize_asset_path(url).unwrap();

				path.extension().unwrap().to_str().unwrap().to_string()
			},
			_ => {
				// Could not resolve how to get raw resource, return empty bytes
				"unknown".to_string()
			}
		}
	}
}

// TODO: test resource caching

#[cfg(test)]
mod tests {
	// TODO: test resource load order

	use super::*;
}