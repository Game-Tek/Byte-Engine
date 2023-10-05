//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

mod image_resource_handler;
pub mod mesh_resource_handler;
pub mod material_resource_handler;

use std::{io::prelude::*, hash::{Hasher, Hash},};

use log::{warn, info, error, trace};
use polodb_core::bson::{Document, doc};

use crate::orchestrator::{System, self};

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

trait ResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool;

	/// Returns a tuple containing the resource description and it's associated binary data.\
	/// 
	/// The returned document is like the following:
	/// ```json
	/// { "class": "X", "resource": { ... }, "hash": 0, "required_resources":[{ "path": "..." }] }
	/// ```
	/// Fields:
	/// - **class**: The resource class. This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// - **resource**: The resource data. Can look like anything.
	/// - **hash**(optional): The resource hash. This is used to identify the resource data. If the resource handler wants to generate a hash for the resource it can do so else the resource manager will generate a hash for it. This is because some resources can generate hashes inteligently (EJ: code generators can output same hash for different looking code if the code is semantically identical).
	/// - **required_resources**(optional): A list of resources that this resource depends on. This is used to load resources that depend on other resources.
	fn process(&self, asset_url: &str, bytes: &[u8]) -> Result<Vec<(Document, Vec<u8>)>, String>;

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>)>;

	fn read(&self, _resource: &Box<dyn std::any::Any>, file: &mut std::fs::File, buffers: &mut [Buffer]) {
		file.read_exact(buffers[0].buffer).unwrap();
	}
}

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
	deserializers: std::collections::HashMap<&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>>,
}

impl orchestrator::Entity for ResourceManager {}
impl System for ResourceManager {}

impl From<polodb_core::Error> for LoadResults {
	fn from(error: polodb_core::Error) -> Self {
		match error {
			_ => LoadResults::LoadFailed
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
	pub	path: String,
	pub size: u64,
	pub hash: u64,
	pub class: String,
	pub resource: Box<dyn std::any::Any>,
	pub required_resources: Vec<String>,
}

pub struct ResourceResponse {
	pub id: u64,
	pub	path: String,
	pub size: u64,
	pub offset: u64,
	pub hash: u64,
	pub class: String,
	pub resource: Box<dyn std::any::Any>,
	pub required_resources: Vec<String>,
}

pub struct Request {
	pub resources: Vec<ResourceRequest>,
}

pub struct Response {
	pub resources: Vec<ResourceResponse>,
}

pub struct Buffer<'a> {
	pub buffer: &'a mut [u8],
	pub tag: String,
}

pub struct OptionResource<'a> {
	pub path: String,
	pub buffers: Vec<Buffer<'a>>,
}

