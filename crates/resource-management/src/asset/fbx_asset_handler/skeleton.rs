use super::*;
pub(crate) fn load_fbx_scene(data: &[u8], filename: &str) -> Result<ufbx::SceneRoot, FbxImportError> {
	ufbx::load_memory(
		data,
		ufbx::LoadOpts {
			filename: ufbx::StringOpt::Ref(filename),
			target_axes: ufbx::CoordinateAxes::left_handed_y_up(),
			target_unit_meters: 1.0,
			handedness_conversion_axis: ufbx::MirrorAxis::Z,
			handedness_conversion_retain_winding: true,
			space_conversion: ufbx::SpaceConversion::AdjustTransforms,
			geometry_transform_handling: ufbx::GeometryTransformHandling::HelperNodes,
			inherit_mode_handling: ufbx::InheritModeHandling::Compensate,
			generate_missing_normals: true,
			clean_skin_weights: true,
			use_blender_pbr_material: true,
			node_depth_limit: 512,
			..Default::default()
		},
	)
	.map_err(|error| FbxImportError::Parse(error.description.to_string()))
}

/// The `ImportedFbxSkeleton` struct keeps source-node remapping beside the stored parent-ordered skeleton.
pub(crate) struct ImportedFbxSkeleton {
	pub(crate) model: SkeletonModel,
	pub(crate) source_to_skeleton: Vec<u32>,
}

/// Imports the adjusted ufbx node tree as the common pose hierarchy used by clips and skin bindings.
pub(crate) fn import_fbx_skeleton(scene: &ufbx::Scene) -> Result<ImportedFbxSkeleton, FbxImportError> {
	if scene.nodes.len() > u32::MAX as usize {
		return Err(FbxImportError::TooManySkeletonNodes);
	}

	let mut nodes = Vec::with_capacity(scene.nodes.len());
	let mut source_to_skeleton = vec![u32::MAX; scene.nodes.len()];
	append_fbx_skeleton_node(&scene.root_node, None, &mut nodes, &mut source_to_skeleton)?;
	if nodes.len() != scene.nodes.len() || source_to_skeleton.contains(&u32::MAX) {
		return Err(FbxImportError::IncompleteSkeleton);
	}

	Ok(ImportedFbxSkeleton {
		model: SkeletonModel { nodes },
		source_to_skeleton,
	})
}

/// Appends one source subtree while assigning remapped parents before their children.
pub(crate) fn append_fbx_skeleton_node(
	node: &ufbx::Node,
	parent: Option<u32>,
	nodes: &mut Vec<SkeletonNode>,
	source_to_skeleton: &mut [u32],
) -> Result<(), FbxImportError> {
	let source_index = node.element.typed_id as usize;
	let mapped = source_to_skeleton
		.get_mut(source_index)
		.ok_or(FbxImportError::InvalidSkeletonNode)?;
	if *mapped != u32::MAX {
		return Err(FbxImportError::DuplicateSkeletonNode);
	}

	let node_index = nodes.len() as u32;
	*mapped = node_index;
	nodes.push(SkeletonNode {
		name: non_empty_name(&node.element.name),
		parent,
		rest_local: local_transform_to_model(node.local_transform)?,
	});
	for child in &node.children {
		append_fbx_skeleton_node(child, Some(node_index), nodes, source_to_skeleton)?;
	}
	Ok(())
}

/// Converts ufbx's adjusted local TRS into the shared CPU-pose representation.
pub(crate) fn local_transform_to_model(transform: ufbx::Transform) -> Result<LocalTransform, FbxImportError> {
	Ok(LocalTransform {
		translation: vec3_to_f32(transform.translation, "skeleton translation")?,
		rotation: quat_to_f32(transform.rotation, "skeleton rotation")?,
		scale: vec3_to_f32(transform.scale, "skeleton scale")?,
	})
}

