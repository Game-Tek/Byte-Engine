use super::*;

pub(crate) const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";

pub(crate) const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";

pub(crate) const SKELETON_FRAGMENT: &str = "skeleton";

pub(crate) const MAX_PRIMITIVE_VERTICES: usize = u16::MAX as usize + 1;

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
		if let Some(resource_type) = context.resource_type(url) {
			if !self.can_handle(resource_type) {
				return Err(LoadErrors::UnsupportedType);
			}
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
				return context.store_primary(ProcessedAsset::new(url, imported_skeleton.model), &[]);
			}

			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());

			let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, imported_skeleton.model, &[])?;

			let animation = import_fbx_animation(&scene, fragment.as_ref(), skeleton, &imported_skeleton.source_to_skeleton)
				.map_err(|error| {
					context.error(format_args!("Failed to import FBX animation '{}': {error}", url.as_ref()));

					LoadErrors::FailedToProcess
				})?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]);
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

			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());

			let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, imported_skeleton.model, &[])?;

			let animation = import_fbx_animation(
				&scene,
				DEFAULT_ANIMATION_FRAGMENT,
				skeleton,
				&imported_skeleton.source_to_skeleton,
			)
			.map_err(|error| {
				context.error(format_args!(
					"Failed to import default FBX animation '{}': {error}",
					url.as_ref()
				));

				LoadErrors::FailedToProcess
			})?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]);
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
				Some(store_model::<SkeletonModel>(context, &skeleton_id, imported.model, &[])?),
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

		let source = import_fbx_meshes(
			&scene,
			&materials,
			skeleton,
			&source_to_skeleton,
			allocator,
			&mut culled_polygons,
		);

		culled_polygons.trace(context);

		let source = source.map_err(|error| {
			context.error(format_args!("Failed to import FBX mesh '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		let mesh = MeshProcessor::new()
			.with_triangle_front_face_winding(self.triangle_front_face_winding)
			.process_owned(source)
			.map_err(|error| {
				context.error(format_args!(
					"Failed to process FBX mesh '{}'. The most likely cause is unsupported or malformed mesh data: {error}",
					url.as_ref()
				));

				LoadErrors::FailedToProcess
			})?;

		context.store_primary(
			ProcessedAsset::new(url, mesh.mesh).with_streams(mesh.stream_descriptions),
			&mesh.buffer,
		)
	}
}