pub struct Options<'a> {
	pub resources: Vec<OptionResource<'a>>,
}

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new() -> Self {
		if let Err(error) = std::fs::create_dir_all("assets") {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create assets directory"),
			}
		}

		if let Err(error) = std::fs::create_dir_all("resources") {
			match error.kind() {
				std::io::ErrorKind::AlreadyExists => {},
				_ => panic!("Could not create resources directory"),
			}
		}

		let mut args = std::env::args();

		let mut memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		if cfg!(test) {
			memory_only = true;
		}

		let db_res = if !memory_only {
			polodb_core::Database::open_file("resources/resources.db")
		} else {
			info!("Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file("assets/resources.db").unwrap();

				warn!("Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file("assets/resources.db");

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

		let resource_handlers: Vec<Box<dyn ResourceHandler + Send>> = vec![
			Box::new(image_resource_handler::ImageResourceHandler::new()),
			Box::new(mesh_resource_handler::MeshResourceHandler::new()),
			Box::new(material_resource_handler::MaterialResourcerHandler::new())
		];

		let mut deserializers = std::collections::HashMap::new();

		for resource_handler in resource_handlers.as_slice() {
			for deserializer in resource_handler.get_deserializers() {
				deserializers.insert(deserializer.0, deserializer.1);
			}
		}

		ResourceManager {
			db,
			resource_handlers,
			deserializers,
		}
	}

	pub fn new_as_system() -> orchestrator::EntityReturn<ResourceManager> {
		orchestrator::EntityReturn::new(Self::new())
	}

	/// Tries to load a resource from cache or source.\
	/// This is a more advanced version of get() as it allows to load resources that depend on other resources.\
	/// 
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	/// If the resource is in cache but it's data cannot be parsed, it will return None.
	/// Return is a tuple containing the resource description and it's associated binary data.\
	/// ```json
	/// { ..., "resources":[{ "id": 01234, "size": 0, "offset": 0, "class": "X" "resource": { ... }, "hash": 0 }] }
	/// ```
	/// Members:
	/// - **id**: u64 - The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
	/// - **size**: u64 - The resource size in bytes.
	/// - **offset**: u64 - The resource offset in the resource bundle, relative to the start of the bundle.
	/// - **class**: String - The resource class. This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// - **resource**: The resource data. Can look like anything.
	/// - **hash**: u64 - The resource hash. This is used to identify the resource data. If the resource handler wants to generate a hash for the resource it can do so else the resource manager will generate a hash for it. This is because some resources can generate hashes inteligently (EJ: code generators can output same hash for different looking code if the code is semantically identical). 
	/// 
	/// The requested resource will always the last one in the array. With the previous resources being the ones it depends on. This way when iterating the array forward the dependencies will be loaded first.
	pub fn get(&mut self, path: &str) -> Option<(Response, Vec<u8>)> {
		let request = self.load_from_cache_or_source(path)?;

		let size = request.resources.iter().map(|r| r.size).sum::<u64>() as usize;

		let mut buffer = Vec::with_capacity(size);

		unsafe { buffer.set_len(size); }

		let response = self.load_data_from_cache(request, None, &mut buffer).ok()?;
		Some((response, buffer))
	}

	/// Tries to load the information/metadata for a resource (and it's dependecies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load_resource()` which will load the binary data.
	pub fn request_resource(&mut self, path: &str) -> Option<Request> {
		let request = self.load_from_cache_or_source(path)?;
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
	pub fn load_resource(&mut self, request: Request, options: Option<Options>, buffer: Option<&mut [u8]>) -> Result<(Response, Option<Vec<u8>>), LoadResults> {
		if let Some(buffer) = buffer {
			let response = self.load_data_from_cache(request, options, buffer)?;
			Ok((response, None))
		} else { 
			let mut buffer = Vec::new();
			let response = self.load_data_from_cache(request, options, &mut buffer)?;
			Ok((response, Some(buffer)))
		}
	}

	/// Tries to load a resource from cache.\
	/// The returned documents is like the following:
	/// ```json
	/// { "_id":"OId" , "id": 01234, "path":"../..", "size": 0, "class": "X", "resource": { ... }, "hash": 0 }
	/// ```
	fn load_from_cache_or_source(&mut self, path: &str) -> Option<Request> {
		fn gather(db: &polodb_core::Database, path: &str) -> Option<Vec<Document>> {
			let doc = db.collection::<Document>("resources").find_one(doc!{ "path": path }).unwrap()?;

			let mut documents = vec![];
			
			if let Some(polodb_core::bson::Bson::Array(required_resources)) = doc.get("required_resources") {
				for required_resource in required_resources {
					if let polodb_core::bson::Bson::Document(required_resource) = required_resource {
						let resource_path = required_resource.get("path").unwrap().as_str().unwrap();
						documents.append(&mut gather(db, resource_path)?);
					};
				}
			}

			documents.push(doc);

			Some(documents)
		}

		let resource_descriptions = if let Some(a) = gather(&self.db, path) {
			a
		} else {
			let r = Self::read_asset_from_source(path).unwrap();

			let mut generated_resources = Vec::new();

			for resource_handler in &self.resource_handlers {
				if resource_handler.can_handle_type(r.1.as_str()) {
					generated_resources.append(&mut resource_handler.process(path, &r.0).unwrap());
				}
			}

			self.write_assets_to_cache(&generated_resources, path)?;

			gather(&self.db, path)?
		};

		for r in &resource_descriptions {
			trace!("Loaded resource: {:#?}", r);
		}

		let request = Request {
			resources: resource_descriptions.iter().map(|r|
				ResourceRequest { 
					_id: r.get_object_id("_id").unwrap().clone(),
					id: r.get_i64("id").unwrap() as u64,
					path: r.get_str("path").unwrap().to_string(),
					size: r.get_i64("size").unwrap() as u64,
					hash: r.get_i64("hash").unwrap() as u64,
					class: r.get_str("class").unwrap().to_string(),
					resource: self.deserializers[r.get_str("class").unwrap()](r.get_document("resource").unwrap()),
					required_resources: if let Ok(rr) = r.get_array("required_resources") { rr.iter().map(|e| e.as_document().unwrap().get_str("path").unwrap().to_string()).collect() } else { vec![] },
				}
			).collect(),
		};

		Some(request)
	}

	/// Stores the asset as a resource.
	/// Returns the resource document.
	fn write_assets_to_cache(&mut self, resource_packages: &[(Document, Vec<u8>)], path: &str) -> Option<()> {
		for resource_package in resource_packages {
			dbg!(&resource_package.0);

			let mut full_resource_document = resource_package.0.clone();

			let mut hasher = std::collections::hash_map::DefaultHasher::new();

			if let Some(polodb_core::bson::Bson::String(p)) = full_resource_document.get("path") {
				p.hash(&mut hasher);
			} else {
				full_resource_document.insert("path", path.to_string());
				path.hash(&mut hasher);
			}

			full_resource_document.insert("id", hasher.finish() as i64);

			full_resource_document.insert("size", resource_package.1.len() as i64);

			if let None = full_resource_document.get("hash") { // TODO: might be a good idea just to generate a random hash, since this method does not reflect changes to the document of the resource
				let mut hasher = std::collections::hash_map::DefaultHasher::new();

				std::hash::Hasher::write(&mut hasher, resource_package.1.as_slice());

				full_resource_document.insert("hash", hasher.finish() as i64);
			}

			let insert_result = self.db.collection::<Document>("resources").insert_one(&full_resource_document).ok()?;

			let resource_id = insert_result.inserted_id.as_object_id()?;

			let resource_path = self.resolve_resource_path(resource_id.to_string().as_str());

			let mut file = std::fs::File::create(resource_path).ok()?;

			file.write_all(resource_package.1.as_slice()).unwrap();
		}

		return Some(());
	}

	/// Tries to load a resource from cache.\
	/// If the resource cannot be found/loaded or if it's become stale it will return None.
	fn load_data_from_cache(&mut self, request: Request, mut options: Option<Options>, buffer: &mut [u8]) -> Result<Response, LoadResults> {
		let mut offset = 0usize;

		let resources = request.resources.into_iter().map(|resource_container| {
			let native_db_resource_id = resource_container._id.to_string();
	
			let mut file = match std::fs::File::open(self.resolve_resource_path(&native_db_resource_id)) {
				Ok(it) => it,
				Err(reason) => {
					match reason { // TODO: handle specific errors
						_ => return Err(LoadResults::CacheFileNotFound),
					}
				}
			};

			let response = ResourceResponse {
				id: resource_container.id,
				path: resource_container.path.clone(),
				size: resource_container.size,
				offset: offset as u64,
				hash: resource_container.hash,
				class: resource_container.class.clone(),
				resource: resource_container.resource,
				required_resources: resource_container.required_resources,
			};

			let _slice = if let Some(options) = &mut options {
				if let Some(x) = options.resources.iter_mut().find(|e| e.path == resource_container.path) {
					self.resource_handlers.iter().find(|h| h.can_handle_type(resource_container.class.as_str())).unwrap().
					read(&response.resource, &mut file, x.buffers.as_mut_slice());
				} else {
					let range = &mut buffer[offset..(offset + resource_container.size as usize)];
					offset += resource_container.size as usize;
					if let Err(_) = file.read_exact(range) { return Err(LoadResults::LoadFailed); }
				}
			} else {
				let range = &mut buffer[offset..(offset + resource_container.size as usize)];
				offset += resource_container.size as usize;
				if let Err(_) = file.read_exact(range) { return Err(LoadResults::LoadFailed); }
			};

			Ok(response)
		}).collect::<Result<Vec<ResourceResponse>, LoadResults>>()?;

		return Ok(Response { resources });
	}

	fn resolve_resource_path(&self, path: &str) -> String { "resources/".to_string() + path	}
	fn resolve_asset_path(&self, path: &str) -> String { "assets/".to_string() + path }

	/// Loads an asset from source.\
	/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
	/// If the asset is not found it will return None.
	/// ```ignore
	/// let (bytes, format) = ResourceManager::read_asset_from_source("textures/concrete").unwrap(); // Path relative to .../assets
	/// ```
	fn read_asset_from_source(path: &str) -> Result<(Vec<u8>, String), Option<Document>> {
		let resource_origin = if path.starts_with("http://") || path.starts_with("https://") { "network" } else { "local" };
		let mut source_bytes;
		let format;
		match resource_origin {
			"network" => {
				let request = if let Ok(request) = ureq::get(path).call() { request } else { return Err(None); };
				let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return Err(None); };
				format = content_type;

				source_bytes = Vec::new();

				request.into_reader().read_to_end(&mut source_bytes);
			},
			"local" => {
				let (mut file, extension) = if let Ok(dir) = std::fs::read_dir("assets") {
					let files = dir.filter(|f| if let Ok(f) = f { f.path().to_str().unwrap().contains(path) } else { false });
					let file_path = files.last().unwrap().unwrap().path();
					(std::fs::File::open(&file_path).unwrap(), file_path.extension().unwrap().to_str().unwrap().to_string())
				} else { return Err(None); };

				format = extension.to_string();

				source_bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);

				if let Err(_) = file.read_to_end(&mut source_bytes) {
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
}

// TODO: test resource caching

#[cfg(test)]
mod tests {
	use crate::resource_manager::{image_resource_handler::Texture, mesh_resource_handler::{Mesh, IntegralTypes, VertexSemantics, Size}};

	/// Tests for the resource manager.
	/// It is important to test the load twice as the first time it will be loaded from source and the second time it will be loaded from cache.

	use super::*;

	#[test]
	fn load_net_image() {
		std::env::set_var("--ResourceManager.memory_only", "true"); // Don't use file database

		let mut resource_manager = ResourceManager::new();

		// Test loading from source

		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let (request, _buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 1);

		let resource_container = &request.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Texture>());

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });

		// Test loading from cache

		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let (request, _buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 1);

		let resource_container = &request.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Texture>());

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });
	}

	#[ignore]
	#[test]
	fn load_local_image() {
		std::env::set_var("--ResourceManager.memory_only", "true"); // Don't use file database

		let mut resource_manager = ResourceManager::new();

		let resource_result = resource_manager.get("test");

		assert!(resource_result.is_some());

		let (request, _buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 1);

		let resource_container = &request.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Texture>());

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert!(texture_info.extent.width == 4096 && texture_info.extent.height == 1024 && texture_info.extent.depth == 1);
	}

	#[test]
	fn load_local_mesh() {
		std::env::set_var("--ResourceManager.memory_only", "true"); // Don't use file database

		let mut resource_manager = ResourceManager::new();

		// Test loading from source

		let resource_result = resource_manager.get("Box");

		assert!(resource_result.is_some());

		let (request, buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 1);

		let resource_container = &request.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		assert_eq!(buffer.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		let mesh = resource.downcast_ref::<Mesh>().unwrap();
		
		assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(mesh.vertex_count, 24);
		assert_eq!(mesh.index_count, 36);
		assert_eq!(mesh.index_type, IntegralTypes::U16);
		assert_eq!(mesh.vertex_components.len(), 2);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);

		// Test loading from cache

		let resource_result = resource_manager.get("Box");

		assert!(resource_result.is_some());

		let (_resource, mesh_buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 1);

		let resource_container = &request.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		assert_eq!(buffer.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(mesh.vertex_count, 24);
		assert_eq!(mesh.index_count, 36);
		assert_eq!(mesh.index_type, IntegralTypes::U16);
		assert_eq!(mesh.vertex_components.len(), 2);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);

		let resource_request = resource_manager.request_resource("Box");

		let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

		let mut options = Options { resources: Vec::new(), };

		let mut vertex_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = &resource_request.resources[0];

		match resource.class.as_str() {
			"Mesh" => {
				options.resources.push(OptionResource {
					path: resource.path.clone(),
					buffers: vec![Buffer{ buffer: vertex_buffer.as_mut_slice(), tag: "Vertex".to_string() }, Buffer{ buffer: index_buffer.as_mut_slice(), tag: "Index".to_string() }],
				});
			}
			_ => {}
		}

		let resource = if let Ok(a) = resource_manager.load_resource(resource_request, Some(options), None) { a } else { return; };

		let (response, _buffer) = (resource.0, resource.1.unwrap());

		for resource in &response.resources {
			match resource.class.as_str() {
				"Mesh" => {
					let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

					assert_eq!(mesh_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize], vertex_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize]);

					assert_eq!(mesh_buffer[576..(576 + mesh.index_count * 2) as usize], index_buffer[0..(mesh.index_count * 2) as usize]);
				}
				_ => {}
			}
		}
	}

	#[test]
	fn load_material() {
		std::env::set_var("--ResourceManager.memory_only", "true"); // Don't use file database

		let mut resource_manager = ResourceManager::new();

		// Test loading from source

		let resource_result = resource_manager.get("cube");

		assert!(resource_result.is_some());

		let (request, _buffer) = resource_result.unwrap();

		assert_eq!(request.resources.len(), 3); // 1 material, 2 shaders

		let resource_container = &request.resources[0];

		assert_eq!(resource_container.class, "Shader");

		let id = resource_container.id;

		let resource_container = &request.resources[1];

		assert_eq!(resource_container.class, "Shader");
		assert_ne!(resource_container.id, id);

		let id = resource_container.id;

		let resource_container = &request.resources[2];

		assert_eq!(resource_container.class, "Material");
		assert_ne!(resource_container.id, id);
	}
}