//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

use std::{io::prelude::*, f32::consts::E};

use polodb_core::bson::{Document, bson, doc};

#[derive(Debug)]
struct Texture {
	pub extent: crate::Extent,
}

#[derive(Debug, PartialEq)]
enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	Uv,
	Color,
}

#[derive(Debug, PartialEq)]
enum IntegralTypes {
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

#[derive(Debug)]
struct VertexComponent {
	pub semantic: VertexSemantics,
	pub format: String,
	pub channel: u32,
}

#[derive(Debug)]
struct Mesh {
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_components: Vec<VertexComponent>,
	pub index_type: IntegralTypes,
	pub vertex_count: u32,
	pub index_count: u32,
}

#[derive(Debug)]
enum Resource {
	Texture(Texture),
	Mesh(Mesh),
}

use cgmath::{Vector3, Matrix3, Quaternion};

fn qtangent(normal: Vector3<f32>, tangent: Vector3<f32>, bi_tangent: Vector3<f32>) -> Quaternion<f32> {
	let tbn: Matrix3<f32> = Matrix3::from_cols(normal, tangent, bi_tangent);

	let mut qTangent = Quaternion::from(tbn);
	//qTangent.normalise();
	
	//Make sure QTangent is always positive
	if qTangent.s < 0f32 {
		qTangent = qTangent.conjugate();
	}
	
	//Bias = 1 / [2^(bits-1) - 1]
	const bias: f32 = 1.0f32 / 32767.0f32;
	
	//Because '-0' sign information is lost when using integers,
	//we need to apply a "bias"; while making sure the Quatenion
	//stays normalized.
	// ** Also our shaders assume qTangent.w is never 0. **
	if qTangent.s < bias {
		let normFactor = f32::sqrt(1f32 - bias * bias);
		qTangent.s = bias;
		qTangent.v.x *= normFactor;
		qTangent.v.y *= normFactor;
		qTangent.v.z *= normFactor;
	}
	
	//If it's reflected, then make sure .w is negative.
	let naturalBinormal = tangent.cross(normal);

	if cgmath::dot(naturalBinormal, bi_tangent/* check if should be binormal */) <= 0f32 {
		qTangent = -qTangent;
	}

	qTangent
}

/// Resource manager.
/// Handles loading assets or resources from different origins (network, local, etc.).
/// It also handles caching of resources.
/// 
/// When in a debug build it will lazily load resources from source and cache them.
struct ResourceManager {
	db: polodb_core::Database,
	collection: polodb_core::Collection<Document>,
}

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

impl ResourceManager {
	/// Creates a new resource manager.
	pub fn new() -> Self {
		let db_res = polodb_core::Database::open_file("assets/resources.db");

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

		let collection = db.collection::<Document>("resources");

		ResourceManager {
			db,
			collection,
		}
	}

