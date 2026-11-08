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

/// The docs section that specifies authored detail levels.
const AUTHORED_LEVELS_DOCS_PATH: &str = "reference/assets#authored-detail-levels";

/// Parses the files the `lods.authored` pattern of `spec` names as the coarser levels of the model at `base`, finest
/// first, stopping at the first that doesn't exist. Returns none when the pattern is unset or doesn't match the model,
/// when detail levels are off, and, with a warning, when the model is skinned or the bake gives it lightmap UVs.
async fn load_authored_levels(
	context: BakeContext<'_>,
	spec: Option<&asset::BEADType>,
	base: &str,
	settings: MeshSettings,
	skinned: bool,
) -> Result<Vec<(String, ufbx::SceneRoot)>, LoadErrors> {
	let pattern = asset::authored_level_pattern(spec).map_err(|error| {
		context.error(format_args!(
			"Failed to read the authored detail levels of FBX asset '{base}': {error}. The most likely cause is a mistyped `lods.authored` pattern in the asset's BEAD file. Write it like \"{{name}}_LOD{{level}}\"."
		));
		LoadErrors::FailedToProcess
	})?;
	let Some(ids) = pattern.as_ref().and_then(|pattern| pattern.coarser_level_ids(base)) else {
		return Ok(Vec::new());
	};
	if settings.lods.max_levels <= 1 {
		return Ok(Vec::new());
	}
	if skinned || settings.lightmap_uvs.is_some() {
		let reason = if skinned { "it has a skeleton" } else { "the bake gives it lightmap UVs" };
		context.warn(format_args!(
			"FBX asset '{base}' leaves its authored detail levels out because {reason}, which authored levels don't support. Its levels are generated instead. See {}.",
			crate::online_docs_url(AUTHORED_LEVELS_DOCS_PATH)
		));
		return Ok(Vec::new());
	}
	let mut levels = Vec::new();
	for id in ids.take(settings.lods.max_levels as usize - 1) {
		// A missing file ends the chain. The bake records its absence, so adding it later rebakes the model.
		let Ok((data, _)) = context.resolve(ResourceId::new(&id)).await else {
			break;
		};
		// The scene owns its geometry, so the source bytes drop once it's parsed.
		let scene = load_fbx_scene(&data, &id).map_err(|error| {
			context.error(format_args!(
				"Failed to import authored detail level '{id}' of FBX asset '{base}': {error}. The most likely cause is a damaged file or one that isn't FBX. Export the level again, or delete it to end the model's levels before it. See {}.",
				crate::online_docs_url(AUTHORED_LEVELS_DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		})?;
		drop(data);
		levels.push((id, scene));
	}
	Ok(levels)
}

/// The `FBXAssetHandler` struct provides the authored-FBX import path used to bake meshes, skeletons, and animation clips.
///
/// Generated materials target the asset manager's
/// [`ProgramGenerator`](crate::asset::handler::implementations::ProgramGenerator).
#[derive(Default)]
pub struct FBXAssetHandler {
	lightmap_uvs: bool,
}

impl FBXAssetHandler {
	/// Returns this handler set to give every unskinned primitive lightmap UVs when `enabled`, unless the model's BEAD
	/// file sets `lightmap_uvs.enabled` to `false`.
	///
	/// The engine's default bake configuration enables it when the bake builds in baked GI, so lightmaps can bake
	/// the model's meshes.
	pub fn with_lightmap_uvs(mut self, enabled: bool) -> Self {
		self.lightmap_uvs = enabled;
		self
	}

	/// Bakes one texture of an FBX as its own image resource.
	///
	/// Materials request these as dependencies, so the scene is parsed without geometry or animation data. The texture
	/// bakes with the semantic of the material input that samples it, from [`fbx_image_semantics`]. `spec` is the FBX's
	/// BEAD manifest, whose `images` object applies to every texture.
	async fn store_texture(
		&self,
		context: BakeContext<'_>,
		url: ResourceId<'_>,
		source_id: ResourceId<'_>,
		spec: Option<&asset::BEADType>,
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

		// The surface input that samples the texture picks its semantic; an unsampled texture keeps all of its color.
		let semantic = fbx_image_semantics(&scene)
			.get(texture_index)
			.copied()
			.flatten()
			.unwrap_or(Semantic::Albedo);

		store_imported_image(context, url, semantic, spec, source).await
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
			return self
				.store_texture(context, url, source_id, spec.as_ref(), &data, texture_index)
				.await;
		}

		let scene = load_fbx_scene(&data, base.as_ref()).map_err(|error| {
			context.error(format_args!("Failed to import FBX asset '{}': {error}", url.as_ref()));

			LoadErrors::FailedToProcess
		})?;

		// A material fragment bakes that one generated material, its shader, and the images it samples, without the mesh.
		if let Some(fragment) = fragment
			&& fragment.starts_with(MATERIAL_FRAGMENT_PREFIX)
		{
			let key = generated_material_key_for_id(&scene, spec.as_ref(), url, allocator)
				.ok_or_else(|| missing_generated_material(context, url))?;

			resolve_fbx_materials(context, spec.as_ref(), source_id, &scene, &[key]).await?;
			return Ok(());
		}

		// Any fragment left names the skeleton or an animation. An unfragmented request bakes the container's default
		// resource, and its diagnostics say so.
		let default_clip = fragment.is_none();
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
				let skeleton = if default_clip { "animation skeleton" } else { "skeleton" };
				context.error(format_args!("Failed to import FBX {skeleton} '{}': {error}", url.as_ref()));

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
				let animation = if default_clip {
					"default FBX animation"
				} else {
					"FBX animation"
				};
				context.error(format_args!("Failed to import {animation} '{}': {error}", url.as_ref()));

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
		let settings = mesh_settings(spec.as_ref(), self.lightmap_uvs).map_err(|error| {
			context.error(format_args!(
				"Failed to read FBX mesh settings '{}': {error}. The most likely cause is a mistyped `lods` or `lightmap_uvs` object in the asset's BEAD file.",
				url.as_ref()
			));
			LoadErrors::FailedToProcess
		})?;
		let level_scenes = load_authored_levels(context, spec.as_ref(), base.as_ref(), settings, skeleton.is_some()).await?;
		let levels: Vec<(&str, &ufbx::Scene)> = level_scenes.iter().map(|(id, scene)| (id.as_str(), &**scene)).collect();
		let mut level_notes = FbxAuthoredLevelNotes::default();
		let mut culled_polygons = FbxCulledPolygonCounts::default();
		let (materials, mesh) = std::future::join!(
			resolve_fbx_materials(context, spec.as_ref(), source_id, &scene, &material_keys,),
			async {
				import_fbx_mesh_session(
					&scene,
					&material_keys,
					skeleton,
					&source_to_skeleton,
					allocator,
					&mut culled_polygons,
					settings,
					&levels,
					&mut level_notes,
				)
			},
		)
		.await;

		culled_polygons.trace(context);
		for warning in &level_notes.warnings {
			context.warn(format_args!(
				"FBX asset '{}': {warning} See {}.",
				url.as_ref(),
				crate::online_docs_url(AUTHORED_LEVELS_DOCS_PATH)
			));
		}
		for note in &level_notes.info {
			context.info(format_args!("FBX asset '{}': {note}", url.as_ref()));
		}

		let mesh = mesh.map_err(|error| {
			context.error(format_args!("Failed to process FBX mesh '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;
		let materials = materials?;

		commit_mesh(context, url, mesh, &materials).await
	}
}
