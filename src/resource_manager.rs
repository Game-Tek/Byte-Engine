//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

use std::{io::prelude::*};

use polodb_core::bson::{Document, bson, doc};
use serde::{Serialize, Deserialize};

use crate::orchestrator::{System, self};

#[derive(Debug, Serialize, Deserialize)]
pub struct Texture {
	pub compression: String,
	pub extent: crate::Extent,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	Uv,
	Color,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum IntegralTypes {
	U8,
	I8,
	U16,
	I16,
	U32,
	I32,
	F16,
	F32,
	F64,
}

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

#[derive(Debug, Serialize, Deserialize)]
pub struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Mesh {
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_components: Vec<VertexComponent>,
	pub index_type: IntegralTypes,
	pub vertex_count: u32,
	pub index_count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ResourceContainer {
	Texture(Texture),
	Mesh(Mesh),
}

#[derive(Debug, Serialize, Deserialize)]
struct Resource {
	id: String,
	resource: ResourceContainer,
}

trait Size {
	fn size(&self) -> usize;
}

impl Size for VertexSemantics {
	fn size(&self) -> usize {
		match self {
			VertexSemantics::Position => 3 * 4,
			VertexSemantics::Normal => 3 * 4,
			VertexSemantics::Tangent => 3 * 4,
			VertexSemantics::BiTangent => 3 * 4,
			VertexSemantics::Uv => 2 * 4,
			VertexSemantics::Color => 4 * 4,
		}
	}
}

impl Size for Vec<VertexComponent> {
	fn size(&self) -> usize {
		let mut size = 0;

		for component in self {
			size += component.semantic.size();
		}

		size
	}
}

impl Size for IntegralTypes {
	fn size(&self) -> usize {
		match self {
			IntegralTypes::U8 => 1,
			IntegralTypes::I8 => 1,
			IntegralTypes::U16 => 2,
			IntegralTypes::I16 => 2,
			IntegralTypes::U32 => 4,
			IntegralTypes::I32 => 4,
			IntegralTypes::F16 => 2,
			IntegralTypes::F32 => 4,
			IntegralTypes::F64 => 8,
		}
	}
}

// fn qtangent(normal: Vector3<f32>, tangent: Vector3<f32>, bi_tangent: Vector3<f32>) -> Quaternion<f32> {
// 	let tbn: Matrix3<f32> = Matrix3::from_cols(normal, tangent, bi_tangent);

// 	let mut qTangent = Quaternion::from(tbn);
// 	//qTangent.normalise();
	
// 	//Make sure QTangent is always positive
// 	if qTangent.s < 0f32 {
// 		qTangent = qTangent.conjugate();
// 	}
	
// 	//Bias = 1 / [2^(bits-1) - 1]
// 	const bias: f32 = 1.0f32 / 32767.0f32;
	
// 	//Because '-0' sign information is lost when using integers,
// 	//we need to apply a "bias"; while making sure the Quatenion
// 	//stays normalized.
// 	// ** Also our shaders assume qTangent.w is never 0. **
// 	if qTangent.s < bias {
// 		let normFactor = f32::sqrt(1f32 - bias * bias);
// 		qTangent.s = bias;
// 		qTangent.v.x *= normFactor;
// 		qTangent.v.y *= normFactor;
// 		qTangent.v.z *= normFactor;
// 	}
	
// 	//If it's reflected, then make sure .w is negative.
// 	let naturalBinormal = tangent.cross(normal);

// 	if cgmath::dot(naturalBinormal, bi_tangent/* check if should be binormal */) <= 0f32 {
// 		qTangent = -qTangent;
// 	}

// 	qTangent
// }

/// Resource manager.
/// Handles loading assets or resources from different origins (network, local, etc.).
/// It also handles caching of resources.
/// 
/// When in a debug build it will lazily load resources from source and cache them.
pub struct ResourceManager {
	db: polodb_core::Database,
}

impl orchestrator::Entity for ResourceManager {}
impl System for ResourceManager {}

fn extension_to_file_type(extension: &str) -> &str {
	match extension {
		"png" => "texture",
		"gltf" => "mesh",
		_ => ""
	}
}

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

pub struct Request {
	pub resource: ResourceContainer,
	id: String,
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

		ResourceManager {
			db,
		}
	}

	pub fn new_as_system(orchestrator: orchestrator::OrchestratorReference) -> ResourceManager {
		Self::new()
	}

	fn resolve_resource_path(&mut self, path: &str) -> String {
		return "resources/".to_string() + path;
	}

	fn get_resource_from_cache(&mut self, path: &str) -> Option<Resource> {
		self.db.collection::<Resource>("resources").find_one(doc!{ "id": path }).unwrap()
	}

	fn get_document_from_cache(&mut self, path: &str) -> Option<Document> {
		self.db.collection::<Document>("resources").find_one(doc!{ "id": path }).unwrap()
	}

	fn load_resource_into_cache(&mut self, path: &str) -> Option<Resource> {
		let resource_origin = if path.starts_with("http://") || path.starts_with("https://") { "network" } else { "local" };

		// Bytes to be stored associated to resource
		let mut bytes;
		let resource_type;
		let format;

		match resource_origin {
			"network" => {
				let request = if let Ok(request) = ureq::get(path).call() { request } else { return None; };

				let content_type = request.header("content-type").unwrap();

				match content_type {
					"image/png" => {
						let mut data = request.into_reader();
						bytes = Vec::with_capacity(4096 * 1024 * 4);
						let res = data.read_to_end(&mut bytes);

						if let Err(_) = res { return None; }

						resource_type = "texture";
						format = "png";
					},
					_ => {
						// Could not resolve how to get raw resource, return empty bytes
						return None;
					}
				}
				
			},
			"local" => {
				let resolved_path = self.resolve_resource_path(path);
				let mut file = std::fs::File::open(resolved_path).unwrap();
				let extension = &path[(path.rfind(".").unwrap() + 1)..];
				resource_type = extension_to_file_type(extension);
				format = extension;
				bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);
				let res = file.read_to_end(&mut bytes);

				if let Err(_) = res {
					return None;
				}
			},
			_ => {
				// Could not resolve how to get raw resource, return empty bytes
				return None;
			}
		}

		let resource: ResourceContainer;

		match format {
			"png" => {
				let mut decoder = png::Decoder::new(bytes.as_slice());
				decoder.set_transformations(png::Transformations::EXPAND);
				let mut reader = decoder.read_info().unwrap();
				let mut buffer = vec![0; reader.output_buffer_size()];
				let info = reader.next_frame(&mut buffer).unwrap();

				let extent = crate::Extent { width: info.width, height: info.height, depth: 1, };

				resource = ResourceContainer::Texture(Texture { compression: "".to_string(), extent });

				let mut buf: Vec<u8> = Vec::with_capacity(extent.width as usize * extent.height as usize * 4);

				// convert rgb to rgba
				for x in 0..extent.width {
					for y in 0..extent.height {
						let index = ((x + y * extent.width) * 3) as usize;
						buf.push(buffer[index]);
						buf.push(buffer[index + 1]);
						buf.push(buffer[index + 2]);
						buf.push(255);
					}
				}

				bytes = buf;
			},
			"gltf" => {
				let (gltf, buffers, _) = gltf::import_slice(bytes.as_slice()).unwrap();

				let mut buf: Vec<u8> = Vec::with_capacity(4096 * 1024 * 3);

				// 'mesh_loop: for mesh in gltf.meshes() {					
				// 	for primitive in mesh.primitives() {
				
				let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();

				resource = {
					let mut vertex_components = Vec::new();
					let index_type;
					let bounding_box: [[f32; 3]; 2];
					let vertex_count = primitive.attributes().next().unwrap().1.count() as u32;
					let index_count = primitive.indices().unwrap().count() as u32;

					let bounds = primitive.bounding_box();

					bounding_box = [
						[bounds.min[0], bounds.min[1], bounds.min[2],],
						[bounds.max[0], bounds.max[1], bounds.max[2],],
					];

					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

					if let Some(positions) = reader.read_positions() {
						positions.for_each(|position| position.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buf.push(*byte))));
						vertex_components.push(VertexComponent { semantic: VertexSemantics::Position, format: "vec3f".to_string(), channel: 0 });
					} else {
						return None;
					}

					if let Some(normals) = reader.read_normals() {
						normals.for_each(|normal| normal.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buf.push(*byte))));
						vertex_components.push(VertexComponent { semantic: VertexSemantics::Normal, format: "vec3f".to_string(), channel: 1 });
					}

