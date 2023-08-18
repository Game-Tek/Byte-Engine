//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

mod image_resource_handler;
mod mesh_resource_handler;
mod material_resource_handler;

use std::{io::prelude::*, str::FromStr, hash::{Hasher, Hash},};

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
	fn process(&self, bytes: Vec<u8>) -> Result<Vec<(Document, Vec<u8>)>, String>;
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

#[derive(Debug)]
enum LoadResults {
	ResourceNotFound,
	LoadFailed,
	CacheFileNotFound {
		document: polodb_core::bson::Document,
	},
	UnsuportedResourceType,
}

fn extent_from_json(field: &polodb_core::bson::Bson) -> Option<crate::Extent> {
	match field {
		polodb_core::bson::Bson::Array(array) => {
			let mut extent = crate::Extent {
				width: 1,
				height: 1,
				depth: 1,
			};

			for (index, field) in array.iter().enumerate() {
				match index {
					0 => extent.width = field.as_i32().unwrap() as u32,
					1 => extent.height = field.as_i32().unwrap() as u32,
					2 => extent.depth = field.as_i32().unwrap() as u32,
					_ => panic!("Invalid extent field"),
				}
			}

			return Some(extent);
		},
		_ => return None,
	}
}

fn vec3f_from_json(field: &polodb_core::bson::Bson) -> Option<[f32; 3]> {
	match field {
		polodb_core::bson::Bson::Array(array) => {
			if let Some(polodb_core::bson::Bson::Double(x)) = array.get(0) {
				if let Some(polodb_core::bson::Bson::Double(y)) = array.get(1) {
					if let Some(polodb_core::bson::Bson::Double(z)) = array.get(2) {
						return Some([*x as f32, *y as f32, *z as f32]);
					} else {
						return None;
					}
				} else {
					return None;
				}
			} else {
				return None;
			}
		},
		_ => return None,
	}
}

