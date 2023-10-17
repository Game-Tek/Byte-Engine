use std::io::{Seek, Read};

use log::error;
use polodb_core::bson::{Document, doc};
use serde::{Serialize, Deserialize};

use super::{ResourceHandler, SerializedResourceDocument, GenericResourceSerialization, Resource, ProcessedResources};

pub struct MeshResourceHandler {

}

impl MeshResourceHandler {
	pub fn new() -> Self {
		Self {}
	}
}

impl ResourceHandler for MeshResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"Mesh" | "mesh" => true,
			"gltf" => true,
			_ => false
		}
	}

	fn process(&self, _: &super::ResourceManager, asset_url: &str, bytes: &[u8]) -> Result<Vec<ProcessedResources>, String> {
		let (gltf, buffers, _) = gltf::import_slice(bytes).unwrap();

		let mut buf: Vec<u8> = Vec::with_capacity(4096 * 1024 * 3);

		// 'mesh_loop: for mesh in gltf.meshes() {					
		// 	for primitive in mesh.primitives() {
		
		let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();

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
			return Err("Mesh does not have positions".to_string());
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
			return Err("Mesh does not have indices".to_string());
		}
		
		let mesh = Mesh {
			bounding_box,
			vertex_components,
			index_count,
			vertex_count,
			index_type
		};

		let resource_document = GenericResourceSerialization::new(asset_url.to_string(), mesh);

		Ok(vec![ProcessedResources::Generated((resource_document, buf))])
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>)> {
		vec![("Mesh", Box::new(|document| {
			let mesh = Mesh::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(mesh)
		}))]
	}

	fn read(&self, resource: &Box<dyn std::any::Any>, file: &mut std::fs::File, buffers: &mut [super::Buffer]) {
		let mesh: &Mesh = resource.downcast_ref().unwrap();

		for buffer in buffers {
			match buffer.tag.as_str() {
				"Vertex" => {
					file.seek(std::io::SeekFrom::Start(0)).unwrap();
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * mesh.vertex_components.size())]).unwrap();
				}
				"Vertex.Position" => {
					file.seek(std::io::SeekFrom::Start(0)).unwrap();
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * 12)]).unwrap();
				}
				"Vertex.Normal" => {
					file.seek(std::io::SeekFrom::Start(mesh.vertex_count as u64 * 12)).unwrap(); // 12 bytes per vertex
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * 12)]).unwrap();
				}
				"Index" => {
					let base_offset = mesh.vertex_count as u64 * mesh.vertex_components.size() as u64;
					let rounded_offset = base_offset.next_multiple_of(16);
					file.seek(std::io::SeekFrom::Start(rounded_offset)).expect("Failed to seek to index buffer");
					file.read(&mut buffer.buffer[0..(mesh.index_count as usize * mesh.index_type.size())]).unwrap();
				}
				_ => {
					error!("Unknown buffer tag: {}", buffer.tag);
				}
			}
		}
	}
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VertexSemantics {
	Position,
	Normal,
	Tangent,
	BiTangent,
	Uv,
	Color,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
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

impl Resource for Mesh {
	fn get_class(&self) -> &'static str { "Mesh" }
}

pub trait Size {
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

#[cfg(test)]
mod tests {
	use crate::{resource_manager::{ResourceManager, Options, OptionResource, Buffer}, Vector3};

	use super::*;

	#[test]
	fn gltf() {
		let bytes = std::fs::read("assets/Box.gltf").unwrap();

		let (gltf, buffers, _) = gltf::import_slice(bytes).unwrap();

		let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();

		let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
		
		let vertex = reader.read_positions().unwrap().next().unwrap();
		
		assert_eq!(vertex, [-0.5f32, -0.5f32, 0.5f32]);
	}

	#[test]
	fn load_with_manager_buffer() {
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

		// Cast buffer to Vector3<f32>
		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 24);
		assert_eq!(vertex_positions[0], Vector3::new(-0.5f32, -0.5f32, 0.5f32));
		assert_eq!(vertex_positions[1], Vector3::new(0.5f32, -0.5f32, 0.5f32));
		assert_eq!(vertex_positions[2], Vector3::new(-0.5f32, 0.5f32, 0.5f32));

		// Cast buffer + 12 * 24 to Vector3<f32>
		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const Vector3).add(24) as *const Vector3, mesh.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 24);
		assert_eq!(vertex_normals[0], Vector3::new(0f32, 0f32, 1f32));
		assert_eq!(vertex_normals[1], Vector3::new(0f32, 0f32, 1f32));
		assert_eq!(vertex_normals[2], Vector3::new(0f32, 0f32, 1f32));

		// Cast buffer + 12 * 24 + 12 * 24 to u16
		let indeces = unsafe { std::slice::from_raw_parts((buffer.as_ptr().add(12 * 24 + 12 * 24)) as *const u16, mesh.index_count as usize) };

		assert_eq!(indeces.len(), 36);
		assert_eq!(indeces[0], 0);
		assert_eq!(indeces[1], 1);
		assert_eq!(indeces[2], 2);
	}

	#[test]
	fn load_with_vertices_and_indices_with_provided_buffer() {
		let mut resource_manager = ResourceManager::new();

		let resource_request = resource_manager.request_resource("Box").expect("Failed to request resource");

		let mut options = Options { resources: Vec::new(), };

		let mut vertex_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = &resource_request.resources[0];

		match resource.class.as_str() {
			"Mesh" => {
				options.resources.push(OptionResource {
					url: resource.url.clone(),
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

					// Cast vertex_buffer to Vector3<f32>
					let vertex_positions = unsafe { std::slice::from_raw_parts(vertex_buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_positions.len(), 24);
					assert_eq!(vertex_positions[0], Vector3::new(-0.5f32, -0.5f32, 0.5f32));
					assert_eq!(vertex_positions[1], Vector3::new(0.5f32, -0.5f32, 0.5f32));
					assert_eq!(vertex_positions[2], Vector3::new(-0.5f32, 0.5f32, 0.5f32));

					// Cast vertex_buffer + 12 * 24 to Vector3<f32>
					let vertex_normals = unsafe { std::slice::from_raw_parts((vertex_buffer.as_ptr() as *const Vector3).add(24) as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_normals.len(), 24);
					assert_eq!(vertex_normals[0], Vector3::new(0f32, 0f32, 1f32));
					assert_eq!(vertex_normals[1], Vector3::new(0f32, 0f32, 1f32));
					assert_eq!(vertex_normals[2], Vector3::new(0f32, 0f32, 1f32));


					// Cast index_buffer to u16
					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, mesh.index_count as usize) };

					assert_eq!(index_buffer.len(), 36);
					assert_eq!(index_buffer[0], 0);
					assert_eq!(index_buffer[1], 1);
					assert_eq!(index_buffer[2], 2);

					// assert_eq!(mesh_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize], vertex_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize]);

					// assert_eq!(mesh_buffer[576..(576 + mesh.index_count * 2) as usize], index_buffer[0..(mesh.index_count * 2) as usize]);
				}
				_ => {}
			}
		}
	}

	#[test]
	fn load_with_non_interleaved_vertices_and_indices_with_provided_buffer() {
		let mut resource_manager = ResourceManager::new();

		let resource_request = resource_manager.request_resource("Box").expect("Failed to request resource");

		let mut options = Options { resources: Vec::new(), };

		let mut vertex_positions_buffer = vec![0u8; 1024];
		let mut vertex_normals_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = &resource_request.resources[0];

		match resource.class.as_str() {
			"Mesh" => {
				options.resources.push(OptionResource {
					url: resource.url.clone(),
					buffers: vec![
						Buffer{ buffer: vertex_positions_buffer.as_mut_slice(), tag: "Vertex.Position".to_string() },
						Buffer{ buffer: vertex_normals_buffer.as_mut_slice(), tag: "Vertex.Normal".to_string() },
						Buffer{ buffer: index_buffer.as_mut_slice(), tag: "Index".to_string() }
					],
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

					// Cast vertex_positions_buffer to Vector3<f32>
					let vertex_positions_buffer = unsafe { std::slice::from_raw_parts(vertex_positions_buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_positions_buffer.len(), 24);
					assert_eq!(vertex_positions_buffer[0], Vector3::new(-0.5f32, -0.5f32, 0.5f32));
					assert_eq!(vertex_positions_buffer[1], Vector3::new(0.5f32, -0.5f32, 0.5f32));
					assert_eq!(vertex_positions_buffer[2], Vector3::new(-0.5f32, 0.5f32, 0.5f32));

					// Cast vertex_normals_buffer to Vector3<f32>
					let vertex_normals_buffer = unsafe { std::slice::from_raw_parts(vertex_normals_buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_normals_buffer.len(), 24);
					assert_eq!(vertex_normals_buffer[0], Vector3::new(0f32, 0f32, 1f32));
					assert_eq!(vertex_normals_buffer[1], Vector3::new(0f32, 0f32, 1f32));
					assert_eq!(vertex_normals_buffer[2], Vector3::new(0f32, 0f32, 1f32));

					// Cast index_buffer to u16

					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, mesh.index_count as usize) };

					assert_eq!(index_buffer.len(), 36);
					assert_eq!(index_buffer[0], 0);
					assert_eq!(index_buffer[1], 1);
					assert_eq!(index_buffer[2], 2);

					// assert_eq!(mesh_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize], vertex_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize]);

					// assert_eq!(mesh_buffer[576..(576 + mesh.index_count * 2) as usize], index_buffer[0..(mesh.index_count * 2) as usize]);
				}
				_ => {}
			}
		}
	}
}