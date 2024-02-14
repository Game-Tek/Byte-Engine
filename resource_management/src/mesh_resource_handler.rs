use polodb_core::bson::doc;
use serde::{Serialize, Deserialize};
use smol::{fs::File, io::{AsyncReadExt, AsyncSeekExt}};

use crate::{texture_resource_handler::{CreateImage, Formats}, CreateInfo};

use super::{GenericResourceSerialization, Resource, ProcessedResources, resource_handler::ResourceHandler, resource_manager::ResourceManager, Stream};

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
			"gltf" | "glb" => true,
			_ => false
		}
	}

	fn process<'a>(&'a self, resource_manager: &'a ResourceManager, asset_url: &'a str) -> utils::BoxedFuture<Result<Vec<ProcessedResources>, String>> {
		Box::pin(async move {
			let result = gltf::import(resource_manager.realize_asset_path(asset_url).ok_or("Could not find asset file.")?);
			let (gltf, buffers, images) = result.or(Err("Could not process GLTF file."))?;

			const MESHLETIZE: bool = true;

			let mut buffer = Vec::with_capacity(4096 * 1024 * 3);

			let mut resources = Vec::with_capacity(8);

			let mut sub_meshes = Vec::with_capacity(gltf.meshes().count());

			for mesh in gltf.meshes() {
				let mut primitives = Vec::with_capacity(mesh.primitives().count());

				for primitive in mesh.primitives() {
					let material = {
						let material = primitive.material();

						// Return the name of the texture
						async fn manage_image<'x>(resource_manager: &'x ResourceManager, images: &'x [gltf::image::Data], texture: &'x gltf::Texture<'x>) -> Result<(String, ProcessedResources), String> {
							let image = &images[texture.source().index()];

							let format = match image.format {
								gltf::image::Format::R8G8B8 => Formats::RGB8,
								gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
								gltf::image::Format::R16G16B16 => Formats::RGB16,
								gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
								_ => return Err("Unsupported image format".to_string()),
							};

							let name = texture.source().name().ok_or("No image name")?.to_string();

							let create_image_info = CreateImage {
								format,
								extent: [image.width, image.height, 1],
							};

							let created_texture_resource = resource_manager.create_resource(&name, "Image", create_image_info, &image.pixels).await.ok_or("Failed to create texture")?;

							Ok((name, created_texture_resource))
						}

						let pbr = material.pbr_metallic_roughness();

						let albedo = if let Some(base_color_texture) = pbr.base_color_texture() {
							let (name, resource) = manage_image(resource_manager, images.as_slice(), &base_color_texture.texture()).await?;
							resources.push(resource);
							Property::Texture(name)
						} else {
							let color = pbr.base_color_factor();
							Property::Factor(Value::Vector4(color))
						};

						let (roughness, metallic) = if let Some(roughness_texture) = pbr.metallic_roughness_texture() {

							({
								let (name, resource) = manage_image(resource_manager, images.as_slice(), &roughness_texture.texture()).await?;
								resources.push(resource);
								Property::Texture(name)
							}, {
								let (name, resource) = manage_image(resource_manager, images.as_slice(), &roughness_texture.texture()).await?;
								resources.push(resource);
								Property::Texture(name)
							})
						} else {
							(Property::Factor(Value::Scalar(pbr.roughness_factor())), Property::Factor(Value::Scalar(pbr.metallic_factor())))
						};

						let normal = if let Some(normal_texture) = material.normal_texture() {
							let (name, resource) = manage_image(resource_manager, images.as_slice(), &normal_texture.texture()).await?;
							resources.push(resource);
							Property::Texture(name)
						} else {
							Property::Factor(Value::Vector3([0.0, 0.0, 1.0]))
						};

						let emissive = if let Some(emissive_texture) = material.emissive_texture() {
							let (name, resource) = manage_image(resource_manager, images.as_slice(), &emissive_texture.texture()).await?;
							resources.push(resource);
							Property::Texture(name)
						} else {
							Property::Factor(Value::Vector3(material.emissive_factor()))
						};

						let occlusion = if let Some(occlusion_texture) = material.occlusion_texture() {
							let (name, resource) = manage_image(resource_manager, images.as_slice(), &occlusion_texture.texture()).await?;
							resources.push(resource);
							Property::Texture(name)
						} else {
							Property::Factor(Value::Scalar(1.0))
						};

						Material {
							name: material.name().unwrap_or("Unnamed").to_string(),
							albedo,
							normal,
							roughness,
							metallic,
							emissive,
							occlusion,
							double_sided: material.double_sided(),
							alpha_mode: match material.alpha_mode() {
								gltf::material::AlphaMode::Blend => AlphaMode::Blend,
								gltf::material::AlphaMode::Mask => AlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5)),
								gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
							}
						}
					};

					let mut vertex_components = Vec::new();

					let bounding_box = Self::make_bounding_box(&primitive);

					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

					let vertex_count = if let Some(positions) = reader.read_positions() {
						let vertex_count = positions.clone().count();
						positions.for_each(|mut position| {
							position[2] = -position[2]; // Convert from right-handed(GLTF) to left-handed coordinate system
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
							normal[2] = -normal[2]; // Convert from right-handed(GLTF) to left-handed coordinate system
							normal.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte)));
						});

						vertex_components.push(VertexComponent { semantic: VertexSemantics::Normal, format: "vec3f".to_string(), channel: 1 });
					}
		
					if let Some(tangents) = reader.read_tangents() {
						tangents.for_each(|tangent| tangent.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))));
						vertex_components.push(VertexComponent { semantic: VertexSemantics::Tangent, format: "vec4f".to_string(), channel: 2 });
					}

					for i in 0..8 {
						if let Some(uv) = reader.read_tex_coords(i) {
							assert_eq!(i, 0);
							uv.into_f32().for_each(|uv| uv.iter().for_each(|m| m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))));
							vertex_components.push(VertexComponent { semantic: VertexSemantics::Uv, format: "vec3f".to_string(), channel: 3 });
						}
					}
		
					// align buffer to 16 bytes for indices
					while buffer.len() % 16 != 0 { buffer.push(0); }

					let mut index_streams = Vec::with_capacity(2);

					let meshlet_stream;

					if MESHLETIZE {
						let cone_weight = 0.0f32; // How much to prioritize cone culling over other forms of culling
						let meshlets = meshopt::clusterize::build_meshlets(&optimized_indices, &meshopt::VertexDataAdapter::new(&buffer[0..12 * vertex_count], 12, 0).unwrap(), 64, 124, cone_weight);

						let offset = buffer.len();

						{
							let index_type = IntegralTypes::U16;
							
							match index_type {
								IntegralTypes::U16 => {
									let mut index_count = 0usize;
									for meshlet in meshlets.iter() {
										index_count += meshlet.vertices.len();
										for x in meshlet.vertices {
											(*x as u16).to_le_bytes().iter().for_each(|byte| buffer.push(*byte));
										}
									}
									index_streams.push(IndexStream{ data_type: IntegralTypes::U16, stream_type: IndexStreamTypes::Vertices, offset, count: index_count as u32 });
								}
								_ => panic!("Unsupported index type")
							}
						}
		
						let offset = buffer.len();

						for meshlet in meshlets.iter() {
							for x in meshlet.triangles {
								assert!(*x <= 64u8, "Meshlet index out of bounds"); // Max vertices per meshlet
								buffer.push(*x);
							}
						}

						index_streams.push(IndexStream{ data_type: IntegralTypes::U8, stream_type: IndexStreamTypes::Meshlets, offset, count: optimized_indices.len() as u32 });

						let offset = buffer.len();

						meshlet_stream = Some(MeshletStream{ offset, count: meshlets.len() as u32 });

						for meshlet in meshlets.iter() {
							buffer.push(meshlet.vertices.len() as u8);
							buffer.push((meshlet.triangles.len() / 3usize) as u8); // TODO: add tests for this
						}
					} else {
						meshlet_stream = None;
					}

					let add_triangle_stream_even_if_using_meshlets = true;

					if !MESHLETIZE || add_triangle_stream_even_if_using_meshlets {
						let offset = buffer.len();

						let index_type = IntegralTypes::U16;

						match index_type {
							IntegralTypes::U16 => {
								optimized_indices.iter().map(|i| {
									assert!(*i <= 0xFFFFu32, "Index out of bounds"); // Max vertices per meshlet
									*i as u16
								}).for_each(|i| i.to_le_bytes().iter().for_each(|byte| buffer.push(*byte)));
								index_streams.push(IndexStream{ data_type: IntegralTypes::U16, stream_type: IndexStreamTypes::Triangles, offset, count: optimized_indices.len() as u32 });
							}
							_ => panic!("Unsupported index type")
						}
					}

					primitives.push(Primitive {
						material,
						compression: CompressionSchemes::None,
						bounding_box,
						vertex_components,
						vertex_count: vertex_count as u32,
						index_streams,
						meshlet_stream,
					});
				}
				
				sub_meshes.push(SubMesh {
					primitives,
				});
			}
			
			let mesh = Mesh {
				sub_meshes,
			};

			let resource_document = GenericResourceSerialization::new(asset_url.to_string(), mesh);
			resources.push(ProcessedResources::Generated((resource_document, buffer)));

			Ok(resources)
		})
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
	Vertices,
	Meshlets,
	Triangles,
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
pub enum Value {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Property {
	Factor(Value),
	Texture(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Material {
	name: String,
	albedo: Property,
	normal: Property,
	roughness: Property,
	metallic: Property,
	emissive: Property,
	occlusion: Property,
	double_sided: bool,
	alpha_mode: AlphaMode,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Primitive {
	pub material: Material,
	pub compression: CompressionSchemes,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_components: Vec<VertexComponent>,
	pub vertex_count: u32,
	pub index_streams: Vec<IndexStream>,
	pub meshlet_stream: Option<MeshletStream>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubMesh {
	pub primitives: Vec<Primitive>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Mesh {
	pub sub_meshes: Vec<SubMesh>,
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
	use crate::{resource_manager::ResourceManager, Stream, LoadRequest, LoadResourceRequest};

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
		use crate::texture_resource_handler::ImageResourceHandler;

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

		let unique_materials = mesh.sub_meshes.iter().map(|s_m| s_m.primitives.iter()).map(|p| p.map(|p| p.material.name.clone()).collect::<Vec<_>>()).flatten().collect::<Vec<_>>().iter().cloned().collect::<std::collections::HashSet<_>>();

		assert_eq!(unique_materials.len(), 5);

		// let image_resources = response.resources.iter().filter(|r| r.class == "Image" || r.class == "Texture");

		// assert_eq!(image_resources.count(), 17);
	}
}