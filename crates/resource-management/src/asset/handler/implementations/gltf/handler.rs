use super::*;

pub(crate) const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";

pub(crate) const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";

pub(crate) const SKELETON_FRAGMENT: &str = "skeleton";

pub(crate) const MAX_SKIN_JOINTS: usize = u16::MAX as usize + 1;

pub(crate) fn select_unfragmented_gltf_resource(
	gltf: &gltf::Gltf,
	spec: Option<&asset::BEADType>,
) -> Result<ContainerDefaultResource, String> {
	let selected = container_default_resource(spec)?;

	let animation_count = gltf.animations().len();

	if let Some(selected) = selected {
		if selected == ContainerDefaultResource::Animation && animation_count != 1 {
			return Err(format!(
				"BEAD selects animation, but the glTF contains {animation_count} clips; use an explicit animation fragment"
			));
		}

		return Ok(selected);
	}

	if gltf.meshes().next().is_some() {
		return Ok(ContainerDefaultResource::Mesh);
	}

	if animation_count == 1 {
		return Ok(ContainerDefaultResource::Animation);
	}

	Err(format!(
		"the glTF contains no mesh and {animation_count} animation clips; use an explicit fragment"
	))
}

/// The `GLTFAssetHandler` struct provides the glTF boundary used to bake renderable meshes, skeletal clips, materials, and images.
#[derive(Default)]

pub struct GLTFAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	generator: Option<Arc<dyn ProgramGenerator>>,
	material_mip_generator: Option<Arc<dyn MipGenerationBackend>>,
}

impl GLTFAssetHandler {
	pub fn new() -> GLTFAssetHandler {
		Self::default()
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

	/// Selects the offline backend used only for image resources generated from glTF materials.
	pub fn set_material_mip_generator(&mut self, generator: Arc<dyn MipGenerationBackend>) {
		self.material_mip_generator = Some(generator);
	}
}

impl AssetHandler for GLTFAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "gltf" || r#type == "glb"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(dt) = context.resource_type(url) {
			if !self.can_handle(dt) {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let asset_storage_backend = context.asset_storage_backend();

		let allocator = context.allocator();

		// Resolve the container base so generated skeleton and animation fragments never become part of the source filename.
		let base = url.get_base();

		let source_id = ResourceId::new(base.as_ref());

		let (data, spec, dt) = context.resolve(source_id).await?;

		let (gltf, binary_blob) = if dt == "glb" {
			// Arena-backed source bytes borrow the bake allocator, so parsing stays in this task instead of crossing a thread boundary.
			let glb = gltf::Glb::from_slice(&data).map_err(|_| LoadErrors::FailedToProcess)?;

			let gltf = gltf::Gltf::from_slice(&glb.json).map_err(|_| LoadErrors::FailedToProcess)?;

			(gltf, glb.bin)
		} else {
			// Keep the allocator-backed `.gltf` bytes local to this bake task.
			let gltf = parse_gltf_json(&data).map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?;

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

			return context.store_primary(ProcessedAsset::new(url, graph.skeleton), &[]).await;
		}

		let default_resource = if url.get_fragment().is_none() {
			Some(select_unfragmented_gltf_resource(&gltf, spec.as_ref()).map_err(|error| {
				log::error!(
					"Failed to select the default glTF resource '{}': {error}. The most likely cause is an ambiguous container without an explicit fragment or BEAD override.",
					url.as_ref()
				);
				LoadErrors::FailedToProcess
			})?)
		} else {
			None
		};

		let animation_fragment = url
			.get_fragment()
			.filter(|fragment| is_gltf_animation_fragment(fragment.as_ref()))
			.map(|fragment| fragment.as_ref().to_string())
			.or_else(|| {
				(default_resource == Some(ContainerDefaultResource::Animation)).then(|| DEFAULT_ANIMATION_FRAGMENT.to_string())
			});

		let required_buffers = animation_fragment
			.as_deref()
			.map(|fragment| required_gltf_animation_buffers(&gltf, fragment))
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

				let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, graph.skeleton, &[]).await?;

				let animation = import_gltf_animation(&gltf, &buffers, fragment.as_ref(), &graph.source_to_dense, skeleton)
					.map_err(|error| {
						log::error!("Failed to import glTF animation '{}': {error}", url.as_ref());

						LoadErrors::FailedToProcess
					})?;