					if let Some(tangents) = reader.read_tangents() {
						tangents.for_each(|tangent| tangent.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buf.push(*byte))));
						vertex_components.push(VertexComponent { semantic: VertexSemantics::Tangent, format: "vec3f".to_string(), channel: 2 });
					}

					if let Some(uv) = reader.read_tex_coords(0) {
						uv.into_f32().for_each(|uv| uv.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buf.push(*byte))));
						vertex_components.push(VertexComponent { semantic: VertexSemantics::Uv, format: "vec3f".to_string(), channel: 3 });
					}

					// align buffer to 16 bytes for indices
					while buf.len() % 16 != 0 { buf.push(0); }

					if let Some(indices) = reader.read_indices() {
						match indices {
							gltf::mesh::util::ReadIndices::U8(indices) => {
								indices.for_each(|index| index.to_le_bytes().iter().for_each(|byte| buf.push(*byte)));
								index_type = IntegralTypes::U8;
							},
							gltf::mesh::util::ReadIndices::U16(indices) => {
								indices.for_each(|index| index.to_le_bytes().iter().for_each(|byte| buf.push(*byte)));
								index_type = IntegralTypes::U16;
							},
							gltf::mesh::util::ReadIndices::U32(indices) => {
								indices.for_each(|index| index.to_le_bytes().iter().for_each(|byte| buf.push(*byte)));
								index_type = IntegralTypes::U32;
							},
						}
					} else {
						return None;
					}

					bytes = buf;
	
					ResourceContainer::Mesh(Mesh { bounding_box, vertex_components, index_type, vertex_count, index_count })
				};
			}
			_ => { return None; }
		}

		match resource_type {
			"texture" => {
				let extent = if let ResourceContainer::Texture(texture) = &resource { texture.extent } else { return None; };

				assert_eq!(extent.depth, 1); // TODO: support 3D textures

				let rgba_surface = intel_tex_2::RgbaSurface {
					data: bytes.as_slice(),
					width: extent.width,
					height: extent.height,
					stride: extent.width * 4,
				};

				let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

				bytes = intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface);
			},
			"mesh" => {
				let mesh = if let ResourceContainer::Mesh(mesh) = &resource { mesh } else { return None; };
			}
			_ => { return None; }
		}

		let resource = Resource{ id: path.to_string(), resource };

		let insert_result = if let Ok(insert_result) = self.db.collection::<Resource>("resources").insert_one(&resource) {
			insert_result
		} else {
			return None;
		};

		let resource_id = insert_result.inserted_id.as_object_id().unwrap();

		let path = "assets/".to_string() + resource_id.to_string().as_str();

		// Write file with resource contents
		let mut file = std::fs::File::create(path).unwrap();

		file.write_all(bytes.as_slice()).unwrap();

		return Some(resource);
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
	pub fn get(&mut self, path: &str) -> Option<(ResourceContainer, Vec<u8>)> {
		let doc = self.get_resource_from_cache(path);

		let resource = if let Some(r) = doc {
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

		return Some((resource.resource, data));
	}

	pub fn get_resource_info(&mut self, path: &str) -> Option<Request> {
		let doc = self.get_resource_from_cache(path);

		let resource = if let Some(r) = doc {
			r
		} else {
			if let Some(r) = self.load_resource_into_cache(path) {
				r
			} else {
				return None;
			}
		};

		return Some(Request { resource: resource.resource, id: path.to_string() });
	}

	pub fn load_resource_into_buffer<'a>(&mut self, request: &Request, vertex_buffer: &mut [u8], index_buffer: &mut [u8]) {
		let doc = self.get_document_from_cache(request.id.as_str()).unwrap();

		let mut file = std::fs::File::open("assets/".to_string() + doc.get("_id").unwrap().as_object_id().unwrap().to_string().as_str()).unwrap();

		let mesh_info = match &request.resource {
			ResourceContainer::Mesh(mesh) => mesh,
			_ => panic!(""),
		};
		let vertex_size = mesh_info.vertex_components.size();

		file.read(&mut vertex_buffer[..(vertex_size * mesh_info.vertex_count as usize)]).unwrap();
		file.seek(std::io::SeekFrom::Start((vertex_size * mesh_info.vertex_count as usize).next_multiple_of(16) as u64)).unwrap();
		file.read(&mut index_buffer[..(mesh_info.index_count as usize * mesh_info.index_type.size())]).unwrap();
	}
}


