const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";
const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";
const SKELETON_FRAGMENT: &str = "skeleton";
const MAX_SKIN_JOINTS: usize = u16::MAX as usize + 1;

/// The `GLTFAssetHandler` struct provides the glTF boundary used to bake renderable meshes, skeletal clips, materials, and images.
pub struct GLTFAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	generator: Option<Arc<dyn ProgramGenerator>>,
}

impl Default for GLTFAssetHandler {
	fn default() -> Self {
		Self::new()
	}
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
		allocator: &'a dyn std::alloc::Allocator,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			// Resolve the container base so generated skeleton and animation fragments never become part of the source filename.
			let base = url.get_base();
			let source_id = ResourceId::new(base.as_ref());
			let (data, spec, dt) = asset_storage_backend
				.resolve_in(source_id, allocator)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			let (gltf, binary_blob) = if dt == "glb" {
				// Arena-backed source bytes borrow the bake allocator, so parsing stays in this task instead of crossing a thread boundary.
				let glb = gltf::Glb::from_slice(&data).map_err(|_| LoadErrors::FailedToProcess)?;
				let gltf = gltf::Gltf::from_slice(&glb.json).map_err(|_| LoadErrors::FailedToProcess)?;
				(gltf, glb.bin)
			} else {
				// Keep the allocator-backed `.gltf` bytes local to this bake task.
				let gltf = gltf::Gltf::from_slice(&data).map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?;
				(gltf, None)
			};

			if url
				.get_fragment()
				.is_some_and(|fragment| fragment.as_ref() == SKELETON_FRAGMENT)
			{
				let graph = import_gltf_node_graph(&gltf).map_err(|error| {
					log::error!("Failed to import glTF skeleton '{}': {error}", url.as_ref());
					LoadErrors::FailedToProcess
				})?;
				return Ok((ProcessedAsset::new(url, graph.skeleton), Vec::new().into_boxed_slice()));
			}

			let required_buffers = url
				.get_fragment()
				.filter(|fragment| is_gltf_animation_fragment(fragment.as_ref()))
				.map(|fragment| required_gltf_animation_buffers(&gltf, fragment.as_ref()))
				.transpose()
				.map_err(|error| {
					log::error!("Failed to select glTF animation '{}': {error}", url.as_ref());
					LoadErrors::FailedToProcess
				})?;
			let buffers = load_gltf_buffers(
				asset_storage_backend,
				source_id,
				&gltf,
				binary_blob,
				required_buffers.as_deref(),
				allocator,
			)
			.await?;

			if let Some(fragment) = url.get_fragment() {
				if is_gltf_animation_fragment(fragment.as_ref()) {
					let graph = import_gltf_node_graph(&gltf).map_err(|error| {
						log::error!("Failed to import glTF animation skeleton '{}': {error}", url.as_ref());
						LoadErrors::FailedToProcess
					})?;
					let skeleton_id = generated_gltf_skeleton_id(source_id);
					let skeleton = store_model::<SkeletonModel>(storage_backend, &skeleton_id, graph.skeleton, &[])?;
					let animation = import_gltf_animation(&gltf, &buffers, fragment.as_ref(), &graph.source_to_dense, skeleton)
						.map_err(|error| {
							log::error!("Failed to import glTF animation '{}': {error}", url.as_ref());
							LoadErrors::FailedToProcess
						})?;
					return Ok((ProcessedAsset::new(url, animation), Vec::new().into_boxed_slice()));
				}

				let image = image_for_gltf_fragment(&gltf, fragment.as_ref()).ok_or(LoadErrors::FailedToProcess)?;
				let image = load_gltf_image_data(asset_storage_backend, url, image, &buffers, allocator).await?;
				let semantic = guess_semantic_from_name(url.get_base());
				return process_gltf_image(url, image, semantic, allocator);
			}

			let spec = spec.as_ref();
			let graph = import_gltf_node_graph(&gltf).map_err(|error| {
				log::error!("Failed to import glTF node hierarchy '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;

			let vertex_layouts = gltf
				.meshes()
				.flat_map(|mesh| {
					mesh.primitives().map(|primitive| {
						primitive
							.attributes()
							.filter_map(|(semantic, _)| gltf_vertex_component(semantic))
							.collect::<Vec<VertexComponent>>()
					})
				})
				.collect::<Vec<Vec<VertexComponent>>>();
			let vertex_layout = include_skin_vertex_layout(normalize_vertex_layouts(&vertex_layouts), &vertex_layouts)
				.map_err(|error| {
					log::error!("Failed to import glTF vertex layout '{}': {error}", url.as_ref());
					LoadErrors::FailedToProcess
				})?;

			// Preserve the existing all-scenes traversal order while sourcing transforms from the canonical node graph.
			let mut flat_tree = Vec::with_capacity(gltf.nodes().len());
			for scene in gltf.scenes() {
				for node in scene.nodes() {
					append_gltf_node_subtree(node, &mut flat_tree);
				}
			}
			let handedness = handedness_matrix();
			let flat_tree = flat_tree
				.into_iter()
				.map(|node| {
					let transform = handedness * graph.source_global_transforms[node.index()];
					(node, transform)
				})
				.collect::<Vec<_>>();

			let primitives = flat_tree
				.iter()
				.filter_map(|(node, _)| node.mesh().map(|mesh| mesh.primitives()))
				.flatten()
				.collect::<Vec<_>>();

			let mut skin_bindings = Vec::new();
			let mut skin_binding_by_node = HashMap::new();
			for (node, _) in &flat_tree {
				if node.mesh().is_none() || node.skin().is_none() || skin_binding_by_node.contains_key(&node.index()) {
					continue;
				}
				let binding = import_gltf_skin_binding(node, &buffers, &graph).map_err(|error| {
					log::error!("Failed to import glTF skin binding '{}': {error}", url.as_ref());
					LoadErrors::FailedToProcess
				})?;
				let binding_index = skin_bindings.len() as u32;
				skin_bindings.push(binding);
				skin_binding_by_node.insert(node.index(), binding_index);
			}
			let retain_skeleton = !skin_bindings.is_empty() || gltf.animations().next().is_some();

			let primitives_and_transform = flat_tree
				.iter()
				.filter_map(|(node, transform)| {
					let skin = skin_binding_by_node.get(&node.index()).copied();
					let transform_node = gltf_primitive_transform_node(&graph, node, retain_skeleton);
					node.mesh().map(|mesh| {
						mesh.primitives()
							.map(move |primitive| (primitive, *transform, transform_node, skin))
					})
				})
				.flatten()
				.collect::<Vec<_>>();

			let flat_mesh_tree = {
				primitives_and_transform
					.iter()
					.map(|(primitive, transform, transform_node, skin)| {
						(
							primitive,
							primitive.reader(|buffer| Some(&buffers[buffer.index()])),
							*transform,
							*transform_node,
							*skin,
						)
					})
			};

			let skeleton = if !retain_skeleton {
				None
			} else {
				let skeleton_id = generated_gltf_skeleton_id(source_id);
				Some(store_model::<SkeletonModel>(
					storage_backend,
					&skeleton_id,
					graph.skeleton,
					&[],
				)?)
			};

			let (unique_materials, material_indices_per_primitive) = unique_gltf_materials(&primitives);
			let mut resolved_materials = Vec::with_capacity(unique_materials.len());
			for material in unique_materials {
				let material = material_for_gltf_primitive(
					asset_manager,
					storage_backend,
					asset_storage_backend,
					spec,
					url,
					&gltf,
					&buffers,
					material,
					self.generator.clone(),
					allocator,
				)
				.await?;
				resolved_materials.push(material);
			}

			let primitives = flat_mesh_tree
				.zip(material_indices_per_primitive)
				.map(|((primitive, reader, transform, transform_node, skin), material_index)| {
					validate_gltf_flattened_animation_transform(transform, transform_node).map_err(|error| {
						log::error!("Failed to import glTF animated mesh transform '{}': {error}", url.as_ref());
						LoadErrors::FailedToProcess
					})?;
					validate_gltf_skin_attribute_sets(primitive, skin.is_some()).map_err(|error| {
						log::error!("Failed to import glTF vertex layout '{}': {error}", url.as_ref());
						LoadErrors::FailedToProcess
					})?;
					let material = resolved_materials[material_index].clone();
					let triangle_indices = reader
						.read_indices()
						.ok_or_else(|| {
							log::error!("glTF primitive has no triangle indices. The most likely cause is an unindexed source primitive.");
							LoadErrors::FailedToProcess
						})?
						.into_u32()
						.collect::<Vec<u32>>();
					let positions = reader
						.read_positions()
						.ok_or_else(|| {
							log::error!("glTF primitive has no positions. The most likely cause is a missing or malformed POSITION accessor.");
							LoadErrors::FailedToProcess
						})?
						.map(|position| {
							let position = maths_rs::Vec3f::new(position[0], position[1], position[2]);
							let transformed = transform * position;
							[transformed[0], transformed[1], transformed[2]]
						})
						.collect::<Vec<_>>();
					let vertex_count = positions.len();
					let bounds =
						bounding_box_from_positions(&positions).ok_or_else(|| {
							log::error!("glTF primitive bounds are invalid. The most likely cause is empty or non-finite position data.");
							LoadErrors::FailedToProcess
						})?;
					let mut primitive = OwnedMeshPrimitive::new(material, bounds, triangle_indices);
					primitive.set_transform_node(transform_node);
					primitive.set_skin(skin);
					primitive.add_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(positions),
					));

					if has_vertex_component(&vertex_layout, VertexSemantics::Normal, 0) {
						let normals = reader.read_normals().ok_or(LoadErrors::FailedToProcess)?;
						let normal_transform = gltf_normal_transform(transform).map_err(|error| {
							log::error!("Failed to import glTF vertex normals '{}': {error}", url.as_ref());
							LoadErrors::FailedToProcess
						})?;
						let normals = normals
							.map(|normal| transform_gltf_unit_direction(&normal_transform, normal))
							.collect::<Result<Vec<_>, _>>()
							.map_err(|error| {
								log::error!("Failed to import glTF vertex normals '{}': {error}", url.as_ref());
								LoadErrors::FailedToProcess
							})?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Normal,
							0,
							OwnedMeshAttributeData::F32x3(normals),
						));
					}

					if has_vertex_component(&vertex_layout, VertexSemantics::Tangent, 0) {
						let tangents = reader.read_tangents().ok_or(LoadErrors::FailedToProcess)?;
						let orientation = gltf_transform_orientation(transform).map_err(|error| {
							log::error!("Failed to import glTF vertex tangents '{}': {error}", url.as_ref());
							LoadErrors::FailedToProcess
						})?;
						let tangents = tangents
							.map(|tangent| transform_gltf_tangent(&transform, orientation, tangent))
							.collect::<Result<Vec<_>, _>>()
							.map_err(|error| {
								log::error!("Failed to import glTF vertex tangents '{}': {error}", url.as_ref());
								LoadErrors::FailedToProcess
							})?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Tangent,
							0,
							OwnedMeshAttributeData::F32x4(tangents),
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

					if let Some(skin) = skin {
						let joint_count = skin_bindings[skin as usize].len();
						let (joints, weights) =
							import_gltf_vertex_skin(&reader, vertex_count, joint_count).map_err(|error| {
								log::error!("Failed to import glTF vertex skin '{}': {error}", url.as_ref());
								LoadErrors::FailedToProcess
							})?;
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Joints,
							0,
							OwnedMeshAttributeData::U16x4(joints),
						));
						primitive.add_attribute(OwnedMeshAttribute::new(
							VertexSemantics::Weights,
							0,
							OwnedMeshAttributeData::F32x4(weights),
						));
					}