/// Converts one selected FBX take into sparse node tracks targeting the imported skeleton.
pub(crate) fn import_fbx_animation(
	scene: &ufbx::Scene,
	fragment: &str,
	skeleton: ReferenceModel<SkeletonModel>,
	source_to_skeleton: &[u32],
) -> Result<AnimationModel, FbxImportError> {
	let stack = select_animation_stack(scene, fragment)?;
	let baked = ufbx::bake_anim(
		scene,
		&stack.anim,
		ufbx::BakeOpts {
			trim_start_time: true,
			..Default::default()
		},
	)
	.map_err(|error| FbxImportError::AnimationBake(error.description.to_string()))?;

	let mut tracks = Vec::with_capacity(baked.nodes.len());

	for node in &baked.nodes {
		let target = remap_skeleton_node(source_to_skeleton, node.typed_id)?;
		let translation = import_vec3_curve(&node.translation_keys, "animation translation")?;
		let rotation = import_quaternion_curve(&node.rotation_keys)?;
		let scale = import_vec3_curve(&node.scale_keys, "animation scale")?;
		if translation.is_some() || rotation.is_some() || scale.is_some() {
			tracks.push(NodeTrack {
				node: target,
				translation,
				rotation,
				scale,
			});
		}
	}
	// ufbx sorts baked tracks by source typed ID, while the CPU graph requires dense hierarchy order.
	tracks.sort_unstable_by_key(|track| track.node);

	Ok(AnimationModel {
		name: non_empty_name(&stack.element.name),
		skeleton,
		duration: finite_f32(baked.playback_duration, "animation duration")?,
		tracks,
	})
}

/// Selects the first, indexed, or named animation stack addressed by an FBX resource fragment.
pub(crate) fn select_animation_stack<'a>(
	scene: &'a ufbx::Scene,
	fragment: &str,
) -> Result<&'a ufbx::AnimStack, FbxImportError> {
	if fragment == DEFAULT_ANIMATION_FRAGMENT {
		return scene
			.anim_stacks
			.as_ref()
			.first()
			.map(AsRef::as_ref)
			.ok_or_else(|| FbxImportError::AnimationNotFound("the FBX scene does not contain animation stacks".to_string()));
	}

	let selector = fragment
		.strip_prefix(ANIMATION_FRAGMENT_PREFIX)
		.ok_or_else(|| FbxImportError::UnsupportedFragment(fragment.to_string()))?;
	if selector.is_empty() {
		return Err(FbxImportError::AnimationNotFound(
			"the animation fragment has no index or name".to_string(),
		));
	}

	if let Ok(index) = selector.parse::<usize>() {
		return scene
			.anim_stacks
			.as_ref()
			.get(index)
			.map(AsRef::as_ref)
			.ok_or_else(|| FbxImportError::AnimationNotFound(format!("animation stack index {index} is out of range")));
	}

	scene
		.anim_stacks
		.as_ref()
		.iter()
		.map(AsRef::as_ref)
		.find(|stack| stack.element.name.as_ref() == selector)
		.ok_or_else(|| FbxImportError::AnimationNotFound(format!("animation stack '{selector}' does not exist")))
}

/// Resolves a source typed ID through the dense hierarchy remap shared by clips and skins.
pub(crate) fn remap_skeleton_node(source_to_skeleton: &[u32], source_node: u32) -> Result<u32, FbxImportError> {
	let mapped = source_to_skeleton
		.get(source_node as usize)
		.copied()
		.ok_or(FbxImportError::InvalidSkeletonNode)?;
	(mapped != u32::MAX)
		.then_some(mapped)
		.ok_or(FbxImportError::InvalidSkeletonNode)
}

/// Converts baked vectors directly into a persistent linear curve without transient keyframe objects.
pub(crate) fn import_vec3_curve(
	keys: &[ufbx::BakedVec3],
	context: &'static str,
) -> Result<Option<Vector3Curve>, FbxImportError> {
	if keys.is_empty() {
		return Ok(None);
	}

	let mut times = Vec::with_capacity(keys.len());
	let mut values = Vec::with_capacity(keys.len());
	for key in keys {
		times.push(finite_f32(key.time, "animation key time")?);
		values.push(vec3_to_f32(key.value, context)?);
	}
	Ok(Some(Vector3Curve::Linear { times, values }))
}

/// Converts baked rotations directly into a persistent linear quaternion curve.
pub(crate) fn import_quaternion_curve(keys: &[ufbx::BakedQuat]) -> Result<Option<QuaternionCurve>, FbxImportError> {
	if keys.is_empty() {
		return Ok(None);
	}

	let mut times = Vec::with_capacity(keys.len());
	let mut values = Vec::with_capacity(keys.len());
	for key in keys {
		times.push(finite_f32(key.time, "animation key time")?);
		values.push(quat_to_f32(key.value, "animation quaternion")?);
	}
	Ok(Some(QuaternionCurve::Linear { times, values }))
}