// TODO: test resource caching

#[cfg(test)]
mod tests {
	/// Tests for the resource manager.
	/// It is important to test the load twice as the first time it will be loaded from source and the second time it will be loaded from cache.

	use super::*;

	#[test]
	fn load_net_image() {
		let mut resource_manager = ResourceManager::new();

		// Test loading from source

		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

		let texture_info = match &resource.0 {
			ResourceContainer::Texture(texture) => texture,
			_ => panic!("")
		};

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });

		// Test loading from cache

		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

		let texture_info = match &resource.0 {
			ResourceContainer::Texture(texture) => texture,
			_ => panic!("")
		};

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });
	}

	#[ignore]
	#[test]
	fn load_local_image() {
		let mut resource_manager = ResourceManager::new();

		let resource_result = resource_manager.get("test.png");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, ResourceContainer::Texture(_)));

		let texture_info = match &resource.0 {
			ResourceContainer::Texture(texture) => texture,
			_ => panic!("")
		};

		assert!(texture_info.extent.width == 4096 && texture_info.extent.height == 1024 && texture_info.extent.depth == 1);
	}

	#[test]
	fn load_local_mesh() {
		let mut resource_manager = ResourceManager::new();

		// Test loading from source

		let resource_result = resource_manager.get("Box.gltf");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, ResourceContainer::Mesh(_)));

		dbg!(&resource.0);

		let bytes = &resource.1;

		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		if let ResourceContainer::Mesh(mesh) = &resource.0 {
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
		}

		// Test loading from cache

		let resource_result = resource_manager.get("Box.gltf");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, ResourceContainer::Mesh(_)));

		let bytes = &resource.1;

		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		if let ResourceContainer::Mesh(mesh) = &resource.0 {
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
		}
	}
}