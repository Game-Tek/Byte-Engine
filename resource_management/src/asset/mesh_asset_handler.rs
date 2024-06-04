use maths_rs::{mat::{MatNew4, MatScale}, vec::Vec3};
use utils::Extent;

use crate::{ asset::{get_base, get_fragment, image_asset_handler::{guess_semantic_from_name, Semantic}}, material::VariantModel, mesh::{MeshModel, PrimitiveModel}, types::{Formats, Gamma, IndexStreamTypes, IntegralTypes, Stream, Streams, VertexComponent, VertexSemantics}, Description, GenericResourceSerialization, StorageBackend};

use super::{asset_handler::AssetHandler, asset_manager::AssetManager};

pub struct MeshAssetHandler {}

impl MeshAssetHandler {
    pub fn new() -> MeshAssetHandler {
        MeshAssetHandler {}
    }
}

impl AssetHandler for MeshAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "gltf" || r#type == "glb"
	}

    fn load<'a>(&'a self, asset_manager: &'a AssetManager, storage_backend: &'a dyn StorageBackend, url: &'a str, json: Option<&'a json::JsonValue>,) -> utils::SendSyncBoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
    	Box::pin(async move {
            if let Some(dt) = storage_backend.get_type(url) {
                if dt != "gltf" && dt != "glb" {
                    return Err("Not my type".to_string());
                }
            }

			let path: String = if cfg!(test) {
				"../assets/".to_string() + get_base(url).ok_or("Bad URL".to_string())?
			} else {
				"assets/".to_string() + get_base(url).ok_or("Bad URL".to_string())?
			};

			let (data, spec, dt) = storage_backend.resolve(url).await.or(Err("Failed to resolve asset".to_string()))?;

			let (gltf, buffers) = if dt == "glb" {
				let glb = gltf::Glb::from_slice(&data).map_err(|e| e.to_string())?;
				let gltf = gltf::Gltf::from_slice(&glb.json).map_err(|e| e.to_string())?;
				let buffers = gltf::import_buffers(&gltf, None, glb.bin.as_ref().map(|b| b.iter().map(|e| *e).collect())).map_err(|e| e.to_string())?;
				(gltf, buffers)
			} else {
				let gltf = gltf::Gltf::open(path).map_err(|e| e.to_string())?;
				
				let buffers = if let Some(bin_file) = gltf.buffers().find_map(|b| if let gltf::buffer::Source::Uri(r) = b.source() { if r.ends_with(".bin") { Some(r) } else { None } } else { None }) {
					let (bin, _, _) = storage_backend.resolve(bin_file).await.or(Err("Failed to resolve binary file"))?;
					gltf.buffers().map(|_| {
						gltf::buffer::Data(bin.clone())
					}).collect::<Vec<_>>()
				} else {
					gltf::import_buffers(&gltf, None, None).map_err(|e| e.to_string())?
				};

				(gltf, buffers)
			};

			if let Some(fragment) = get_fragment(url) {
				let image = gltf.images().find(|i| i.name() == Some(fragment.as_str())).ok_or("Image not found")?;
				let image = gltf::image::Data::from_source(image.source(), None, &buffers).map_err(|e| e.to_string())?;
				let format = match image.format {
					gltf::image::Format::R8G8B8 => Formats::RGB8,
					gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
					gltf::image::Format::R16G16B16 => Formats::RGB16,
					gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
					_ => return Err("Unsupported image format".to_string()),
				};
				let extent = Extent::rectangle(image.width, image.height);

				let semantic = guess_semantic_from_name(&url);

				let image_description = crate::asset::image_asset_handler::ImageDescription {
					format,
					extent,
					semantic,
					gamma: if semantic == Semantic::Albedo { Gamma::SRGB } else { Gamma::Linear },
				};

				let resource = asset_manager.produce(&url, "image/png", &image_description, &image.pixels).await;

				return Ok(Some(resource));
			}

			let spec = if let Some(spec) = spec {
				spec
			} else {
				log::error!("No spec found for {}", url);
				return Err("Need .bead file".to_string());
			};

            const MESHLETIZE: bool = true;

            // for mesh in gltf.meshes() {
            //     for primitive in mesh.primitives() {
            //         {
            //             let material = primitive.material();

            //             // Return the name of the texture
            //             async fn manage_image<'x>(
            //                 images: &'x [gltf::image::Data],
            //                 texture: &'x gltf::Texture<'x>,
            //             ) -> Result<(String, ()), String> {
            //                 let image = &images[texture.source().index()];

            //                 let format = match image.format {
            //                     gltf::image::Format::R8G8B8 => Formats::RGB8,
            //                     gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
            //                     gltf::image::Format::R16G16B16 => Formats::RGB16,
            //                     gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
            //                     _ => return Err("Unsupported image format".to_string()),
            //                 };

            //                 let name = texture.source().name().ok_or("No image name")?.to_string();

            //                 Ok((name, ()))
            //             }

            //             let pbr = material.pbr_metallic_roughness();

            //             let albedo = if let Some(base_color_texture) = pbr.base_color_texture() {
            //                 let (name, resource) = manage_image(images.as_slice(), &base_color_texture.texture()).await.or_else(|e| Err(e))?;
            //                 resources.push(resource);
            //                 Property::Texture(name)
            //             } else {
            //                 let color = pbr.base_color_factor();
            //                 Property::Factor(Value::Vector4(color))
            //             };

            //             let (roughness, metallic) =
            //                 if let Some(roughness_texture) = pbr.metallic_roughness_texture() {
            //                     (
            //                         {
            //                             let (name, resource) = manage_image(
            //                                 images.as_slice(),
            //                                 &roughness_texture.texture(),
            //                             )
            //                             .await.or_else(|e| Err(e))?;
            //                             resources.push(resource);
            //                             Property::Texture(name)
            //                         },
            //                         {
            //                             let (name, resource) = manage_image(
            //                                 images.as_slice(),
            //                                 &roughness_texture.texture(),
            //                             )
            //                             .await.or_else(|e| Err(e))?;
            //                             resources.push(resource);
            //                             Property::Texture(name)
            //                         },
            //                     )
            //                 } else {
            //                     (
            //                         Property::Factor(Value::Scalar(pbr.roughness_factor())),
            //                         Property::Factor(Value::Scalar(pbr.metallic_factor())),
            //                     )
            //                 };

            //             let normal = if let Some(normal_texture) = material.normal_texture() {
            //                 let (name, resource) =
            //                     manage_image(images.as_slice(), &normal_texture.texture())
            //                         .await.or_else(|e| Err(e))?;
            //                 resources.push(resource);
            //                 Property::Texture(name)
            //             } else {
            //                 Property::Factor(Value::Vector3([0.0, 0.0, 1.0]))
            //             };

            //             let emissive = if let Some(emissive_texture) = material.emissive_texture() {
            //                 let (name, resource) =
            //                     manage_image(images.as_slice(), &emissive_texture.texture())
            //                         .await.or_else(|e| Err(e))?;
            //                 resources.push(resource);
            //                 Property::Texture(name)
            //             } else {
            //                 Property::Factor(Value::Vector3(material.emissive_factor()))
            //             };

            //             let occlusion =
            //                 if let Some(occlusion_texture) = material.occlusion_texture() {
            //                     let (name, resource) =
            //                         manage_image(images.as_slice(), &occlusion_texture.texture())
            //                             .await.or_else(|e| Err(e))?;
            //                     resources.push(resource);
            //                     Property::Texture(name)
            //                 } else {
            //                     Property::Factor(Value::Scalar(1.0))
            //                 };

            //             Material {
            //                 double_sided: material.double_sided(),
            //                 alpha_mode: match material.alpha_mode() {
            //                     gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            //                     gltf::material::AlphaMode::Mask => {
            //                         AlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5))
            //                     }
            //                     gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            //                 },
            //                 model: Model {
            //                     name: "".to_string(),
            //                     pass: "".to_string(),
            //                 },
			// 				shaders: Vec::new(),
			// 				parameters: Vec::new(),
            //             };
            //         }
			// 	}
			// }

			// Gather vertex components and check that they are all equal
			let all = gltf.meshes().map(|mesh| {
                mesh.primitives().map(|primitive| {
					primitive.attributes().scan(0, |state, (semantic, _)| {
						let channel = *state;

						*state += 1;

						match semantic {
							gltf::Semantic::Positions => VertexComponent {
								semantic: VertexSemantics::Position,
								format: "vec3f".to_string(),
								channel,
							},
							gltf::Semantic::Normals => VertexComponent {
								semantic: VertexSemantics::Normal,
								format: "vec3f".to_string(),
								channel,
							},
							gltf::Semantic::Tangents => VertexComponent {
								semantic: VertexSemantics::Tangent,
								format: "vec3f".to_string(),
								channel,
							},
							gltf::Semantic::Colors(_) => todo!(),
							gltf::Semantic::TexCoords(count) => {
								VertexComponent {
									semantic: VertexSemantics::UV,
									format: "vec2f".to_string(),
									channel,
								}
							},
							gltf::Semantic::Joints(_) => todo!(),
							gltf::Semantic::Weights(_) => todo!(),
						}.into()
					}).collect::<Vec<VertexComponent>>()
				})
			}).flatten();

			let vertex_layouts = all.collect::<Vec<Vec<VertexComponent>>>();
			let vertex_layout = vertex_layouts.first().unwrap().clone();

			fn flatten_tree(base: maths_rs::Mat4f, node: gltf::Node) -> Vec<(gltf::Node, maths_rs::Mat4f)> {
				let transform = node.transform().matrix();
				let transform = base * maths_rs::Mat4f::new(transform[0][0], transform[1][0], transform[2][0], transform[3][0], transform[0][1], transform[1][1], transform[2][1], transform[3][1], transform[0][2], transform[1][2], transform[2][2], transform[3][2], transform[0][3], transform[1][3], transform[2][3], transform[3][3]);

				let mut nodes = vec![(node.clone(), transform)];

				for child in node.children() {
					nodes.extend(flatten_tree(transform, child));
				}

				nodes
			}

			let mut flat_tree = gltf.scenes().map(|scene| {
				scene.nodes().map(|node| {
					flatten_tree(maths_rs::Mat4f::identity(), node)
				}).flatten()
			}).flatten().collect::<Vec<(gltf::Node, maths_rs::Mat4f)>>();

			for (_, transform) in &mut flat_tree {
				*transform = maths_rs::Mat4f::from_scale(Vec3::new(1f32, 1f32, -1f32)) * *transform; // Make vertices left-handed
			}

			let primitives = flat_tree.iter().filter_map(|(node, transform)| {
				node.mesh().map(|mesh| mesh.primitives().map(|primitive| (primitive, *transform)))
			}).flatten().collect::<Vec<_>>();

			let flat_mesh_tree = {
				primitives.iter().map(|(primitive, transform)| {
					(primitive, primitive.reader(|buffer| Some(&buffers[buffer.index()])), *transform)
				})
			};

			let materials_per_primitive = flat_mesh_tree.clone().map(move |(primitive, _, _)| {
				let asset = &spec["asset"];

				let gltf_material = primitive.material();
				let gltf_material_name = gltf_material.name().unwrap();

				let material = &asset[gltf_material_name];
				let material_asset_name = material["asset"].as_str().unwrap();

				smol::block_on(asset_manager.load_typed_resource::<VariantModel>(material_asset_name)).unwrap()
			});

			let vertex_counts = flat_mesh_tree.clone().map(|(_, reader, _)| {
				if let Some(positions) = reader.read_positions() {
					positions.clone().count()
				} else {
					panic!("We should not be here");
				}
			}).collect::<Vec<usize>>();

			enum MeshBuilds {
				// Join all primitives into one mesh big mesh with a contiguous index buffer
				Whole,
				// Each primitive is a separate mesh
				Primitive,
			}

			let mesh_vertex_count = vertex_counts.iter().sum::<usize>();

			// Create vertex count prefix sum, from 0
			let vertex_prefix_sum = vertex_counts.iter().scan(0, |state, &x| {
				let old = *state;
				*state += x;
				Some(old)
			}).collect::<Vec<usize>>();

			let (mesh, buffer) = match MeshBuilds::Primitive {
				MeshBuilds::Primitive => {
					let buffer_blocks = [Streams::Vertices(VertexSemantics::Position), Streams::Vertices(VertexSemantics::Normal), Streams::Vertices(VertexSemantics::UV), Streams::Indices(IndexStreamTypes::Vertices), Streams::Indices(IndexStreamTypes::Meshlets), Streams::Meshlets];

					let indices_per_primitive = flat_mesh_tree.clone().map(|(a, reader, _)| {
						let vertex_count = reader.read_positions().unwrap().len();
						let indices = reader.read_indices().unwrap().into_u32().collect::<Vec<u32>>();
						meshopt::optimize_vertex_cache(&indices, vertex_count)
					}).collect::<Vec<Vec<u32>>>();

					// let indices_per_primitive = flat_mesh_tree.clone().zip(vertex_prefix_sum.iter()).map(|((_, reader, _), vps)| {
					// 	let vertex_count = reader.read_positions().unwrap().len();
					// 	let indices = reader.read_indices().unwrap().into_u32().map(|i| i + *vps as u32).collect::<Vec<u32>>();
					// 	meshopt::optimize_vertex_cache(&indices, vertex_count)
					// }).collect::<Vec<Vec<u32>>>();

					let vertices_per_primitive = flat_mesh_tree.clone().map(|(_, reader, transform)| {
						if let Some(positions) = reader.read_positions() {
							positions.map(|position| {
								let position = maths_rs::Vec3f::new(position[0], position[1], position[2]);
								let transformed_position = transform * position;
								transformed_position.iter().map(|m| m.to_le_bytes()).flatten().collect::<Vec<u8>>()
							}).flatten().collect::<Vec<u8>>()
						} else {
							panic!("We should not be here");
						}
					}).collect::<Vec<Vec<u8>>>();

					let meshlets_per_primitive = vertices_per_primitive.iter().zip(indices_per_primitive.iter()).map(|(vertices, indices)| {
						meshopt::clusterize::build_meshlets(&indices, &meshopt::VertexDataAdapter::new(&vertices, 12, 0).unwrap(), 64, 124, 0.0f32)
					}).collect::<Vec<meshopt::Meshlets>>();

					let blocks = buffer_blocks.iter().map(|&block| {
						match block {
							Streams::Vertices(VertexSemantics::Position) => {
								vertices_per_primitive.clone() // TODO: try to avoid cloning
							}
							Streams::Vertices(VertexSemantics::Normal) => {
								flat_mesh_tree.clone().map(|(_, reader, transform)| {
									if let Some(normals) = reader.read_normals() {
										normals.map(|normal| {
											let normal = maths_rs::Vec3f::new(normal[0], normal[1], normal[2]);
											
											let transformed_normal = transform * normal;
			
											transformed_normal.iter().map(|m| m.to_le_bytes()).flatten().collect::<Vec<u8>>()
										}).flatten().collect::<Vec<u8>>()
									} else {
										panic!("We should not be here");
									}
								}).collect::<Vec<Vec<u8>>>()
							}
							Streams::Vertices(VertexSemantics::UV) => {
								flat_mesh_tree.clone().map(|(_, reader, _)|{
									(0..1).map(|i| {
										if let Some(uv) = reader.read_tex_coords(i) {
											assert_eq!(i, 0);
											uv.into_f32().map(|uv| {
												uv.iter().map(|m| { m.to_le_bytes() }).flatten().collect::<Vec<u8>>()
											}).flatten()
										} else {
											panic!("We should not be here");
										}
									}).flatten().collect::<Vec<u8>>()
								}).collect::<Vec<Vec<u8>>>()
							}
							Streams::Indices(IndexStreamTypes::Vertices) => {
								#[allow(unused_variables)]
								meshlets_per_primitive.iter().zip(vertex_prefix_sum.iter()).map(|(meshlets, vps)| {
									let index_type = IntegralTypes::U16;

									let max_size = match index_type {
										IntegralTypes::U16 => 0xFFFFu32,
										IntegralTypes::U32 => 0xFFFFFFFFu32,
										_ => panic!("Unsupported index type"),
									};

									debug_assert!(meshlets.iter().all(|e| e.vertices.iter().all(|e| *e <= max_size)), "Vertex index out of bounds");

									match index_type {
										IntegralTypes::U16 => {
											meshlets.iter().map(|e| e.vertices.iter().map(|i| (*i as u16).to_le_bytes())).flatten().flatten().collect::<Vec<u8>>() // Indices per primitive
											// meshlets.iter().map(|e| e.vertices.iter().map(|i| (*i as u16 + *vps as u16).to_le_bytes())).flatten().flatten().collect::<Vec<u8>>() // Indices per mesh
										}
										IntegralTypes::U32 => {
											meshlets.iter().map(|e| e.vertices.iter().map(|i| *i)).flatten().map(|e| e.to_le_bytes()).flatten().collect::<Vec<u8>>()
										}
										_ => panic!("Unsupported index type"),
									}
								}).collect::<Vec<Vec<u8>>>()
							}
							Streams::Indices(IndexStreamTypes::Meshlets) => {
								meshlets_per_primitive.iter().map(|meshlets| {
									debug_assert!(meshlets.iter().all(|e| e.triangles.iter().all(|e| *e <= 64)), "Meshlet vertex index out of bounds");

									meshlets.iter().map(|e| e.triangles.iter().map(|i| *i)).flatten().collect::<Vec<u8>>()
								}).collect::<Vec<Vec<u8>>>()
							}
							Streams::Indices(IndexStreamTypes::Triangles) => {
								indices_per_primitive.iter().map(|indices| {
									let index_type = IntegralTypes::U16;

									let max_value = match index_type {
										IntegralTypes::U16 => 0xFFFFu32,
										IntegralTypes::U32 => 0xFFFFFFFFu32,
										_ => panic!("Unsupported index type"),
									};

									debug_assert!(indices.iter().all(|e| *e <= max_value), "Index out of bounds");

									match index_type {
										IntegralTypes::U16 => {
											indices.iter().map(|i| *i as u16).map(|e| e.to_le_bytes()).flatten().collect::<Vec<u8>>()
										}
										IntegralTypes::U32 => {
											indices.iter().map(|i| *i).map(|e| e.to_le_bytes()).flatten().collect::<Vec<u8>>()
										}
										_ => panic!("Unsupported index type"),
									}
								}).collect::<Vec<Vec<u8>>>()
							}
							Streams::Meshlets => {
								meshlets_per_primitive.iter().map(|meshlets| {
									meshlets.iter().map(|meshlet| {
										let vertices = meshlet.vertices.len() as u8;
										let triangles = (meshlet.triangles.len() / 3) as u8;
										[vertices, triangles]
									}).flatten().collect::<Vec<u8>>()
								}).collect::<Vec<Vec<u8>>>()
							}
							_ => todo!()
						}
					}).collect::<Vec<Vec<Vec<u8>>>>();

					let primitives = flat_mesh_tree.clone().enumerate().zip(materials_per_primitive).map(|((i, (primitive, reader, _)), material)| {
						let global = false;

						let streams = if global {
							buffer_blocks.iter().zip(blocks.iter()).scan(0, |state, (streams, primitives)| {
								// This offset is global
								let offset = *state + primitives.iter().take(i).map(|e| e.len()).sum::<usize>();
								let size = primitives[i].len();

								*state += primitives.iter().map(|e| e.len()).sum::<usize>();

								Stream {
									offset,
									size,
									stream_type: *streams,
									stride: match streams {
										Streams::Vertices(VertexSemantics::Position) => 12,
										Streams::Vertices(VertexSemantics::Normal) => 12,
										Streams::Vertices(VertexSemantics::UV) => 8,
										Streams::Indices(IndexStreamTypes::Vertices) => 2,
										Streams::Indices(IndexStreamTypes::Meshlets) => 1,
										Streams::Indices(IndexStreamTypes::Triangles) => 2,
										Streams::Meshlets => 2,
										_ => panic!("Unsupported stream type"),
									},
								}.into()
							}).collect::<Vec<_>>()
						} else {
							buffer_blocks.iter().zip(blocks.iter()).map(|(streams, primitives)| {
								// This offset is per stream
								let offset = primitives.iter().take(i).map(|e| e.len()).sum::<usize>();
								let size = primitives[i].len();

								Stream {
									offset,
									size,
									stream_type: *streams,
									stride: match streams {
										Streams::Vertices(VertexSemantics::Position) => 12,
										Streams::Vertices(VertexSemantics::Normal) => 12,
										Streams::Vertices(VertexSemantics::UV) => 8,
										Streams::Indices(IndexStreamTypes::Vertices) => 2,
										Streams::Indices(IndexStreamTypes::Meshlets) => 1,
										Streams::Indices(IndexStreamTypes::Triangles) => 2,
										Streams::Meshlets => 2,
										_ => panic!("Unsupported stream type"),
									},
								}.into()
							}).collect::<Vec<_>>()
						};

						PrimitiveModel {
							material,
							streams,
							quantization: None,
							bounding_box: make_bounding_box(primitive),
							vertex_count: reader.read_positions().unwrap().len() as u32,
						}
					}).collect::<Vec<_>>();

					let streams = buffer_blocks.iter().zip(blocks.iter()).scan(0usize, |state, (streams, block)| {
						let offset = *state;
						let size = block.iter().map(|e| e.len()).sum::<usize>();
						*state += size;
						Stream {
							offset,
							size,
							stream_type: *streams,
							stride: match streams {
								Streams::Vertices(VertexSemantics::Position) => 12,
								Streams::Vertices(VertexSemantics::Normal) => 12,
								Streams::Vertices(VertexSemantics::UV) => 8,
								Streams::Indices(IndexStreamTypes::Vertices) => 2,
								Streams::Indices(IndexStreamTypes::Meshlets) => 1,
								Streams::Indices(IndexStreamTypes::Triangles) => 2,
								Streams::Meshlets => 2,
								_ => panic!("Unsupported stream type"),
							},
						}.into()
					}).collect::<Vec<_>>();

					(MeshModel {
						streams,
						primitives,
						vertex_components: vertex_layout,
					},
					blocks.into_iter().flatten().flatten().collect::<Vec<u8>>())
				}
				MeshBuilds::Whole => {
					panic!("Not implemented");
				}
			};			

            let resource_document = GenericResourceSerialization::new(url, mesh);
            storage_backend.store(&resource_document, &buffer).await;

            Ok(Some(resource_document))
        })
    }
}

