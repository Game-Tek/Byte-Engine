use maths_rs::{
	mat::{MatNew4, MatScale},
	vec::Vec3,
};
use utils::{json::JsonValueTrait, Extent};

use crate::{
	asset::{self},
	processors::{
		image_processor::{gamma_from_semantic, guess_semantic_from_name, process_image, ImageDescription},
		mesh_processor::{MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource},
	},
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::material::VariantModel,
	types::{Formats, VertexComponent, VertexSemantics},
	ProcessedAsset,
};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};

pub use crate::processors::mesh_processor::TriangleFrontFaceWinding;

/// The `GLTFAssetHandler` struct stores glTF import settings for meshes and images.
pub struct GLTFAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
}

impl GLTFAssetHandler {
	pub fn new() -> GLTFAssetHandler {
		GLTFAssetHandler {
			triangle_front_face_winding: TriangleFrontFaceWinding::Clockwise,
		}
	}

	pub fn triangle_front_face_winding(&self) -> TriangleFrontFaceWinding {
		self.triangle_front_face_winding
	}

	pub fn set_triangle_front_face_winding(&mut self, winding: TriangleFrontFaceWinding) {
		self.triangle_front_face_winding = winding;
	}

	pub fn with_triangle_front_face_winding(mut self, winding: TriangleFrontFaceWinding) -> GLTFAssetHandler {
		self.set_triangle_front_face_winding(winding);
		self
	}
}

