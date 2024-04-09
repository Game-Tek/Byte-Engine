use std::primitive;

use maths_rs::mat::MatNew4;
use utils::Extent;

use crate::{
    types::{
        AlphaMode, CreateImage, Formats, Image, IndexStream, IndexStreamTypes, IntegralTypes, Material, Mesh, MeshletStream, Model, Primitive, Property, SubMesh, Value, VertexComponent, VertexSemantics
    }, Description, GenericResourceResponse, GenericResourceSerialization, Resource, StorageBackend, TypedResource
};

use super::{asset_handler::AssetHandler, asset_manager::AssetManager, AssetResolver};

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

    fn load<'a>(&'a self, asset_manager: &'a AssetManager, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, url: &'a str, json: Option<&'a json::JsonValue>,) -> utils::BoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
    	Box::pin(async move {
            if let Some(dt) = asset_resolver.get_type(url) {
                if dt != "gltf" && dt != "glb" {
                    return Err("Not my type".to_string());
                }
            }

			let path: String = if cfg!(test) {
				"../assets/".to_string() + asset_resolver.get_base(url).ok_or("Bad URL".to_string())?
			} else {
				"assets/".to_string() + asset_resolver.get_base(url).ok_or("Bad URL".to_string())?
			};

			let (gltf, buffers, images) = match gltf::import(path) {
				Ok((gltf, buffers, images)) => (gltf, buffers, images),
				Err(e) => return Err(e.to_string()),
			};

			if let Some(fragment) = asset_resolver.get_fragment(url) {
				let image = gltf.images().find(|i| i.name() == Some(fragment.as_str())).ok_or("Image not found")?;
				let image = &images[image.index()];
				let format = match image.format {
					gltf::image::Format::R8G8B8 => Formats::RGB8,
					gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
					gltf::image::Format::R16G16B16 => Formats::RGB16,
					gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
					_ => return Err("Unsupported image format".to_string()),
				};
				let extent = Extent::rectangle(image.width, image.height);

				let image_description = crate::asset::image_asset_handler::ImageDescription {
					format,
					extent,
				};

				let resource: TypedResource<Image> = asset_manager.produce(&url, "image/png", &image_description, &image.pixels).await;

				return Ok(Some(resource.into()));
			}

            const MESHLETIZE: bool = true;

            let mut buffer = Vec::with_capacity(4096 * 1024 * 3);

            let mut resources = Vec::with_capacity(8);

			let primitives_iterator = gltf.meshes().map(|e| e.primitives()).flatten();

            for mesh in gltf.meshes() {
                for primitive in mesh.primitives() {
                    {
                        let material = primitive.material();

                        // Return the name of the texture
                        async fn manage_image<'x>(
                            images: &'x [gltf::image::Data],
                            texture: &'x gltf::Texture<'x>,
                        ) -> Result<(String, ()), String> {
                            let image = &images[texture.source().index()];

                            let format = match image.format {
                                gltf::image::Format::R8G8B8 => Formats::RGB8,
                                gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
                                gltf::image::Format::R16G16B16 => Formats::RGB16,
                                gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
                                _ => return Err("Unsupported image format".to_string()),
                            };

                            let name = texture.source().name().ok_or("No image name")?.to_string();

                            Ok((name, ()))
                        }

                        let pbr = material.pbr_metallic_roughness();

                        let albedo = if let Some(base_color_texture) = pbr.base_color_texture() {
                            let (name, resource) = manage_image(images.as_slice(), &base_color_texture.texture()).await.or_else(|e| Err(e))?;
                            resources.push(resource);
                            Property::Texture(name)
                        } else {
                            let color = pbr.base_color_factor();
                            Property::Factor(Value::Vector4(color))
                        };

                        let (roughness, metallic) =
                            if let Some(roughness_texture) = pbr.metallic_roughness_texture() {
                                (
                                    {
                                        let (name, resource) = manage_image(
                                            images.as_slice(),
                                            &roughness_texture.texture(),
                                        )
                                        .await.or_else(|e| Err(e))?;
                                        resources.push(resource);
                                        Property::Texture(name)
                                    },
                                    {
                                        let (name, resource) = manage_image(
                                            images.as_slice(),
                                            &roughness_texture.texture(),
                                        )
                                        .await.or_else(|e| Err(e))?;
                                        resources.push(resource);
                                        Property::Texture(name)
                                    },
                                )
                            } else {
                                (
                                    Property::Factor(Value::Scalar(pbr.roughness_factor())),
                                    Property::Factor(Value::Scalar(pbr.metallic_factor())),
                                )
                            };

                        let normal = if let Some(normal_texture) = material.normal_texture() {
                            let (name, resource) =
                                manage_image(images.as_slice(), &normal_texture.texture())
                                    .await.or_else(|e| Err(e))?;
                            resources.push(resource);
                            Property::Texture(name)
                        } else {
                            Property::Factor(Value::Vector3([0.0, 0.0, 1.0]))
                        };

                        let emissive = if let Some(emissive_texture) = material.emissive_texture() {
                            let (name, resource) =
                                manage_image(images.as_slice(), &emissive_texture.texture())
                                    .await.or_else(|e| Err(e))?;
                            resources.push(resource);
                            Property::Texture(name)
                        } else {
                            Property::Factor(Value::Vector3(material.emissive_factor()))
                        };

                        let occlusion =
                            if let Some(occlusion_texture) = material.occlusion_texture() {
                                let (name, resource) =
                                    manage_image(images.as_slice(), &occlusion_texture.texture())
                                        .await.or_else(|e| Err(e))?;
                                resources.push(resource);
                                Property::Texture(name)
                            } else {
                                Property::Factor(Value::Scalar(1.0))
                            };

                        Material {
                            double_sided: material.double_sided(),
                            alpha_mode: match material.alpha_mode() {
                                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                                gltf::material::AlphaMode::Mask => {
                                    AlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5))
                                }
                                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                            },
                            model: Model {
                                name: "".to_string(),
                                pass: "".to_string(),
                            },
							shaders: Vec::new(),
							parameters: Vec::new(),
                        };
                    }
				}
			}

			// Gather vertex components and check that they are all equal
			let all = gltf.meshes().map(|mesh| {
                mesh.primitives().map(|primitive| {
                    let mut vertex_components = Vec::new();

                    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

                    if let Some(_) = reader.read_positions() {
                        vertex_components.push(VertexComponent {
                            semantic: VertexSemantics::Position,
                            format: "vec3f".to_string(),
                            channel: 0,
                        });
                    } else {
                        // return Err("Mesh does not have positions".to_string());
						panic!("Mesh does not have positions");
                    };

					if let Some(_) = reader.read_normals() {
                        vertex_components.push(VertexComponent {
                            semantic: VertexSemantics::Normal,
                            format: "vec3f".to_string(),
                            channel: 1,
                        });
                    }

					if let Some(_) = reader.read_tangents() {
						vertex_components.push(VertexComponent {
							semantic: VertexSemantics::Tangent,
							format: "vec3f".to_string(),
							channel: 2,
						});
					};

					for i in 0..1 {
						if let Some(_) = reader.read_tex_coords(i) {
							assert_eq!(i, 0);
							vertex_components.push(VertexComponent {
								semantic: VertexSemantics::Uv,
								format: "vec2f".to_string(),
								channel: 3,
							});
						}
					}

					vertex_components
				})
			});

			let vertex_layouts = all.flatten().collect::<Vec<Vec<VertexComponent>>>();
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
				transform[2 * 4 + 2] *= -1f32;
			}

			let flat_mesh_tree = {
				let mut c = flat_tree.iter().filter_map(|(node, transform)| {
					if let Some(mesh) = node.mesh() {
						Some((mesh, *transform))
					} else {
						None
					}
				}).collect::<Vec<_>>();
				
				c.sort_by(|a, b| a.0.index().cmp(&b.0.index()));

				c
			};

			let flat_mesh_tree = flat_mesh_tree.iter();

			let vertex_counts = flat_mesh_tree.clone().map(|(mesh, _)| {
				mesh.primitives().map(|primitive| {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

					if let Some(positions) = reader.read_positions() {
						let vertex_count = positions.clone().count();
						vertex_count
					} else {
						panic!("We should not be here");
					}
				}).sum()
			}).collect::<Vec<usize>>();

			let vertex_count = vertex_counts.iter().sum::<usize>();

			// Create vertex count prefix sum, from 0
			let vertex_prefix_sum = vertex_counts.iter().scan(0, |state, &x| {
				let old = *state;
				*state += x;
				Some(old)
			}).collect::<Vec<usize>>();

			let indices = vertex_prefix_sum.iter().zip(flat_mesh_tree.clone()).map(|(accumulated_vertex_count, (mesh, _))| {
				mesh.primitives().filter_map(|primitive| {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
					if let Some(indices) = reader.read_indices() {
						Some(indices.into_u32().map(|i| i + *accumulated_vertex_count as u32).collect::<Vec<u32>>())
					} else {
						None
					}
				}).flatten()
			}).flatten().collect::<Vec<u32>>();

			let indices = meshopt::optimize::optimize_vertex_cache(&indices, vertex_count);

			for (mesh, transform) in flat_mesh_tree.clone() {
				for primitive in mesh.primitives() {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
	
					if let Some(positions) = reader.read_positions() {
						positions.for_each(|position| {
							let position = maths_rs::Vec3f::new(position[0], position[1], position[2]); // Convert from right-handed(GLTF) to left-handed coordinate system
							
							let transformed_position = transform * position;
							
							transformed_position.iter().for_each(|m| {
								m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
							});
						});
					}
				}
			}

			for (mesh, transform) in flat_mesh_tree.clone() {
				for primitive in mesh.primitives() {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
					if let Some(normals) = reader.read_normals() {
						normals.for_each(|normal| {
							let normal = maths_rs::Vec3f::new(normal[0], normal[1], normal[2]);
							
							let transformed_normal = transform * normal;

							transformed_normal.iter().for_each(|m| {
								m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
							});
						});
					}
				}
			}

			for (mesh, transform) in flat_mesh_tree.clone() {
				for primitive in mesh.primitives() {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
					if let Some(tangents) = reader.read_tangents() {
						tangents.for_each(|tangent| {
							let tangent = maths_rs::Vec3f::new(tangent[0], tangent[1], tangent[2]);

							let transformed_tangent = transform * tangent;

							transformed_tangent.iter().for_each(|m| {
								m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
							})
						});
					}
				}
			}

			for (mesh, _) in flat_mesh_tree.clone() {
				for primitive in mesh.primitives() {
					let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));					

					for i in 0..8 {
						if let Some(uv) = reader.read_tex_coords(i) {
							assert_eq!(i, 0);
							uv.into_f32().for_each(|uv| {
								uv.iter().for_each(|m| {
									m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
								})
							});
						}
					}
				}
			}

			// align buffer to 16 bytes for indices
			while buffer.len() % 16 != 0 {
				buffer.push(0);
			}

			let mut index_streams = Vec::with_capacity(4);

			let meshlets = if MESHLETIZE {
				let cone_weight = 0.0f32; // How much to prioritize cone culling over other forms of culling
				let meshlets = meshopt::clusterize::build_meshlets(&indices, &meshopt::VertexDataAdapter::new(&buffer[0..12 * vertex_count], 12, 0).unwrap(), 64, 124, cone_weight,);
				
				{
					let offset = buffer.len();

					let index_type = IntegralTypes::U16;

					match index_type {
						IntegralTypes::U16 => {
							for meshlet in meshlets.iter() {
								for x in meshlet.vertices {
									(*x as u16)
										.to_le_bytes()
										.iter()
										.for_each(|byte| buffer.push(*byte));
								}
							}
						}
						_ => panic!("Unsupported index type"),
					}

					let vertex_index_count = meshlets.iter().map(|e| e.vertices.len()).sum::<usize>();

					index_streams.push(IndexStream {
						data_type: IntegralTypes::U16,
						stream_type: IndexStreamTypes::Vertices,
						offset,
						count: vertex_index_count as u32,
					});
				}

				{
					let offset = buffer.len();

					let mut c = 0;

					for meshlet in meshlets.iter() {
						for x in meshlet.triangles {
							assert!(*x <= 64u8, "Meshlet index out of bounds"); // Max vertices per meshlet
							buffer.push(*x);
							c += 1;
						}
					}

					assert_eq!(c, indices.len());

					index_streams.push(IndexStream {
						data_type: IntegralTypes::U8,
						stream_type: IndexStreamTypes::Meshlets,
						offset,
						count: indices.len() as u32,
					});
				}

				Some(meshlets)
			} else {
				None
			};

			{
				let offset = buffer.len();

				let index_type = IntegralTypes::U16;

				match index_type {
					IntegralTypes::U16 => {
						indices
							.iter()
							.map(|i| {
								assert!(*i <= 0xFFFFu32, "Index out of bounds"); // Max vertices per meshlet
								*i as u16
							})
							.for_each(|i| {
								i.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
							});
					}
					_ => panic!("Unsupported index type"),
				}

				index_streams.push(IndexStream {
					data_type: IntegralTypes::U16,
					stream_type: IndexStreamTypes::Triangles,
					offset,
					count: indices.len() as u32,
				});
			}
				
			let meshlet_stream;

			if let Some(meshlets) = meshlets {
				let offset = buffer.len();

				for meshlet in meshlets.iter() {
					buffer.push(meshlet.vertices.len() as u8);
					buffer.push((meshlet.triangles.len() / 3usize) as u8);
				}

				meshlet_stream = Some(MeshletStream {
					offset,
					count: meshlets.len() as u32,
				});
			} else {
				meshlet_stream = None;
			}

			let sub_meshes = gltf.meshes().map(|mesh| {
				SubMesh {
					primitives: mesh.primitives().map(|primitive| {
						let bounding_box = make_bounding_box(&primitive);

						Primitive {
							// material,
							quantization: None,
							bounding_box,
							vertex_count: primitive.get(&gltf::Semantic::Positions).unwrap().count() as u32,
						}
					}).collect()
				}
			}).collect();

            let mesh = Mesh {
				sub_meshes,
				vertex_components: vertex_layout,
				index_streams,
				meshlet_stream,
				vertex_count: vertex_count as u32,
			};

            let resource_document = GenericResourceSerialization::new(url, mesh);
            storage_backend.store(&resource_document, &buffer).await;

            Ok(Some(resource_document))
        })
    }

	fn produce<'a>(&'a self, _: &'a dyn Description, _: &'a [u8]) -> utils::BoxedFuture<'a, Result<(Box<dyn Resource>, Box<[u8]>), String>> {
		Box::pin(async move {
			Err("Not implemented".to_string())
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
        asset_handler::AssetHandler, asset_manager::AssetManager, image_asset_handler::ImageAssetHandler, tests::{TestAssetResolver, TestStorageBackend}
    };

    #[test]
    fn load_gltf() {
		let asset_manager = AssetManager::new();
        let asset_handler = MeshAssetHandler::new();
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();

        let url = "Box.gltf";

        smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, &url, None)).unwrap().expect("Failed to parse asset");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, "Box.gltf");
        assert_eq!(resource.class, "Mesh");

        // assert_eq!(
        //     resource.resource,
        //     bson::doc! {
        //         "sub_meshes": [
        //             {
        //                 "primitives": [
        //                     {
        //                         "material": {
        //                             "albedo": {
        //                                 "Factor": {
        //                                     "Vector4": [0.800000011920929, 0.0, 0.0, 1.0],
        //                                 }
        //                             },
        //                             "normal": {
        //                                 "Factor": {
        //                                     "Vector3": [0.0, 0.0, 1.0],
        //                                 }
        //                             },
        //                             "roughness": {
        //                                 "Factor": {
        //                                     "Scalar": 1.0,
        //                                 }
        //                             },
        //                             "metallic": {
        //                                 "Factor": {
        //                                     "Scalar": 0.0,
        //                                 }
        //                             },
        //                             "emissive": {
        //                                 "Factor": {
        //                                     "Vector3": [0.0, 0.0, 0.0],
        //                                 }
        //                             },
        //                             "occlusion": {
        //                                 "Factor": {
        //                                     "Scalar": 1.0,
        //                                 }
        //                             },
        //                             "double_sided": false,
        //                             "alpha_mode": "Opaque",
        //                             "model": {
        //                                 "name": "",
        //                                 "pass": "",
        //                             },
        //                         },
        //                         "quantization": null,
        //                         "bounding_box": [[-0.5, -0.5, -0.5],[0.5, 0.5, 0.5],],
        //                         "vertex_count": 24i64,
        //                         "vertex_components": [
        //                             {
        //                                 "semantic": "Position",
        //                                 "format": "vec3f",
        //                                 "channel": 0i64,
        //                             },
        //                             {
        //                                 "semantic": "Normal",
        //                                 "format": "vec3f",
        //                                 "channel": 1i64,
        //                             },
        //                         ],
        //                         "index_streams": [
        //                             {
        //                                 "data_type": "U16",
        //                                 "stream_type": "Vertices",
        //                                 "offset": 576i64,
        //                                 "count": 24i64,
        //                             },
        //                             {
        //                                 "data_type": "U8",
        //                                 "stream_type": "Meshlets",
        //                                 "offset": 624i64,
        //                                 "count": 36i64,
        //                             },
        //                             {
        //                                 "data_type": "U16",
        //                                 "stream_type": "Triangles",
        //                                 "offset": 662i64,
        //                                 "count": 36i64,
        //                             },
        //                         ],
        //                         "meshlet_stream": {
        //                             "offset": 660i64,
        //                             "count": 1i64,
        //                         },
        //                     },
        //                 ],
        //             },
        //         ],
        //     }.into()
        // );

        // TODO: ASSERT BINARY DATA
    }

	#[test]
    fn load_gltf_with_bin() {
		let asset_manager = AssetManager::new();
        let asset_handler = MeshAssetHandler::new();
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();

        let url = "Suzanne.gltf";

        smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, &url, None)).expect("Mesh asset handler did not handle asset").expect("Failed to parse asset");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, "Suzanne.gltf");
        assert_eq!(resource.class, "Mesh");

        // TODO: ASSERT BINARY DATA

		let vertex_count = resource.resource.as_document().unwrap().get_i64("vertex_count").unwrap() as usize;

		assert_eq!(vertex_count, 11808);

		let buffer = storage_backend.get_resource_data_by_name("Suzanne.gltf").unwrap();

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
		let asset_manager = AssetManager::new();
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();
        let asset_handler = MeshAssetHandler::new();

        let url = "Revolver.glb";

        let _ = smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, &url, None));

		let buffer = storage_backend.get_resource_data_by_name("Revolver.glb").unwrap();

		let generated_resources = storage_backend.get_resources();

		let resource = &generated_resources[0];

		let vertex_count = resource.resource.as_document().unwrap().get_i64("vertex_count").unwrap() as usize;

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
		let mut asset_manager = AssetManager::new_with_storage_backend(TestStorageBackend::new());
		
        let asset_resolver = TestAssetResolver::new();
		
        let asset_handler = MeshAssetHandler::new();
		
		let image_asset_handler = ImageAssetHandler::new();
		
		asset_manager.add_asset_handler(image_asset_handler);

		let storage_backend = asset_manager.get_storage_backend().downcast_ref::<TestStorageBackend>().unwrap();

        let url = "Revolver.glb#Revolver_Base_color";

        let _ = smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, storage_backend, &url, None));

		let _ = storage_backend.get_resource_data_by_name("Revolver.glb#Revolver_Base_color").unwrap();

		let generated_resources = storage_backend.get_resources();

		let resource = &generated_resources[0];

		assert_eq!(resource.class, "Image");
    }
}
