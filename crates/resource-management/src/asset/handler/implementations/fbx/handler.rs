use super::*;

pub(crate) const MAX_PRIMITIVE_VERTICES: usize = u16::MAX as usize + 1;

/// Maps imported FBX skeleton indices into a compatible canonical skeleton by unique node name.
pub(crate) fn canonical_animation_node_map(source: &SkeletonModel, target: &SkeletonModel) -> Result<Vec<u32>, String> {
	let mut target_by_name = std::collections::HashMap::with_capacity(target.nodes.len());
	for (index, node) in target.nodes.iter().enumerate() {
		let Some(name) = node.name.as_deref() else {
			continue;
		};
		target_by_name
			.entry(name)
			.and_modify(|target| *target = None)
			.or_insert(Some(index as u32));
	}

	source
		.nodes
		.iter()
		.enumerate()
		.map(|(source_index, node)| {
			if let Some(name) = node.name.as_deref() {
				return target_by_name.get(name).copied().flatten().ok_or_else(|| {
					format!(
						"Animation skeleton is incompatible. The most likely cause is that source node '{name}' is missing or duplicated in the canonical skeleton."
					)
				});
			}

			target
				.nodes
				.get(source_index)
				.filter(|target| target.name.is_none())
				.map(|_| source_index as u32)
				.ok_or_else(|| {
					format!(
						"Animation skeleton is incompatible. The most likely cause is that unnamed source node {source_index} has no matching canonical node."
					)
				})
		})
		.collect()
}

/// Resolves the canonical animation skeleton and composes source FBX node indices into its node order.
async fn resolve_animation_skeleton(
	context: &BakeContext<'_>,
	spec: Option<&asset::BEADType>,
	imported: ImportedFbxSkeleton,
	base: &str,
) -> Result<(ReferenceModel<SkeletonModel>, Vec<u32>), LoadErrors> {
	// The sidecar's `skeleton` setting names the canonical skeleton resource its animations target.
	let target_id = spec
		.and_then(|spec| spec.get("skeleton"))
		.map(|value| {
			value.as_str().ok_or_else(|| {
				context
					.error("Invalid animation skeleton. The most likely cause is that `skeleton` is not a resource ID string.");
				LoadErrors::FailedToProcess
			})
		})
		.transpose()?;

	let Some(target_id) = target_id else {
		let skeleton_id = generated_skeleton_id(base);
		let skeleton = store_model::<SkeletonModel>(*context, &skeleton_id, imported.model, &[]).await?;
		return Ok((skeleton, imported.source_to_skeleton));
	};

	let target = context.bake_dependency::<SkeletonModel>(target_id).await?;
	let target_model = crate::from_slice::<SkeletonModel>(&target.resource).map_err(|error| {
		context.error(format_args!(
			"Animation skeleton could not be read. The most likely cause is that '{target_id}' contains invalid skeleton metadata: {error}."
		));
		LoadErrors::FailedToProcess
	})?;
	let source_to_target = canonical_animation_node_map(&imported.model, &target_model).map_err(|error| {
		context.error(error);
		LoadErrors::FailedToProcess
	})?;
	let source_to_skeleton = imported
		.source_to_skeleton
		.into_iter()
		.map(|source| source_to_target[source as usize])
		.collect();
	Ok((target, source_to_skeleton))
}

/// The `FBXAssetHandler` struct provides the authored-FBX import path used to bake meshes, skeletons, and animation clips.
#[derive(Default)]
pub struct FBXAssetHandler {
	generator: Option<Box<dyn ProgramGenerator>>,
	material_mip_generator: Option<Arc<MipGenerator>>,
}

impl FBXAssetHandler {
	/// Creates an FBX importer using the engine's clockwise mesh-processing convention.
	pub fn new() -> Self {
		Self::default()
	}

