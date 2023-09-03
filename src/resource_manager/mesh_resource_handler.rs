use std::io::{Seek, Read};

use polodb_core::bson::{Document, doc};
use serde::{Serialize, Deserialize};

use super::{ResourceHandler};

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

	fn process(&self, bytes: &[u8]) -> Result<Vec<(Document, Vec<u8>)>, String> {
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

		let doc = doc!{
			"class": "Mesh",
			"resource": mesh.serialize(polodb_core::bson::Serializer::new()).unwrap()
		};

		Ok(vec![(doc, buf)])
	}

	fn get_deserializer(&self) -> Box<dyn Fn(&Document) -> Box<dyn std::any::Any> + Send> {
		Box::new(|document| {
			let mesh = Mesh::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(mesh)
		})
	}

	fn read(&self, resource: &Box<dyn std::any::Any>, file: &mut std::fs::File, buffers: &mut [super::Buffer]) {
		let mesh: &Mesh = resource.downcast_ref().unwrap();

		for buffer in buffers {
			match buffer.tag.as_str() {
				"Vertex" => {
					file.seek(std::io::SeekFrom::Start(0)).unwrap();
					file.read(buffer.buffer).unwrap();
				}
				"Index" => {
					let base_offset = mesh.vertex_count as u64 * mesh.vertex_components.size() as u64;
					let rounded_offset = base_offset + (16 - base_offset % 16);
					file.seek(std::io::SeekFrom::Start(rounded_offset));
					file.read(buffer.buffer).unwrap();
				}
				_ => {}
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