use std::{path::Path, sync::Arc};

use maths_rs::{
	mat::{MatNew4, MatScale},
	vec::Vec3,
};
use utils::{json, json::JsonValueTrait, Extent};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	bema_asset_handler::{compile_shader_program, ProgramGenerator},
	ResourceId,
};
pub use crate::processors::mesh_processor::TriangleFrontFaceWinding;
use crate::{
	asset::{self},
	pbr::{
		brdf_material_from_gltf, generate_textured_brdf_program, BrdfMaterialDescription, BrdfMaterialValidationError,
		BrdfNode, BrdfNodeId,
	},
	processors::{
		image_processor::{gamma_from_semantic, guess_semantic_from_name, process_image, ImageDescription, Semantic},
		mesh_processor::{MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource},
	},
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::{
		image::Image,
		material::{MaterialModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel},
	},
	types::{AlphaMode, Formats, VertexComponent, VertexSemantics},
	ProcessedAsset, ReferenceModel,
};

/// The `GLTFAssetHandler` struct stores glTF import settings for meshes and images.
pub struct GLTFAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	generator: Option<Arc<dyn ProgramGenerator>>,
}

impl GLTFAssetHandler {
	pub fn new() -> GLTFAssetHandler {
		GLTFAssetHandler {
			triangle_front_face_winding: TriangleFrontFaceWinding::Clockwise,
			generator: None,
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

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
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
				let image = image_for_gltf_fragment(&gltf, fragment.as_ref()).ok_or(LoadErrors::FailedToProcess)?;
				let image = load_gltf_image_data(asset_storage_backend, url, image, &buffers).await?;
				let semantic = guess_semantic_from_name(url.get_base());
				return process_gltf_image(url, image, semantic);
			}

			let spec = spec.as_ref();

			let vertex_layouts = gltf
				.meshes()
				.map(|mesh| {
					mesh.primitives().map(|primitive| {
						primitive
							.attributes()
							.filter_map(|(semantic, _)| gltf_vertex_component(semantic))
							.collect::<Vec<VertexComponent>>()
					})
				})
				.flatten()
				.collect::<Vec<Vec<VertexComponent>>>();
			let vertex_layout = normalize_vertex_layouts(&vertex_layouts);

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

			let mut materials_per_primitive = Vec::with_capacity(primitives.len());
			for primitive in &primitives {
				let material = material_for_gltf_primitive(
					asset_manager,
					storage_backend,
					asset_storage_backend,
					spec,
					url,
					&gltf,
					&buffers,
					primitive.material(),
					self.generator.clone(),
				)
				.await?;
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

					if has_vertex_component(&vertex_layout, VertexSemantics::Normal, 0) {
						let normals = reader.read_normals().ok_or(LoadErrors::FailedToProcess)?;
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

					if has_vertex_component(&vertex_layout, VertexSemantics::Tangent, 0) {
						let tangents = reader.read_tangents().ok_or(LoadErrors::FailedToProcess)?;
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

					if has_vertex_component(&vertex_layout, VertexSemantics::Color, 0) {
						let colors = reader.read_colors(0).ok_or(LoadErrors::FailedToProcess)?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Color,
							0,
							OwnedMeshAttributeData::F32x4(colors.into_rgba_f32().collect()),
						));
					}

					if has_vertex_component(&vertex_layout, VertexSemantics::UV, 0) {
						let uvs = reader.read_tex_coords(0).ok_or(LoadErrors::FailedToProcess)?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::UV,
							0,
							OwnedMeshAttributeData::F32x2(uvs.into_f32().collect()),
						));
					}