	/// Installs the renderer-specific shader transformation used for generated FBX materials.
	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Box::new(generator));
	}

	/// Selects the generator that produces mips for the texture resources an FBX contains.
	pub fn set_material_mip_generator(&mut self, generator: Arc<MipGenerator>) {
		self.material_mip_generator = Some(generator);
	}

	/// Bakes one texture of an FBX as its own image resource.
	///
	/// Materials request these as dependencies, so the scene is parsed without geometry or animation data.
	async fn store_texture(
		&self,
		context: BakeContext<'_>,
		url: ResourceId<'_>,
		source_id: ResourceId<'_>,
		data: &[u8],
		texture_index: usize,
	) -> Result<(), LoadErrors> {
		let scene = load_fbx_scene_with(data, source_id.as_ref(), true).map_err(|error| {
			context.error(format_args!("Failed to import FBX asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		let texture = scene.textures.as_ref().get(texture_index).ok_or_else(|| {
			context.error(format_args!(
				"FBX texture '{}' does not exist. The most likely cause is that the FBX changed after the material referencing it was baked.",
				url.as_ref()
			));

			LoadErrors::FailedToProcess
		})?;

		let (pixels, width, height) = load_fbx_texture_image(context, source_id, texture).await?;
		let source = ImageSource::new(
			Extent::rectangle(width, height),
			SourceChannels::RGBA,
			SourceEncoding::U8,
			&pixels,
		);

		store_imported_image(context, url, Semantic::Albedo, source, self.material_mip_generator.as_deref()).await
	}
}

impl AssetHandler for FBXAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("fbx")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let allocator = context.allocator();

		// Resolve the container base so animation fragments never become part of the source filename.
		let base = url.get_base();

		let source_id = ResourceId::new(base.as_ref());

		let (data, source_type) = context.resolve(source_id).await?;
		let spec = context.load_sidecar(source_id).await?;

		if !self.can_handle(&source_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let fragment = url.get_fragment();
		let fragment = fragment.as_ref().map(|fragment| fragment.as_ref());

		if let Some(texture_index) = fragment.and_then(fbx_image_fragment_texture_index) {
			return self.store_texture(context, url, source_id, &data, texture_index).await;
		}

		let scene = load_fbx_scene(&data, base.as_ref()).map_err(|error| {
			context.error(format_args!("Failed to import FBX asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		// Any fragment left names the skeleton or an animation. An unfragmented request bakes the container's default
		// resource.
		let fragment = match fragment {
			Some(fragment) => Some(fragment),
			None => {
				let default_resource = select_unfragmented_resource(
					spec.as_ref(),
					!scene.meshes.is_empty(),
					scene.anim_stacks.len(),
					"FBX",
					"animation stacks",
				)
				.map_err(|error| {
					context.error(format_args!(
						"Failed to select the default FBX resource '{}': {error}. The most likely cause is an ambiguous container without an explicit fragment or BEAD override.",
						url.as_ref()
					));
					LoadErrors::FailedToProcess
				})?;
				(default_resource == ContainerDefaultResource::Animation).then_some(DEFAULT_ANIMATION_FRAGMENT)
			}
		};

		if let Some(fragment) = fragment {
			let imported_skeleton = import_fbx_skeleton(&scene).map_err(|error| {
				context.error(format_args!("Failed to import FBX skeleton '{}': {error}", url.as_ref()));

				LoadErrors::FailedToProcess
			})?;

			if fragment == SKELETON_FRAGMENT {
				return context
					.store_primary(ProcessedAsset::new(url, imported_skeleton.model), &[])
					.await;
			}

			let (skeleton, source_to_skeleton) =
				resolve_animation_skeleton(&context, spec.as_ref(), imported_skeleton, base.as_ref()).await?;

			let animation = import_fbx_animation(&scene, fragment, skeleton, &source_to_skeleton).map_err(|error| {
				context.error(format_args!("Failed to import FBX animation '{}': {error}", url.as_ref()));

				LoadErrors::FailedToProcess
			})?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]).await;
		}

		let imported_skeleton = (scene.meshes.iter().any(|mesh| !mesh.skin_deformers.is_empty())
			|| !scene.anim_stacks.is_empty())
		.then(|| import_fbx_skeleton(&scene))
		.transpose()
		.map_err(|error| {
			context.error(format_args!("Failed to import FBX skeleton '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		let (skeleton, source_to_skeleton) = if let Some(imported) = imported_skeleton {
			let skeleton_id = generated_skeleton_id(base.as_ref());

			(
				Some(store_model::<SkeletonModel>(context, &skeleton_id, imported.model, &[]).await?),
				imported.source_to_skeleton,
			)
		} else {
			(None, Vec::new())
		};

		// Geometry doesn't depend on material contents, so it is processed while the material textures and shaders
		// bake on the worker pool, and the resolved materials are attached when the mesh is committed.
		let material_keys = used_material_keys(&scene, allocator);
		let mut culled_polygons = FbxCulledPolygonCounts::default();
		let (materials, mesh) = std::future::join!(
			resolve_fbx_materials(
				context,
				spec.as_ref(),
				source_id,
				&scene,
				&material_keys,
				self.generator.as_deref()
			),
			async {
				import_fbx_mesh_session(
					&scene,
					&material_keys,
					skeleton,
					&source_to_skeleton,
					allocator,
					&mut culled_polygons,
				)
			},
		)
		.await;

		culled_polygons.trace(context);

		let mesh = mesh.map_err(|error| {
			context.error(format_args!("Failed to process FBX mesh '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;
		let materials = materials?;

		commit_mesh(context, url, mesh, &materials).await
	}
}
