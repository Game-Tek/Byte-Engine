use smol::future::FutureExt;

use crate::{
    resource::resource_manager::ResourceManager,
    types::{
        AlphaMode, CompressionSchemes, CreateImage, Formats, IndexStream, IndexStreamTypes,
        IntegralTypes, Material, Mesh, MeshletStream, Model, Primitive, Property, SubMesh, Value,
        VertexComponent, VertexSemantics,
    },
    GenericResourceSerialization, ProcessedResources, StorageBackend,
};

use super::{asset_handler::AssetHandler, AssetResolver,};

pub struct MeshAssetHandler {}

impl MeshAssetHandler {
    pub fn new() -> MeshAssetHandler {
        MeshAssetHandler {}
    }
}

impl AssetHandler for MeshAssetHandler {
    fn load<'a>(
        &'a self,
        asset_resolver: &'a dyn AssetResolver,
        storage_backend: &'a dyn StorageBackend,
        id: &'a str,
        json: &'a json::JsonValue,
    ) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
    	Box::pin(async move {
			let url = json["url"].as_str().ok_or("No url").ok()?;

            if let Some(dt) = asset_resolver.get_type(url) {
                if dt != "gltf" && dt != "glb" {
                    return None;
                }
            }

            // let (data, dt) = asset_resolver.resolve(url).await?;

            // if dt != "gltf" && dt != "glb" {
            //     return None;
            // }

			let path: String = if cfg!(test) {
				"../assets/".to_string() + url
			} else {
				"assets/".to_string() + url
			};

            let (gltf, buffers, images) = gltf::import(path).unwrap();

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

                            let create_image_info = CreateImage {
                                format,
                                extent: [image.width, image.height, 1],
                            };

                            // let created_texture_resource = resource_manager.create_resource(&name, "Image", create_image_info, &image.pixels).await.ok_or("Failed to create texture")?;

                            Ok((name, ()))
                        }

                        let pbr = material.pbr_metallic_roughness();

                        let albedo = if let Some(base_color_texture) = pbr.base_color_texture() {
                            let (name, resource) =
                                manage_image(images.as_slice(), &base_color_texture.texture())
                                    .await
                                    .ok()?;
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
                                        .await
                                        .ok()?;
                                        resources.push(resource);
                                        Property::Texture(name)
                                    },
                                    {
                                        let (name, resource) = manage_image(
                                            images.as_slice(),
                                            &roughness_texture.texture(),
                                        )
                                        .await
                                        .ok()?;
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
                                    .await
                                    .ok()?;
                            resources.push(resource);
                            Property::Texture(name)
                        } else {
                            Property::Factor(Value::Vector3([0.0, 0.0, 1.0]))
                        };

                        let emissive = if let Some(emissive_texture) = material.emissive_texture() {
                            let (name, resource) =
                                manage_image(images.as_slice(), &emissive_texture.texture())
                                    .await
                                    .ok()?;
                            resources.push(resource);
                            Property::Texture(name)
                        } else {
                            Property::Factor(Value::Vector3(material.emissive_factor()))
                        };

                        let occlusion =
                            if let Some(occlusion_texture) = material.occlusion_texture() {
                                let (name, resource) =
                                    manage_image(images.as_slice(), &occlusion_texture.texture())
                                        .await
                                        .ok()?;
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
                                gltf::material::AlphaMode::Mask => {
                                    AlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5))
                                }
                                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                            },
                            model: Model {
                                name: "".to_string(),
                                pass: "".to_string(),
                            },
                        }
                    };

                    let mut vertex_components = Vec::new();

                    let bounding_box = make_bounding_box(&primitive);

                    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

                    let vertex_count = if let Some(positions) = reader.read_positions() {
                        let vertex_count = positions.clone().count();
                        positions.for_each(|mut position| {
                            position[2] = -position[2]; // Convert from right-handed(GLTF) to left-handed coordinate system
                            position.iter().for_each(|m| {
                                m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
                            })
                        });
                        vertex_components.push(VertexComponent {
                            semantic: VertexSemantics::Position,
                            format: "vec3f".to_string(),
                            channel: 0,
                        });
                        vertex_count
                    } else {
                        return Some(Err("Mesh does not have positions".to_string()));
                    };

                    let indices = reader
                        .read_indices()
                        .expect("Cannot create mesh which does not have indices")
                        .into_u32()
                        .collect::<Vec<u32>>();

                    let optimized_indices =
                        meshopt::optimize::optimize_vertex_cache(&indices, vertex_count);

                    if let Some(normals) = reader.read_normals() {
                        normals.for_each(|mut normal| {
                            normal[2] = -normal[2]; // Convert from right-handed(GLTF) to left-handed coordinate system
                            normal.iter().for_each(|m| {
                                m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
                            });
                        });

                        vertex_components.push(VertexComponent {
                            semantic: VertexSemantics::Normal,
                            format: "vec3f".to_string(),
                            channel: 1,
                        });
                    }

                    if let Some(tangents) = reader.read_tangents() {
                        tangents.for_each(|tangent| {
                            tangent.iter().for_each(|m| {
                                m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
                            })
                        });
                        vertex_components.push(VertexComponent {
                            semantic: VertexSemantics::Tangent,
                            format: "vec4f".to_string(),
                            channel: 2,
                        });
                    }

                    for i in 0..8 {
                        if let Some(uv) = reader.read_tex_coords(i) {
                            assert_eq!(i, 0);
                            uv.into_f32().for_each(|uv| {
                                uv.iter().for_each(|m| {
                                    m.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
                                })
                            });
                            vertex_components.push(VertexComponent {
                                semantic: VertexSemantics::Uv,
                                format: "vec3f".to_string(),
                                channel: 3,
                            });
                        }
                    }

                    // align buffer to 16 bytes for indices
                    while buffer.len() % 16 != 0 {
                        buffer.push(0);
                    }

                    let mut index_streams = Vec::with_capacity(2);

                    let meshlet_stream;

                    if MESHLETIZE {
                        let cone_weight = 0.0f32; // How much to prioritize cone culling over other forms of culling
                        let meshlets = meshopt::clusterize::build_meshlets(
                            &optimized_indices,
                            &meshopt::VertexDataAdapter::new(&buffer[0..12 * vertex_count], 12, 0)
                                .unwrap(),
                            64,
                            124,
                            cone_weight,
                        );

                        let offset = buffer.len();

                        {
                            let index_type = IntegralTypes::U16;

                            match index_type {
                                IntegralTypes::U16 => {
                                    let mut index_count = 0usize;
                                    for meshlet in meshlets.iter() {
                                        index_count += meshlet.vertices.len();
                                        for x in meshlet.vertices {
                                            (*x as u16)
                                                .to_le_bytes()
                                                .iter()
                                                .for_each(|byte| buffer.push(*byte));
                                        }
                                    }
                                    index_streams.push(IndexStream {
                                        data_type: IntegralTypes::U16,
                                        stream_type: IndexStreamTypes::Vertices,
                                        offset,
                                        count: index_count as u32,
                                    });
                                }
                                _ => panic!("Unsupported index type"),
                            }
                        }

                        let offset = buffer.len();

                        for meshlet in meshlets.iter() {
                            for x in meshlet.triangles {
                                assert!(*x <= 64u8, "Meshlet index out of bounds"); // Max vertices per meshlet
                                buffer.push(*x);
                            }
                        }

                        index_streams.push(IndexStream {
                            data_type: IntegralTypes::U8,
                            stream_type: IndexStreamTypes::Meshlets,
                            offset,
                            count: optimized_indices.len() as u32,
                        });

                        let offset = buffer.len();

                        meshlet_stream = Some(MeshletStream {
                            offset,
                            count: meshlets.len() as u32,
                        });

                        for meshlet in meshlets.iter() {
                            buffer.push(meshlet.vertices.len() as u8);
                            buffer.push((meshlet.triangles.len() / 3usize) as u8);
                            // TODO: add tests for this
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
                                optimized_indices
                                    .iter()
                                    .map(|i| {
                                        assert!(*i <= 0xFFFFu32, "Index out of bounds"); // Max vertices per meshlet
                                        *i as u16
                                    })
                                    .for_each(|i| {
                                        i.to_le_bytes().iter().for_each(|byte| buffer.push(*byte))
                                    });
                                index_streams.push(IndexStream {
                                    data_type: IntegralTypes::U16,
                                    stream_type: IndexStreamTypes::Triangles,
                                    offset,
                                    count: optimized_indices.len() as u32,
                                });
                            }
                            _ => panic!("Unsupported index type"),
                        }
                    }

                    primitives.push(Primitive {
                        material,
                        quantization: None,
                        bounding_box,
                        vertex_components,
                        vertex_count: vertex_count as u32,
                        index_streams,
                        meshlet_stream,
                    });
                }

                sub_meshes.push(SubMesh { primitives });
            }

            let mesh = Mesh { sub_meshes };

            let resource_document = GenericResourceSerialization::new(id, mesh);
            storage_backend.store(resource_document, &buffer).await;

            Some(Ok(()))
        })
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
    use polodb_core::bson::{self, bson};

    use super::MeshAssetHandler;
    use crate::asset::{
        asset_handler::AssetHandler,
        tests::{TestAssetResolver, TestStorageBackend},
    };

    #[test]
    fn load_gltf() {
        let asset_handler = MeshAssetHandler::new();
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();

        let url = "Box.gltf";
        let doc = json::object! {
            "url": url,
        };

        smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &url, &doc)).expect("Failed to get resource");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, "Box.gltf");
        assert_eq!(resource.class, "Mesh");

        assert_eq!(
            resource.resource,
            bson::doc! {
                "sub_meshes": [
                    {
                        "primitives": [
                            {
                                "material": {
                                    "albedo": {
                                        "Factor": {
                                            "Vector4": [0.800000011920929, 0.0, 0.0, 1.0],
                                        }
                                    },
                                    "normal": {
                                        "Factor": {
                                            "Vector3": [0.0, 0.0, 1.0],
                                        }
                                    },
                                    "roughness": {
                                        "Factor": {
                                            "Scalar": 1.0,
                                        }
                                    },
                                    "metallic": {
                                        "Factor": {
                                            "Scalar": 0.0,
                                        }
                                    },
                                    "emissive": {
                                        "Factor": {
                                            "Vector3": [0.0, 0.0, 0.0],
                                        }
                                    },
                                    "occlusion": {
                                        "Factor": {
                                            "Scalar": 1.0,
                                        }
                                    },
                                    "double_sided": false,
                                    "alpha_mode": "Opaque",
                                    "model": {
                                        "name": "",
                                        "pass": "",
                                    },
                                },
                                "quantization": null,
                                "bounding_box": [[-0.5, -0.5, -0.5],[0.5, 0.5, 0.5],],
                                "vertex_count": 24i64,
                                "vertex_components": [
                                    {
                                        "semantic": "Position",
                                        "format": "vec3f",
                                        "channel": 0i64,
                                    },
                                    {
                                        "semantic": "Normal",
                                        "format": "vec3f",
                                        "channel": 1i64,
                                    },
                                ],
                                "index_streams": [
                                    {
                                        "data_type": "U16",
                                        "stream_type": "Vertices",
                                        "offset": 576i64,
                                        "count": 24i64,
                                    },
                                    {
                                        "data_type": "U8",
                                        "stream_type": "Meshlets",
                                        "offset": 624i64,
                                        "count": 36i64,
                                    },
                                    {
                                        "data_type": "U16",
                                        "stream_type": "Triangles",
                                        "offset": 662i64,
                                        "count": 36i64,
                                    },
                                ],
                                "meshlet_stream": {
                                    "offset": 660i64,
                                    "count": 1i64,
                                },
                            },
                        ],
                    },
                ],
            }.into()
        );

        // TODO: ASSERT BINARY DATA

        // 	let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

        // 	assert_eq!(vertex_positions.len(), 11808);

        // 	assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
        // 	assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
        // 	assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

        // 	let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(11808), primitive.vertex_count as usize) };

        // 	assert_eq!(vertex_normals.len(), 11808);

        // 	assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
        // 	assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
        // 	assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);

        // 	let triangle_indices = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(triangle_index_stream.offset) as *const u16, triangle_index_stream.count as usize) };

        // 	assert_eq!(triangle_indices[0..3], [0, 1, 2]);
        // 	assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);

        // let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

        // TODO: ASSERT BINARY DATA
    }

	#[test]
    fn load_gltf_with_bin() {
        let asset_handler = MeshAssetHandler::new();
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();

        let url = "Suzanne.gltf";
        let doc = json::object! {
            "url": url,
        };

        smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &url, &doc)).expect("Mesh asset handler did not handle asset").expect("Failed to parse asset");

        let generated_resources = storage_backend.get_resources();

        assert_eq!(generated_resources.len(), 1);

        let resource = &generated_resources[0];

        assert_eq!(resource.id, "Suzanne.gltf");
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
        //     }
        // );

        // TODO: ASSERT BINARY DATA

        // 	let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], primitive.vertex_count as usize) };

        // 	assert_eq!(vertex_positions.len(), 11808);

        // 	assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
        // 	assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
        // 	assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

        // 	let vertex_normals = unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(11808), primitive.vertex_count as usize) };

        // 	assert_eq!(vertex_normals.len(), 11808);

        // 	assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
        // 	assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
        // 	assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);

        // 	let triangle_indices = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(triangle_index_stream.offset) as *const u16, triangle_index_stream.count as usize) };

        // 	assert_eq!(triangle_indices[0..3], [0, 1, 2]);
        // 	assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);

        // let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

        // TODO: ASSERT BINARY DATA
    }

    #[test]
    #[ignore="Test uses data not pushed to the repository"]
    fn load_glb() {
        let asset_resolver = TestAssetResolver::new();
        let storage_backend = TestStorageBackend::new();
        let asset_handler = MeshAssetHandler::new();

        let url = "Revolver.glb";
        let doc = json::object! {
            "url": url,
        };

        let result = smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &url, &doc));

        assert!(result.is_some());
        assert!(result.unwrap().is_ok());
    }
}