					Ok::<_, LoadErrors>(primitive)
				})
				.collect::<Result<Vec<_>, _>>()?;

			let mut mesh_source = OwnedMeshSource::new(vertex_layout, primitives).with_skins(skin_bindings);
			if let Some(skeleton) = skeleton {
				mesh_source = mesh_source.with_skeleton(skeleton);
			}
			let mesh = MeshProcessor::new()
				.with_triangle_front_face_winding(self.triangle_front_face_winding)
				.process_owned(mesh_source)
				.map_err(|_| LoadErrors::FailedToProcess)?;

			Ok((
				ProcessedAsset::new(url, mesh.mesh).with_streams(mesh.stream_descriptions),
				mesh.buffer,
			))
		})
	}
}

/// The `GltfNodeGraph` struct keeps the imported skeleton and source-node lookup data aligned for mesh and clip conversion.
struct GltfNodeGraph {
	skeleton: SkeletonModel,
	source_to_dense: Vec<u32>,
	source_global_transforms: Vec<maths_rs::Mat4f>,
}

/// Imports every glTF node into a deterministic parent-before-child graph shared by animation channels and skin bindings.
fn import_gltf_node_graph(gltf: &gltf::Gltf) -> Result<GltfNodeGraph, GltfSkeletalImportError> {
	let source_nodes = gltf.nodes().collect::<Vec<_>>();
	let mut source_parents = vec![None; source_nodes.len()];
	for node in &source_nodes {
		for child in node.children() {
			let parent = &mut source_parents[child.index()];
			if parent.replace(node.index()).is_some() {
				return Err(GltfSkeletalImportError::MultipleNodeParents);
			}
		}
	}

	let mut state = vec![0u8; source_nodes.len()];
	let mut source_to_dense = vec![u32::MAX; source_nodes.len()];
	let mut source_global_transforms = vec![maths_rs::Mat4f::identity(); source_nodes.len()];
	let mut nodes = Vec::with_capacity(source_nodes.len());

	// Source-index root ordering is stable even when files list scenes or scene roots in a different order.
	for source_index in 0..source_nodes.len() {
		if source_parents[source_index].is_none() {
			append_gltf_skeleton_subtree(
				source_index,
				&source_nodes,
				&source_parents,
				&mut state,
				&mut source_to_dense,
				&mut source_global_transforms,
				&mut nodes,
			)?;
		}
	}

	if state.iter().any(|state| *state != 2) {
		return Err(GltfSkeletalImportError::CyclicNodeHierarchy);
	}

	Ok(GltfNodeGraph {
		skeleton: SkeletonModel { nodes },
		source_to_dense,
		source_global_transforms,
	})
}

/// Appends one source subtree while computing source-space global transforms used to adjust inverse bind matrices.
fn append_gltf_skeleton_subtree(
	source_index: usize,
	source_nodes: &[gltf::Node<'_>],
	source_parents: &[Option<usize>],
	state: &mut [u8],
	source_to_dense: &mut [u32],
	source_global_transforms: &mut [maths_rs::Mat4f],
	nodes: &mut Vec<SkeletonNode>,
) -> Result<(), GltfSkeletalImportError> {
	match state[source_index] {
		1 => return Err(GltfSkeletalImportError::CyclicNodeHierarchy),
		2 => return Ok(()),
		_ => {}
	}
	state[source_index] = 1;

	let source_node = &source_nodes[source_index];
	let source_local = mat4_from_columns(source_node.transform().matrix());
	validate_finite_matrix(&source_local, "node transform")?;
	let parent = source_parents[source_index].map(|source_parent| source_to_dense[source_parent]);
	if parent == Some(u32::MAX) {
		return Err(GltfSkeletalImportError::CyclicNodeHierarchy);
	}
	let source_global = source_parents[source_index]
		.map(|source_parent| source_global_transforms[source_parent] * source_local)
		.unwrap_or(source_local);
	let (translation, rotation, scale) = source_node.transform().decomposed();
	let rest_local = convert_gltf_local_transform(translation, rotation, scale)?;
	let dense_index = nodes.len() as u32;
	source_to_dense[source_index] = dense_index;
	source_global_transforms[source_index] = source_global;
	nodes.push(SkeletonNode {
		name: source_node.name().map(ToString::to_string),
		parent,
		rest_local,
	});

	for child in source_node.children() {
		append_gltf_skeleton_subtree(
			child.index(),
			source_nodes,
			source_parents,
			state,
			source_to_dense,
			source_global_transforms,
			nodes,
		)?;
	}
	state[source_index] = 2;
	Ok(())
}

/// Appends scene nodes in authored traversal order for the existing flattened-mesh behavior.
fn append_gltf_node_subtree<'a>(node: gltf::Node<'a>, nodes: &mut Vec<gltf::Node<'a>>) {
	nodes.push(node.clone());
	for child in node.children() {
		append_gltf_node_subtree(child, nodes);
	}
}

/// Retains the dense node identity needed to drive both skinned and rigid primitives from CPU animation output.
fn gltf_primitive_transform_node(graph: &GltfNodeGraph, node: &gltf::Node<'_>, retain_skeleton: bool) -> Option<u32> {
	retain_skeleton.then_some(graph.source_to_dense[node.index()])
}

/// Rejects a singular bind transform when flattened geometry must later be recovered by a CPU-driven node pose.
fn validate_gltf_flattened_animation_transform(
	transform: maths_rs::Mat4f,
	transform_node: Option<u32>,
) -> Result<(), GltfSkeletalImportError> {
	if transform_node.is_none() {
		return Ok(());
	}

	let determinant = transform.determinant();
	if determinant.is_finite() && determinant.abs() > f32::EPSILON {
		Ok(())
	} else {
		Err(GltfSkeletalImportError::SingularMeshTransform)
	}
}

/// Converts glTF local TRS values from right-handed coordinates into the engine's left-handed basis.
fn convert_gltf_local_transform(
	translation: [f32; 3],
	rotation: [f32; 4],
	scale: [f32; 3],
) -> Result<LocalTransform, GltfSkeletalImportError> {
	if translation
		.iter()
		.chain(rotation.iter())
		.chain(scale.iter())
		.any(|component| !component.is_finite())
	{
		return Err(GltfSkeletalImportError::NonFinite("node local transform"));
	}
	let rotation = normalize_gltf_quaternion_value([-rotation[0], -rotation[1], rotation[2], rotation[3]])
		.map_err(|_| GltfSkeletalImportError::InvalidRestRotation)?;

	Ok(LocalTransform {
		translation: [translation[0], translation[1], -translation[2]],
		rotation,
		scale,
	})
}

fn handedness_matrix() -> maths_rs::Mat4f {
	maths_rs::Mat4f::from_scale(Vec3::new(1.0, 1.0, -1.0))
}

/// Builds the inverse-transpose matrix required to preserve normals under nonuniform node scale.
fn gltf_normal_transform(transform: maths_rs::Mat4f) -> Result<maths_rs::Mat4f, GltfSkeletalImportError> {
	let determinant = transform.determinant();
	if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}
	Ok(transform.inverse().transpose())
}

/// Reports whether an affine transform preserves or flips tangent-space handedness.
fn gltf_transform_orientation(transform: maths_rs::Mat4f) -> Result<f32, GltfSkeletalImportError> {
	let determinant = transform.determinant();
	if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}
	Ok(determinant.signum())
}

