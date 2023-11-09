use std::io::{Seek, Read};

use log::error;
use polodb_core::bson::doc;
use serde::{Serialize, Deserialize};

use super::{GenericResourceSerialization, Resource, ProcessedResources, resource_handler::ResourceHandler, resource_manager::ResourceManager};

pub struct MeshResourceHandler {

}

impl MeshResourceHandler {
	pub fn new() -> Self {
		Self {}
	}

	fn make_bounding_box(mesh: &gltf::Primitive) -> [[f32; 3]; 2] {
		let bounds = mesh.bounding_box();

		[
			[bounds.min[0], bounds.min[1], bounds.min[2],],
			[bounds.max[0], bounds.max[1], bounds.max[2],],
		]
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

	fn process(&self, resource_manager: &ResourceManager, asset_url: &str) -> Result<Vec<ProcessedResources>, String> {
		let (gltf, buffers, _) = gltf::import(resource_manager.realize_asset_path(asset_url).unwrap()).unwrap();

		const MESHLETIZE: bool = true;

		let mut resources = Vec::with_capacity(2);

		for mesh in gltf.meshes() {
			for primitive in mesh.primitives() {
				let mut vertex_components = Vec::new();

				let bounding_box = Self::make_bounding_box(&primitive);

				let mut buffer = Vec::with_capacity(4096 * 1024 * 3);

				let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

				let vertex_count = if let Some(positions) = reader.read_positions() {
					let vertex_count = positions.clone().count();
					positions.for_each(|mut position| {
						position[2] = -position[2];
						position.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte)))
				});
					vertex_components.push(VertexComponent { semantic: VertexSemantics::Position, format: "vec3f".to_string(), channel: 0 });
					vertex_count
				} else {
					return Err("Mesh does not have positions".to_string());
				};

				let indices = reader.read_indices().expect("Cannot create mesh which does not have indices").into_u32().collect::<Vec<u32>>();

				let optimized_indices = meshopt::optimize::optimize_vertex_cache(&indices, vertex_count);				
	
				if let Some(normals) = reader.read_normals() {
					normals.for_each(|mut normal| {
						normal[2] = -normal[2];
						normal.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte)));
					});

					vertex_components.push(VertexComponent { semantic: VertexSemantics::Normal, format: "vec3f".to_string(), channel: 1 });
				}
	
				if let Some(tangents) = reader.read_tangents() {
					tangents.for_each(|tangent| tangent.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))));
					vertex_components.push(VertexComponent { semantic: VertexSemantics::Tangent, format: "vec4f".to_string(), channel: 2 });
				}
	
				if let Some(uv) = reader.read_tex_coords(0) {
					uv.into_f32().for_each(|uv| uv.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))));
					vertex_components.push(VertexComponent { semantic: VertexSemantics::Uv, format: "vec3f".to_string(), channel: 3 });
				}
	
				// align buffer to 16 bytes for indices
				while buffer.len() % 16 != 0 { buffer.push(0); }

				let mut index_streams = Vec::with_capacity(2);

				let meshlet_stream;

				if MESHLETIZE {
					let meshlets = meshopt::clusterize::build_meshlets(&optimized_indices, vertex_count, 64, 126);

					let offset = buffer.len();

					{
						let index_type = IntegralTypes::U16;
						
						match index_type {
							IntegralTypes::U16 => {
								let mut index_count = 0usize;
								for meshlet in &meshlets {
									index_count += meshlet.vertex_count as usize;
									for i in 0..meshlet.vertex_count as usize {
										(meshlet.vertices[i] as u16).to_le_bytes().iter().for_each(|byte| buffer.push(*byte));
									}
								}
								index_streams.push(IndexStream{ data_type: IntegralTypes::U16, stream_type: IndexStreamTypes::Raw, offset, count: index_count as u32 });
							}
							_ => panic!("Unsupported index type")
						}
					}
	
					let offset = buffer.len();

					let mut index_count: usize = 0;

					for meshlet in &meshlets {
						index_count += meshlet.triangle_count as usize * 3;
						for i in 0..meshlet.triangle_count as usize {
							for x in meshlet.indices[i] {
								assert!(x <= 64, "Meshlet index out of bounds"); // Max vertices per meshlet
								buffer.push(x);
							}
						}
					}

					assert_eq!(index_count, optimized_indices.len());

					index_streams.push(IndexStream{ data_type: IntegralTypes::U8, stream_type: IndexStreamTypes::Meshlets, offset, count: index_count as u32 });

					let offset = buffer.len();

					meshlet_stream = Some(MeshletStream{ offset, count: meshlets.len() as u32 });

					for meshlet in &meshlets {
						buffer.push(meshlet.vertex_count);
						buffer.push(meshlet.triangle_count);
					}
				} else {
					let offset = buffer.len();

					{
						let index_type = IntegralTypes::U16;
	
						match index_type {
							IntegralTypes::U16 => {
								optimized_indices.iter().map(|i| *i as u16).for_each(|i| i.to_le_bytes().iter().for_each(|byte| buffer.push(*byte)));
								index_streams.push(IndexStream{ data_type: IntegralTypes::U16, stream_type: IndexStreamTypes::Raw, offset, count: optimized_indices.len() as u32 });
							}
							_ => panic!("Unsupported index type")
						}
					}

					meshlet_stream = None;
				}

				let mesh = Mesh {
					compression: CompressionSchemes::None,
					bounding_box,
					vertex_components,
					vertex_count: vertex_count as u32,
					index_streams,
					meshlet_stream,
				};
	
				let resource_document = GenericResourceSerialization::new(asset_url.to_string(), mesh);

				resources.push(ProcessedResources::Generated((resource_document, buffer)));
			}
		}

		Ok(resources)
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send>)> {
		vec![("Mesh", Box::new(|document| {
			let mesh = Mesh::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(mesh)
		}))]
	}

	fn read(&self, resource: &Box<dyn std::any::Any>, file: &mut std::fs::File, buffers: &mut [super::Stream]) {
		let mesh: &Mesh = resource.downcast_ref().unwrap();

		for buffer in buffers {
			match buffer.name.as_str() {
				"Vertex" => {
					file.seek(std::io::SeekFrom::Start(0)).unwrap();
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * mesh.vertex_components.size())]).unwrap();
				}
				"Vertex.Position" => {
					file.seek(std::io::SeekFrom::Start(0)).unwrap();
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * 12)]).unwrap();
				}
				"Vertex.Normal" => {
					#[cfg(debug_assertions)]
					if !mesh.vertex_components.iter().any(|v| v.semantic == VertexSemantics::Normal) { error!("Requested Vertex.Normal stream but mesh does not have normals."); continue; }

					file.seek(std::io::SeekFrom::Start(mesh.vertex_count as u64 * 12)).unwrap(); // 12 bytes per vertex
					file.read(&mut buffer.buffer[0..(mesh.vertex_count as usize * 12)]).unwrap();
				}
				"Indices" => {
					#[cfg(debug_assertions)]
					if !mesh.index_streams.iter().any(|stream| stream.stream_type == IndexStreamTypes::Raw) { error!("Requested Index stream but mesh does not have RAW indices."); continue; }

					let raw_index_stram = mesh.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Raw).unwrap();

					file.seek(std::io::SeekFrom::Start(raw_index_stram.offset as u64)).expect("Failed to seek to index buffer");
					file.read(&mut buffer.buffer[0..(raw_index_stram.count as usize * raw_index_stram.data_type.size())]).unwrap();
				}
				"MeshletIndices" => {
					#[cfg(debug_assertions)]
					if !mesh.index_streams.iter().any(|stream| stream.stream_type == IndexStreamTypes::Meshlets) { error!("Requested MeshletIndices stream but mesh does not have meshlet indices indices."); continue; }

					let meshlet_indices_streams = mesh.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Meshlets).unwrap();

					file.seek(std::io::SeekFrom::Start(meshlet_indices_streams.offset as u64)).expect("Failed to seek to index buffer");
					file.read(&mut buffer.buffer[0..(meshlet_indices_streams.count as usize * meshlet_indices_streams.data_type.size())]).unwrap();
				}
				"Meshlets" => {
					#[cfg(debug_assertions)]
					if mesh.meshlet_stream.is_none() { error!("Requested Meshlets stream but mesh does not have meshlets."); continue; }

					let meshlet_stream = mesh.meshlet_stream.as_ref().unwrap();

					file.seek(std::io::SeekFrom::Start(meshlet_stream.offset as u64)).expect("Failed to seek to index buffer");
					file.read(&mut buffer.buffer[0..(meshlet_stream.count as usize * 2)]).unwrap();
				}
				_ => {
					error!("Unknown buffer tag: {}", buffer.name);
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
pub enum CompressionSchemes {
	None,
	Quantization,
	Octahedral,
	OctahedralQuantization,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum IndexStreamTypes {
	Raw,
	Meshlets,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexStream {
	pub stream_type: IndexStreamTypes,
	pub offset: usize,
	pub count: u32,
	pub data_type: IntegralTypes,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeshletStream {
	pub offset: usize,
	pub count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Mesh {
	pub compression: CompressionSchemes,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_components: Vec<VertexComponent>,
	pub vertex_count: u32,
	pub index_streams: Vec<IndexStream>,
	pub meshlet_stream: Option<MeshletStream>,
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
			VertexSemantics::Tangent => 4 * 4,
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
	use crate::{resource_manager::{resource_manager::ResourceManager, Options, OptionResource, Stream}, Vector3};

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
	fn load_local_mesh() {
		let mut resource_manager = ResourceManager::new();

		let (response, buffer) = resource_manager.get("Box").expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		let _offset = 0usize;

		assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(mesh.vertex_count, 24);
		assert_eq!(mesh.vertex_components.len(), 2);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);

		assert_eq!(mesh.index_streams.len(), 2);

		let offset = ((mesh.vertex_count * mesh.vertex_components.size() as u32) as usize).next_multiple_of(16);

		assert_eq!(mesh.index_streams[0].stream_type, IndexStreamTypes::Raw);
		assert_eq!(mesh.index_streams[0].offset, offset);
		assert_eq!(mesh.index_streams[0].count, 36);
		assert_eq!(mesh.index_streams[0].data_type, IntegralTypes::U16);

		let meshlet_stream_info = mesh.meshlet_stream.as_ref().unwrap();

		let offset = offset + mesh.index_streams[0].count as usize * mesh.index_streams[0].data_type.size();

		assert_eq!(mesh.index_streams[1].stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(mesh.index_streams[1].offset, offset);
		assert_eq!(mesh.index_streams[1].count, 36);
		assert_eq!(mesh.index_streams[1].data_type, IntegralTypes::U8);

		let resource_request = resource_manager.request_resource("Box");

		let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

		let mut options = Options { resources: Vec::new(), };

		let mut vertex_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = &resource_request.resources[0];

		match resource.class.as_str() {
			"Mesh" => {
				options.resources.push(OptionResource {
					url: resource.url.clone(),
					streams: vec![Stream{ buffer: vertex_buffer.as_mut_slice(), name: "Vertex".to_string() }, Stream{ buffer: index_buffer.as_mut_slice(), name: "Indices".to_string() }],
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

					assert_eq!(buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize], vertex_buffer[0..(mesh.vertex_count * mesh.vertex_components.size() as u32) as usize]);

					assert_eq!(buffer[576..(576 + mesh.index_streams[0].count * 2) as usize], index_buffer[0..(mesh.index_streams[0].count * 2) as usize]);
				}
				_ => {}
			}
		}
	}

	#[test]
	fn load_local_gltf_mesh_with_external_binaries() {
		let mut resource_manager = ResourceManager::new();

		let (response, buffer) = resource_manager.get("Suzanne").expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		let _offset = 0usize;

		// assert_eq!(mesh.bounding_box, [[-2.674f32, -1.925f32, -1.626f32], [2.674f32, 1.925f32, 1.626f32]]);
		assert_eq!(mesh.vertex_count, 11808);
		assert_eq!(mesh.vertex_components.len(), 4);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);

		assert_eq!(mesh.index_streams.len(), 2);

		let offset = ((mesh.vertex_count * mesh.vertex_components.size() as u32) as usize).next_multiple_of(16);

		assert_eq!(mesh.index_streams[0].stream_type, IndexStreamTypes::Raw);
		assert_eq!(mesh.index_streams[0].offset, offset);
		assert_eq!(mesh.index_streams[0].count, 3936 * 3);
		assert_eq!(mesh.index_streams[0].data_type, IntegralTypes::U16);

		let offset = offset + mesh.index_streams[0].count as usize * mesh.index_streams[0].data_type.size();

		assert_eq!(mesh.index_streams[1].stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(mesh.index_streams[1].offset, offset);
		assert_eq!(mesh.index_streams[1].count, 3936 * 3);
		assert_eq!(mesh.index_streams[1].data_type, IntegralTypes::U8);

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 11808);

		assert_eq!(vertex_positions[0], Vector3::new(0.492188f32, 0.185547f32, -0.720703f32));
		assert_eq!(vertex_positions[1], Vector3::new(0.472656f32, 0.243042f32, -0.751221f32));
		assert_eq!(vertex_positions[2], Vector3::new(0.463867f32, 0.198242f32, -0.753418f32));

		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const Vector3).add(11808), mesh.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 11808);

		assert_eq!(vertex_normals[0], Vector3::new(0.703351f32, -0.228379f32, -0.673156f32));
		assert_eq!(vertex_normals[1], Vector3::new(0.818977f32, -0.001884f32, -0.573824f32));
		assert_eq!(vertex_normals[2], Vector3::new(0.776439f32, -0.262265f32, -0.573027f32));
	}

	#[test]
	fn load_with_manager_buffer() {
		let mut resource_manager = ResourceManager::new();

		let (response, buffer) = resource_manager.get("Box").expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		let mesh = resource.downcast_ref::<Mesh>().unwrap();
		
		let offset = 0usize;

		assert_eq!(mesh.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(mesh.vertex_count, 24);
		assert_eq!(mesh.vertex_components.len(), 2);
		assert_eq!(offset, 0);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);
		assert_eq!(mesh.index_streams.len(), 2);

		let offset = ((mesh.vertex_count * mesh.vertex_components.size() as u32) as usize).next_multiple_of(16);

		assert_eq!(mesh.index_streams[0].stream_type, IndexStreamTypes::Raw);
		assert_eq!(mesh.index_streams[0].offset, offset);
		assert_eq!(mesh.index_streams[0].count, 36);
		assert_eq!(mesh.index_streams[0].data_type, IntegralTypes::U16);

		let offset = offset + mesh.index_streams[0].count as usize * mesh.index_streams[0].data_type.size();

		assert_eq!(mesh.index_streams[1].stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(mesh.index_streams[1].offset, offset);
		assert_eq!(mesh.index_streams[1].count, 36);
		assert_eq!(mesh.index_streams[1].data_type, IntegralTypes::U8);

		// Cast buffer to Vector3<f32>
		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 24);
		assert_eq!(vertex_positions[0], Vector3::new(-0.5f32, -0.5f32, -0.5f32));
		assert_eq!(vertex_positions[1], Vector3::new(0.5f32, -0.5f32, -0.5f32));
		assert_eq!(vertex_positions[2], Vector3::new(-0.5f32, 0.5f32, -0.5f32));

		// Cast buffer + 12 * 24 to Vector3<f32>
		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const Vector3).add(24), mesh.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 24);
		assert_eq!(vertex_normals[0], Vector3::new(0f32, 0f32, -1f32));
		assert_eq!(vertex_normals[1], Vector3::new(0f32, 0f32, -1f32));
		assert_eq!(vertex_normals[2], Vector3::new(0f32, 0f32, -1f32));

		// Cast buffer + 12 * 24 + 12 * 24 to u16
		let indeces = unsafe { std::slice::from_raw_parts((buffer.as_ptr().add(12 * 24 + 12 * 24)) as *const u16, mesh.index_streams[0].count as usize) };

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
					streams: vec![Stream{ buffer: vertex_buffer.as_mut_slice(), name: "Vertex".to_string() }, Stream{ buffer: index_buffer.as_mut_slice(), name: "Indices".to_string() }],
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
					assert_eq!(vertex_positions[0], Vector3::new(-0.5f32, -0.5f32, -0.5f32));
					assert_eq!(vertex_positions[1], Vector3::new(0.5f32, -0.5f32, -0.5f32));
					assert_eq!(vertex_positions[2], Vector3::new(-0.5f32, 0.5f32, -0.5f32));

					// Cast vertex_buffer + 12 * 24 to Vector3<f32>
					let vertex_normals = unsafe { std::slice::from_raw_parts((vertex_buffer.as_ptr() as *const Vector3).add(24) as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_normals.len(), 24);
					assert_eq!(vertex_normals[0], Vector3::new(0f32, 0f32, -1f32));
					assert_eq!(vertex_normals[1], Vector3::new(0f32, 0f32, -1f32));
					assert_eq!(vertex_normals[2], Vector3::new(0f32, 0f32, -1f32));


					// Cast index_buffer to u16
					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, mesh.index_streams[0].count as usize) };

					assert_eq!(index_buffer.len(), 36);
					assert_eq!(index_buffer[0], 0);
					assert_eq!(index_buffer[1], 1);
					assert_eq!(index_buffer[2], 2);
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
					streams: vec![
						Stream{ buffer: vertex_positions_buffer.as_mut_slice(), name: "Vertex.Position".to_string() },
						Stream{ buffer: vertex_normals_buffer.as_mut_slice(), name: "Vertex.Normal".to_string() },
						Stream{ buffer: index_buffer.as_mut_slice(), name: "Indices".to_string() }
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
					assert_eq!(vertex_positions_buffer[0], Vector3::new(-0.5f32, -0.5f32, -0.5f32));
					assert_eq!(vertex_positions_buffer[1], Vector3::new(0.5f32, -0.5f32, -0.5f32));
					assert_eq!(vertex_positions_buffer[2], Vector3::new(-0.5f32, 0.5f32, -0.5f32));

					// Cast vertex_normals_buffer to Vector3<f32>
					let vertex_normals_buffer = unsafe { std::slice::from_raw_parts(vertex_normals_buffer.as_ptr() as *const Vector3, mesh.vertex_count as usize) };

					assert_eq!(vertex_normals_buffer.len(), 24);
					assert_eq!(vertex_normals_buffer[0], Vector3::new(0f32, 0f32, -1f32));
					assert_eq!(vertex_normals_buffer[1], Vector3::new(0f32, 0f32, -1f32));
					assert_eq!(vertex_normals_buffer[2], Vector3::new(0f32, 0f32, -1f32));

					// Cast index_buffer to u16

					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, mesh.index_streams[0].count as usize) };

					assert_eq!(index_buffer.len(), 36);
					assert_eq!(index_buffer[0], 0);
					assert_eq!(index_buffer[1], 1);
					assert_eq!(index_buffer[2], 2);
				}
				_ => {}
			}
		}
	}
}