struct MeshDescription {
}

impl Description for MeshDescription {
	fn get_resource_class() -> &'static str where Self: Sized {
		"Mesh"
	}
}

fn make_bounding_box(mesh: &gltf::Primitive) -> [[f32; 3]; 2] {
    let bounds = mesh.bounding_box();

    [
        [bounds.min[0], bounds.min[1], bounds.min[2]],
        [bounds.max[0], bounds.max[1], bounds.max[2]],
    ]
}

#[cfg(test)]
mod tests {
    use super::MeshAssetHandler;
    use crate::asset::{
        asset_handler::AssetHandler, asset_manager::AssetManager, image_asset_handler::ImageAssetHandler, material_asset_handler::{tests::RootTestShaderGenerator, MaterialAssetHandler}, tests::TestStorageBackend
    };

    #[test]
    fn load_gltf() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("shader.besl", "main: fn () -> void {}".as_bytes());
		storage_backend.add_file("Box.bema", r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "shader.besl"
			},
			"variables": []
		}"#.as_bytes());
		storage_backend.add_file("Texture.bema", r#"{
			"parent": "Box.bema",
			"variables": []
		}"#.as_bytes());
		storage_backend.add_file("Box.glb.bead", r#"{"asset": {"Texture": {"asset": "Texture.bema" }}}"#.as_bytes());

		let mut asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new());
        let asset_handler = MeshAssetHandler::new();
		asset_manager.add_asset_handler({
			let mut material_asset_handler = MaterialAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});

        let url = "Box.glb";

        smol::block_on(asset_handler.load(&asset_manager, &storage_backend, &url, None)).unwrap().expect("Failed to parse asset");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, url);
        assert_eq!(resource.class, "Mesh");

        // TODO: ASSERT BINARY DATA
    }

	#[test]
    fn load_gltf_with_bin() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("shader.besl", "main: fn () -> void {}".as_bytes());
		storage_backend.add_file("Material.bema", r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "shader.besl"
			},
			"variables": []
		}"#.as_bytes());
		storage_backend.add_file("Suzanne.bema", r#"{
			"parent": "Material.bema",
			"variables": []
		}"#.as_bytes());
		storage_backend.add_file("Suzanne.gltf.bead", r#"{"asset": {"Suzanne": {"asset": "Suzanne.bema" }}}"#.as_bytes());

		let mut asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new());
		asset_manager.add_asset_handler({
			let mut material_asset_handler = MaterialAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});
        let asset_handler = MeshAssetHandler::new();

        let url = "Suzanne.gltf";

        smol::block_on(asset_handler.load(&asset_manager, &storage_backend, &url, None)).expect("Mesh asset handler did not handle asset").expect("Failed to parse asset");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, url);
        assert_eq!(resource.class, "Mesh");

        // TODO: ASSERT BINARY DATA

		// let vertex_count = resource.resource.as_document().unwrap().get_i64("vertex_count").unwrap() as usize;

		// assert_eq!(vertex_count, 11808);
		let vertex_count = 11808;

		let buffer = storage_backend.get_resource_data_by_name(url).unwrap();

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], vertex_count) };

		assert_eq!(vertex_positions.len(), 11808);

		assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
		assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
		assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

		let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(11808), vertex_count) };

		assert_eq!(vertex_normals.len(), 11808);

		assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
		assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
		assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);

		// let triangle_indices = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(triangle_index_stream.offset) as *const u16, triangle_index_stream.count as usize) };

		// assert_eq!(triangle_indices[0..3], [0, 1, 2]);
		// assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);
    }

    #[test]
    #[ignore="Test uses data not pushed to the repository"]
    fn load_glb() {
		let storage_backend = TestStorageBackend::new();

		storage_backend.add_file("shader.besl", "main: fn () -> void {}".as_bytes());
		storage_backend.add_file("Material.bema", r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "shader.besl"
			},
			"variables": [
				{
					"name": "color",
					"data_type": "Texture2D",
					"value": "Revolver.glb#Revolver_Base_color"
				},
				{
					"name": "normalll",
					"data_type": "Texture2D",
					"value": "Revolver.glb#Revolver_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"data_type": "Texture2D",
					"value": "Revolver.glb#Revolver_Metallic-Revolver_Roughness"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("Revolver.bema", r#"{
			"parent": "PBR.bema",
			"variables": [
				{
					"name": "color",
					"value": "Revolver.glb#Revolver_Base_color"
				},
				{
					"name": "normalll",
					"value": "Revolver.glb#Revolver_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"value": "Revolver.glb#Revolver_Metallic-Revolver_Roughness"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("Material.001.bema", r#"{
			"parent": "PBR.bema",
			"variables": [
				{
					"name": "color",
					"value": "Revolver.glb#Material.001_Base_color"
				},
				{
					"name": "normalll",
					"value": "Revolver.glb#Material.001_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"value": "Revolver.glb#Material.001_Metallic-Material.001_Roughness"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("RedDotScopeLens.bema", r#"{
			"parent": "PBR.bema",
			"variables": [
				{
					"name": "color",
					"value": "Revolver.glb#RedDotScopeLens_Base_color"
				},
				{
					"name": "normalll",
					"value": "Revolver.glb#RedDotScopeLens_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"value": "Revolver.glb#RedDotScopeLens_Metallic-RedDotScopeLens_Roughness"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("RedDotScopeDot.bema", r#"{
			"parent": "PBR.bema",
			"variables": [
				{
					"name": "color",
					"value": "Revolver.glb#RedDotScopeDot_Base_color-RedDotScopeDot_Opacity.png"
				},
				{
					"name": "normalll",
					"value": "Revolver.glb#RedDotScopeDot_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"value": "Revolver.glb#RedDotScopeDot_Metallic.png-RedDotScopeDot_Roughness.png"
				},
				{
					"name": "emissive",
					"value": "Revolver.glb#RedDotScopeDot_Emissive"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("FlashLight.bema", r#"{
			"parent": "PBR.bema",
			"variables": [
				{
					"name": "color",
					"value": "Revolver.glb#FlashLight_Base_color"
				},
				{
					"name": "normalll",
					"value": "Revolver.glb#FlashLight_Normal_OpenGL"
				},
				{
					"name": "metallic_roughness",
					"value": "Revolver.glb#FlashLight_Metallic-FlashLight_Roughness"
				},
				{
					"name": "emissive",
					"value": "Revolver.glb#FlashLight_Emissive"
				}
			]
		}"#.as_bytes());
		storage_backend.add_file("Revolver.glb.bead", r#"{
			"asset": {
				"Revolver": {
					"asset": "Revolver.bema"
				},
				"Material.001": {
					"asset": "Material.001.bema"
				},
				"RedDotScopeLens": {
					"asset": "RedDotScopeLens.bema"
				},
				"RedDotScopeDot": {
					"asset": "RedDotScopeDot.bema"
				},
				"FlashLight": {
					"asset": "FlashLight.bema"
				}
			}
		}"#.as_bytes());

		let mut asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new());
		asset_manager.add_asset_handler({
			let mut material_asset_handler = MaterialAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});
		asset_manager.add_asset_handler({
			ImageAssetHandler::new()
		});
		asset_manager.add_asset_handler({
			MeshAssetHandler::new()
		});
        let storage_backend = TestStorageBackend::new();
        let asset_handler = MeshAssetHandler::new();

        let url = "Revolver.glb";

        let _ = smol::block_on(asset_handler.load(&asset_manager, &storage_backend, &url, None)).unwrap().unwrap();

		let buffer = storage_backend.get_resource_data_by_name("Revolver.glb").unwrap();

		let generated_resources = storage_backend.get_resources();

		let resource = &generated_resources[0];

		// let vertex_count = resource.resource.as_document().unwrap().get_i64("vertex_count").unwrap() as usize;
		let vertex_count = 27022;

		assert_eq!(vertex_count, 27022);

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], vertex_count) };

		assert_eq!(vertex_positions.len(), 27022);

		// assert_eq!(vertex_positions[0], [-0.00322f32, -0.00197f32, -0.00322f32]);
		// assert_eq!(vertex_positions[1], [-0.00174f32, -0.00197f32, -0.00420f32]);
		// assert_eq!(vertex_positions[2], [0.00000f32, -0.00197f32, -0.00455f32]);
		
		assert_eq!(vertex_positions[27019], [-0.112022735, -0.0056253895, 0.013142529]);
		assert_eq!(vertex_positions[27020], [-0.112022735, -0.0056253895, 0.013142529]);
		assert_eq!(vertex_positions[27021], [-0.112022735, -0.0056253895, 0.013142529]);
    }

	#[test]
    #[ignore="Test uses data not pushed to the repository"]
    fn load_glb_image() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), storage_backend);
		
        let asset_handler = MeshAssetHandler::new();
		
		let image_asset_handler = ImageAssetHandler::new();
		
		asset_manager.add_asset_handler(image_asset_handler);

		let storage_backend = asset_manager.get_storage_backend().downcast_ref::<TestStorageBackend>().unwrap();

        let url = "Revolver.glb#Revolver_Metallic-Revolver_Roughness";

        let _ = smol::block_on(asset_handler.load(&asset_manager, storage_backend, &url, None));

		let _ = storage_backend.get_resource_data_by_name(&url).unwrap();

		let generated_resources = storage_backend.get_resources();

		let resource = &generated_resources[0];

		assert_eq!(resource.class, "Image");
    }

	#[test]
	#[ignore]
	fn load_16bit_normal_image() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), storage_backend);
		asset_manager.add_asset_handler(ImageAssetHandler::new());
		let asset_handler = MeshAssetHandler::new();

		let url = "Revolver.glb#Revolver_Normal_OpenGL";

		let _ = smol::block_on(asset_handler.load(&asset_manager, asset_manager.get_storage_backend(), &url, None)).unwrap().expect("Image asset handler did not handle asset");

		// let generated_resources = asset_manager.get_storage_backend().get_resources();

		// assert_eq!(generated_resources.len(), 1);

		// let resource = &generated_resources[0];

		// assert_eq!(resource.id, url);
		// assert_eq!(resource.class, "Image");
	}
}
