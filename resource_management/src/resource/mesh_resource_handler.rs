use polodb_core::bson;
use serde::Deserialize;

use crate::{types::{IndexStreamTypes, Mesh, MeshModel, Streams, VertexSemantics}, GenericResourceResponse, Reference, ReferenceModel, ResourceResponse, Solver, StorageBackend};

use super::resource_handler::{ReadTargets, ResourceHandler, ResourceReader};

pub struct MeshResourceHandler {

}

impl MeshResourceHandler {
	pub fn new() -> Self {
		Self {}
	}
}

impl ResourceHandler for MeshResourceHandler {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&["Mesh"]
	}

	fn read<'s, 'a, 'b>(&'s self, mut resource: GenericResourceResponse<'a>, reader: Option<Box<dyn ResourceReader>>, s: &'b dyn StorageBackend) -> utils::BoxedFuture<'b, Option<ResourceResponse<'a>>> where 'a: 'b {
		Box::pin(async move {
			let re = ReferenceModel::new(&resource.id, resource.hash);
			let r: Reference<Mesh> = re.solve(s).unwrap();
			let mesh_resource = r.resource();

			if let Some(mut reader) = reader {
				let mut buffers = if let Some(read_target) = &mut resource.read_target {
					match read_target {
						ReadTargets::Streams(streams) => {
							streams.iter_mut().map(|b| {
								(b.name, utils::BufferAllocator::new(b.buffer))
							}).collect::<Vec<_>>()
						}
						_ => {
							return None;
						}
						
					}
				} else {
					let mut buffer = Vec::with_capacity(resource.size);
					unsafe {
						buffer.set_len(resource.size);
					}
					reader.read_into(0, &mut buffer).await?;
	
					panic!();
				};

				for (name, buffer) in &mut buffers {
					let stream = match *name {
						"Vertex.Position" => {
							mesh_resource.position_stream()
						}
						"Vertex.Normal" => {
							mesh_resource.normal_stream()
						}
						"Vertex.Tangent" => {
							mesh_resource.tangent_stream()
						}
						"Vertex.UV" => {
							mesh_resource.uv_stream()
						}
						"TriangleIndices" => {
							mesh_resource.triangle_indices_stream()
						}
						"VertexIndices" => {
							mesh_resource.vertex_indices_stream()
						}
						"MeshletIndices" => {
							mesh_resource.meshlet_indices_stream()
						}
						"Meshlets" => {
							mesh_resource.meshlets_stream()
						}
						_ => {
							log::error!("Unknown buffer tag: {}", name);
							None
						}
					};

					if let Some(stream) = stream {
						reader.read_into(stream.offset, buffer.take(stream.size)).await?;
					} else {
						log::error!("Failed to read stream: {}", name);
					}
				}
			}

			Some(ResourceResponse::new(resource, mesh_resource.clone()))
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
	use crate::{asset::{asset_handler::AssetHandler, asset_manager::AssetManager, mesh_asset_handler::MeshAssetHandler, tests::{TestAssetResolver, TestStorageBackend}}, types::{IntegralTypes, Streams}, Stream,};
	
	use super::*;

	#[test]
	fn load_suzanne() {
		// Create resource from asset

		let mesh_asset_handler = MeshAssetHandler::new();

		let url = "Suzanne.gltf";

		let asset_resolver = TestAssetResolver::new();
		let asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new(), asset_resolver);
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		smol::block_on(mesh_asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, url, None)).expect("Mesh asset handler did not handle asset").expect("Mesh asset handler failed to load asset");

		// Load resource from storage

		let mesh_resource_handler = MeshResourceHandler::new();

		let (mut resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let mut vertex_positions_buffer = vec![0; 11808 * 12];
		let mut vertex_normals_buffer = vec![0; 11808 * 12];
		let mut index_buffer = vec![0; 11808 * 2];
		let mut meshlet_buffer = vec![0; 11808 * 2];
		let mut meshlet_index_buffer = vec![0; 11808 * 3];

		unsafe {
			vertex_positions_buffer.set_len(11808 * 12);
			vertex_normals_buffer.set_len(11808 * 12);
			index_buffer.set_len(11808 * 2);
			meshlet_buffer.set_len(11808 * 1);
			meshlet_index_buffer.set_len(11808 * 3);
		}

		let streams = vec![Stream::new("Vertex.Position", &mut vertex_positions_buffer), Stream::new("Vertex.Normal", &mut vertex_normals_buffer), Stream::new("TriangleIndices", &mut index_buffer), Stream::new("Meshlets", &mut meshlet_buffer)];

		resource.set_streams(streams);

		let resource = smol::block_on(mesh_resource_handler.read(resource, Some(reader), &storage_backend)).unwrap();

		let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.vertex_components.len(), 4);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);

		if let Some(triangle_index_stream) = mesh.triangle_indices_stream() {
			assert_eq!(triangle_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Triangles));
			assert_eq!(triangle_index_stream.stride, 2);
			assert_eq!(triangle_index_stream.size / triangle_index_stream.stride, 3936 * 3);

			let triangle_indices = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, triangle_index_stream.size / triangle_index_stream.stride) };

			assert_eq!(triangle_indices[0..3], [0, 1, 2]);
			assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);
		}

		let vertex_index_stream = mesh.vertex_indices_stream().unwrap();

		assert_eq!(vertex_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Vertices));
		assert_eq!(vertex_index_stream.stride, 2);
		assert_eq!(vertex_index_stream.size / vertex_index_stream.stride, 3936 * 3);

		let meshlet_index_stream = mesh.meshlet_indices_stream().unwrap();

		assert_eq!(meshlet_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Meshlets));
		assert_eq!(meshlet_index_stream.stride, 1);
		assert_eq!(meshlet_index_stream.size / meshlet_index_stream.stride, 3936 * 3);

		let primitive = &mesh.primitives[0];

		let _offset = 0usize;

		assert_eq!(primitive.bounding_box, [[-1.336914f32, -0.974609f32, -0.800781f32], [1.336914f32, 0.950195f32, 0.825684f32]]);
		assert_eq!(primitive.vertex_count, 11808);

		let vertex_positions = unsafe { std::slice::from_raw_parts(vertex_positions_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 11808);

		assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
		assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
		assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

		let vertex_normals = unsafe { std::slice::from_raw_parts(vertex_normals_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 11808);

		assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
		assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
		assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);
	}

	#[test]
	fn load_box_streams() {
		// Create resource from asset

		let mesh_asset_handler = MeshAssetHandler::new();

		let url = "Box.glb";

		let asset_resolver = TestAssetResolver::new();
		let asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new(), asset_resolver);
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		smol::block_on(mesh_asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, url, None)).expect("Mesh asset handler did not handle asset").expect("Mesh asset handler failed to load asset");

		// Load resource from storage

		let mesh_resource_handler = MeshResourceHandler::new();

		let (mut resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let mut vertex_positions_buffer = vec![0; 24 * 12];
		let mut vertex_normals_buffer = vec![0; 24 * 12];
		let mut index_buffer = vec![0; 36 * 2];
		let mut meshlet_buffer = vec![0; 36 * 1];
		let mut meshlet_index_buffer = vec![0; 36 * 3];

		unsafe {
			vertex_positions_buffer.set_len(24 * 12);
			vertex_normals_buffer.set_len(24 * 12);
			index_buffer.set_len(36 * 2);
			meshlet_buffer.set_len(36 * 1);
			meshlet_index_buffer.set_len(36 * 3);
		}

		let streams = vec![Stream::new("Vertex.Position", &mut vertex_positions_buffer), Stream::new("Vertex.Normal", &mut vertex_normals_buffer), Stream::new("TriangleIndices", &mut index_buffer), Stream::new("Meshlets", &mut meshlet_buffer)];

		resource.set_streams(streams);

		let resource = smol::block_on(mesh_resource_handler.read(resource, Some(reader), &storage_backend)).unwrap();

		let mesh = resource.resource.downcast_ref::<Mesh>().unwrap();

		assert_eq!(mesh.vertex_components.len(), 3);
		assert_eq!(mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert_eq!(mesh.vertex_components[0].format, "vec3f");
		assert_eq!(mesh.vertex_components[0].channel, 0);
		assert_eq!(mesh.vertex_components[1].semantic, VertexSemantics::Normal);
		assert_eq!(mesh.vertex_components[1].format, "vec3f");
		assert_eq!(mesh.vertex_components[1].channel, 1);
		assert_eq!(mesh.vertex_components[2].semantic, VertexSemantics::UV);
		assert_eq!(mesh.vertex_components[2].format, "vec2f");
		assert_eq!(mesh.vertex_components[2].channel, 2);

		assert_eq!(mesh.primitives.len(), 1);

		let primitive = &mesh.primitives[0];

		assert_eq!(primitive.streams.len(), 6);

		if let Some(triangle_index_stream) = primitive.streams.iter().find(|stream| stream.stream_type == Streams::Indices(IndexStreamTypes::Triangles)) {
			assert_eq!(triangle_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Triangles));
			assert_eq!(triangle_index_stream.stride, 2);
			assert_eq!(triangle_index_stream.size / triangle_index_stream.stride, 36);

			let indeces = unsafe { std::slice::from_raw_parts(index_buffer.as_ptr() as *const u16, triangle_index_stream.size / triangle_index_stream.stride) };

			assert_eq!(indeces.len(), 24);
			assert_eq!(indeces[0], 0);
			assert_eq!(indeces[1], 1);
			assert_eq!(indeces[2], 2);
		}

		let vertex_index_stream = primitive.streams.iter().find(|stream| stream.stream_type == Streams::Indices(IndexStreamTypes::Vertices)).unwrap();

		assert_eq!(vertex_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Vertices));
		assert_eq!(vertex_index_stream.stride, 2);
		assert_eq!(vertex_index_stream.size / vertex_index_stream.stride, 24);

		let meshlet_index_stream = primitive.streams.iter().find(|stream| stream.stream_type == Streams::Indices(IndexStreamTypes::Meshlets)).unwrap();

		assert_eq!(meshlet_index_stream.stream_type, Streams::Indices(IndexStreamTypes::Meshlets));
		assert_eq!(meshlet_index_stream.stride, 1);
		assert_eq!(meshlet_index_stream.size / meshlet_index_stream.stride, 36);

		let meshlet_stream_info = primitive.streams.iter().find(|stream| stream.stream_type == Streams::Meshlets).unwrap();

		assert_eq!(meshlet_stream_info.stream_type, Streams::Meshlets);
		assert_eq!(meshlet_stream_info.stride, 2);
		assert_eq!(meshlet_stream_info.size / meshlet_stream_info.stride, 1);

		assert_eq!(primitive.bounding_box, [[-0.5f32, -0.5f32, -0.5f32], [0.5f32, 0.5f32, 0.5f32]]);
		assert_eq!(primitive.vertex_count, 24);

		let vertex_positions = unsafe { std::slice::from_raw_parts(vertex_positions_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_positions.len(), 24);
		assert_eq!(vertex_positions[0], [-0.5f32, 0.5f32, -0.5f32]);
		assert_eq!(vertex_positions[1], [0.5f32, 0.5f32, -0.5f32]);
		assert_eq!(vertex_positions[2], [-0.5f32, 0.5f32, 0.5f32]);

		let vertex_normals = unsafe { std::slice::from_raw_parts(vertex_normals_buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

		assert_eq!(vertex_normals.len(), 24);
		assert_eq!(vertex_normals[0], [0f32, 1f32, 0f32]);
		assert_eq!(vertex_normals[1], [0f32, 1f32, 0f32]);
		assert_eq!(vertex_normals[2], [0f32, 1f32, 0f32]);
	}
}