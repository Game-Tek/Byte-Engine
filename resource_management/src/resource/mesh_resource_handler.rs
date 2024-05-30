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