/// Applies only the linear matrix portion to a direction and returns a normalized result without allocating.
fn transform_gltf_unit_direction(
	transform: &maths_rs::Mat4f,
	direction: [f32; 3],
) -> Result<[f32; 3], GltfSkeletalImportError> {
	let mut transformed = [
		transform[(0, 0)] * direction[0] + transform[(0, 1)] * direction[1] + transform[(0, 2)] * direction[2],
		transform[(1, 0)] * direction[0] + transform[(1, 1)] * direction[1] + transform[(1, 2)] * direction[2],
		transform[(2, 0)] * direction[0] + transform[(2, 1)] * direction[1] + transform[(2, 2)] * direction[2],
	];
	let length_squared = transformed.iter().map(|component| component * component).sum::<f32>();
	if !length_squared.is_finite() || length_squared <= f32::MIN_POSITIVE {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}
	let inverse_length = length_squared.sqrt().recip();
	for component in &mut transformed {
		*component *= inverse_length;
	}
	Ok(transformed)
}

/// Transforms and normalizes a tangent while carrying affine reflection into its handedness sign.
fn transform_gltf_tangent(
	transform: &maths_rs::Mat4f,
	orientation: f32,
	tangent: [f32; 4],
) -> Result<[f32; 4], GltfSkeletalImportError> {
	if !tangent[3].is_finite() {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}
	let direction = transform_gltf_unit_direction(transform, [tangent[0], tangent[1], tangent[2]])?;
	Ok([direction[0], direction[1], direction[2], tangent[3] * orientation])
}

/// Converts the column-major matrix representation used by glTF resources into maths-rs row-major storage.
fn mat4_from_columns(matrix: [[f32; 4]; 4]) -> maths_rs::Mat4f {
	maths_rs::Mat4f::new(
		matrix[0][0],
		matrix[1][0],
		matrix[2][0],
		matrix[3][0],
		matrix[0][1],
		matrix[1][1],
		matrix[2][1],
		matrix[3][1],
		matrix[0][2],
		matrix[1][2],
		matrix[2][2],
		matrix[3][2],
		matrix[0][3],
		matrix[1][3],
		matrix[2][3],
		matrix[3][3],
	)
}

/// Converts maths-rs row-major storage back into the resource model's column-major matrix representation.
fn mat4_to_columns(matrix: maths_rs::Mat4f) -> Matrix4Columns {
	[
		[matrix[(0, 0)], matrix[(1, 0)], matrix[(2, 0)], matrix[(3, 0)]],
		[matrix[(0, 1)], matrix[(1, 1)], matrix[(2, 1)], matrix[(3, 1)]],
		[matrix[(0, 2)], matrix[(1, 2)], matrix[(2, 2)], matrix[(3, 2)]],
		[matrix[(0, 3)], matrix[(1, 3)], matrix[(2, 3)], matrix[(3, 3)]],
	]
}

/// Rejects non-finite matrix components before they enter serializable skeletal resources.
fn validate_finite_matrix(matrix: &maths_rs::Mat4f, context: &'static str) -> Result<(), GltfSkeletalImportError> {
	if matrix.m.iter().all(|component| component.is_finite()) {
		Ok(())
	} else {
		Err(GltfSkeletalImportError::NonFinite(context))
	}
}

#[derive(Debug, PartialEq, Eq)]
enum GltfSkeletalImportError {
	MultipleNodeParents,
	CyclicNodeHierarchy,
	AnimationNotFound(String),
	MissingAnimationInput,
	MissingAnimationOutput,
	InvalidAnimationTimes,
	InvalidAnimationOutput,
	InvalidRestRotation,
	DuplicateAnimationTrack,
	MorphTargetAnimationUnsupported,
	MissingSkin,
	MissingSkinJoint,
	MismatchedInverseBindMatrices,
	TooManySkinJoints,
	SingularMeshTransform,
	UnpairedSkinAttributes(u32),
	UnsupportedSkinAttributeSet(u32),
	MissingSkinAttributes,
	MismatchedSkinAttributeCount,
	InvalidSkinWeight,
	SkinJointOutOfRange,
	InvalidVertexDirection,
	NonFinite(&'static str),
}

impl std::fmt::Display for GltfSkeletalImportError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::MultipleNodeParents => write!(formatter, "glTF node hierarchy is invalid. The most likely cause is a node referenced by multiple parents."),
			Self::CyclicNodeHierarchy => write!(formatter, "glTF node hierarchy is cyclic. The most likely cause is malformed child-node references."),
			Self::AnimationNotFound(selector) => write!(formatter, "glTF animation was not found. The most likely cause is an incorrect animation selector '{selector}'."),
			Self::MissingAnimationInput => write!(formatter, "glTF animation input is missing. The most likely cause is a malformed sampler input accessor."),
			Self::MissingAnimationOutput => write!(formatter, "glTF animation output is missing. The most likely cause is a malformed sampler output accessor."),
			Self::InvalidAnimationTimes => write!(formatter, "glTF animation times are invalid. The most likely cause is non-finite or non-increasing key times."),
			Self::InvalidAnimationOutput => write!(formatter, "glTF animation output is invalid. The most likely cause is a sampler output type or key count that does not match its target."),
			Self::InvalidRestRotation => write!(formatter, "glTF rest rotation is invalid. The most likely cause is a zero-length or non-finite node quaternion."),
			Self::DuplicateAnimationTrack => write!(formatter, "glTF animation track is duplicated. The most likely cause is multiple channels targeting the same node property."),
			Self::MorphTargetAnimationUnsupported => write!(formatter, "glTF morph-target animation is unsupported. The most likely cause is a selected clip mixing skeletal and morph-weight channels."),
			Self::MissingSkin => write!(formatter, "glTF skin binding is missing. The most likely cause is a skinned mesh node without a valid skin."),
			Self::MissingSkinJoint => write!(formatter, "glTF skin joint is missing. The most likely cause is a skin referencing a node outside the imported hierarchy."),
			Self::MismatchedInverseBindMatrices => write!(formatter, "glTF inverse bind matrices are invalid. The most likely cause is an accessor count that does not match the skin joint count."),
			Self::TooManySkinJoints => write!(formatter, "glTF skin has too many joints. The most likely cause is a palette larger than the u16 vertex-joint stream."),
			Self::SingularMeshTransform => write!(formatter, "glTF animated mesh transform is singular. The most likely cause is a zero bind scale that cannot be recovered after flattening geometry."),
			Self::UnpairedSkinAttributes(set) => write!(formatter, "glTF skin attribute set {set} is incomplete. The most likely cause is JOINTS_{set} without matching WEIGHTS_{set}, or vice versa."),
			Self::UnsupportedSkinAttributeSet(set) => write!(formatter, "glTF skin attribute set {set} is unsupported. The most likely cause is a primitive containing more than eight joint influences per vertex."),
			Self::MissingSkinAttributes => write!(formatter, "glTF skinned primitive has no joint weights. The most likely cause is a skin node referencing geometry without JOINTS_0 and WEIGHTS_0."),
			Self::MismatchedSkinAttributeCount => write!(formatter, "glTF skin attribute count is invalid. The most likely cause is joint or weight streams that do not contain one value per vertex."),
			Self::InvalidSkinWeight => write!(formatter, "glTF skin weight is invalid. The most likely cause is non-finite, negative, or zero-sum vertex influences."),
			Self::SkinJointOutOfRange => write!(formatter, "glTF vertex joint is out of range. The most likely cause is a JOINTS value outside the selected skin palette."),
			Self::InvalidVertexDirection => write!(formatter, "glTF vertex direction is invalid. The most likely cause is a zero-length direction or a singular node transform."),
			Self::NonFinite(context) => write!(formatter, "glTF numeric data is invalid. The most likely cause is a non-finite {context}."),
		}
	}
}

impl std::error::Error for GltfSkeletalImportError {}

fn is_gltf_animation_fragment(fragment: &str) -> bool {
	fragment == DEFAULT_ANIMATION_FRAGMENT || fragment.starts_with(ANIMATION_FRAGMENT_PREFIX)
}

fn generated_gltf_skeleton_id(source: ResourceId<'_>) -> String {
	format!("{}#{SKELETON_FRAGMENT}", source.get_base().as_ref())
}

/// Selects the first, indexed, or named clip addressed by a reserved glTF animation fragment.
fn select_gltf_animation<'a>(gltf: &'a gltf::Gltf, fragment: &str) -> Result<gltf::Animation<'a>, GltfSkeletalImportError> {
	if fragment == DEFAULT_ANIMATION_FRAGMENT {
		return gltf
			.animations()
			.next()
			.ok_or_else(|| GltfSkeletalImportError::AnimationNotFound("first animation".to_string()));
	}

	let selector = fragment
		.strip_prefix(ANIMATION_FRAGMENT_PREFIX)
		.ok_or_else(|| GltfSkeletalImportError::AnimationNotFound(fragment.to_string()))?;
	if selector.is_empty() {
		return Err(GltfSkeletalImportError::AnimationNotFound("empty selector".to_string()));
	}
	if let Ok(index) = selector.parse::<usize>() {
		return gltf
			.animations()
			.nth(index)
			.ok_or_else(|| GltfSkeletalImportError::AnimationNotFound(format!("index {index}")));
	}

	gltf.animations()
		.find(|animation| animation.name() == Some(selector))
		.ok_or_else(|| GltfSkeletalImportError::AnimationNotFound(selector.to_string()))
}

/// Marks only the source buffers needed by one selected clip so unrelated mesh payloads stay unloaded.
fn required_gltf_animation_buffers(gltf: &gltf::Gltf, fragment: &str) -> Result<Vec<bool>, GltfSkeletalImportError> {
	let animation = select_gltf_animation(gltf, fragment)?;
	let mut required = vec![false; gltf.buffers().len()];
	for channel in animation.channels() {
		let sampler = channel.sampler();
		mark_gltf_accessor_buffers(sampler.input(), &mut required);
		mark_gltf_accessor_buffers(sampler.output(), &mut required);
	}
	Ok(required)
}