					if has_vertex_component(&vertex_layout, VertexSemantics::Joints, 0) {
						let joints = reader.read_joints(0).ok_or(LoadErrors::FailedToProcess)?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Joints,
							0,
							OwnedMeshAttributeData::U16x4(joints.into_u16().collect()),
						));
					}

					if has_vertex_component(&vertex_layout, VertexSemantics::Weights, 0) {
						let weights = reader.read_weights(0).ok_or(LoadErrors::FailedToProcess)?;
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

async fn material_for_gltf_primitive(
	asset_manager: &AssetManager,
	storage_backend: &dyn resource::StorageBackend,
	asset_storage_backend: &dyn asset::StorageBackend,
	spec: Option<&json::Value>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	material: gltf::Material<'_>,
	generator: Option<Arc<dyn ProgramGenerator>>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	if let Some(override_asset) = material_override(spec, &material) {
		return asset_manager
			.bake_if_not_exists::<VariantModel>(&override_asset, storage_backend)
			.await
			.map_err(|_| LoadErrors::FailedToProcess);
	}

	generate_gltf_material_variant(
		storage_backend,
		asset_storage_backend,
		mesh_url,
		gltf,
		buffers,
		material,
		generator,
	)
	.await
}

async fn generate_gltf_material_variant(
	storage_backend: &dyn resource::StorageBackend,
	asset_storage_backend: &dyn asset::StorageBackend,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	material: gltf::Material<'_>,
	generator: Option<Arc<dyn ProgramGenerator>>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let generator = generator.ok_or(LoadErrors::FailedToProcess)?;
	let brdf = brdf_material_from_gltf(&material);
	let alpha_mode = AlphaMode::from(brdf.alpha_mode);
	let texture_dependencies = collect_gltf_texture_dependencies(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;
	let texture_variables = store_gltf_texture_dependencies(
		storage_backend,
		asset_storage_backend,
		mesh_url,
		gltf,
		buffers,
		&texture_dependencies,
	)
	.await?;
	let program = generate_textured_brdf_program(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;
	let base_id = generated_material_base_id(mesh_url, &material);
	let shader_id = format!("{base_id}.shader");
	let material_id = format!("{base_id}.material");
	let variant_id = format!("{base_id}.variant");
	let shader_name = shader_id.clone();
	let material_json = generated_material_json(&texture_variables);

	let (shader, shader_bytes) = spawn_cpu_task(move || {
		compile_shader_program(generator.as_ref(), &shader_name, program, "World", &material_json, "Compute")
	})
	.await
	.map_err(|_| LoadErrors::FailedToProcess)?
	.map_err(|_| LoadErrors::FailedToProcess)?;

	let shader = store_model::<Shader>(storage_backend, &shader_id, shader, &shader_bytes)?;
	let material = MaterialModel {
		double_sided: brdf.double_sided,
		alpha_mode: alpha_mode.clone(),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};
	let material = store_model::<MaterialModel>(storage_backend, &material_id, material, &[])?;
	let variant = VariantModel {
		material,
		variables: texture_variables,
		alpha_mode,
	};

	store_model::<VariantModel>(storage_backend, &variant_id, variant, &[])
}

fn material_override(spec: Option<&json::Value>, material: &gltf::Material<'_>) -> Option<String> {
	let material_name = material.name()?;
	let material = &spec?["asset"][material_name];
	material["asset"].as_str().map(ToString::to_string)
}

fn generated_material_base_id(mesh_url: ResourceId<'_>, material: &gltf::Material<'_>) -> String {
	let material_name = material
		.name()
		.map(sanitize_material_name)
		.unwrap_or_else(|| format!("material_{}", material.index().unwrap_or(0)));
	format!("{}#materials/{material_name}", mesh_url.as_ref())
}

fn sanitize_material_name(name: &str) -> String {
	let sanitized = name
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
				c
			} else {
				'_'
			}
		})
		.collect::<String>();

	if sanitized.is_empty() {
		"material".to_string()
	} else {
		sanitized
	}
}

fn store_model<M: crate::Model>(
	storage_backend: &dyn resource::StorageBackend,
	id: &str,
	model: M,
	data: &[u8],
) -> Result<ReferenceModel<M>, LoadErrors> {
	let resource = ProcessedAsset::new(ResourceId::new(id), model);
	storage_backend
		.store(&resource, data)
		.map(|resource| resource.into())
		.map_err(|_| LoadErrors::FailedToProcess)
}

/// The `GltfTextureDependency` struct records a glTF image required by a generated material variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GltfTextureDependency {
	image_index: u32,
	semantic: Semantic,
}

