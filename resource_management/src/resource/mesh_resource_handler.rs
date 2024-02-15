use serde::Deserialize;
use smol::{fs::File, io::{AsyncReadExt, AsyncSeekExt}};

use crate::{types::{IndexStreamTypes, Mesh, Size, VertexSemantics}, Resource, Stream};

use super::resource_handler::ResourceHandler;

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
			"gltf" | "glb" => true,
			_ => false
		}
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Mesh", Box::new(|document| {
			let mesh = Mesh::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(mesh)
		}))]
	}

	fn read<'a>(&'a self, resource: &'a dyn Resource, file: &'a mut File, buffers: &'a mut [Stream<'a>]) -> utils::BoxedFuture<()> {
		Box::pin(async move {
			let mesh: &Mesh = resource.downcast_ref().unwrap();

			let mut buffers = buffers.iter_mut().map(|b| {
				(&b.name, utils::BufferAllocator::new(b.buffer))
			}).collect::<Vec<_>>();

			for sub_mesh in &mesh.sub_meshes {
				for primitive in &sub_mesh.primitives {
					for (name, buffer) in &mut buffers {
						match name.as_str() {
							"Vertex" => {
								file.seek(std::io::SeekFrom::Start(0)).await.expect("Failed to seek to vertex buffer");
								file.read_exact(buffer.take(primitive.vertex_count as usize * primitive.vertex_components.size())).await.expect("Failed to read vertex buffer");
							}
							"Vertex.Position" => {
								file.seek(std::io::SeekFrom::Start(0)).await.expect("Failed to seek to vertex buffer");
								file.read_exact(buffer.take(primitive.vertex_count as usize * 12)).await.expect("Failed to read vertex buffer");
							}
							"Vertex.Normal" => {
								#[cfg(debug_assertions)]
								if !primitive.vertex_components.iter().any(|v| v.semantic == VertexSemantics::Normal) { log::error!("Requested Vertex.Normal stream but mesh does not have normals."); continue; }
		
								file.seek(std::io::SeekFrom::Start(primitive.vertex_count as u64 * 12)).await.expect("Failed to seek to vertex buffer");
								file.read_exact(buffer.take(primitive.vertex_count as usize * 12)).await.expect("Failed to read vertex buffer");
							}
							"TriangleIndices" => {
								#[cfg(debug_assertions)]
								if !primitive.index_streams.iter().any(|stream| stream.stream_type == IndexStreamTypes::Triangles) { log::error!("Requested Index stream but mesh does not have triangle indices."); continue; }
		
								let triangle_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();
		
								file.seek(std::io::SeekFrom::Start(triangle_index_stream.offset as u64)).await.expect("Failed to seek to index buffer");
								file.read_exact(buffer.take(triangle_index_stream.count as usize * triangle_index_stream.data_type.size())).await.unwrap();
							}
							"VertexIndices" => {
								#[cfg(debug_assertions)]
								if !primitive.index_streams.iter().any(|stream| stream.stream_type == IndexStreamTypes::Vertices) { log::error!("Requested Index stream but mesh does not have vertex indices."); continue; }
		
								let vertex_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Vertices).unwrap();
		
								file.seek(std::io::SeekFrom::Start(vertex_index_stream.offset as u64)).await.expect("Failed to seek to index buffer");
								file.read_exact(buffer.take(vertex_index_stream.count as usize * vertex_index_stream.data_type.size())).await.unwrap();
							}
							"MeshletIndices" => {
								#[cfg(debug_assertions)]
								if !primitive.index_streams.iter().any(|stream| stream.stream_type == IndexStreamTypes::Meshlets) { log::error!("Requested MeshletIndices stream but mesh does not have meshlet indices."); continue; }
		
								let meshlet_indices_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Meshlets).unwrap();
		
								file.seek(std::io::SeekFrom::Start(meshlet_indices_stream.offset as u64)).await.expect("Failed to seek to index buffer");
								file.read_exact(buffer.take(meshlet_indices_stream.count as usize * meshlet_indices_stream.data_type.size())).await.unwrap();
							}
							"Meshlets" => {
								#[cfg(debug_assertions)]
								if primitive.meshlet_stream.is_none() { log::error!("Requested Meshlets stream but mesh does not have meshlets."); continue; }
		
								let meshlet_stream = primitive.meshlet_stream.as_ref().unwrap();
		
								file.seek(std::io::SeekFrom::Start(meshlet_stream.offset as u64)).await.expect("Failed to seek to index buffer");
								file.read_exact(buffer.take(meshlet_stream.count as usize * 2)).await.unwrap();
							}
							_ => {
								log::error!("Unknown buffer tag: {}", name);
							}
						}
					}
				}
			}
		})
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
	use crate::{resource::{resource_manager::ResourceManager, texture_resource_handler::ImageResourceHandler}, types::{IndexStreamTypes, IntegralTypes, Mesh, VertexSemantics}, LoadRequest, LoadResourceRequest, Stream};

	use super::*;

	#[test]
	fn load_local_mesh() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(MeshResourceHandler::new());

		let (response, buffer) = smol::block_on(resource_manager.get("Box")).expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.sub_meshes.len(), 1);

		let sub_mesh = &mesh.sub_meshes[0];

		assert_eq!(sub_mesh.primitives.len(), 1);

		let primitive = &sub_mesh.primitives[0];


		let _offset = 0usize;

		assert_eq!(primitive.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(primitive.vertex_count, 24);
		assert_eq!(primitive.vertex_components.len(), 2);
		assert_eq!(primitive.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(primitive.vertex_components[0].format, "vec3f");
		assert_eq!(primitive.vertex_components[0].channel, 0);
		assert_eq!(primitive.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(primitive.vertex_components[1].format, "vec3f");
		assert_eq!(primitive.vertex_components[1].channel, 1);

		assert_eq!(primitive.index_streams.len(), 3);

		let triangle_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();

		assert_eq!(triangle_index_stream.stream_type, IndexStreamTypes::Triangles);
		assert_eq!(triangle_index_stream.count, 36);
		assert_eq!(triangle_index_stream.data_type, IntegralTypes::U16);

		let vertex_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Vertices).unwrap();

		assert_eq!(vertex_index_stream.stream_type, IndexStreamTypes::Vertices);
		assert_eq!(vertex_index_stream.count, 24);
		assert_eq!(vertex_index_stream.data_type, IntegralTypes::U16);

		let meshlet_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Meshlets).unwrap();

		assert_eq!(meshlet_index_stream.stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(meshlet_index_stream.count, 36);
		assert_eq!(meshlet_index_stream.data_type, IntegralTypes::U8);

		let meshlet_stream_info = primitive.meshlet_stream.as_ref().unwrap();

		assert_eq!(meshlet_stream_info.count, 1);

		let resource_request = smol::block_on(resource_manager.request_resource("Box"));

		let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

		let mut vertex_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = resource_request.resources.into_iter().next().unwrap();

		let request = match resource.class.as_str() {
			"Mesh" => {
				LoadResourceRequest::new(resource).streams(vec![Stream{ buffer: vertex_buffer.as_mut_slice(), name: "Vertex".to_string() }, Stream{ buffer: index_buffer.as_mut_slice(), name: "TriangleIndices".to_string() }])
			}
			_ => { panic!("Invalid resource type") }
		};

		let load_request = LoadRequest::new(vec![request]);

		let resource = if let Ok(a) = smol::block_on(resource_manager.load_resource(load_request,)) { a } else { return; };

		let response = resource.0;

		for resource in &response.resources {
			match resource.class.as_str() {
				"Mesh" => {
					let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

					assert_eq!(buffer[0..(primitive.vertex_count * primitive.vertex_components.size() as u32) as usize], vertex_buffer[0..(primitive.vertex_count * primitive.vertex_components.size() as u32) as usize]);

					assert_eq!(buffer[triangle_index_stream.offset..(triangle_index_stream.offset + triangle_index_stream.count as usize * 2) as usize], index_buffer[0..(triangle_index_stream.count * 2) as usize]);
				}
				_ => {}
			}
		}
	}

	#[test]
	fn load_local_gltf_mesh_with_external_binaries() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(MeshResourceHandler::new());

		let (response, buffer) = smol::block_on(resource_manager.get("Suzanne")).expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Mesh>());

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.sub_meshes.len(), 1);

		let sub_mesh = &mesh.sub_meshes[0];

		assert_eq!(sub_mesh.primitives.len(), 1);

		let primitive = &sub_mesh.primitives[0];

		let _offset = 0usize;

		assert_eq!(primitive.bounding_box, [[-1.336914f32, -0.974609f32, -0.800781f32], [1.336914f32, 0.950195f32, 0.825684f32]]);
		assert_eq!(primitive.vertex_count, 11808);
		assert_eq!(primitive.vertex_components.len(), 4);
		assert_eq!(primitive.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(primitive.vertex_components[0].format, "vec3f");
		assert_eq!(primitive.vertex_components[0].channel, 0);
		assert_eq!(primitive.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(primitive.vertex_components[1].format, "vec3f");
		assert_eq!(primitive.vertex_components[1].channel, 1);

		assert_eq!(primitive.index_streams.len(), 3);

		let triangle_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();

		assert_eq!(triangle_index_stream.stream_type, IndexStreamTypes::Triangles);
		// assert_eq!(vertex_index_stream.offset, offset);
		assert_eq!(triangle_index_stream.count, 3936 * 3);
		assert_eq!(triangle_index_stream.data_type, IntegralTypes::U16);

		let vertex_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Vertices).unwrap();

		assert_eq!(vertex_index_stream.stream_type, IndexStreamTypes::Vertices);
		// assert_eq!(mesh.index_streams[0].offset, offset);
		assert_eq!(vertex_index_stream.count, 3936 * 3);
		assert_eq!(vertex_index_stream.data_type, IntegralTypes::U16);

		let meshlet_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Meshlets).unwrap();

		assert_eq!(meshlet_index_stream.stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(meshlet_index_stream.count, 3936 * 3);
		assert_eq!(meshlet_index_stream.data_type, IntegralTypes::U8);

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 11808);

		assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
		assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
		assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(11808), primitive.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 11808);

		assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
		assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
		assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);

		let triangle_indices = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(triangle_index_stream.offset) as *const u16, triangle_index_stream.count as usize) };

		assert_eq!(triangle_indices[0..3], [0, 1, 2]);
		assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);
	}

	#[test]
	fn load_with_manager_buffer() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(MeshResourceHandler::new());

		let (response, buffer) = smol::block_on(resource_manager.get("Box")).expect("Failed to get resource");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		let mesh = resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.sub_meshes.len(), 1);

		let sub_mesh = &mesh.sub_meshes[0];

		assert_eq!(sub_mesh.primitives.len(), 1);

		let primitive = &sub_mesh.primitives[0];

		assert_eq!(primitive.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(primitive.vertex_count, 24);
		assert_eq!(primitive.vertex_components.len(), 2);
		assert_eq!(primitive.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(primitive.vertex_components[0].format, "vec3f");
		assert_eq!(primitive.vertex_components[0].channel, 0);
		assert_eq!(primitive.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(primitive.vertex_components[1].format, "vec3f");
		assert_eq!(primitive.vertex_components[1].channel, 1);

		assert_eq!(primitive.index_streams.len(), 3);

		let triangle_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();

		assert_eq!(triangle_index_stream.stream_type, IndexStreamTypes::Triangles);
		assert_eq!(triangle_index_stream.count, 36);
		assert_eq!(triangle_index_stream.data_type, IntegralTypes::U16);

		let vertex_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Vertices).unwrap();

		assert_eq!(vertex_index_stream.stream_type, IndexStreamTypes::Vertices);
		assert_eq!(vertex_index_stream.count, 24);
		assert_eq!(vertex_index_stream.data_type, IntegralTypes::U16);

		let meshlet_index_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Meshlets).unwrap();

		assert_eq!(meshlet_index_stream.stream_type, IndexStreamTypes::Meshlets);
		assert_eq!(meshlet_index_stream.count, 36);
		assert_eq!(meshlet_index_stream.data_type, IntegralTypes::U8);

		let meshlet_stream_info = primitive.meshlet_stream.as_ref().unwrap();

		assert_eq!(meshlet_stream_info.count, 1);

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 24);
		assert_eq!(vertex_positions[0], [-0.5f32, -0.5f32, -0.5f32]);
		assert_eq!(vertex_positions[1], [0.5f32, -0.5f32, -0.5f32]);
		assert_eq!(vertex_positions[2], [-0.5f32, 0.5f32, -0.5f32]);

		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(24), primitive.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 24);
		assert_eq!(vertex_normals[0], [0f32, 0f32, -1f32]);
		assert_eq!(vertex_normals[1], [0f32, 0f32, -1f32]);
		assert_eq!(vertex_normals[2], [0f32, 0f32, -1f32]);

		// let indeces = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(vertex_index_stream.offset) as *const u16, vertex_index_stream.count as usize) };

		// assert_eq!(indeces.len(), 24);
		// assert_eq!(indeces[0], 0);
		// assert_eq!(indeces[1], 1);
		// assert_eq!(indeces[2], 2);
	}

	#[test]
	fn load_with_vertices_and_indices_with_provided_buffer() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(MeshResourceHandler::new());

		let resource_request = smol::block_on(resource_manager.request_resource("Box")).expect("Failed to request resource");

		let mut vertex_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = resource_request.resources.into_iter().next().unwrap();

		let resource = match resource.class.as_str() {
			"Mesh" => {
				LoadResourceRequest::new(resource).streams(vec![Stream{ buffer: vertex_buffer.as_mut_slice(), name: "Vertex".to_string() }, Stream{ buffer: index_buffer.as_mut_slice(), name: "TriangleIndices".to_string() }])
			}
			_ => { panic!("Invalid resource type") }
		};

		let request = LoadRequest::new(vec![resource]);

		let resource = if let Ok(a) = smol::block_on(resource_manager.load_resource(request,)) { a } else { return; };

		let response = resource.0;

		for resource in &response.resources {
			match resource.class.as_str() {
				"Mesh" => {
					let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

					assert_eq!(mesh.sub_meshes.len(), 1);

					let sub_mesh = &mesh.sub_meshes[0];

					assert_eq!(sub_mesh.primitives.len(), 1);

					let primitive = &sub_mesh.primitives[0];

					let triangle_indices_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();

					let vertex_positions = unsafe { std::slice::from_raw_parts(vertex_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

					assert_eq!(vertex_positions.len(), 24);
					assert_eq!(vertex_positions[0], [-0.5f32, -0.5f32, -0.5f32]);
					assert_eq!(vertex_positions[1], [0.5f32, -0.5f32, -0.5f32]);
					assert_eq!(vertex_positions[2], [-0.5f32, 0.5f32, -0.5f32]);

					let vertex_normals = unsafe { std::slice::from_raw_parts((vertex_buffer.as_ptr() as *const [f32; 3]).add(24) as *const [f32; 3], primitive.vertex_count as usize) };

					assert_eq!(vertex_normals.len(), 24);
					assert_eq!(vertex_normals[0], [0f32, 0f32, -1f32]);
					assert_eq!(vertex_normals[1], [0f32, 0f32, -1f32]);
					assert_eq!(vertex_normals[2], [0f32, 0f32, -1f32]);

					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, triangle_indices_stream.count as usize) };

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

		resource_manager.add_resource_handler(MeshResourceHandler::new());

		let resource_request = smol::block_on(resource_manager.request_resource("Box")).expect("Failed to request resource");

		let mut vertex_positions_buffer = vec![0u8; 1024];
		let mut vertex_normals_buffer = vec![0u8; 1024];
		let mut index_buffer = vec![0u8; 1024];

		let resource = resource_request.resources.into_iter().next().unwrap();

		let resource = match resource.class.as_str() {
			"Mesh" => {
				LoadResourceRequest::new(resource).streams(vec![
					Stream{ buffer: vertex_positions_buffer.as_mut_slice(), name: "Vertex.Position".to_string() },
					Stream{ buffer: vertex_normals_buffer.as_mut_slice(), name: "Vertex.Normal".to_string() },
					Stream{ buffer: index_buffer.as_mut_slice(), name: "TriangleIndices".to_string() }
				])
			}
			_ => { panic!("Invalid resource type") }
		};

		let request = LoadRequest::new(vec![resource]);

		let resource = if let Ok(a) = smol::block_on(resource_manager.load_resource(request,)) { a } else { return; };

		let response = resource.0;

		for resource in &response.resources {
			match resource.class.as_str() {
				"Mesh" => {
					let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

					assert_eq!(mesh.sub_meshes.len(), 1);

					let sub_mesh = &mesh.sub_meshes[0];

					assert_eq!(sub_mesh.primitives.len(), 1);

					let primitive = &sub_mesh.primitives[0];

					let triangle_indices_stream = primitive.index_streams.iter().find(|stream| stream.stream_type == IndexStreamTypes::Triangles).unwrap();

					let vertex_positions_buffer = unsafe { std::slice::from_raw_parts(vertex_positions_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

					assert_eq!(vertex_positions_buffer.len(), 24);
					assert_eq!(vertex_positions_buffer[0], [-0.5f32, -0.5f32, -0.5f32]);
					assert_eq!(vertex_positions_buffer[1], [0.5f32, -0.5f32, -0.5f32]);
					assert_eq!(vertex_positions_buffer[2], [-0.5f32, 0.5f32, -0.5f32]);

					let vertex_normals_buffer = unsafe { std::slice::from_raw_parts(vertex_normals_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

					assert_eq!(vertex_normals_buffer.len(), 24);
					assert_eq!(vertex_normals_buffer[0], [0f32, 0f32, -1f32]);
					assert_eq!(vertex_normals_buffer[1], [0f32, 0f32, -1f32]);
					assert_eq!(vertex_normals_buffer[2], [0f32, 0f32, -1f32]);

					let index_buffer = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, triangle_indices_stream.count as usize) };

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
	#[ignore="This test is too heavy."]
	fn load_glb() {
		let mut resource_manager = ResourceManager::new();
		let resource_handler = MeshResourceHandler::new();

		assert!(resource_handler.can_handle_type("glb"));

		resource_manager.add_resource_handler(resource_handler);
		resource_manager.add_resource_handler(ImageResourceHandler::new()); // Needed for the textures

		let result = smol::block_on(resource_manager.get("Revolver")).expect("Failed to process resource");

		// let resource = result.iter().find(|r| {
		// 	match r {
		// 		ProcessedResources::Generated(g) => g.0.url == "Revolver",
		// 		_ => false
		// 	}
		// }).expect("Failed to find resource");

		// let resource = match resource {
		// 	ProcessedResources::Generated(g) => g.0.clone(),
		// 	_ => panic!("Invalid resource type")
		// };

		let (response, buffer) = result;

		let resource = response.resources.iter().find(|r| r.url == "Revolver").expect("Failed to find resource");

		let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.sub_meshes.len(), 27);

		// let unique_materials = mesh.sub_meshes.iter().map(|s_m| s_m.primitives.iter()).map(|p| p.map(|p| p.material.name.clone()).collect::<Vec<_>>()).flatten().collect::<Vec<_>>().iter().cloned().collect::<std::collections::HashSet<_>>();

		// assert_eq!(unique_materials.len(), 5);

		// let image_resources = response.resources.iter().filter(|r| r.class == "Image" || r.class == "Texture");

		// assert_eq!(image_resources.count(), 17);
	}
}