impl AssetHandler for GLTFAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "gltf" || r#type == "glb"
	}

	fn bake<'a>(
		&'a self,
		asset_manager: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let (data, spec, dt) = asset_storage_backend
				.resolve(url)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			let (gltf, buffers) = if dt == "glb" {
				let parsed = spawn_cpu_task(move || -> Result<(gltf::Gltf, Vec<gltf::buffer::Data>), LoadErrors> {
					let glb = gltf::Glb::from_slice(&data).map_err(|_| LoadErrors::FailedToProcess)?;
					let gltf = gltf::Gltf::from_slice(&glb.json).map_err(|_| LoadErrors::FailedToProcess)?;
					let buffers = gltf::import_buffers(&gltf, None, glb.bin.as_ref().map(|b| b.iter().map(|e| *e).collect()))
						.map_err(|_| LoadErrors::FailedToProcess)?;
					Ok((gltf, buffers))
				})
				.await
				.map_err(|_| LoadErrors::FailedToProcess)??;

				parsed
			} else {
				let gltf = spawn_cpu_task(move || gltf::Gltf::from_slice(&data).map_err(|_| LoadErrors::AssetCouldNotBeLoaded))
					.await
					.map_err(|_| LoadErrors::FailedToProcess)??;

				let buffers = if let Some(bin_file) = gltf.buffers().find_map(|b| {
					if let gltf::buffer::Source::Uri(r) = b.source() {
						if r.ends_with(".bin") {
							Some(r)
						} else {
							None
						}
					} else {
						None
					}
				}) {
					let bin_file = ResourceId::new(bin_file);
					let (bin, ..) = asset_storage_backend
						.resolve(bin_file)
						.await
						.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;
					gltf.buffers()
						.map(|_| gltf::buffer::Data(bin.clone().into()))
						.collect::<Vec<_>>()
				} else {
					gltf::import_buffers(&gltf, None, None).map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?
				};

				(gltf, buffers)
			};

			if let Some(fragment) = url.get_fragment() {
				let image = gltf
					.images()
					.find(|i| i.name() == Some(fragment.as_ref()))
					.ok_or(LoadErrors::FailedToProcess)?;
				let image =
					gltf::image::Data::from_source(image.source(), None, &buffers).map_err(|_| LoadErrors::FailedToProcess)?;
				let format = match image.format {
					gltf::image::Format::R8G8B8 => Formats::RGB8,
					gltf::image::Format::R8G8B8A8 => Formats::RGBA8,
					gltf::image::Format::R16G16B16 => Formats::RGB16,
					gltf::image::Format::R16G16B16A16 => Formats::RGBA16,
					_ => return Err(LoadErrors::UnsupportedType),
				};
				let extent = Extent::rectangle(image.width, image.height);

				let semantic = guess_semantic_from_name(url.get_base());

				let image_description = ImageDescription {
					format,
					extent,
					semantic,
					gamma: gamma_from_semantic(semantic),
				};

				return process_image(url, image_description, image.pixels.into_boxed_slice());
			}

			let spec = if let Some(spec) = &spec {
				spec
			} else {
				log::error!("No spec found for {:#?}", url);
				return Err(LoadErrors::FailedToProcess);
			};

			// Gather vertex components and check that they are all equal
			let all = gltf
				.meshes()
				.map(|mesh| {
					mesh.primitives().map(|primitive| {
						primitive
							.attributes()
							.scan(0, |state, (semantic, _)| {
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
										format: "vec4f".to_string(),
										channel,
									},
									gltf::Semantic::Colors(_) => VertexComponent {
										semantic: VertexSemantics::Color,
										format: "vec4f".to_string(),
										channel,
									},
									gltf::Semantic::TexCoords(_count) => VertexComponent {
										semantic: VertexSemantics::UV,
										format: "vec2f".to_string(),
										channel,
									},
									gltf::Semantic::Joints(_) => VertexComponent {
										semantic: VertexSemantics::Joints,
										format: "vec4u".to_string(),
										channel,
									},
									gltf::Semantic::Weights(_) => VertexComponent {
										semantic: VertexSemantics::Weights,
										format: "vec4f".to_string(),
										channel,
									},
								}
								.into()
							})
							.collect::<Vec<VertexComponent>>()
					})
				})
				.flatten();

			let vertex_layouts = all.collect::<Vec<Vec<VertexComponent>>>();
			let vertex_layout = vertex_layouts.first().unwrap().clone();

			fn flatten_tree(base: maths_rs::Mat4f, node: gltf::Node) -> Vec<(gltf::Node, maths_rs::Mat4f)> {
				let transform = node.transform().matrix();
				let transform = base
					* maths_rs::Mat4f::new(
						transform[0][0],
						transform[1][0],
						transform[2][0],
						transform[3][0],
						transform[0][1],
						transform[1][1],
						transform[2][1],
						transform[3][1],
						transform[0][2],
						transform[1][2],
						transform[2][2],
						transform[3][2],
						transform[0][3],
						transform[1][3],
						transform[2][3],
						transform[3][3],
					);

				let mut nodes = vec![(node.clone(), transform)];

				for child in node.children() {
					nodes.extend(flatten_tree(transform, child));
				}

				nodes
			}

			let mut flat_tree = gltf
				.scenes()
				.map(|scene| {
					scene
						.nodes()
						.map(|node| flatten_tree(maths_rs::Mat4f::identity(), node))
						.flatten()
				})
				.flatten()
				.collect::<Vec<(gltf::Node, maths_rs::Mat4f)>>();

			for (_, transform) in &mut flat_tree {
				*transform = maths_rs::Mat4f::from_scale(Vec3::new(1f32, 1f32, -1f32)) * *transform;
				// Make vertices left-handed
			}

			let primitives = flat_tree
				.iter()
				.filter_map(|(node, _)| node.mesh().map(|mesh| mesh.primitives().map(|primitive| primitive)))
				.flatten()
				.collect::<Vec<_>>();

			let primitives_and_transform = flat_tree
				.iter()
				.filter_map(|(node, transform)| {
					node.mesh()
						.map(|mesh| mesh.primitives().map(|primitive| (primitive, *transform)))
				})
				.flatten()
				.collect::<Vec<_>>();

			let flat_mesh_tree = {
				primitives_and_transform.iter().map(|(primitive, transform)| {
					(
						primitive,
						primitive.reader(|buffer| Some(&buffers[buffer.index()])),
						*transform,
					)
				})
			};

			let material_name_per_primitive = primitives
				.iter()
				.map(|primitive: &gltf::Primitive| {
					let asset = &spec["asset"];

					let gltf_material = primitive.material();
					let gltf_material_name = gltf_material.name().unwrap();

					let material = &asset[gltf_material_name];
					material["asset"].as_str().unwrap().to_string()
				})
				.collect::<Vec<String>>();

			let mut materials_per_primitive = Vec::with_capacity(material_name_per_primitive.len());
			for name in material_name_per_primitive {
				let material = asset_manager
					.load::<VariantModel>(&name, storage_backend)
					.await
					.map_err(|_| LoadErrors::FailedToProcess)?;
				materials_per_primitive.push(material);
			}

			let primitives = flat_mesh_tree
				.zip(materials_per_primitive.into_iter())
				.map(|((primitive, reader, transform), material)| {
					let triangle_indices = reader
						.read_indices()
						.ok_or(LoadErrors::FailedToProcess)?
						.into_u32()
						.collect::<Vec<u32>>();

					let mut primitive = OwnedMeshPrimitive::new(material, make_bounding_box(primitive), triangle_indices);
					primitive.add_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(
							reader
								.read_positions()
								.ok_or(LoadErrors::FailedToProcess)?
								.map(|position| {
									let position = maths_rs::Vec3f::new(position[0], position[1], position[2]);
									let transformed = transform * position;
									[transformed[0], transformed[1], transformed[2]]
								})
								.collect(),
						),
					));

					if let Some(normals) = reader.read_normals() {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Normal,
							0,
							OwnedMeshAttributeData::F32x3(
								normals
									.map(|normal| {
										let normal = maths_rs::Vec3f::new(normal[0], normal[1], normal[2]);
										let transformed = transform * normal;
										[transformed[0], transformed[1], transformed[2]]
									})
									.collect(),
							),
						));
					}

					if let Some(tangents) = reader.read_tangents() {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Tangent,
							0,
							OwnedMeshAttributeData::F32x4(
								tangents
									.map(|tangent| {
										let direction = maths_rs::Vec3f::new(tangent[0], tangent[1], tangent[2]);
										let transformed = transform * direction;
										[transformed[0], transformed[1], transformed[2], tangent[3]]
									})
									.collect(),
							),
						));
					}

					if let Some(colors) = reader.read_colors(0) {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Color,
							0,
							OwnedMeshAttributeData::F32x4(colors.into_rgba_f32().collect()),
						));
					}

					if let Some(uvs) = reader.read_tex_coords(0) {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::UV,
							0,
							OwnedMeshAttributeData::F32x2(uvs.into_f32().collect()),
						));
					}

					if let Some(joints) = reader.read_joints(0) {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Joints,
							0,
							OwnedMeshAttributeData::U16x4(joints.into_u16().collect()),
						));
					}

					if let Some(weights) = reader.read_weights(0) {
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Weights,
							0,
							OwnedMeshAttributeData::F32x4(weights.into_f32().collect()),
						));
					}

					Ok::<_, LoadErrors>(primitive)
				})
				.collect::<Result<Vec<_>, _>>()?;

			let mesh_source = OwnedMeshSource::new(vertex_layout, primitives);
			let mesh = MeshProcessor::new()
				.with_triangle_front_face_winding(self.triangle_front_face_winding)
				.process(&mesh_source)
				.map_err(|_| LoadErrors::FailedToProcess)?;

			Ok((
				ProcessedAsset::new(url, mesh.mesh).with_streams(mesh.stream_descriptions),
				mesh.buffer,
			))
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
	use super::{GLTFAssetHandler, TriangleFrontFaceWinding};
	use crate::r#async;
	use crate::{
		asset::{
			asset_handler::AssetHandler,
			asset_manager::AssetManager,
			bema_asset_handler::{tests::RootTestShaderGenerator, BEMAAssetHandler},
			png_asset_handler::PNGAssetHandler,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
			ResourceId,
		},
		processors::mesh_processor::orient_triangle_indices_for_front_face,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::mesh::MeshModel,
		ReferenceModel,
	};

	#[test]
	fn defaults_to_clockwise_front_faces() {
		let asset_handler = GLTFAssetHandler::new();

		assert_eq!(
			asset_handler.triangle_front_face_winding(),
			TriangleFrontFaceWinding::Clockwise
		);
	}

	#[test]
	fn preserves_triangle_order_for_counter_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::CounterClockwise);

		assert_eq!(oriented, vec![0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn rewinds_triangle_order_for_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::Clockwise);

		assert_eq!(oriented, vec![0, 2, 1, 3, 5, 4]);
	}

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn load_gltf() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("shader.besl", "main: fn () -> void {}".as_bytes());
		asset_storage_backend.add_file(
			"Box.bema",
			r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "shader.besl"
			},
			"variables": []
		}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"Texture.bema",
			r#"{
			"parent": "Box.bema",
			"variables": []
		}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"Box.glb.bead",
			r#"{"asset": {"Texture": {"asset": "Texture.bema" }}}"#.as_bytes(),
		);

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend);

		let asset_handler = GLTFAssetHandler::new();
		asset_manager.add_asset_handler({
			let mut material_asset_handler = BEMAAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});

		asset_manager.add_asset_handler(asset_handler);

		let url = "Box.glb";

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.load(url, &resource_storage_backend)
			.await
			.expect("Failed to parse asset");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 4);

		assert_eq!(mesh.id().as_ref(), url);
		assert_eq!(mesh.class(), "Mesh");

		// TODO: ASSERT BINARY DATA
	}

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn load_gltf_with_bin() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("shader.besl", "main: fn () -> void {}".as_bytes());
		asset_storage_backend.add_file(
			"Material.bema",
			r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "shader.besl"
			},
			"variables": []
		}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"Suzanne.bema",
			r#"{
			"parent": "Material.bema",
			"variables": []
		}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"Suzanne.gltf.bead",
			r#"{"asset": {"Suzanne": {"asset": "Suzanne.bema" }}}"#.as_bytes(),
		);

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend);

		asset_manager.add_asset_handler({
			let mut material_asset_handler = BEMAAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});

		let asset_handler = GLTFAssetHandler::new();

		asset_manager.add_asset_handler(asset_handler);

		let url = "Suzanne.gltf";

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.load(url, &resource_storage_backend)
			.await
			.expect("Failed to parse asset");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 4);

		let url = ResourceId::new(url);

		assert_eq!(mesh.id(), url);
		assert_eq!(mesh.class(), "Mesh");

		// TODO: ASSERT BINARY DATA

		// let vertex_count = resource.resource.as_document().unwrap().get_i64("vertex_count").unwrap() as usize;

		// assert_eq!(vertex_count, 11808);
		let vertex_count = 11808;

		let buffer = resource_storage_backend.get_resource_data_by_name(url).unwrap();

		let vertex_positions = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const [f32; 3], vertex_count) };

		assert_eq!(vertex_positions.len(), 11808);

		assert_eq!(vertex_positions[0], [0.492188f32, 0.185547f32, -0.720703f32]);
		assert_eq!(vertex_positions[1], [0.472656f32, 0.243042f32, -0.751221f32]);
		assert_eq!(vertex_positions[2], [0.463867f32, 0.198242f32, -0.753418f32]);

		let vertex_normals =
			unsafe { std::slice::from_raw_parts((buffer.as_ptr() as *const [f32; 3]).add(11808), vertex_count) };

		assert_eq!(vertex_normals.len(), 11808);

		assert_eq!(vertex_normals[0], [0.703351f32, -0.228379f32, -0.673156f32]);
		assert_eq!(vertex_normals[1], [0.818977f32, -0.001884f32, -0.573824f32]);
		assert_eq!(vertex_normals[2], [0.776439f32, -0.262265f32, -0.573027f32]);

		// let triangle_indices = unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(triangle_index_stream.offset) as *const u16, triangle_index_stream.count as usize) };

		// assert_eq!(triangle_indices[0..3], [0, 1, 2]);
		// assert_eq!(triangle_indices[3935 * 3..3936 * 3], [11805, 11806, 11807]);
	}

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn load_glb() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("shaders/pbr.besl", "main: fn () -> void {}".as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend);

		// storage_backend.add_file("PBR.bema", r#"{
		// 	"domain": "World",
		// 	"type": "Surface",
		// 	"shaders": {
		// 		"Compute": "shader.besl"
		// 	},
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"data_type": "Texture2D",
		// 			"value": "Revolver.glb#Revolver_Base_color"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"data_type": "Texture2D",
		// 			"value": "Revolver.glb#Revolver_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"data_type": "Texture2D",
		// 			"value": "Revolver.glb#Revolver_Metallic-Revolver_Roughness"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("Revolver.bema", r#"{
		// 	"parent": "PBR.bema",
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"value": "Revolver.glb#Revolver_Base_color"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"value": "Revolver.glb#Revolver_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"value": "Revolver.glb#Revolver_Metallic-Revolver_Roughness"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("Material.001.bema", r#"{
		// 	"parent": "PBR.bema",
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"value": "Revolver.glb#Material.001_Base_color"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"value": "Revolver.glb#Material.001_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"value": "Revolver.glb#Material.001_Metallic-Material.001_Roughness"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("RedDotScopeLens.bema", r#"{
		// 	"parent": "PBR.bema",
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"value": "Revolver.glb#RedDotScopeLens_Base_color"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"value": "Revolver.glb#RedDotScopeLens_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"value": "Revolver.glb#RedDotScopeLens_Metallic-RedDotScopeLens_Roughness"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("RedDotScopeDot.bema", r#"{
		// 	"parent": "PBR.bema",
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"value": "Revolver.glb#RedDotScopeDot_Base_color-RedDotScopeDot_Opacity.png"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"value": "Revolver.glb#RedDotScopeDot_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"value": "Revolver.glb#RedDotScopeDot_Metallic.png-RedDotScopeDot_Roughness.png"
		// 		},
		// 		{
		// 			"name": "emissive",
		// 			"value": "Revolver.glb#RedDotScopeDot_Emissive"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("FlashLight.bema", r#"{
		// 	"parent": "PBR.bema",
		// 	"variables": [
		// 		{
		// 			"name": "color",
		// 			"value": "Revolver.glb#FlashLight_Base_color"
		// 		},
		// 		{
		// 			"name": "normalll",
		// 			"value": "Revolver.glb#FlashLight_Normal_OpenGL"
		// 		},
		// 		{
		// 			"name": "metallic_roughness",
		// 			"value": "Revolver.glb#FlashLight_Metallic-FlashLight_Roughness"
		// 		},
		// 		{
		// 			"name": "emissive",
		// 			"value": "Revolver.glb#FlashLight_Emissive"
		// 		}
		// 	]
		// }"#.as_bytes());
		// storage_backend.add_file("Revolver.glb.bead", r#"{
		// 	"asset": {
		// 		"Revolver": {
		// 			"asset": "Revolver.bema"
		// 		},
		// 		"Material.001": {
		// 			"asset": "Material.001.bema"
		// 		},
		// 		"RedDotScopeLens": {
		// 			"asset": "RedDotScopeLens.bema"
		// 		},
		// 		"RedDotScopeDot": {
		// 			"asset": "RedDotScopeDot.bema"
		// 		},
		// 		"FlashLight": {
		// 			"asset": "FlashLight.bema"
		// 		}
		// 	}
		// }"#.as_bytes());

		asset_manager.add_asset_handler({
			let mut material_asset_handler = BEMAAssetHandler::new();
			let shader_generator = RootTestShaderGenerator::new();
			material_asset_handler.set_shader_generator(shader_generator);
			material_asset_handler
		});
		asset_manager.add_asset_handler(PNGAssetHandler::new());
		asset_manager.add_asset_handler(GLTFAssetHandler::new());
		let _asset_handler = GLTFAssetHandler::new();

		let url = "Revolver.glb";

		let _mesh: ReferenceModel<MeshModel> = asset_manager.load(&url, &resource_storage_backend).await.unwrap();

		let url = ResourceId::new(url);

		let buffer = resource_storage_backend.get_resource_data_by_name(url).unwrap();

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

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn load_glb_image() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend);

		let asset_handler = GLTFAssetHandler::new();

		let image_asset_handler = PNGAssetHandler::new();

		asset_manager.add_asset_handler(image_asset_handler);

		let url = ResourceId::new("Revolver.glb#Revolver_Metallic-Revolver_Roughness");

		let (resource, data) = asset_handler
			.bake(
				&asset_manager,
				&resource_storage_backend,
				asset_manager.get_storage_backend(),
				url,
			)
			.await
			.expect("Image asset handler did not handle asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
			.expect("Image asset handler did not store asset");

		let _ = resource_storage_backend.get_resource_data_by_name(url).unwrap();

		let generated_resources = resource_storage_backend.get_resources();

		let resource = &generated_resources[0];

		assert_eq!(resource.class, "Image");
	}

	// #[test]
	// #[ignore]
	// fn load_16bit_normal_image() {
	// 	let asset_storage_backend = asset::storage_backend::FileStorageBackend::new("../assets".into());
	// 	let resource_storage_backend = resource::storage_backend::TestStorageBackend::new();

	// 	let mut asset_manager = AssetManager::new_with_storage_backends(asset_storage_backend, resource_storage_backend.clone());

	// 	asset_manager.add_asset_handler(ImageAssetHandler::new());
	// 	let asset_handler = MeshAssetHandler::new();

	// 	let url = ResourceId::new("Revolver.glb#Revolver_Normal_OpenGL");

	// 	let _ = block_on(asset_handler.load(&asset_manager, &resource_storage_backend, &asset_storage_backend, url,)).expect("Image asset handler did not handle asset");

	// 	// let generated_resources = asset_manager.get_storage_backend().get_resources();

	// 	// assert_eq!(generated_resources.len(), 1);

	// 	// let resource = &generated_resources[0];

	// 	// assert_eq!(resource.id, url);
	// 	// assert_eq!(resource.class, "Image");
	// }
}