/// Marks regular and sparse storage used by one accessor without allocating temporary index lists.
fn mark_gltf_accessor_buffers(accessor: gltf::Accessor<'_>, required: &mut [bool]) {
	if let Some(view) = accessor.view() {
		required[view.buffer().index()] = true;
	}
	if let Some(sparse) = accessor.sparse() {
		required[sparse.indices().view().buffer().index()] = true;
		required[sparse.values().view().buffer().index()] = true;
	}
}

/// Converts one glTF clip into node-indexed curves ready for a future CPU animation graph.
fn import_gltf_animation(
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	fragment: &str,
	source_to_dense: &[u32],
	skeleton: ReferenceModel<SkeletonModel>,
) -> Result<AnimationModel, GltfSkeletalImportError> {
	let animation = select_gltf_animation(gltf, fragment)?;
	let mut tracks = Vec::<NodeTrack>::with_capacity(animation.channels().count());
	let mut duration = 0.0f32;

	for channel in animation.channels() {
		let target = channel.target();
		let property = target.property();
		if property == gltf::animation::Property::MorphTargetWeights {
			return Err(GltfSkeletalImportError::MorphTargetAnimationUnsupported);
		}
		let source_node = target.node().index();
		let dense_node = *source_to_dense
			.get(source_node)
			.filter(|dense| **dense != u32::MAX)
			.ok_or(GltfSkeletalImportError::MissingSkinJoint)?;
		let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
		let times = reader
			.read_inputs()
			.ok_or(GltfSkeletalImportError::MissingAnimationInput)?
			.collect::<Vec<_>>();
		validate_animation_times(&times)?;
		duration = duration.max(times.last().copied().unwrap_or(0.0));
		let outputs = reader.read_outputs().ok_or(GltfSkeletalImportError::MissingAnimationOutput)?;
		let interpolation = channel.sampler().interpolation();
		let track_index = match tracks.binary_search_by_key(&dense_node, |track| track.node) {
			Ok(index) => index,
			Err(index) => {
				tracks.insert(
					index,
					NodeTrack {
						node: dense_node,
						translation: None,
						rotation: None,
						scale: None,
					},
				);
				index
			}
		};
		let track = &mut tracks[track_index];

		match (property, outputs) {
			(gltf::animation::Property::Translation, gltf::animation::util::ReadOutputs::Translations(values)) => {
				let values = values
					.map(|value| convert_gltf_vector3(value, GltfVector3Semantic::Translation))
					.collect::<Result<Vec<_>, _>>()?;
				let curve = make_vector3_curve(interpolation, times, values)?;
				if track.translation.replace(curve).is_some() {
					return Err(GltfSkeletalImportError::DuplicateAnimationTrack);
				}
			}
			(gltf::animation::Property::Scale, gltf::animation::util::ReadOutputs::Scales(values)) => {
				let values = values
					.map(|value| convert_gltf_vector3(value, GltfVector3Semantic::Scale))
					.collect::<Result<Vec<_>, _>>()?;
				let curve = make_vector3_curve(interpolation, times, values)?;
				if track.scale.replace(curve).is_some() {
					return Err(GltfSkeletalImportError::DuplicateAnimationTrack);
				}
			}
			(gltf::animation::Property::Rotation, gltf::animation::util::ReadOutputs::Rotations(values)) => {
				let values = values
					.into_f32()
					.map(convert_gltf_quaternion)
					.collect::<Result<Vec<_>, _>>()?;
				let curve = make_quaternion_curve(interpolation, times, values)?;
				if track.rotation.replace(curve).is_some() {
					return Err(GltfSkeletalImportError::DuplicateAnimationTrack);
				}
			}
			_ => return Err(GltfSkeletalImportError::InvalidAnimationOutput),
		}
	}

	Ok(AnimationModel {
		name: animation.name().map(ToString::to_string),
		skeleton,
		duration,
		tracks,
	})
}

/// Validates the finite, non-negative, strictly increasing key order required by CPU clip evaluation.
fn validate_animation_times(times: &[f32]) -> Result<(), GltfSkeletalImportError> {
	if times.is_empty()
		|| times.iter().any(|time| !time.is_finite() || *time < 0.0)
		|| times.windows(2).any(|pair| pair[0] >= pair[1])
	{
		Err(GltfSkeletalImportError::InvalidAnimationTimes)
	} else {
		Ok(())
	}
}

#[derive(Clone, Copy)]
enum GltfVector3Semantic {
	Translation,
	Scale,
}

fn convert_gltf_vector3(value: [f32; 3], semantic: GltfVector3Semantic) -> Result<[f32; 3], GltfSkeletalImportError> {
	if value.iter().any(|component| !component.is_finite()) {
		return Err(GltfSkeletalImportError::NonFinite("animation vector key"));
	}
	Ok(match semantic {
		GltfVector3Semantic::Translation => [value[0], value[1], -value[2]],
		GltfVector3Semantic::Scale => value,
	})
}

fn convert_gltf_quaternion(value: [f32; 4]) -> Result<[f32; 4], GltfSkeletalImportError> {
	if value.iter().any(|component| !component.is_finite()) {
		return Err(GltfSkeletalImportError::NonFinite("animation quaternion key"));
	}
	Ok([-value[0], -value[1], value[2], value[3]])
}

/// Splits glTF's interleaved cubic spline triplets into graph-friendly tangent and value arrays.
fn make_vector3_curve(
	interpolation: gltf::animation::Interpolation,
	times: Vec<f32>,
	values: Vec<[f32; 3]>,
) -> Result<Vector3Curve, GltfSkeletalImportError> {
	match interpolation {
		gltf::animation::Interpolation::Step if values.len() == times.len() => Ok(Vector3Curve::Step { times, values }),
		gltf::animation::Interpolation::Linear if values.len() == times.len() => Ok(Vector3Curve::Linear { times, values }),
		gltf::animation::Interpolation::CubicSpline if values.len() == times.len().saturating_mul(3) => {
			let mut in_tangents = Vec::with_capacity(times.len());
			let mut key_values = Vec::with_capacity(times.len());
			let mut out_tangents = Vec::with_capacity(times.len());
			for triplet in values.chunks_exact(3) {
				in_tangents.push(triplet[0]);
				key_values.push(triplet[1]);
				out_tangents.push(triplet[2]);
			}
			Ok(Vector3Curve::CubicSpline {
				times,
				values: key_values,
				in_tangents,
				out_tangents,
			})
		}
		_ => Err(GltfSkeletalImportError::InvalidAnimationOutput),
	}
}

/// Splits quaternion cubic spline triplets without normalizing derivative tangents.
fn make_quaternion_curve(
	interpolation: gltf::animation::Interpolation,
	times: Vec<f32>,
	values: Vec<[f32; 4]>,
) -> Result<QuaternionCurve, GltfSkeletalImportError> {
	match interpolation {
		gltf::animation::Interpolation::Step if values.len() == times.len() => Ok(QuaternionCurve::Step {
			times,
			values: values
				.into_iter()
				.map(normalize_gltf_quaternion_value)
				.collect::<Result<Vec<_>, _>>()?,
		}),
		gltf::animation::Interpolation::Linear if values.len() == times.len() => Ok(QuaternionCurve::Linear {
			times,
			values: values
				.into_iter()
				.map(normalize_gltf_quaternion_value)
				.collect::<Result<Vec<_>, _>>()?,
		}),
		gltf::animation::Interpolation::CubicSpline if values.len() == times.len().saturating_mul(3) => {
			let mut in_tangents = Vec::with_capacity(times.len());
			let mut key_values = Vec::with_capacity(times.len());
			let mut out_tangents = Vec::with_capacity(times.len());
			for triplet in values.chunks_exact(3) {
				in_tangents.push(triplet[0]);
				key_values.push(normalize_gltf_quaternion_value(triplet[1])?);
				out_tangents.push(triplet[2]);
			}
			Ok(QuaternionCurve::CubicSpline {
				times,
				values: key_values,
				in_tangents,
				out_tangents,
			})
		}
		_ => Err(GltfSkeletalImportError::InvalidAnimationOutput),
	}
}

/// Normalizes a quaternion key while rejecting values that cannot represent a rotation.
fn normalize_gltf_quaternion_value(mut value: [f32; 4]) -> Result<[f32; 4], GltfSkeletalImportError> {
	let length_squared = value.iter().map(|component| component * component).sum::<f32>();
	if !length_squared.is_finite() || length_squared <= f32::MIN_POSITIVE {
		return Err(GltfSkeletalImportError::InvalidAnimationOutput);
	}
	let inverse_length = length_squared.sqrt().recip();
	for component in &mut value {
		*component *= inverse_length;
	}
	Ok(value)
}

/// Imports one mesh-node skin and adjusts source inverse binds for the handler's flattened bind-pose vertices.
fn import_gltf_skin_binding(
	node: &gltf::Node<'_>,
	buffers: &[gltf::buffer::Data],
	graph: &GltfNodeGraph,
) -> Result<SkinBinding, GltfSkeletalImportError> {
	let skin = node.skin().ok_or(GltfSkeletalImportError::MissingSkin)?;
	let joint_count = skin.joints().count();
	if joint_count > MAX_SKIN_JOINTS {
		return Err(GltfSkeletalImportError::TooManySkinJoints);
	}
	let source_global = *graph
		.source_global_transforms
		.get(node.index())
		.ok_or(GltfSkeletalImportError::MissingSkinJoint)?;
	let determinant = source_global.determinant();
	if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
		return Err(GltfSkeletalImportError::SingularMeshTransform);
	}
	let inverse_source_global = source_global.inverse();
	let handedness = handedness_matrix();
	let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
	let remap_joint = |joint: gltf::Node<'_>| {
		graph
			.source_to_dense
			.get(joint.index())
			.filter(|dense| **dense != u32::MAX)
			.copied()
			.map(SkinJoint::Node)
			.ok_or(GltfSkeletalImportError::MissingSkinJoint)
	};
	let mut entries = Vec::with_capacity(joint_count);
	if let Some(inverse_binds) = reader.read_inverse_bind_matrices() {
		if inverse_binds.len() != joint_count {
			return Err(GltfSkeletalImportError::MismatchedInverseBindMatrices);
		}
		for (joint, inverse_bind) in skin.joints().zip(inverse_binds) {
			entries.push(SkinPaletteEntry {
				joint: remap_joint(joint)?,
				adjusted_inverse_bind_matrix: adjust_gltf_inverse_bind(inverse_bind, inverse_source_global, handedness)?,
			});
		}
	} else {
		for joint in skin.joints() {
			entries.push(SkinPaletteEntry {
				joint: remap_joint(joint)?,
				adjusted_inverse_bind_matrix: adjust_gltf_inverse_bind(
					identity_matrix4_columns(),
					inverse_source_global,
					handedness,
				)?,
			});
		}
	}

	Ok(SkinBinding { entries })
}