	/// Tries to read a resource from the cache.\
	/// If the resource is not in the cache, it will return an error.
	/// If the resource is in the cache, it will return the resource and the bytes associated to it.\
	/// If the resource is in cache but it's data cannot be parsed, it will return an error. (This is useful if the data layout changed and you want to trigger a reload from source).
	fn load_from_cache(&mut self, path: &str) -> Result<(Resource, Vec<u8>), LoadResults> {
		let result = self.collection.find_one(doc!{ "id": path })?;

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

			match resource.get("type").unwrap().as_str().unwrap() {
				"texture" => {
					let texture_object = if let Some(polodb_core::bson::Bson::Document(texture_object)) = resource.get("texture") {
						texture_object
					} else {
						return Err(LoadResults::LoadFailed);
					};

					let _ = texture_object.get("compression").unwrap().as_str().unwrap();

					if let Some(extent) = extent_from_json(&texture_object.get("extent").unwrap()) {
						return Ok((Resource::Texture(Texture { extent }), bytes))
					} else {
						return Err(LoadResults::LoadFailed);
					}
				},
				"mesh" => {
					let bounding_box = if let Some(polodb_core::bson::Bson::Document(bbox)) = resource.get("bounding_box"){
						let min = if let Some(v) = bbox.get("min") {
							vec3f_from_json(v)
						} else {
							return Err(LoadResults::LoadFailed);
						};

						let max = if let Some(v) = bbox.get("max") {
							vec3f_from_json(v)
						} else {
							return Err(LoadResults::LoadFailed);
						};

						[min.unwrap(), max.unwrap()]
					} else {
						return Err(LoadResults::LoadFailed);
					};

					let vertex_components = if let Some(vertex_components_document) = resource.get("vertex_components") {
						let mut vertex_components = Vec::new();
						for vertex_component in vertex_components_document.as_array().unwrap() {
							let vertex_component = vertex_component.as_document().unwrap();
							vertex_components.push(VertexComponent {
								semantic: match vertex_component.get("semantic").unwrap().as_str().unwrap() {
									"Position" => VertexSemantics::Position,
									"Normal" => VertexSemantics::Normal,
									"Tangent" => VertexSemantics::Tangent,
									"BiTangent" => VertexSemantics::BiTangent,
									"UV" => VertexSemantics::Uv,
									"Color" => VertexSemantics::Color,
									_ => return Err(LoadResults::LoadFailed),
								},
								format: vertex_component.get("format").unwrap().as_str().unwrap().to_string(),
								channel: vertex_component.get("channel").unwrap().as_i32().unwrap() as u32,
							});
						}
						vertex_components
					} else {
						return Err(LoadResults::LoadFailed);
					};

					let vertex_count = if let Some(polodb_core::bson::Bson::Int32(vertex_count)) = resource.get("vertex_count") {
						*vertex_count as u32
					} else {
						return Err(LoadResults::LoadFailed);
					};

					let index_count = if let Some(polodb_core::bson::Bson::Int32(index_count)) = resource.get("index_count") {
						*index_count as u32
					} else {
						return Err(LoadResults::LoadFailed);
					};

					let index_type = if let Some(polodb_core::bson::Bson::String(index_type)) = resource.get("index_type") {
						match index_type.as_str() {
							"U8" => IntegralTypes::U8,
							"U16" => IntegralTypes::U16,
							"U32" => IntegralTypes::U32,
							_ => return Err(LoadResults::LoadFailed),
						}
					} else {
						return Err(LoadResults::LoadFailed);
					};

					return Ok((Resource::Mesh(Mesh { bounding_box, vertex_components, vertex_count, index_count, index_type }), bytes))
				},
				_ => {
					return Err(LoadResults::UnsuportedResourceType);
				}
			}
		}

		return Err(LoadResults::ResourceNotFound);
	}