/// Finds the image addressed by a glTF resource fragment.
/// Generated fragments use `images/<index>...` so unnamed GLB images remain addressable.
fn image_for_gltf_fragment<'a>(gltf: &'a gltf::Gltf, fragment: &str) -> Option<gltf::Image<'a>> {
	if let Some(index) = generated_image_fragment_index(fragment) {
		return gltf.images().find(|image| image.index() == index as usize);
	}

	gltf.images().find(|image| image.name() == Some(fragment))
}

fn generated_image_fragment_index(fragment: &str) -> Option<u32> {
	let suffix = fragment.strip_prefix("images/")?;
	let digits = suffix
		.chars()
		.take_while(|character| character.is_ascii_digit())
		.collect::<String>();
	if digits.is_empty() {
		None
	} else {
		digits.parse().ok()
	}
}

/// Loads a glTF image from embedded buffer data, data URIs, or file-local URI references.
/// File-local references are resolved through the engine asset backend so ad-hoc textures inside `.gltf` assets do not need to be standalone engine resources.
async fn load_gltf_image_data(
	asset_storage_backend: &dyn asset::StorageBackend,
	mesh_url: ResourceId<'_>,
	image: gltf::Image<'_>,
	buffers: &[gltf::buffer::Data],
) -> Result<gltf::image::Data, LoadErrors> {
	match image.source() {
		gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
			let image_url = resolve_gltf_uri(mesh_url, uri);
			let (bytes, ..) = asset_storage_backend
				.resolve(ResourceId::new(&image_url))
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;
			decode_external_gltf_image(&bytes)
		}
		_ => gltf::image::Data::from_source(image.source(), None, buffers).map_err(|_| LoadErrors::FailedToProcess),
	}
}

fn resolve_gltf_uri(mesh_url: ResourceId<'_>, uri: &str) -> String {
	if uri.contains("://") || uri.starts_with('/') {
		return uri.to_string();
	}

	let base = mesh_url.get_base();
	let parent = Path::new(base.as_ref()).parent();
	if let Some(parent) = parent {
		parent.join(uri).to_string_lossy().replace('\\', "/")
	} else {
		uri.to_string()
	}
}

fn decode_external_gltf_image(bytes: &[u8]) -> Result<gltf::image::Data, LoadErrors> {
	let image = image::load_from_memory(bytes).map_err(|_| LoadErrors::FailedToProcess)?;
	let rgba = image.to_rgba8();
	let (width, height) = rgba.dimensions();

	Ok(gltf::image::Data {
		pixels: rgba.into_raw(),
		format: gltf::image::Format::R8G8B8A8,
		width,
		height,
	})
}

fn process_gltf_image(
	id: ResourceId<'_>,
	image: gltf::image::Data,
	semantic: Semantic,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	let format = gltf_image_format(image.format)?;
	let image_description = ImageDescription {
		format,
		extent: Extent::rectangle(image.width, image.height),
		semantic,
		gamma: gamma_from_semantic(semantic),
		generate_mipmaps: false,
	};

	process_image(id, image_description, image.pixels.into_boxed_slice())
}

fn gltf_image_format(format: gltf::image::Format) -> Result<Formats, LoadErrors> {
	match format {
		gltf::image::Format::R8G8B8 => Ok(Formats::RGB8),
		gltf::image::Format::R8G8B8A8 => Ok(Formats::RGBA8),
		gltf::image::Format::R16G16B16 => Ok(Formats::RGB16),
		gltf::image::Format::R16G16B16A16 => Ok(Formats::RGBA16),
		_ => Err(LoadErrors::UnsupportedType),
	}
}

/// Collects unique glTF image dependencies in material-slot order.
/// The generated shader uses `gltf_texture_<image_index>` names while the runtime fills those slots with bindless descriptor indices.
fn collect_gltf_texture_dependencies(
	material: &BrdfMaterialDescription,
) -> Result<Vec<GltfTextureDependency>, BrdfMaterialValidationError> {
	material.validate()?;
	let mut dependencies = Vec::new();
	let BrdfNode::MetallicRoughness(surface) = material.node(material.surface)? else {
		return Ok(dependencies);
	};

	collect_texture_dependencies_from_node(material, surface.base_color, Semantic::Albedo, &mut dependencies)?;
	collect_texture_dependencies_from_node(material, surface.metallic, Semantic::Metallic, &mut dependencies)?;
	collect_texture_dependencies_from_node(material, surface.roughness, Semantic::Roughness, &mut dependencies)?;
	if let Some(normal) = surface.normal {
		collect_texture_dependencies_from_node(material, normal, Semantic::Normal, &mut dependencies)?;
	}
	if let Some(occlusion) = surface.occlusion {
		collect_texture_dependencies_from_node(material, occlusion, Semantic::AO, &mut dependencies)?;
	}
	if let Some(emission) = surface.emission {
		collect_texture_dependencies_from_node(material, emission, Semantic::Emissive, &mut dependencies)?;
	}

	Ok(dependencies)
}