pub struct Request<T> {
	id: String,
	document: Document,
	pub resource: T,
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
				_ => panic!("Could not create assets directory"),
			}
		}

		let mut args = std::env::args();

		let memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		let db_res = if !memory_only {
			polodb_core::Database::open_file("resources/resources.db")
		} else {
			println!("\x1B[INFO]Using memory database instead of file database.");
			polodb_core::Database::open_memory()
		};

		let db = match db_res {
			Ok(db) => db,
			Err(_) => {
				// Delete file and try again
				std::fs::remove_file("assets/resources.db").unwrap();

				println!("\x1B[WARNING]Database file was corrupted, deleting and trying again.");

				let db_res = polodb_core::Database::open_file("assets/resources.db");

				match db_res {
					Ok(db) => db,
					Err(_) => match polodb_core::Database::open_memory() { // If we can't create a file database, create a memory database. This way we can still run the application.
						Ok(db) => {
							println!("\x1B[WARNING]Could not create database file, using memory database instead.");
							db
						},
						Err(_) => panic!("Could not create database"),
					}
				}
			}
		};

		let resource_handlers: Vec<Box<dyn ResourceHandler + Send>> = vec![Box::new(image_resource_handler::ImageResourceHandler::new()), Box::new(mesh_resource_handler::MeshResourceHandler::new()), Box::new(material_resource_handler::MaterialResourcerHandler::new())];

		ResourceManager {
			db,
			resource_handlers,
		}
	}

	pub fn new_as_system(orchestrator: orchestrator::OrchestratorReference) -> Self {
		Self::new()
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
	pub fn get(&mut self, path: &str) -> Option<(Document, Vec<u8>)> {
		let resource_description = if let Some(r) = self.get_document_from_cache(path) {
			r
		} else {
			if let Some(r) = self.load_asset_from_source_and_cache_it(path) { r } else { return None; }
		};

		let mut buffer = Vec::with_capacity(8192);

		let mut bundle_description = doc!{ "resources": [] };

		if let Some(polodb_core::bson::Bson::Array(required_resources)) = resource_description.get("required_resources") {
			let mut native_id_array = polodb_core::bson::Array::new();
			let mut path_array = polodb_core::bson::Array::new();

			for required_resource in required_resources {
				let required_resource = if let polodb_core::bson::Bson::Document(required_resource) = required_resource { required_resource } else { break; };

				if let Some(polodb_core::bson::Bson::String(resource_path)) = required_resource.get("path") {
					native_id_array.push(polodb_core::bson::oid::ObjectId::from_str(resource_path).unwrap().into());
				} else if let Some(polodb_core::bson::Bson::String(required_resource_path)) = required_resource.get("path") {
					path_array.push(required_resource_path.into());
				} else {
					panic!("Invalid required resource");	
				}
			}

			let search_doc = polodb_core::bson::doc! {
				"$or": [
					{ "_id": { "$in": native_id_array } },
					{ "path": { "$in": path_array } }
				]
			};

			// TODO: make recursive
			let required_resources = self.db.collection::<Document>("resources").find(search_doc).unwrap().map(|e| e.unwrap()).collect::<Vec<_>>();

			for required_resource in &required_resources {
				let size = required_resource.get_i64("size").unwrap();
				let offset = buffer.len() as i64;

				bundle_description.get_array_mut("resources").unwrap().push(doc!{ "size": size, "offset": offset, "resource": required_resource.get("resource").unwrap() }.into());

				self.load_data_from_cache(&required_resource, &mut buffer).unwrap();
			}
		}

		let data = self.load_data_from_cache(&resource_description, &mut buffer);

		if data.is_err() {
			return None;
		}

		return Some((resource_description, buffer));
	}

	/// Tries to load the information/metadata for a resource (and it's dependecies).\
	/// This is a more advanced version of get() as it allows to use your own buffer and/or apply some transformation to the resources when loading.\
	/// The result of this function can be later fed into `load_resource()` which will load the binary data.
	pub fn request_resource(&mut self, path: &str) -> Option<Document> {
		let resource_description = self.get_document_from_cache(path).or_else(|| self.load_asset_from_source_and_cache_it(path))?;
		Some(resource_description)
	}

	/// Loads the resource binary data from cache.\
	/// If a buffer range is provided it will load the data into the buffer.\
	/// If no buffer range is provided it will return the data in a vector.
	pub fn load_resource(&self, resource: &Document, options: Option<Document>, buffer: Option<&mut [u8]>) -> Result<Option<Vec<u8>>, LoadResults> {
		if let Some(buffer) = buffer {
			self.load_data_from_cache(resource, buffer);
			Ok(None)
		} else { 
			let mut b = Vec::new();

			self.load_data_from_cache(resource, &mut b);

			Ok(Some(b))
		}

		//let resource: T = T::deserialize(polodb_core::bson::Deserializer::new(document.get("resource").unwrap().into())).unwrap();
	}

	fn resolve_resource_path(&self, path: &str) -> String { "resources/".to_string() + path	}
	fn resolve_asset_path(&self, path: &str) -> String { "assets/".to_string() + path }

	/// Tries to load a resource from cache.\
	/// The returned documents is like the following:
	/// ```json
	/// { "_id":"OId" , "id": 01234, "path":"../..", "size": 0, "class": "X", "resource": { ... }, "hash": 0 }
	/// ```
	fn get_document_from_cache(&mut self, path: &str) -> Option<Document> {
		self.db.collection::<Document>("resources").find_one(doc!{ "path": path }).unwrap()
	}

	/// Tries to load a resource from source and cache it.\
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.
	fn load_asset_from_source_and_cache_it(&mut self, path: &str) -> Option<Document> {
		let (source_bytes, format) = match get_asset_from_source(path) {
			Ok(value) => value,
			Err(value) => return value,
		};

		let mut resource_packages = None;

		for handler in &self.resource_handlers {
			if handler.can_handle_type(&format) {
				if let Ok(e) = handler.process(source_bytes) {
					resource_packages = Some(e);
					break;
				} else {
					return None;
				}
			}
		};

		return self.cache_resources(resource_packages?, path);
	}

	/// Stores the asset as a resource.
	/// Returns the resource document.
	fn cache_resources(&mut self, resource_packages: Vec<(Document, Vec<u8>)>, path: &str) -> Option<Document> {
		let mut document = None;

		for resource_package in resource_packages {
			let mut full_resource_document = resource_package.0;

			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			path.hash(&mut hasher);

			full_resource_document.insert("id", hasher.finish() as i64);
			full_resource_document.insert("path", path.to_string());
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

			document = Some(self.db.collection::<Document>("resources").find_one(doc!{ "_id": resource_id }).unwrap().unwrap());
		}

		return document;
	}

	/// Tries to load a resource from cache.\
	/// If the resource cannot be found/loaded or if it's become stale it will return None.
	fn load_data_from_cache(&mut self, resource_document: &polodb_core::bson::Document, buffer: &mut [u8]) -> Result<(), LoadResults> {
		let native_db_resource_id =	if let Some(polodb_core::bson::Bson::ObjectId(oid)) = resource_document.get("_id") { oid } else { return Err(LoadResults::LoadFailed); };

		let native_db_resource_id = native_db_resource_id.to_string();

		let mut file = match std::fs::File::open(self.resolve_asset_path(&native_db_resource_id)) {
			Ok(it) => it,
			Err(reason) => {
				match reason { // TODO: handle specific errors
					_ => return Err(LoadResults::CacheFileNotFound { document: resource_document.clone() }),
				}
			}
		};

		let res = file.read_exact(buffer);

		if res.is_err() { return Err(LoadResults::LoadFailed); }

		return Ok(());
	}
}

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
/// ```rust
/// let (bytes, format) = resource_manager::get_asset_from_source("textures/concrete").unwrap(); // Path relative to .../assets
/// ```
fn get_asset_from_source(path: &str) -> Result<(Vec<u8>, String), Option<Document>> {
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
			let files = if let Ok(r) = std::fs::read_dir("assets") { r } else { return Err(None); };
			let files = files.filter(|f| if let Ok(f) = f { f.path().to_str().unwrap().contains(path) } else { false });
			let p = files.last().unwrap().unwrap().path();

			let (mut file, extension) = if let Ok(dir) = std::fs::read_dir("assets") {
				let files = dir.filter(|f| if let Ok(f) = f { f.path().to_str().unwrap().contains(path) } else { false });
				let file_path = files.last().unwrap().unwrap().path();
				(std::fs::File::open(&p).unwrap(), p.extension().unwrap().to_str().unwrap().to_string())
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

// TODO: test resource caching

// #[cfg(test)]
// mod tests {
// 	/// Tests for the resource manager.
// 	/// It is important to test the load twice as the first time it will be loaded from source and the second time it will be loaded from cache.

// 	use super::*;

// 	#[test]
// 	fn load_net_image() {
// 		let mut resource_manager = ResourceManager::new();

// 		// Test loading from source

// 		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

// 		assert!(resource_result.is_some());

// 		let resource = resource_result.unwrap();

// 		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

// 		let texture_info = match &resource.0 {
// 			ResourceContainer::Texture(texture) => texture,
// 			_ => panic!("")
// 		};

// 		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });

// 		// Test loading from cache

// 		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

// 		assert!(resource_result.is_some());

// 		let resource = resource_result.unwrap();

// 		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

// 		let texture_info = match &resource.0 {
// 			ResourceContainer::Texture(texture) => texture,
// 			_ => panic!("")
// 		};

// 		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });
// 	}

// 	#[ignore]
// 	#[test]
// 	fn load_local_image() {
// 		let mut resource_manager = ResourceManager::new();

// 		let resource_result = resource_manager.get("test.png");

// 		assert!(resource_result.is_some());

// 		let resource = resource_result.unwrap();

// 		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

// 		let texture_info = match &resource.0 {
// 			ResourceContainer::Texture(texture) => texture,
// 			_ => panic!("")
// 		};

// 		assert!(texture_info.extent.width == 4096 && texture_info.extent.height == 1024 && texture_info.extent.depth == 1);
// 	}

// 	#[test]
// 	fn load_local_mesh() {
// 		let mut resource_manager = ResourceManager::new();

// 		// Test loading from source

// 		let resource_result = resource_manager.get("Box.gltf");

// 		assert!(resource_result.is_some());

// 		let resource = resource_result.unwrap();

// 		assert!(matches!(resource.0, ResourceContainer::Mesh(_)));

// 		dbg!(&resource.0);

// 		let bytes = &resource.1;

// 		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

// 		if let ResourceContainer::Mesh(mesh) = &resource.0 {
// 			assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
// 			assert_eq!(mesh.vertex_count, 24);
// 			assert_eq!(mesh.index_count, 36);
// 			assert_eq!(mesh.index_type, IntegralTypes::U16);
// 			assert_eq!(mesh.vertex_components.len(), 2);
// 			assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
// 			assert_eq!(mesh.vertex_components[0].format, "vec3f");
// 			assert_eq!(mesh.vertex_components[0].channel, 0);
// 			assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
// 			assert_eq!(mesh.vertex_components[1].format, "vec3f");
// 			assert_eq!(mesh.vertex_components[1].channel, 1);
// 		}

// 		// Test loading from cache

// 		let resource_result = resource_manager.get("Box.gltf");

// 		assert!(resource_result.is_some());

// 		let resource = resource_result.unwrap();

// 		assert!(matches!(resource.0, ResourceContainer::Mesh(_)));

// 		let bytes = &resource.1;

// 		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

// 		if let ResourceContainer::Mesh(mesh) = &resource.0 {
// 			assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
// 			assert_eq!(mesh.vertex_count, 24);
// 			assert_eq!(mesh.index_count, 36);
// 			assert_eq!(mesh.index_type, IntegralTypes::U16);
// 			assert_eq!(mesh.vertex_components.len(), 2);
// 			assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
// 			assert_eq!(mesh.vertex_components[0].format, "vec3f");
// 			assert_eq!(mesh.vertex_components[0].channel, 0);
// 			assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
// 			assert_eq!(mesh.vertex_components[1].format, "vec3f");
// 			assert_eq!(mesh.vertex_components[1].channel, 1);
// 		}
// 	}
// }