/// Converts one source inverse bind into the flattened left-handed vertex basis used by the mesh resource.
fn adjust_gltf_inverse_bind(
	inverse_bind: Matrix4Columns,
	inverse_source_global: maths_rs::Mat4f,
	handedness: maths_rs::Mat4f,
) -> Result<Matrix4Columns, GltfSkeletalImportError> {
	let inverse_bind = mat4_from_columns(inverse_bind);
	validate_finite_matrix(&inverse_bind, "inverse bind matrix")?;
	// Vertices are flattened by S*G, so S*IBM*inverse(G)*S keeps
	// J_lh*adjustedIBM*flattenedVertex equivalent to S*J*IBM*vertex.
	let adjusted = handedness * inverse_bind * inverse_source_global * handedness;
	validate_finite_matrix(&adjusted, "adjusted inverse bind matrix")?;
	Ok(mat4_to_columns(adjusted))
}

/// Reads both supported glTF influence sets, keeps the strongest four, and normalizes the fixed GPU stream shape.
fn import_gltf_vertex_skin<'a, 's, F>(
	reader: &gltf::mesh::Reader<'a, 's, F>,
	vertex_count: usize,
	joint_count: usize,
) -> Result<(Vec<[u16; 4]>, Vec<[f32; 4]>), GltfSkeletalImportError>
where
	F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
	let set0_joints = reader.read_joints(0);
	let set0_weights = reader.read_weights(0);
	if set0_joints.is_some() != set0_weights.is_some() {
		return Err(GltfSkeletalImportError::UnpairedSkinAttributes(0));
	}
	let (Some(set0_joints), Some(set0_weights)) = (set0_joints, set0_weights) else {
		return Err(GltfSkeletalImportError::MissingSkinAttributes);
	};
	let mut set0_joints = set0_joints.into_u16();
	let mut set0_weights = set0_weights.into_f32();

	let set1_joints = reader.read_joints(1);
	let set1_weights = reader.read_weights(1);
	if set1_joints.is_some() != set1_weights.is_some() {
		return Err(GltfSkeletalImportError::UnpairedSkinAttributes(1));
	}
	let mut set1 = match (set1_joints, set1_weights) {
		(Some(joints), Some(weights)) => Some((joints.into_u16(), weights.into_f32())),
		(None, None) => None,
		_ => unreachable!("paired skin attributes were checked above"),
	};

	if set0_joints.len() != vertex_count
		|| set0_weights.len() != vertex_count
		|| set1
			.as_ref()
			.is_some_and(|(joints, weights)| joints.len() != vertex_count || weights.len() != vertex_count)
	{
		return Err(GltfSkeletalImportError::MismatchedSkinAttributeCount);
	}

	let mut output_joints = Vec::with_capacity(vertex_count);
	let mut output_weights = Vec::with_capacity(vertex_count);
	for _ in 0..vertex_count {
		let mut influences = [(0u16, 0.0f32); 8];
		let joints = set0_joints
			.next()
			.ok_or(GltfSkeletalImportError::MismatchedSkinAttributeCount)?;
		let weights = set0_weights
			.next()
			.ok_or(GltfSkeletalImportError::MismatchedSkinAttributeCount)?;
		for influence in 0..4 {
			influences[influence] = (joints[influence], weights[influence]);
		}
		let influence_count = if let Some((joints, weights)) = &mut set1 {
			let joints = joints.next().ok_or(GltfSkeletalImportError::MismatchedSkinAttributeCount)?;
			let weights = weights.next().ok_or(GltfSkeletalImportError::MismatchedSkinAttributeCount)?;
			for influence in 0..4 {
				influences[influence + 4] = (joints[influence], weights[influence]);
			}
			8
		} else {
			4
		};

		for &(joint, weight) in &influences[..influence_count] {
			if joint as usize >= joint_count {
				return Err(GltfSkeletalImportError::SkinJointOutOfRange);
			}
			if !weight.is_finite() || weight < 0.0 {
				return Err(GltfSkeletalImportError::InvalidSkinWeight);
			}
		}
		influences[..influence_count]
			.sort_unstable_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
		let total = influences[..4].iter().map(|(_, weight)| *weight).sum::<f32>();
		if !total.is_finite() || total <= f32::EPSILON {
			return Err(GltfSkeletalImportError::InvalidSkinWeight);
		}
		let mut joints = [0u16; 4];
		let mut weights = [0.0f32; 4];
		for influence in 0..4 {
			joints[influence] = influences[influence].0;
			weights[influence] = influences[influence].1 / total;
		}
		output_joints.push(joints);
		output_weights.push(weights);
	}

	Ok((output_joints, output_weights))
}

/// Validates the paired influence sets consumed by skinned instances while allowing a shared mesh to be instanced rigidly.
fn validate_gltf_skin_attribute_sets(primitive: &gltf::Primitive<'_>, is_skinned: bool) -> Result<(), GltfSkeletalImportError> {
	// A mesh may be instanced by both skinned and rigid nodes; rigid instances deliberately ignore complete skin streams.
	if !is_skinned {
		return Ok(());
	}
	let mut joints = [false; 2];
	let mut weights = [false; 2];
	for (semantic, _) in primitive.attributes() {
		match semantic {
			gltf::Semantic::Joints(set) if set > 1 => {
				return Err(GltfSkeletalImportError::UnsupportedSkinAttributeSet(set));
			}
			gltf::Semantic::Weights(set) if set > 1 => {
				return Err(GltfSkeletalImportError::UnsupportedSkinAttributeSet(set));
			}
			gltf::Semantic::Joints(set) => joints[set as usize] = true,
			gltf::Semantic::Weights(set) => weights[set as usize] = true,
			_ => {}
		}
	}
	for set in 0..=1 {
		if joints[set] != weights[set] {
			return Err(GltfSkeletalImportError::UnpairedSkinAttributes(set as u32));
		}
	}
	if !joints[0] {
		return Err(GltfSkeletalImportError::MissingSkinAttributes);
	}
	Ok(())
}

/// Keeps the existing shared-layout policy for rendering attributes while retaining aligned skin streams for mixed meshes.
fn include_skin_vertex_layout(
	mut normalized: Vec<VertexComponent>,
	vertex_layouts: &[Vec<VertexComponent>],
) -> Result<Vec<VertexComponent>, GltfSkeletalImportError> {
	let has_joints = vertex_layouts
		.iter()
		.flatten()
		.any(|component| component.semantic == VertexSemantics::Joints);
	let has_weights = vertex_layouts
		.iter()
		.flatten()
		.any(|component| component.semantic == VertexSemantics::Weights);
	if has_joints != has_weights {
		return Err(GltfSkeletalImportError::UnpairedSkinAttributes(0));
	}
	if has_joints {
		for component in [
			VertexComponent {
				semantic: VertexSemantics::Joints,
				format: "vec4u16".to_string(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Weights,
				format: "vec4f".to_string(),
				channel: 0,
			},
		] {
			if !normalized.iter().any(|existing| existing.semantic == component.semantic) {
				normalized.push(component);
			}
		}
	}
	Ok(normalized)
}

/// Recomputes finite bounds after node transforms are baked into flattened vertex positions.
fn bounding_box_from_positions(positions: &[[f32; 3]]) -> Option<[[f32; 3]; 2]> {
	let first = *positions.first()?;
	if first.iter().any(|component| !component.is_finite()) {
		return None;
	}
	let mut minimum = first;
	let mut maximum = first;
	for position in &positions[1..] {
		if position.iter().any(|component| !component.is_finite()) {
			return None;
		}
		for axis in 0..3 {
			minimum[axis] = minimum[axis].min(position[axis]);
			maximum[axis] = maximum[axis].max(position[axis]);
		}
	}
	Some([minimum, maximum])
}

fn unique_gltf_materials<'a>(primitives: &[gltf::Primitive<'a>]) -> (Vec<gltf::Material<'a>>, Vec<usize>) {
	let mut unique_materials = Vec::new();
	let mut unique_material_indices = HashMap::new();
	let mut material_indices_per_primitive = Vec::with_capacity(primitives.len());

	for primitive in primitives {
		let material = primitive.material();
		let key = material.index();
		let material_index = if let Some(index) = unique_material_indices.get(&key) {
			*index
		} else {
			let index = unique_materials.len();
			unique_materials.push(material);
			unique_material_indices.insert(key, index);
			index
		};
		material_indices_per_primitive.push(material_index);
	}

	(unique_materials, material_indices_per_primitive)
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
	allocator: &dyn std::alloc::Allocator,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	if let Some(override_asset) = material_override(spec, &material) {
		return asset_manager
			.bake_if_not_exists_in::<VariantModel>(&override_asset, storage_backend, allocator)
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
		allocator,
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
	allocator: &dyn std::alloc::Allocator,
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
		allocator,
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
		.unwrap_or_else(|| match material.index() {
			Some(index) => format!("material_{index}"),
			None => "material_default".to_string(),
		});
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
		.store(resource, data)
		.map(|resource| resource.into())
		.map_err(|_| LoadErrors::FailedToProcess)
}

/// The `GltfTextureDependency` struct records a glTF image required by a generated material variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GltfTextureDependency {
	image_index: u32,
	semantic: Semantic,
}