fn collect_texture_dependencies_from_node(
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
	semantic: Semantic,
	dependencies: &mut Vec<GltfTextureDependency>,
) -> Result<(), BrdfMaterialValidationError> {
	match material.node(node)? {
		BrdfNode::Texture(texture) => push_gltf_texture_dependency(dependencies, texture.image_index, semantic),
		BrdfNode::Multiply { left, right } => {
			collect_texture_dependencies_from_node(material, *left, semantic, dependencies)?;
			collect_texture_dependencies_from_node(material, *right, semantic, dependencies)?;
		}
		BrdfNode::ExtractChannel { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, semantic, dependencies)?;
		}
		BrdfNode::NormalMap { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, Semantic::Normal, dependencies)?;
		}
		BrdfNode::Occlusion { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, Semantic::AO, dependencies)?;
		}
		BrdfNode::Emission { color } => {
			collect_texture_dependencies_from_node(material, *color, Semantic::Emissive, dependencies)?;
		}
		BrdfNode::Constant(_) | BrdfNode::MetallicRoughness(_) => {}
	}

	Ok(())
}

fn push_gltf_texture_dependency(dependencies: &mut Vec<GltfTextureDependency>, image_index: u32, semantic: Semantic) {
	if let Some(existing) = dependencies
		.iter_mut()
		.find(|dependency| dependency.image_index == image_index)
	{
		existing.semantic = merge_texture_semantics(existing.semantic, semantic);
		return;
	}

	dependencies.push(GltfTextureDependency { image_index, semantic });
}

fn merge_texture_semantics(left: Semantic, right: Semantic) -> Semantic {
	if left == right {
		return left;
	}

	// Prefer color semantics when an unusual glTF reuses the same image for color and data textures.
	// This avoids accidentally sampling an albedo texture as linear data after processing.
	match (left, right) {
		(Semantic::Albedo, _) | (_, Semantic::Albedo) => Semantic::Albedo,
		(Semantic::Emissive, _) | (_, Semantic::Emissive) => Semantic::Emissive,
		(Semantic::Normal, _) | (_, Semantic::Normal) => Semantic::Normal,
		(Semantic::AO, _) | (_, Semantic::AO) => Semantic::AO,
		(Semantic::Metallic, _) | (_, Semantic::Metallic) => Semantic::Metallic,
		(Semantic::Roughness, _) | (_, Semantic::Roughness) => Semantic::Roughness,
		_ => left,
	}
}

async fn store_gltf_texture_dependencies(
	storage_backend: &dyn resource::StorageBackend,
	asset_storage_backend: &dyn asset::StorageBackend,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	dependencies: &[GltfTextureDependency],
) -> Result<Vec<VariantVariableModel>, LoadErrors> {
	let mut variables = Vec::with_capacity(dependencies.len());

	for dependency in dependencies {
		let image = gltf
			.images()
			.find(|image| image.index() == dependency.image_index as usize)
			.ok_or(LoadErrors::FailedToProcess)?;
		let id = generated_gltf_image_id(mesh_url, image.index() as u32, image.name());
		let image_ref = store_gltf_image_resource(
			storage_backend,
			asset_storage_backend,
			mesh_url,
			&id,
			image,
			buffers,
			dependency.semantic,
		)
		.await?;

		variables.push(VariantVariableModel {
			name: generated_texture_variable_name(dependency.image_index),
			r#type: "Texture2D".to_string(),
			value: ValueModel::Image(image_ref),
		});
	}

	Ok(variables)
}

