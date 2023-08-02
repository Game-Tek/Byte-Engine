//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

mod image_resource_handler;
mod mesh_resource_handler;
mod material_resource_handler;

use std::{io::prelude::*, sync::Arc};

use polodb_core::bson::{Document, doc};

use crate::orchestrator::{System, self};

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

trait ResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool;
	fn process(&self, bytes: Vec<u8>) -> Result<(Document, Vec<u8>), String>;
}

/// Resource manager.
/// Handles loading assets or resources from different origins (network, local, etc.).
/// It also handles caching of resources.
/// 
/// When in a debug build it will lazily load resources from source and cache them.
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
		std::fs::create_dir_all("assets").unwrap();

		let mut args = std::env::args();

		let memory_only = args.find(|arg| arg == "--ResourceManager.memory_only").is_some();

		let db_res = if !memory_only {
			polodb_core::Database::open_file("assets/resources.db")
		} else {
			println!("\x1B[WARNING]Using memory database instead of file database.");
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

	fn resolve_resource_path(&self, path: &str) -> String { "resources/".to_string() + path	}
	fn resolve_asset_path(&self, path: &str) -> String { "assets/".to_string() + path }

	fn get_document_from_cache(&mut self, path: &str) -> Option<Document> {
		self.db.collection::<Document>("resources").find_one(doc!{ "id": path }).unwrap()
	}

	fn load_resource_into_cache(&mut self, path: &str) -> Option<Document> {
		let resource_origin = if path.starts_with("http://") || path.starts_with("https://") { "network" } else { "local" };

		let mut source_bytes;
		let format;

		match resource_origin {
			"network" => {
				let request = if let Ok(request) = ureq::get(path).call() { request } else { return None; };
				let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return None; };
				format = content_type;

				source_bytes = Vec::new();

				request.into_reader().read_to_end(&mut source_bytes);
			},
			"local" => {
				let files = if let Ok(r) = std::fs::read_dir("resources") { r } else { return None; };
				let files = files.filter(|f| if let Ok(f) = f { f.path().to_str().unwrap().starts_with(path) } else { false });
				let p = files.last().unwrap().unwrap().path();

				let mut file = std::fs::File::open(&p).unwrap();
				let extension = p.extension().unwrap().to_str().unwrap();

				format = extension.to_string();

				source_bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);

				if let Err(_) = file.read_to_end(&mut source_bytes) {
					return None;
				}
			},
			_ => {
				// Could not resolve how to get raw resource, return empty bytes
				return None;
			}
		}

		let mut bytes = None;

		for handler in &self.resource_handlers {
			if handler.can_handle_type(&format) {
				if let Ok(e) = handler.process(source_bytes) {
					bytes = Some(e);
					break;
				} else {
					return None;
				}
			}
		};

		let (resource_document, bytes) = if let Some(bytes) = bytes { bytes } else { return None; };

		let id = path.to_string();

		let document = doc! {
			"id": id,
			"resource": resource_document
		};

		let insert_result = if let Ok(insert_result) = self.db.collection::<Document>("resources").insert_one(&document) {
			insert_result
		} else {
			return None;
		};

		let resource_id = insert_result.inserted_id.as_object_id().unwrap();

		let asset_path = self.resolve_asset_path(resource_id.to_string().as_str());

		let mut file = if let Ok(file) = std::fs::File::create(asset_path) { file } else { return None; };

		file.write_all(bytes.as_slice()).unwrap();

		return Some(document);
	}

	fn load_data_from_cache(&mut self, path: &str) -> Result<Vec<u8>, LoadResults> {
		let result = self.db.collection::<Document>("resources").find_one(doc!{ "id": path })?;

		if let Some(resource) = result {
			let native_db_resource_id =	if let Some(polodb_core::bson::Bson::ObjectId(id)) = resource.get("_id") {
				id
			} else {
				return Err(LoadResults::LoadFailed);
			};

			let native_db_resource_id = native_db_resource_id.to_string();

			let mut file = match std::fs::File::open("assets/".to_string() + native_db_resource_id.as_str()) {
				Ok(it) => it,
				Err(reason) => {
					match reason { // TODO: handle specific errors
						_ => return Err(LoadResults::CacheFileNotFound { document: resource }),
					}
				}
			};

			let mut bytes = Vec::new();

			let res = file.read_to_end(&mut bytes);

			if res.is_err() { return Err(LoadResults::LoadFailed); }

			return Ok(bytes);
		}

		return Err(LoadResults::ResourceNotFound);
	}

	/// Tries to load a resource from cache or source.\
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	/// If the resource is in cache but it's data cannot be parsed, it will return None.
	pub fn get(&mut self, path: &str) -> Option<(Document, Vec<u8>)> {
		let document = if let Some(r) = self.get_document_from_cache(path) {
			r
		} else {
			if let Some(r) = self.load_resource_into_cache(path) {
				r
			} else {
				return None;
			}
		};

		let data = self.load_data_from_cache(path);

		if data.is_err() {
			return None;
		}

		let data = data.unwrap();

		return Some((document, data));
	}

	pub fn get_resource_info<'a, T: serde::Deserialize<'a>>(&mut self, path: &str) -> Option<Request<T>> {
		let document = if let Some(r) = self.get_document_from_cache(path) {
			r
		} else {
			if let Some(r) = self.load_resource_into_cache(path) {
				r
			} else {
				return None;
			}
		};

		let resource: T = T::deserialize(polodb_core::bson::Deserializer::new(document.clone().into())).unwrap();

		return Some(Request { document, resource, id: path.to_string() });
	}

	pub fn load_resource_into_buffer<'a>(&mut self, request: &Request<mesh_resource_handler::Mesh>, vertex_buffer: &mut [u8], index_buffer: &mut [u8]) {
		let _id = request.document.get("_id").unwrap().as_object_id().unwrap().to_string();
		let mut file = std::fs::File::open("assets/".to_string() + _id.as_str()).unwrap();

		let mesh_info =& request.resource;

		use mesh_resource_handler::Size;

		let vertex_size = mesh_info.vertex_components.size();

		file.read(&mut vertex_buffer[..(vertex_size * mesh_info.vertex_count as usize)]).unwrap();
		file.seek(std::io::SeekFrom::Start((vertex_size * mesh_info.vertex_count as usize).next_multiple_of(16) as u64)).unwrap();
		file.read(&mut index_buffer[..(mesh_info.index_count as usize * mesh_info.index_type.size())]).unwrap();
	}
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