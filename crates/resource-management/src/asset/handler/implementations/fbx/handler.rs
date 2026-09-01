use super::*;

pub(crate) const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";

pub(crate) const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";

pub(crate) const SKELETON_FRAGMENT: &str = "skeleton";

pub(crate) const MAX_PRIMITIVE_VERTICES: usize = u16::MAX as usize + 1;

const ANIMATION_SKELETON_SETTING: &str = "skeleton";

/// Returns the canonical skeleton resource selected for animations in one FBX sidecar.
fn animation_skeleton_setting(spec: Option<&asset::BEADType>) -> Result<Option<&str>, String> {
	let Some(value) = spec.and_then(|spec| spec.get(ANIMATION_SKELETON_SETTING)) else {
		return Ok(None);
	};
	value.as_str().map(Some).ok_or_else(|| {
		"Invalid animation skeleton. The most likely cause is that `skeleton` is not a resource ID string.".to_string()
	})
}

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
	let Some(target_id) = animation_skeleton_setting(spec).map_err(|error| {
		context.error(error);
		LoadErrors::FailedToProcess
	})?
	else {
		let skeleton_id = format!("{base}#{SKELETON_FRAGMENT}");
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

pub(crate) fn select_unfragmented_fbx_resource(
	scene: &ufbx::Scene,
	spec: Option<&asset::BEADType>,
) -> Result<ContainerDefaultResource, String> {
	let selected = container_default_resource(spec)?;

	if let Some(selected) = selected {
		if selected == ContainerDefaultResource::Animation && scene.anim_stacks.len() != 1 {
			return Err(format!(
				"BEAD selects animation, but the FBX contains {} animation stacks; use an explicit animation fragment",
				scene.anim_stacks.len()
			));
		}

		return Ok(selected);
	}

	if !scene.meshes.is_empty() {
		return Ok(ContainerDefaultResource::Mesh);
	}

	if scene.anim_stacks.len() == 1 {
		return Ok(ContainerDefaultResource::Animation);
	}

	Err(format!(
		"the FBX contains no mesh and {} animation stacks; use an explicit fragment",
		scene.anim_stacks.len()
	))
}

/// The `FBXAssetHandler` struct provides the authored-FBX import path used to bake meshes, skeletons, and animation clips.
#[derive(Default)]

pub struct FBXAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	generator: Option<Arc<dyn ProgramGenerator>>,
	material_mip_generator: Option<Arc<dyn MipGenerationBackend>>,
}

impl FBXAssetHandler {
	/// Creates an FBX importer using the engine's clockwise mesh-processing convention.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the winding convention that will be forwarded to mesh processing.
	pub fn triangle_front_face_winding(&self) -> TriangleFrontFaceWinding {
		self.triangle_front_face_winding
	}

	/// Selects the winding convention used when FBX triangles are packed into mesh streams.
	pub fn set_triangle_front_face_winding(&mut self, winding: TriangleFrontFaceWinding) {
		self.triangle_front_face_winding = winding;
	}

	/// Returns this handler configured with the requested triangle winding convention.
	pub fn with_triangle_front_face_winding(mut self, winding: TriangleFrontFaceWinding) -> Self {
		self.set_triangle_front_face_winding(winding);

		self
	}

	/// Installs the renderer-specific shader transformation used for generated FBX materials.
	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
	}

	/// Selects the offline backend used only for image resources generated from FBX materials.
	pub fn set_material_mip_generator(&mut self, generator: Arc<dyn MipGenerationBackend>) {
		self.material_mip_generator = Some(generator);
	}
}

impl AssetHandler for FBXAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("fbx")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(resource_type) = context.resource_type(url)
			&& !self.can_handle(resource_type)
		{
			return Err(LoadErrors::UnsupportedType);
		}

		let allocator = context.allocator();

		// Resolve the container base so animation fragments never become part of the source filename.
		let base = url.get_base();

		let source_id = ResourceId::new(base.as_ref());

		let (data, spec, source_type) = context.resolve(source_id).await?;

		if !self.can_handle(&source_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let scene = load_fbx_scene(&data, base.as_ref()).map_err(|error| {
			context.error(format_args!("Failed to import FBX asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		if let Some(fragment) = url.get_fragment() {
			let imported_skeleton = import_fbx_skeleton(&scene).map_err(|error| {
				context.error(format_args!("Failed to import FBX skeleton '{}': {error}", url.as_ref()));

				LoadErrors::FailedToProcess
			})?;

			if fragment.as_ref() == SKELETON_FRAGMENT {
				return context
					.store_primary(ProcessedAsset::new(url, imported_skeleton.model), &[])
					.await;
			}

			let (skeleton, source_to_skeleton) =
				resolve_animation_skeleton(&context, spec.as_ref(), imported_skeleton, base.as_ref()).await?;

			let animation =
				import_fbx_animation(&scene, fragment.as_ref(), skeleton, &source_to_skeleton).map_err(|error| {
					context.error(format_args!("Failed to import FBX animation '{}': {error}", url.as_ref()));

					LoadErrors::FailedToProcess
				})?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]).await;
		}

		let default_resource = select_unfragmented_fbx_resource(&scene, spec.as_ref()).map_err(|error| {
			context.error(format_args!(
				"Failed to select the default FBX resource '{}': {error}. The most likely cause is an ambiguous container without an explicit fragment or BEAD override.",
				url.as_ref()
			));
			LoadErrors::FailedToProcess
		})?;

		if default_resource == ContainerDefaultResource::Animation {
			let imported_skeleton = import_fbx_skeleton(&scene).map_err(|error| {
				context.error(format_args!(
					"Failed to import FBX animation skeleton '{}': {error}",
					url.as_ref()
				));

				LoadErrors::FailedToProcess
			})?;

			let (skeleton, source_to_skeleton) =
				resolve_animation_skeleton(&context, spec.as_ref(), imported_skeleton, base.as_ref()).await?;

			let animation =
				import_fbx_animation(&scene, DEFAULT_ANIMATION_FRAGMENT, skeleton, &source_to_skeleton).map_err(|error| {
					context.error(format_args!(
						"Failed to import default FBX animation '{}': {error}",
						url.as_ref()
					));

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
			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());

			(
				Some(store_model::<SkeletonModel>(context, &skeleton_id, imported.model, &[]).await?),
				imported.source_to_skeleton,
			)
		} else {
			(None, Vec::new())
		};

		let materials = resolve_fbx_materials(
			context,
			spec.as_ref(),
			source_id,
			&scene,
			self.generator.clone(),
			self.material_mip_generator.as_deref(),
		)
		.await?;

		let mut culled_polygons = FbxCulledPolygonCounts::default();

		let mesh = import_fbx_mesh_session(
			&scene,
			&materials,
			skeleton,
			&source_to_skeleton,
			MeshProcessor::new().with_triangle_front_face_winding(self.triangle_front_face_winding),
			allocator,
			&mut culled_polygons,
		);

		culled_polygons.trace(context);

		let mesh = mesh.map_err(|error| {
			context.error(format_args!("Failed to process FBX mesh '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;

		let mut transaction = context.begin_resource(url, mesh.payload_size()).await?;
		let (mesh, stream_descriptions) = mesh
			.finish_into_resource(&mut transaction)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		context
			.commit_primary(transaction, ProcessedAsset::new(url, mesh).with_streams(stream_descriptions))
			.await
	}
}