async fn store_gltf_image_resource(
	storage_backend: &dyn resource::StorageBackend,
	asset_storage_backend: &dyn asset::StorageBackend,
	mesh_url: ResourceId<'_>,
	id: &str,
	image: gltf::Image<'_>,
	buffers: &[gltf::buffer::Data],
	semantic: Semantic,
) -> Result<ReferenceModel<Image>, LoadErrors> {
	let image_data = load_gltf_image_data(asset_storage_backend, mesh_url, image, buffers).await?;
	let (resource, bytes) = process_gltf_image(ResourceId::new(id), image_data, semantic)?;
	storage_backend
		.store(&resource, &bytes)
		.map(|resource| resource.into())
		.map_err(|_| LoadErrors::FailedToProcess)
}

fn generated_material_json(variables: &[VariantVariableModel]) -> json::Object {
	let variables = variables
		.iter()
		.map(|variable| {
			json::object! {
				"name": variable.name.as_str(),
				"data_type": variable.r#type.as_str()
			}
		})
		.collect::<Vec<_>>();

	json::object! {
		"variables": variables
	}
}

fn generated_texture_variable_name(image_index: u32) -> String {
	format!("gltf_texture_{image_index}")
}

fn generated_gltf_image_id(mesh_url: ResourceId<'_>, image_index: u32, image_name: Option<&str>) -> String {
	let readable_name = image_name
		.map(sanitize_material_name)
		.filter(|name| !name.is_empty())
		.map(|name| format!("_{name}"))
		.unwrap_or_default();
	format!("{}#images/{image_index}{readable_name}", mesh_url.as_ref())
}