	/// Tries to load a resource from source.\
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	fn load_from_source(&mut self, path: &str) -> Option<(Resource, Vec<u8>)> {
		let resource_origin = if path.starts_with("http://") || path.starts_with("https://") {
			"network"
		} else {
			"local"
		};

		// Bytes to be stored associated to resource
		let mut bytes;
		let resource_type;
		let format;

		match resource_origin {
			"network" => {
				let request = ureq::get(path).call();

				if let Err(_) = request { return None; }

				let request = request.unwrap();

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
				let mut file = std::fs::File::open(path).unwrap();
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

		let resource: Resource;

		match format {
			"png" => {
				let mut decoder = png::Decoder::new(bytes.as_slice());
				decoder.set_transformations(png::Transformations::EXPAND);
				let mut reader = decoder.read_info().unwrap();
				let mut buffer = vec![0; reader.output_buffer_size()];
				let info = reader.next_frame(&mut buffer).unwrap();

				let extent = crate::Extent { width: info.width, height: info.height, depth: 1, };

				resource = Resource::Texture(Texture { extent });

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
	
					Resource::Mesh(Mesh { bounding_box, vertex_components, index_type, vertex_count, index_count })
				};
			}
			_ => { return None; }
		}

		let mut document = polodb_core::bson::doc! {
			"id": path,
			"type": resource_type,
		};

		match resource_type {
			"texture" => {
				let extent = if let Resource::Texture(texture) = &resource { texture.extent } else { return None; };

				document.insert("texture", bson!({ "compression": "BC7", "extent": [extent.width, extent.height, extent.depth] }));

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
				let mesh = if let Resource::Mesh(mesh) = &resource { mesh } else { return None; };

				let bounding_box = bson!({
					"min": [mesh.bounding_box[0][0], mesh.bounding_box[0][1], mesh.bounding_box[0][2]],
					"max": [mesh.bounding_box[1][0], mesh.bounding_box[1][1], mesh.bounding_box[1][2]],
				});

				document.insert("bounding_box", bounding_box);
				document.insert("vertex_count", mesh.vertex_count);
				document.insert("index_count", mesh.index_count);

				let comps = mesh.vertex_components.iter().map(
					|vertex_component| {
						bson!({
							"semantic": match vertex_component.semantic {
								VertexSemantics::Position => "Position",
								VertexSemantics::Normal => "Normal",
								VertexSemantics::Tangent => "Tangent",
								VertexSemantics::BiTangent => "BiTangent",
								VertexSemantics::Uv => "UV",
								VertexSemantics::Color => "Color",
							},
							"format": vertex_component.format.as_str(),
							"channel": vertex_component.channel,
						})
					}
				).collect::<Vec<_>>();

				document.insert("vertex_components", comps);

				document.insert("index_type", match mesh.index_type {
					IntegralTypes::U8 => "U8",
					IntegralTypes::U16 => "U16",
					IntegralTypes::U32 => "U32",
					_ => return None,
				});
			}
			_ => { return None; }
		}

		let insert_result = self.collection.insert_one(document);

		let insert_result = insert_result.unwrap();

		let resource_id = insert_result.inserted_id.as_object_id().unwrap();

		let path = "assets/".to_string() + resource_id.to_string().as_str();

		// Write file with resource contents
		let mut file = std::fs::File::create(path).unwrap();

		file.write_all(bytes.as_slice()).unwrap();

		return Some((resource, bytes));
	}

	/// Tries to load a resource from cache or source.\
	/// If the resource cannot be found (non existent file, unreacheble network address, fails to parse, etc.) it will return None.\
	/// If the resource is in cache but it's data cannot be parsed, it will return None.
	pub fn get(&mut self, path: &str) -> Option<(Resource, Vec<u8>)> {
		let load_result = self.load_from_cache(path);

		match load_result {
			Ok(load_result) => return Some(load_result),
			Err(LoadResults::ResourceNotFound) => return self.load_from_source(path),
			Err(LoadResults::CacheFileNotFound { document }) => {
				println!("\x1B[WARNING]Resource load failed, cache file not found. Could have been deleted, renamed, or moved. Loading from source.");

				let _result = self.collection.delete_many(doc!{ "id": path }); // Delete all documents that point to non existent file

				return self.load_from_source(path) // If load from cache fails, try loading from source. Can happen mostly if resource fields/layout changed or if data was corrupted during debugging.
			}
			Err(LoadResults::LoadFailed) => {
				println!("\x1B[WARNING]Resource load failed, trying to load from source.");
				let result = self.collection.find_one(doc!{ "id": path }).unwrap().unwrap();
				let file_deletion_result = std::fs::remove_file(result.get("_id").unwrap().as_object_id().unwrap().to_string());

				if let Err(_) = file_deletion_result {
					println!("\x1B[WARNING]Could not delete resource file! This may leave a dangling file in the resources folder.");
				}

				let result = self.collection.delete_many(doc!{ "id": path });

				if let Ok(a) = result {
					if a.deleted_count != 1 {
						println!("\x1B[WARNING]Could not delete resource from database! This may leave a dangling file in the resources folder.");
					}
				}

				return self.load_from_source(path) // If load from cache fails, try loading from source. Can happen mostly if resource fields/layout changed or if data was corrupted during debugging.
			}
			Err(LoadResults::UnsuportedResourceType) => return None,
		}
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

		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, Resource::Texture(_)));

		let texture_info = match &resource.0 {
			Resource::Texture(texture) => texture,
			_ => panic!("")
		};

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });

		//let bytes = resource_manager.get("https://upload.wikimedia.org/wikipedia/commons/6/6a/PNG_Test.png");
		let resource_result = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, Resource::Texture(_)));

		let texture_info = match &resource.0 {
			Resource::Texture(texture) => texture,
			_ => panic!("")
		};

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });
	}

	#[ignore]
	#[test]
	fn load_local_image() {
		let mut resource_manager = ResourceManager::new();

		let resource_result = resource_manager.get("resources/test.png");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, Resource::Texture(_)));

		let texture_info = match &resource.0 {
			Resource::Texture(texture) => texture,
			_ => panic!("")
		};

		assert!(texture_info.extent.width == 4096 && texture_info.extent.height == 1024 && texture_info.extent.depth == 1);
	}

	#[test]
	fn load_local_mesh() {
		let mut resource_manager = ResourceManager::new();

		let resource_result = resource_manager.get("resources/Box.gltf");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, Resource::Mesh(_)));

		dbg!(&resource.0);

		let bytes = &resource.1;

		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		if let Resource::Mesh(mesh) = &resource.0 {
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

		let resource_result = resource_manager.get("resources/Box.gltf");

		assert!(resource_result.is_some());

		let resource = resource_result.unwrap();

		assert!(matches!(resource.0, Resource::Mesh(_)));

		let bytes = &resource.1;

		assert_eq!(bytes.len(), (24 /* vertices */ * (3 /* components per position */ * 4 /* float size */ + 3/*normals */ * 4) as usize).next_multiple_of(16) + 6/* cube faces */ * 2 /* triangles per face */ * 3 /* indices per triangle */ * 2 /* bytes per index */);

		if let Resource::Mesh(mesh) = &resource.0 {
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