				return context.store_primary(ProcessedAsset::new(url, animation), &[]).await;
			}

			let image = image_for_gltf_fragment(&gltf, fragment.as_ref()).ok_or(LoadErrors::FailedToProcess)?;

			let image = load_gltf_image_data(asset_storage_backend, url, image, &buffers, allocator).await?;

			let semantic = guess_semantic_from_name(url.get_base());

			store_gltf_image(context, url, image, semantic, None).await?;

			return Ok(());
		}

		if default_resource == Some(ContainerDefaultResource::Animation) {
			let graph = import_gltf_node_graph(&gltf).map_err(|error| {
				log::error!("Failed to import default glTF animation skeleton '{}': {error}", url.as_ref());

				LoadErrors::FailedToProcess
			})?;

			let skeleton_id = generated_gltf_skeleton_id(source_id);

			let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, graph.skeleton, &[]).await?;

			let animation =
				import_gltf_animation(&gltf, &buffers, DEFAULT_ANIMATION_FRAGMENT, &graph.source_to_dense, skeleton).map_err(
					|error| {
						log::error!("Failed to import default glTF animation '{}': {error}", url.as_ref());

						LoadErrors::FailedToProcess
					},
				)?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]).await;
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

		let vertex_layout =
			include_skin_vertex_layout(normalize_vertex_layouts(&vertex_layouts), &vertex_layouts).map_err(|error| {
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
				.map(|(primitive, transform, transform_node, skin)| (primitive, *transform, *transform_node, *skin))
		};

		let skeleton = if !retain_skeleton {
			None
		} else {
			let skeleton_id = generated_gltf_skeleton_id(source_id);

			Some(store_model::<SkeletonModel>(context, &skeleton_id, graph.skeleton, &[]).await?)
		};

		let (unique_materials, material_indices_per_primitive) = unique_gltf_materials(&primitives);

		let mut resolved_materials = Vec::with_capacity(unique_materials.len());

		for material in unique_materials {
			let material = material_for_gltf_primitive(
				context,
				spec,
				url,
				&gltf,
				&buffers,
				material,
				self.generator.clone(),
				self.material_mip_generator.as_deref(),
			)
			.await?;

			resolved_materials.push(material);
		}

		let skin_joint_counts = skin_bindings.iter().map(SkinBinding::len).collect::<Vec<_>>();
		let primitive_attributes = GltfPrimitiveAttributes::from_layout(&vertex_layout);

		let mut mesh_processor = MeshProcessor::new()
			.with_triangle_front_face_winding(self.triangle_front_face_winding)
			.begin(vertex_layout, skeleton, skin_bindings)
			.map_err(|error| {
				log::error!("Failed to initialize glTF mesh processing '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;

		for ((primitive, transform, transform_node, skin), material_index) in flat_mesh_tree.zip(material_indices_per_primitive)
		{
			validate_gltf_flattened_animation_transform(transform, transform_node).map_err(|error| {
				log::error!("Failed to import glTF animated mesh transform '{}': {error}", url.as_ref());

				LoadErrors::FailedToProcess
			})?;

			validate_gltf_skin_attribute_sets(primitive, skin.is_some()).map_err(|error| {
				log::error!("Failed to import glTF vertex layout '{}': {error}", url.as_ref());

				LoadErrors::FailedToProcess
			})?;

			let source = GltfPrimitiveSource::new(
				primitive,
				&buffers,
				&resolved_materials[material_index],
				transform,
				transform_node,
				skin,
				skin.map(|skin| skin_joint_counts[skin as usize]),
				primitive_attributes,
			);
			mesh_processor.push_primitive(&source).map_err(|error| {
				match error {
					MeshPrimitiveProcessingError::Source(error) => {
						log::error!("Failed to import glTF mesh '{}': {error}", url.as_ref());
					}
					MeshPrimitiveProcessingError::Processing(error) => {
						log::error!("Failed to process glTF mesh '{}': {error}", url.as_ref());
					}
				}
				LoadErrors::FailedToProcess
			})?;
		}

		let mut transaction = context.begin_resource(url, mesh_processor.payload_size()).await?;
		let (mesh, stream_descriptions) = mesh_processor
			.finish_into_resource(&mut transaction)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		context
			.commit_primary(transaction, ProcessedAsset::new(url, mesh).with_streams(stream_descriptions))
			.await
	}
}
