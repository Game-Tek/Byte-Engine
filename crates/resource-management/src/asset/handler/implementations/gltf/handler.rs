use super::*;

pub(crate) const MAX_SKIN_JOINTS: usize = u16::MAX as usize + 1;

/// The `GLTFAssetHandler` struct provides the glTF boundary used to bake renderable meshes, skeletal clips, materials, and images.
#[derive(Default)]
pub struct GLTFAssetHandler {
	generator: Option<Box<dyn ProgramGenerator>>,
	material_mip_generator: Option<Arc<MipGenerator>>,
}

impl GLTFAssetHandler {
	pub fn new() -> GLTFAssetHandler {
		Self::default()
	}

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Box::new(generator));
	}

	/// Selects the generator that produces mips for the image resources a glTF contains.
	pub fn set_material_mip_generator(&mut self, generator: Arc<MipGenerator>) {
		self.material_mip_generator = Some(generator);
	}

	/// Packs glTF primitives through the shared mesh processor, naming each primitive's material by its slot.
	///
	/// The work is synchronous, so it yields between primitives to let futures joined with it, such as the material
	/// bakes, keep dispatching and collecting their work.
	async fn process_geometry<'a>(
		url: ResourceId<'_>,
		buffers: &[Cow<'_, [u8]>],
		vertex_layout: Vec<VertexComponent>,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skin_bindings: Vec<SkinBinding>,
		primitives: &[(gltf::Primitive<'a>, math::Matrix, Option<u32>, Option<u32>)],
		material_slots: &[usize],
	) -> Result<MeshProcessorSession, LoadErrors> {
		let skin_joint_counts = skin_bindings.iter().map(SkinBinding::len).collect::<Vec<_>>();
		let primitive_attributes = GltfPrimitiveAttributes::from_layout(&vertex_layout);
		let mut mesh_processor = MeshProcessor::new()
			.begin(vertex_layout, skeleton, skin_bindings)
			.map_err(|error| {
				log::error!("Failed to initialize glTF mesh processing '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;

		for ((primitive, transform, transform_node, skin), material_slot) in primitives.iter().zip(material_slots) {
			validate_gltf_flattened_animation_transform(*transform, *transform_node).map_err(|error| {
				log::error!("Failed to import glTF animated mesh transform '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;
			// A mesh may be instanced by both skinned and rigid nodes; rigid instances deliberately ignore complete skin
			// streams.
			if skin.is_some() {
				validate_gltf_skin_attribute_sets(primitive).map_err(|error| {
					log::error!("Failed to import glTF vertex layout '{}': {error}", url.as_ref());
					LoadErrors::FailedToProcess
				})?;
			}

			let source = GltfPrimitiveSource {
				primitive,
				buffers,
				material_slot: *material_slot,
				transform: *transform,
				transform_node: *transform_node,
				skin: *skin,
				skin_joint_count: skin.map(|skin| skin_joint_counts[skin as usize]),
				attributes: primitive_attributes,
			};
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
			crate::r#async::yield_now().await;
		}

		Ok(mesh_processor)
	}

	/// Imports the mesh hierarchy, skins, materials, and primitives selected by an unfragmented glTF request.
	async fn store_selected_mesh<'a>(
		&self,
		context: BakeContext<'_>,
		url: ResourceId<'_>,
		source_id: ResourceId<'_>,
		spec: Option<&serde_json::Value>,
		gltf: &'a gltf::Gltf,
		buffers: &[Cow<'_, [u8]>],
	) -> Result<(), LoadErrors> {
		let graph = import_gltf_node_graph(gltf).map_err(|error| {
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
			.collect::<Vec<_>>();
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

		let mut skin_bindings = Vec::new();
		let mut skin_binding_by_node = HashMap::new();
		for node in &flat_tree {
			if node.mesh().is_none() || node.skin().is_none() || skin_binding_by_node.contains_key(&node.index()) {
				continue;
			}
			let binding = import_gltf_skin_binding(node, buffers, &graph).map_err(|error| {
				log::error!("Failed to import glTF skin binding '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;
			let binding_index = skin_bindings.len() as u32;
			skin_bindings.push(binding);
			skin_binding_by_node.insert(node.index(), binding_index);
		}

		let retain_skeleton = !skin_bindings.is_empty() || gltf.animations().next().is_some();
		let handedness = handedness_matrix();
		let primitives_and_transform = flat_tree
			.iter()
			.filter_map(|node| {
				let mesh = node.mesh()?;
				let transform = handedness * graph.source_global_transforms[node.index()];
				// Retaining the dense node lets CPU animation drive both skinned and rigid primitives.
				let transform_node = retain_skeleton.then_some(graph.source_to_dense[node.index()]);
				let skin = skin_binding_by_node.get(&node.index()).copied();
				Some(
					mesh.primitives()
						.map(move |primitive| (primitive, transform, transform_node, skin)),
				)
			})
			.flatten()
			.collect::<Vec<_>>();
		let skeleton = if retain_skeleton {
			let skeleton_id = generated_skeleton_id(source_id.get_base().as_ref());
			Some(store_model::<SkeletonModel>(context, &skeleton_id, graph.skeleton, &[]).await?)
		} else {
			None
		};
		// Geometry doesn't depend on material contents, so it is packed while the material textures and shaders bake on
		// the worker pool, and the resolved materials are attached when the mesh is committed.
		let (unique_materials, material_slots) =
			unique_gltf_materials(primitives_and_transform.iter().map(|(primitive, ..)| primitive.material()));
		let (materials, mesh) = std::future::join!(
			resolve_gltf_materials(context, spec, url, gltf, &unique_materials, self.generator.as_deref()),
			Self::process_geometry(
				url,
				buffers,
				vertex_layout,
				skeleton,
				skin_bindings,
				&primitives_and_transform,
				&material_slots,
			),
		)
		.await;

		let (materials, mesh) = (materials?, mesh?);
		commit_mesh(context, url, mesh, &materials).await
	}
}

impl AssetHandler for GLTFAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "gltf" || r#type == "glb"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		// Resolve the container base so generated skeleton and animation fragments never become part of the source filename.
		let base = url.get_base();

		let source_id = ResourceId::new(base.as_ref());

		let (data, dt) = context.resolve(source_id).await?;
		let spec = context.load_sidecar(source_id).await?;

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

		let fragment = url.get_fragment();
		let fragment = fragment.as_ref().map(|fragment| fragment.as_ref());

		if fragment == Some(SKELETON_FRAGMENT) {
			let graph = import_gltf_node_graph(&gltf).map_err(|error| {
				log::error!("Failed to import glTF skeleton '{}': {error}", url.as_ref());

				LoadErrors::FailedToProcess
			})?;

			return context.store_primary(ProcessedAsset::new(url, graph.skeleton), &[]).await;
		}

		// Any fragment other than the skeleton or an animation names an image.
		if let Some(fragment) = fragment
			&& fragment != DEFAULT_ANIMATION_FRAGMENT
			&& !fragment.starts_with(ANIMATION_FRAGMENT_PREFIX)
		{
			let image = image_for_gltf_fragment(&gltf, fragment).ok_or(LoadErrors::FailedToProcess)?;

			// Materials decide how an image is sampled; a standalone image falls back to its file name.
			let semantic =
				gltf_image_semantics(&gltf)[image.index()].unwrap_or_else(|| guess_semantic_from_name(url.get_base()));

			let image = load_gltf_fragment_image(context, source_id, image, binary_blob.as_deref()).await?;
			let (channels, encoding) = gltf_image_source_layout(image.format)?;
			let extent = Extent::rectangle(image.width, image.height);
			let source = ImageSource::new(extent, channels, encoding, &image.pixels);

			return store_imported_image(context, url, semantic, source, self.material_mip_generator.as_deref()).await;
		}

		// Any fragment left names an animation. An unfragmented request bakes the container's default resource.
		let animation_fragment = match fragment {
			Some(fragment) => Some(fragment),
			None => {
				let default_resource = select_unfragmented_resource(
					spec.as_ref(),
					gltf.meshes().next().is_some(),
					gltf.animations().len(),
					"glTF",
					"animation clips",
				)
				.map_err(|error| {
					log::error!(
						"Failed to select the default glTF resource '{}': {error}. The most likely cause is an ambiguous container without an explicit fragment or BEAD override.",
						url.as_ref()
					);
					LoadErrors::FailedToProcess
				})?;
				(default_resource == ContainerDefaultResource::Animation).then_some(DEFAULT_ANIMATION_FRAGMENT)
			}
		};

		let required_buffers = animation_fragment
			.map(|fragment| required_gltf_animation_buffers(&gltf, fragment))
			.transpose()
			.map_err(|error| {
				log::error!("Failed to select glTF animation '{}': {error}", url.as_ref());

				LoadErrors::FailedToProcess
			})?;

		let buffers = load_gltf_buffers(
			context.asset_storage_backend(),
			source_id,
			&gltf,
			binary_blob,
			required_buffers.as_deref(),
			context.allocator(),
		)
		.await?;

		let Some(fragment) = animation_fragment else {
			return self
				.store_selected_mesh(context, url, source_id, spec.as_ref(), &gltf, &buffers)
				.await;
		};

		// Store the clip together with its generated skeleton dependency.
		let graph = import_gltf_node_graph(&gltf).map_err(|error| {
			log::error!("Failed to import glTF animation skeleton '{}': {error}", url.as_ref());
			LoadErrors::FailedToProcess
		})?;
		let skeleton_id = generated_skeleton_id(base.as_ref());
		let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, graph.skeleton, &[]).await?;
		let animation =
			import_gltf_animation(&gltf, &buffers, fragment, &graph.source_to_dense, skeleton).map_err(|error| {
				log::error!("Failed to import glTF animation '{}': {error}", url.as_ref());
				LoadErrors::FailedToProcess
			})?;

		context.store_primary(ProcessedAsset::new(url, animation), &[]).await
	}
}