/// Loads every declared glTF buffer in source order so accessor indices remain valid for meshes, skins, and clips.
async fn load_gltf_buffers(
	asset_storage_backend: &dyn asset::StorageBackend,
	source: ResourceId<'_>,
	gltf: &gltf::Gltf,
	mut binary_blob: Option<std::borrow::Cow<'_, [u8]>>,
	required: Option<&[bool]>,
	allocator: &dyn std::alloc::Allocator,
) -> Result<Vec<gltf::buffer::Data>, LoadErrors> {
	let mut buffers = Vec::with_capacity(gltf.buffers().len());
	for buffer in gltf.buffers() {
		if required.is_some_and(|required| !required.get(buffer.index()).copied().unwrap_or(false)) {
			buffers.push(gltf::buffer::Data(Vec::new()));
			continue;
		}
		let mut data = match buffer.source() {
			gltf::buffer::Source::Bin => binary_blob.take().map(std::borrow::Cow::into_owned).ok_or_else(|| {
				log::error!("glTF binary buffer is missing. The most likely cause is a GLB without its required BIN chunk.");
				LoadErrors::FailedToProcess
			})?,
			gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => decode_gltf_buffer_data_uri(uri)?,
			gltf::buffer::Source::Uri(uri) => {
				let buffer_url = resolve_gltf_uri(source, uri)?;
				let (bytes, ..) = asset_storage_backend
					.resolve_in(ResourceId::new(&buffer_url), allocator)
					.await
					.map_err(|_| {
						log::error!(
							"glTF external buffer could not be loaded. The most likely cause is a missing file-local URI '{buffer_url}'."
						);
						LoadErrors::AssetCouldNotBeLoaded
					})?;
				copy_gltf_buffer_bytes(&bytes)?
			}
		};

		let raw_length = data.len();
		if raw_length < buffer.length() {
			log::error!(
				"glTF buffer is shorter than declared. The most likely cause is truncated data for buffer {}: expected at least {} bytes but loaded {}.",
				buffer.index(),
				buffer.length(),
				raw_length
			);
			return Err(LoadErrors::FailedToProcess);
		}

		// Reserve once before adding the alignment bytes required by glTF buffer-view access.
		let aligned_length = aligned_gltf_buffer_length(raw_length)?;
		if data.capacity() < aligned_length {
			data.reserve_exact(aligned_length - raw_length);
		}
		data.resize(aligned_length, 0);
		buffers.push(gltf::buffer::Data(data));
	}

	Ok(buffers)
}

/// Decodes a glTF data URI into storage with enough capacity for final four-byte alignment.
fn decode_gltf_buffer_data_uri(uri: &str) -> Result<Vec<u8>, LoadErrors> {
	let data = uri.strip_prefix("data:").ok_or_else(|| {
		log::error!("glTF data buffer URI is invalid. The most likely cause is a missing data URI payload.");
		LoadErrors::FailedToProcess
	})?;
	let encoded = data.split_once(";base64,").map_or(data, |(_, encoded)| encoded);
	let decoded_capacity = encoded
		.len()
		.checked_add(3)
		.and_then(|length| length.checked_div(4))
		.and_then(|chunks| chunks.checked_mul(3))
		.ok_or_else(|| {
			log::error!("glTF data buffer is too large. The most likely cause is an overflowing data URI length.");
			LoadErrors::FailedToProcess
		})?;
	let mut decoded = vec![0; aligned_gltf_buffer_length(decoded_capacity)?];
	let written = base64::decode_config_slice(encoded, base64::STANDARD, &mut decoded).map_err(|error| {
		log::error!("glTF data buffer could not be decoded. The most likely cause is a malformed data URI: {error}.");
		LoadErrors::FailedToProcess
	})?;
	decoded.truncate(written);
	Ok(decoded)
}

/// Copies external buffer bytes once into storage already reserved for glTF alignment padding.
fn copy_gltf_buffer_bytes(bytes: &[u8]) -> Result<Vec<u8>, LoadErrors> {
	let mut data = Vec::with_capacity(aligned_gltf_buffer_length(bytes.len())?);
	data.extend_from_slice(bytes);
	Ok(data)
}

/// Rounds a glTF payload length up to its required four-byte buffer alignment.
fn aligned_gltf_buffer_length(length: usize) -> Result<usize, LoadErrors> {
	length.checked_add(3).map(|length| length & !3).ok_or_else(|| {
		log::error!("glTF buffer is too large. The most likely cause is a payload length that overflows alignment.");
		LoadErrors::FailedToProcess
	})
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
	allocator: &dyn std::alloc::Allocator,
) -> Result<gltf::image::Data, LoadErrors> {
	match image.source() {
		gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
			let image_url = resolve_gltf_uri(mesh_url, uri)?;
			let (bytes, ..) = asset_storage_backend
				.resolve_in(ResourceId::new(&image_url), allocator)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;
			decode_external_gltf_image(&bytes)
		}
		_ => gltf::image::Data::from_source(image.source(), None, buffers).map_err(|_| LoadErrors::FailedToProcess),
	}
}