fn gltf_vertex_component(semantic: gltf::Semantic) -> Option<VertexComponent> {
	match semantic {
		gltf::Semantic::Positions => Some(VertexComponent {
			semantic: VertexSemantics::Position,
			format: "vec3f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Normals => Some(VertexComponent {
			semantic: VertexSemantics::Normal,
			format: "vec3f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Tangents => Some(VertexComponent {
			semantic: VertexSemantics::Tangent,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Colors(0) => Some(VertexComponent {
			semantic: VertexSemantics::Color,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::TexCoords(0) => Some(VertexComponent {
			semantic: VertexSemantics::UV,
			format: "vec2f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Joints(0) => Some(VertexComponent {
			semantic: VertexSemantics::Joints,
			format: "vec4u".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Weights(0) => Some(VertexComponent {
			semantic: VertexSemantics::Weights,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		_ => None,
	}
}

fn normalize_vertex_layouts(vertex_layouts: &[Vec<VertexComponent>]) -> Vec<VertexComponent> {
	let Some(first_layout) = vertex_layouts.first() else {
		return Vec::new();
	};

	first_layout
		.iter()
		.filter(|component| component.semantic != VertexSemantics::BiTangent)
		.filter(|component| {
			vertex_layouts
				.iter()
				.all(|layout| layout.iter().any(|candidate| candidate == *component))
		})
		.cloned()
		.collect()
}

fn has_vertex_component(vertex_layout: &[VertexComponent], semantic: VertexSemantics, channel: u32) -> bool {
	vertex_layout
		.iter()
		.any(|component| component.semantic == semantic && component.channel == channel)
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
	use utils::json;

	use super::{
		collect_gltf_texture_dependencies, generated_gltf_image_id, generated_image_fragment_index, generated_material_base_id,
		gltf_vertex_component, has_vertex_component, material_override, normalize_vertex_layouts, sanitize_material_name,
		GLTFAssetHandler, GltfTextureDependency, TriangleFrontFaceWinding,
	};
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
		pbr::{BrdfAlphaMode, BrdfChannel, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture, BrdfValue},
		processors::{image_processor::Semantic, mesh_processor::orient_triangle_indices_for_front_face},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::mesh::MeshModel,
		types::{VertexComponent, VertexSemantics},
		ReferenceModel,
	};

	#[test]
	fn normalizes_gltf_layouts_to_shared_supported_streams() {
		let normalized = normalize_vertex_layouts(&[
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::Normal,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::BiTangent,
					format: "vec3f".to_string(),
					channel: 0,
				},
			],
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::Normal,
					format: "vec3f".to_string(),
					channel: 0,
				},
			],
		]);

		assert_eq!(normalized.len(), 2);
		assert!(has_vertex_component(&normalized, VertexSemantics::Position, 0));
		assert!(has_vertex_component(&normalized, VertexSemantics::Normal, 0));
		assert!(!has_vertex_component(&normalized, VertexSemantics::BiTangent, 0));
	}

	#[test]
	fn maps_gltf_semantics_to_normalized_channels() {
		assert_eq!(gltf_vertex_component(gltf::Semantic::Normals).unwrap().channel, 0);
		assert_eq!(gltf_vertex_component(gltf::Semantic::TexCoords(0)).unwrap().channel, 0);
		assert!(gltf_vertex_component(gltf::Semantic::TexCoords(1)).is_none());
	}

	#[test]
	fn reads_bead_material_override_when_present() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Paint"}]}"#.as_bytes())
			.expect("test glTF should parse");
		let material = gltf.materials().next().unwrap();
		let spec = json::from_str(r#"{"asset":{"Paint":{"asset":"Paint.bema"}}}"#).unwrap();

		assert_eq!(material_override(Some(&spec), &material), Some("Paint.bema".to_string()));
	}

	#[test]
	fn misses_bead_material_override_when_absent() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Paint"}]}"#.as_bytes())
			.expect("test glTF should parse");
		let material = gltf.materials().next().unwrap();

		assert_eq!(material_override(None, &material), None);
	}

	#[test]
	fn generated_material_ids_are_stable_and_sanitized() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Red Paint/Gloss"}]}"#.as_bytes())
			.expect("test glTF should parse");
		let material = gltf.materials().next().unwrap();

		assert_eq!(sanitize_material_name("Red Paint/Gloss"), "Red_Paint_Gloss");
		assert_eq!(
			generated_material_base_id(ResourceId::new("models/car.glb"), &material),
			"models/car.glb#materials/Red_Paint_Gloss"
		);
	}

	#[test]
	fn generated_image_ids_use_stable_indices_and_optional_names() {
		assert_eq!(
			generated_gltf_image_id(ResourceId::new("models/robot.glb"), 0, None),
			"models/robot.glb#images/0"
		);
		assert_eq!(
			generated_gltf_image_id(ResourceId::new("models/robot.glb"), 12, Some("Base Color/PNG")),
			"models/robot.glb#images/12_Base_Color_PNG"
		);
		assert_eq!(generated_image_fragment_index("images/12_Base_Color_PNG"), Some(12));
		assert_eq!(generated_image_fragment_index("Base Color"), None);
	}

	#[test]
	fn collects_gltf_texture_dependencies_in_material_slot_order() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.texture(BrdfTexture {
			image_index: 2,
			texcoord_channel: 0,
		});
		let metallic_roughness = builder.texture(BrdfTexture {
			image_index: 5,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(metallic_roughness, BrdfChannel::Blue);
		let roughness = builder.extract_channel(metallic_roughness, BrdfChannel::Green);
		let normal_source = builder.texture(BrdfTexture {
			image_index: 8,
			texcoord_channel: 0,
		});
		let normal = builder.add(BrdfNode::NormalMap {
			source: normal_source,
			scale: 1.0,
		});
		let occlusion_source = builder.texture(BrdfTexture {
			image_index: 10,
			texcoord_channel: 0,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: occlusion_source,
			strength: 0.75,
		});
		let emission_color = builder.constant(BrdfValue::Vector3([1.0, 0.25, 0.5]));
		let emission = builder.add(BrdfNode::Emission { color: emission_color });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let dependencies = collect_gltf_texture_dependencies(&material).expect("dependencies should collect");

		assert_eq!(
			dependencies,
			vec![
				GltfTextureDependency {
					image_index: 2,
					semantic: Semantic::Albedo,
				},
				GltfTextureDependency {
					image_index: 5,
					semantic: Semantic::Metallic,
				},
				GltfTextureDependency {
					image_index: 8,
					semantic: Semantic::Normal,
				},
				GltfTextureDependency {
					image_index: 10,
					semantic: Semantic::AO,
				},
			]
		);
	}

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
			.bake_if_not_exists(url, &resource_storage_backend)
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
			.bake_if_not_exists(url, &resource_storage_backend)
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

		let _mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists(&url, &resource_storage_backend)
			.await
			.unwrap();

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
