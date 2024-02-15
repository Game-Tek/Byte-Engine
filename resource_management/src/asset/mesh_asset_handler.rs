use smol::future::FutureExt;

use crate::{resource::resource_manager::ResourceManager, types::{AlphaMode, CompressionSchemes, CreateImage, Formats, IndexStream, IndexStreamTypes, IntegralTypes, Material, Mesh, MeshletStream, Primitive, Property, SubMesh, Value, VertexComponent, VertexSemantics}, GenericResourceSerialization, ProcessedResources};

use super::asset_handler::AssetHandler;

struct MeshAssetHandler {

}

impl MeshAssetHandler {
	fn new() -> MeshAssetHandler {
		MeshAssetHandler {}
	}
}

impl AssetHandler for MeshAssetHandler {
	fn load(&self, url: &str, json: &json::JsonValue) -> utils::BoxedFuture<Option<Result<(), String>>> {
		async move {
			let (gltf, buffers, images) = gltf::import(url).or(Err("Could not process GLTF file."))?;

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
						quantization: CompressionSchemes::None,
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
		}.boxed()
	}
}