fn resolve_gltf_uri(mesh_url: ResourceId<'_>, uri: &str) -> Result<String, LoadErrors> {
	if uri.contains("://") || uri.starts_with('/') {
		return Ok(uri.to_string());
	}

	let uri = urlencoding::decode(uri).map_err(|error| {
		log::error!("glTF file-local URI is invalid. The most likely cause is malformed percent encoding: {error}.");
		LoadErrors::FailedToProcess
	})?;
	let base = mesh_url.get_base();
	let parent = Path::new(base.as_ref()).parent();
	if let Some(parent) = parent {
		Ok(parent.join(uri.as_ref()).to_string_lossy().replace('\\', "/"))
	} else {
		Ok(uri.into_owned())
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
	allocator: &dyn std::alloc::Allocator,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	let format = gltf_image_format(image.format)?;
	let image_description = ImageDescription {
		format,
		extent: Extent::rectangle(image.width, image.height),
		semantic,
		gamma: gamma_from_semantic(semantic),
		generate_mipmaps: false,
	};

	let (asset, data) = process_image_in(id, image_description, image.pixels.into_boxed_slice(), allocator)?;
	Ok((asset, data.to_vec().into_boxed_slice()))
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
	allocator: &dyn std::alloc::Allocator,
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
			allocator,
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
	allocator: &dyn std::alloc::Allocator,
) -> Result<ReferenceModel<Image>, LoadErrors> {
	let image_data = load_gltf_image_data(asset_storage_backend, mesh_url, image, buffers, allocator).await?;
	let (resource, bytes) = process_gltf_image(ResourceId::new(id), image_data, semantic, allocator)?;
	storage_backend
		.store(resource, &bytes)
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
			format: "vec4u16".to_string(),
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

#[cfg(test)]
mod tests {
	use maths_rs::mat::MatNew4;
	use utils::json;

	use super::{
		collect_gltf_texture_dependencies, generated_gltf_image_id, generated_image_fragment_index, generated_material_base_id,
		gltf_normal_transform, gltf_primitive_transform_node, gltf_transform_orientation, gltf_vertex_component,
		has_vertex_component, import_gltf_animation, import_gltf_node_graph, import_gltf_skin_binding, import_gltf_vertex_skin,
		load_gltf_buffers, material_override, normalize_vertex_layouts, sanitize_material_name, transform_gltf_tangent,
		transform_gltf_unit_direction, unique_gltf_materials, validate_gltf_flattened_animation_transform,
		validate_gltf_skin_attribute_sets, GLTFAssetHandler, GltfSkeletalImportError, GltfTextureDependency,
		TriangleFrontFaceWinding,
	};
	use crate::r#async;
	use crate::{
		asset::{
			asset_handler::AssetHandler,
			asset_manager::AssetManager,
			bema_asset_handler::{
				tests::{MinimalTestShaderGenerator, RootTestShaderGenerator},
				BEMAAssetHandler,
			},
			png_asset_handler::PNGAssetHandler,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
			ResourceId,
		},
		pbr::{BrdfAlphaMode, BrdfChannel, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture, BrdfValue},
		processors::{image_processor::Semantic, mesh_processor::orient_triangle_indices_for_front_face},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::{
			animation::{AnimationModel, QuaternionCurve, Vector3Curve},
			mesh::MeshModel,
			skeleton::{SkeletonModel, SkinJoint},
		},
		types::{VertexComponent, VertexSemantics},
		ReferenceModel,
	};

	/// Appends one aligned binary payload and returns the byte range used by its glTF buffer view.
	fn append_fixture_bytes(binary: &mut Vec<u8>, bytes: &[u8]) -> (usize, usize) {
		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}
		let offset = binary.len();
		binary.extend_from_slice(bytes);
		(offset, bytes.len())
	}

	/// Appends little-endian floating-point data used by generated accessors.
	fn append_fixture_f32(binary: &mut Vec<u8>, values: &[f32]) -> (usize, usize) {
		let bytes = values.iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
		append_fixture_bytes(binary, &bytes)
	}

	/// Builds a self-contained GLB that exercises hierarchy remapping, mixed instancing, two influence sets, and pose curves.
	fn generated_skeletal_glb() -> Vec<u8> {
		let mut binary = Vec::new();
		let positions = append_fixture_f32(&mut binary, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
		let indices = append_fixture_bytes(&mut binary, &[0, 0, 1, 0, 2, 0]);
		let joints_0 = append_fixture_bytes(&mut binary, &[0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]);
		let weights_0 = append_fixture_f32(&mut binary, &[0.1, 0.2, 0.3, 0.4, 1.0, 0.0, 0.0, 0.0, 0.25, 0.25, 0.25, 0.25]);
		let joints_1 = append_fixture_bytes(&mut binary, &[1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0]);
		let weights_1 = append_fixture_f32(&mut binary, &[0.8, 0.7, 0.6, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
		let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
		let inverse_binds = append_fixture_f32(&mut binary, &identity.into_iter().chain(identity).collect::<Vec<_>>());
		let times = append_fixture_f32(&mut binary, &[0.0, 2.0]);
		let translations = append_fixture_f32(&mut binary, &[0.0, 0.0, 2.0, 1.0, 2.0, 3.0]);
		let rotations = append_fixture_f32(
			&mut binary,
			&[
				2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 8.0, 0.0,
				0.0, 0.0,
			],
		);

		let document = serde_json::json!({
			"asset": { "version": "2.0" },
			"scene": 0,
			"scenes": [{ "nodes": [2] }],
			"nodes": [
				{ "name": "Joint", "translation": [0.0, 0.0, 2.0] },
				{ "name": "SkinnedMesh", "mesh": 0, "skin": 0, "translation": [3.0, 0.0, 0.0] },
				{ "name": "Root", "children": [0, 1, 3], "translation": [0.0, 0.0, 1.0] },
				{ "name": "RigidMesh", "mesh": 0, "translation": [-3.0, 0.0, 0.0] }
			],
			"meshes": [{
				"primitives": [{
					"attributes": { "POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3, "JOINTS_1": 4, "WEIGHTS_1": 5 },
					"indices": 1,
					"material": 0
				}]
			}],
			"materials": [{ "name": "TestMaterial" }],
			"skins": [{ "inverseBindMatrices": 6, "joints": [0, 2], "skeleton": 2 }],
			"animations": [{
				"name": "Walk",
				"samplers": [
					{ "input": 7, "output": 8, "interpolation": "LINEAR" },
					{ "input": 7, "output": 9, "interpolation": "CUBICSPLINE" }
				],
				"channels": [
					{ "sampler": 0, "target": { "node": 0, "path": "translation" } },
					{ "sampler": 1, "target": { "node": 0, "path": "rotation" } }
				]
			}],
			"buffers": [{ "byteLength": binary.len() }],
			"bufferViews": [
				{ "buffer": 0, "byteOffset": positions.0, "byteLength": positions.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": indices.0, "byteLength": indices.1, "target": 34963 },
				{ "buffer": 0, "byteOffset": joints_0.0, "byteLength": joints_0.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": weights_0.0, "byteLength": weights_0.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": joints_1.0, "byteLength": joints_1.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": weights_1.0, "byteLength": weights_1.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": inverse_binds.0, "byteLength": inverse_binds.1 },
				{ "buffer": 0, "byteOffset": times.0, "byteLength": times.1 },
				{ "buffer": 0, "byteOffset": translations.0, "byteLength": translations.1 },
				{ "buffer": 0, "byteOffset": rotations.0, "byteLength": rotations.1 }
			],
			"accessors": [
				{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] },
				{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" },
				{ "bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4" },
				{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4" },
				{ "bufferView": 4, "componentType": 5121, "count": 3, "type": "VEC4" },
				{ "bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC4" },
				{ "bufferView": 6, "componentType": 5126, "count": 2, "type": "MAT4" },
				{ "bufferView": 7, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [2.0] },
				{ "bufferView": 8, "componentType": 5126, "count": 2, "type": "VEC3" },
				{ "bufferView": 9, "componentType": 5126, "count": 6, "type": "VEC4" }
			]
		});
		let mut json = serde_json::to_vec(&document).expect("fixture JSON should serialize");
		while !json.len().is_multiple_of(4) {
			json.push(b' ');
		}
		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}

		let total_length = 12 + 8 + json.len() + 8 + binary.len();
		let mut glb = Vec::with_capacity(total_length);
		glb.extend_from_slice(b"glTF");
		glb.extend_from_slice(&2u32.to_le_bytes());
		glb.extend_from_slice(&(total_length as u32).to_le_bytes());
		glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
		glb.extend_from_slice(b"JSON");
		glb.extend_from_slice(&json);
		glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
		glb.extend_from_slice(b"BIN\0");
		glb.extend_from_slice(&binary);
		glb
	}

	/// Parses the generated GLB through the same glTF reader utilities used by the importer.
	fn parse_skeletal_fixture() -> (gltf::Gltf, Vec<gltf::buffer::Data>) {
		let gltf = gltf::Gltf::from_slice(&generated_skeletal_glb()).expect("generated skeletal GLB should parse");
		let buffers = gltf::import_buffers(&gltf, None, gltf.blob.clone()).expect("generated binary buffer should import");
		(gltf, buffers)
	}

	fn assert_near(actual: f32, expected: f32) {
		assert!((actual - expected).abs() < 1.0e-5, "expected {expected}, got {actual}");
	}

	#[test]
	fn imports_parent_before_child_skeleton_with_left_handed_rest_pose() {
		let (gltf, _) = parse_skeletal_fixture();
		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");

		assert_eq!(graph.source_to_dense, vec![1, 2, 0, 3]);
		assert_eq!(
			graph
				.skeleton
				.nodes
				.iter()
				.map(|node| (node.name.as_deref(), node.parent))
				.collect::<Vec<_>>(),
			vec![
				(Some("Root"), None),
				(Some("Joint"), Some(0)),
				(Some("SkinnedMesh"), Some(0)),
				(Some("RigidMesh"), Some(0))
			]
		);
		assert_eq!(graph.skeleton.nodes[0].rest_local.translation, [0.0, 0.0, -1.0]);
		assert_eq!(graph.skeleton.nodes[1].rest_local.translation, [0.0, 0.0, -2.0]);
	}

	#[test]
	fn transforms_normals_and_tangents_without_translation_contamination() {
		let transform = maths_rs::Mat4f::new(
			2.0, 0.0, 0.0, 10.0, 0.0, 3.0, 0.0, 20.0, 0.0, 0.0, -4.0, 30.0, 0.0, 0.0, 0.0, 1.0,
		);
		let normal_transform = gltf_normal_transform(transform).expect("normal transform should invert");
		let normal = transform_gltf_unit_direction(&normal_transform, [1.0, 1.0, 0.0]).unwrap();
		let orientation = gltf_transform_orientation(transform).unwrap();
		let tangent = transform_gltf_tangent(&transform, orientation, [1.0, 1.0, 0.0, 1.0]).unwrap();

		assert_near(normal[0], 0.8320503);
		assert_near(normal[1], 0.5547002);
		assert_near(normal[2], 0.0);
		assert_near(tangent[0], 0.5547002);
		assert_near(tangent[1], 0.8320503);
		assert_near(tangent[2], 0.0);
		assert_eq!(tangent[3], -1.0);
	}

	#[test]
	fn rejects_singular_bind_transforms_only_when_geometry_retains_an_animation_node() {
		let singular = maths_rs::Mat4f::new(
			0.0, 0.0, 0.0, 3.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
		);

		assert!(validate_gltf_flattened_animation_transform(singular, None).is_ok());
		assert_eq!(
			validate_gltf_flattened_animation_transform(singular, Some(0)),
			Err(GltfSkeletalImportError::SingularMeshTransform)
		);
	}

	#[test]
	fn imports_adjusted_binding_and_merges_strongest_influences_for_only_skinned_instances() {
		let (gltf, buffers) = parse_skeletal_fixture();
		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");
		let skinned_node = gltf.nodes().find(|node| node.name() == Some("SkinnedMesh")).unwrap();
		let rigid_node = gltf.nodes().find(|node| node.name() == Some("RigidMesh")).unwrap();
		let binding = import_gltf_skin_binding(&skinned_node, &buffers, &graph).expect("skin binding should import");

		assert_eq!(
			binding.entries.iter().map(|entry| entry.joint).collect::<Vec<_>>(),
			vec![SkinJoint::Node(1), SkinJoint::Node(0)]
		);
		for entry in &binding.entries {
			let inverse_bind = &entry.adjusted_inverse_bind_matrix;
			assert_near(inverse_bind[3][0], -3.0);
			assert_near(inverse_bind[3][1], 0.0);
			assert_near(inverse_bind[3][2], 1.0);
		}

		let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
		validate_gltf_skin_attribute_sets(&primitive, true).expect("skinned instance should validate");
		validate_gltf_skin_attribute_sets(&primitive, false).expect("rigid instance should ignore skin streams");
		let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
		let (joints, weights) = import_gltf_vertex_skin(&reader, 3, binding.len()).expect("weights should import");

		assert_eq!(joints[0], [1, 0, 1, 0]);
		for (actual, expected) in weights[0].into_iter().zip([8.0 / 26.0, 7.0 / 26.0, 6.0 / 26.0, 5.0 / 26.0]) {
			assert_near(actual, expected);
		}
		assert!(skinned_node.skin().is_some());
		assert!(rigid_node.skin().is_none());
		assert_eq!(gltf_primitive_transform_node(&graph, &skinned_node, true), Some(2));
		assert_eq!(gltf_primitive_transform_node(&graph, &rigid_node, true), Some(3));
	}

	#[test]
	fn imports_pose_curves_with_dense_targets_and_preserved_cubic_derivatives() {
		let (gltf, buffers) = parse_skeletal_fixture();
		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");
		let skeleton = ReferenceModel::<SkeletonModel>::new("fixture.glb#skeleton", 0, 0, &graph.skeleton, None);
		let animation = import_gltf_animation(&gltf, &buffers, "animations/Walk", &graph.source_to_dense, skeleton)
			.expect("animation should import");

		assert_eq!(animation.name.as_deref(), Some("Walk"));
		assert_eq!(animation.duration, 2.0);
		assert_eq!(animation.tracks.len(), 1);
		assert_eq!(animation.tracks[0].node, 1);
		match animation.tracks[0].translation.as_ref().unwrap() {
			Vector3Curve::Linear { times, values } => {
				assert_eq!(times, &[0.0, 2.0]);
				assert_eq!(values, &[[0.0, 0.0, -2.0], [1.0, 2.0, -3.0]]);
			}
			curve => panic!("expected linear translation curve, got {curve:?}"),
		}
		match animation.tracks[0].rotation.as_ref().unwrap() {
			QuaternionCurve::CubicSpline {
				times,
				values,
				in_tangents,
				out_tangents,
			} => {
				assert_eq!(times, &[0.0, 2.0]);
				assert_eq!(values, &[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]]);
				assert_eq!(in_tangents, &[[-2.0, 0.0, 0.0, 0.0], [-6.0, 0.0, 0.0, 0.0]]);
				assert_eq!(out_tangents, &[[-4.0, 0.0, 0.0, 0.0], [-8.0, 0.0, 0.0, 0.0]]);
			}
			curve => panic!("expected cubic rotation curve, got {curve:?}"),
		}
	}

	#[r#async::test]
	async fn bakes_generated_skeleton_fragment_from_the_base_glb() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend);
		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb#skeleton", &resource_storage_backend)
			.await
			.expect("generated skeleton fragment should bake");
		let skeleton = crate::from_slice::<SkeletonModel>(&skeleton.resource).expect("skeleton should deserialize");

		assert_eq!(skeleton.nodes.len(), 4);
		assert_eq!(skeleton.nodes[0].name.as_deref(), Some("Root"));
		assert_eq!(skeleton.nodes[1].parent, Some(0));
	}

	#[r#async::test]
	async fn bakes_named_animation_fragment_with_generated_skeleton_dependency() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend);
		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb#animations/Walk", &resource_storage_backend)
			.await
			.expect("generated animation fragment should bake");
		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation should deserialize");

		assert_eq!(animation.name.as_deref(), Some("Walk"));
		assert_eq!(animation.duration, 2.0);
		assert_eq!(animation.tracks.len(), 1);
		assert_eq!(animation.skeleton.id().as_ref(), "generated_skeletal.glb#skeleton");
	}

	#[r#async::test]
	async fn bakes_nested_gltf_animation_from_ordered_file_relative_buffers() {
		let glb_bytes = generated_skeletal_glb();
		let glb = gltf::Glb::from_slice(&glb_bytes).expect("generated skeletal GLB should parse");
		let mut document: serde_json::Value = serde_json::from_slice(&glb.json).expect("generated skeletal JSON should parse");
		let binary = glb
			.bin
			.expect("generated skeletal GLB should contain a BIN chunk")
			.into_owned();
		let times_offset = document["bufferViews"][7]["byteOffset"]
			.as_u64()
			.expect("animation times should have a byte offset") as usize;
		let values_offset = document["bufferViews"][8]["byteOffset"]
			.as_u64()
			.expect("animation values should have a byte offset") as usize;
		let times = &binary[times_offset..values_offset];
		let values = &binary[values_offset..];

		// Buffer zero is deliberately absent: skeletons need no binary data and selected clips load only their accessor buffers.
		document["buffers"] = serde_json::json!([
			{ "byteLength": times_offset, "uri": "missing_geometry" },
			{ "byteLength": times.len(), "uri": "timeline" },
			{ "byteLength": values.len(), "uri": "animation%20values" }
		]);
		document["bufferViews"][7]["buffer"] = 1.into();
		document["bufferViews"][7]["byteOffset"] = 0.into();
		for view_index in 8..=9 {
			let source_offset = document["bufferViews"][view_index]["byteOffset"]
				.as_u64()
				.expect("animation value view should have a byte offset") as usize;
			document["bufferViews"][view_index]["buffer"] = 2.into();
			document["bufferViews"][view_index]["byteOffset"] = (source_offset - values_offset).into();
		}
		let document = serde_json::to_vec(&document).expect("external-buffer glTF JSON should serialize");

		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file("characters/generated_skeletal.gltf", &document);
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend.clone());
		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("characters/generated_skeletal.gltf#skeleton", &resource_storage_backend)
			.await
			.expect("nested glTF skeleton should not load unrelated buffers");
		assert_eq!(skeleton.id().as_ref(), "characters/generated_skeletal.gltf#skeleton");

		asset_storage_backend.add_file("characters/timeline", times);
		asset_storage_backend.add_file("characters/animation values", values);
		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists(
				"characters/generated_skeletal.gltf#animations/Walk",
				&resource_storage_backend,
			)
			.await
			.expect("nested glTF animation should load its sibling buffer");
		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation should deserialize");

		assert_eq!(animation.duration, 2.0);
		assert_eq!(
			animation.skeleton.id().as_ref(),
			"characters/generated_skeletal.gltf#skeleton"
		);
	}

	#[r#async::test]
	async fn rejects_a_truncated_data_uri_before_adding_alignment_padding() {
		let gltf = gltf::Gltf::from_slice(
			br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AQID"}]}"#,
		)
		.expect("truncated data URI fixture should parse");
		let asset_storage_backend = AssetTestStorageBackend::new();

		let result = load_gltf_buffers(
			&asset_storage_backend,
			ResourceId::new("truncated.gltf"),
			&gltf,
			None,
			None,
			&std::alloc::Global,
		)
		.await;

		assert!(result.is_err());
	}

	#[r#async::test]
	async fn bakes_base_skeletal_mesh_with_primitive_node_and_skin_bindings() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend);
		let mut handler = GLTFAssetHandler::new();
		handler.set_shader_generator(MinimalTestShaderGenerator);
		asset_manager.add_asset_handler(handler);

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb", &resource_storage_backend)
			.await
			.expect("generated skeletal mesh should bake");
		let mesh = crate::from_slice::<MeshModel>(&mesh.resource).expect("generated mesh should deserialize");

		assert_eq!(
			mesh.skeleton
				.as_ref()
				.expect("generated mesh should retain its skeleton")
				.id()
				.as_ref(),
			"generated_skeletal.glb#skeleton"
		);
		assert_eq!(mesh.skins.len(), 1);
		assert_eq!(mesh.primitives.len(), 2);
		assert_eq!(mesh.primitives[0].transform_node, Some(2));
		assert_eq!(mesh.primitives[0].skin, Some(0));
		assert_eq!(mesh.primitives[1].transform_node, Some(3));
		assert_eq!(mesh.primitives[1].skin, None);
	}

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
	fn deduplicates_indexed_and_default_materials_in_primitive_order() {
		let gltf = gltf::Gltf::from_slice(
			r#"{
				"asset":{"version":"2.0"},
				"buffers":[{"byteLength":36}],
				"bufferViews":[{"buffer":0,"byteLength":36}],
				"accessors":[{
					"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
					"min":[0,0,0],"max":[1,1,0]
				}],
				"materials":[{},{}],
				"meshes":[{"primitives":[
					{"attributes":{"POSITION":0},"material":1},
					{"attributes":{"POSITION":0}},
					{"attributes":{"POSITION":0},"material":1},
					{"attributes":{"POSITION":0},"material":0},
					{"attributes":{"POSITION":0}},
					{"attributes":{"POSITION":0},"material":0}
				]}]
			}"#
			.as_bytes(),
		)
		.expect("test glTF should parse");
		let primitives = gltf.meshes().flat_map(|mesh| mesh.primitives()).collect::<Vec<_>>();

		let (materials, material_indices_per_primitive) = unique_gltf_materials(&primitives);

		assert_eq!(
			materials.iter().map(|material| material.index()).collect::<Vec<_>>(),
			vec![Some(1), None, Some(0)]
		);
		assert_eq!(material_indices_per_primitive, vec![0, 1, 0, 2, 1, 2]);
		assert_eq!(
			materials
				.iter()
				.map(|material| generated_material_base_id(ResourceId::new("models/drone.glb"), material))
				.collect::<Vec<_>>(),
			vec![
				"models/drone.glb#materials/material_1",
				"models/drone.glb#materials/material_default",
				"models/drone.glb#materials/material_0",
			]
		);
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
				&std::alloc::Global,
			)
			.await
			.expect("Image asset handler did not handle asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, resource, &data)
			.expect("Image asset handler did not store asset");

		let _ = resource_storage_backend.get_resource_data_by_name(url).unwrap();

		let generated_resources = resource_storage_backend.get_resources();

		let resource = &generated_resources[0];

		assert_eq!(resource.class, "Image");
	}
}

use std::{collections::HashMap, path::Path, sync::Arc};

use maths_rs::{
	mat::{MatDeterminant, MatInverse, MatNew4, MatScale, MatTranspose},
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
		image_processor::{gamma_from_semantic, guess_semantic_from_name, process_image_in, ImageDescription, Semantic},
		mesh_processor::{MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource},
	},
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::{
		animation::{AnimationModel, NodeTrack, QuaternionCurve, Vector3Curve},
		image::Image,
		material::{MaterialModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel},
		skeleton::{
			identity_matrix4_columns, LocalTransform, Matrix4Columns, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint,
			SkinPaletteEntry,
		},
	},
	types::{AlphaMode, Formats, VertexComponent, VertexSemantics},
	ProcessedAsset, ReferenceModel,
};
