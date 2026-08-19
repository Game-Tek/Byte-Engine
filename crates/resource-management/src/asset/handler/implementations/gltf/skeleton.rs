use super::*;

pub(crate) fn parse_gltf_json(source: &[u8]) -> Result<gltf::Gltf, LoadErrors> {
	let source = std::str::from_utf8(source).map_err(|_| LoadErrors::FailedToProcess)?;

	if let Ok(gltf) = gltf::Gltf::from_slice(source.as_bytes()) {
		return Ok(gltf);
	}

	// GLB JSON chunks may use NUL padding, which is outside JSON5 but permitted by the GLB container.
	let json_source = source.trim_end_matches([' ', '\0']);

	let document: serde_json::Value = json5::from_str(json_source).map_err(|_| LoadErrors::FailedToProcess)?;

	let normalized = serde_json::to_vec(&document).map_err(|_| LoadErrors::FailedToProcess)?;

	gltf::Gltf::from_slice(&normalized).map_err(|_| LoadErrors::FailedToProcess)
}

/// The `GltfNodeGraph` struct keeps the imported skeleton and source-node lookup data aligned for mesh and clip conversion.
pub(crate) struct GltfNodeGraph {
	pub(crate) skeleton: SkeletonModel,
	pub(crate) source_to_dense: Vec<u32>,
	pub(crate) source_global_transforms: Vec<maths_rs::Mat4f>,
}

/// Imports every glTF node into a deterministic parent-before-child graph shared by animation channels and skin bindings.
pub(crate) fn import_gltf_node_graph(gltf: &gltf::Gltf) -> Result<GltfNodeGraph, GltfSkeletalImportError> {
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
pub(crate) fn append_gltf_skeleton_subtree(
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
pub(crate) fn append_gltf_node_subtree<'a>(node: gltf::Node<'a>, nodes: &mut Vec<gltf::Node<'a>>) {
	nodes.push(node.clone());

	for child in node.children() {
		append_gltf_node_subtree(child, nodes);
	}
}

/// Retains the dense node identity needed to drive both skinned and rigid primitives from CPU animation output.
pub(crate) fn gltf_primitive_transform_node(
	graph: &GltfNodeGraph,
	node: &gltf::Node<'_>,
	retain_skeleton: bool,
) -> Option<u32> {
	retain_skeleton.then_some(graph.source_to_dense[node.index()])
}

/// Rejects a singular bind transform when flattened geometry must later be recovered by a CPU-driven node pose.
pub(crate) fn validate_gltf_flattened_animation_transform(
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
pub(crate) fn convert_gltf_local_transform(
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

pub(crate) fn handedness_matrix() -> maths_rs::Mat4f {
	maths_rs::Mat4f::from_scale(Vec3::new(1.0, 1.0, -1.0))
}

/// Builds the inverse-transpose matrix required to preserve normals under nonuniform node scale.
pub(crate) fn gltf_normal_transform(transform: maths_rs::Mat4f) -> Result<maths_rs::Mat4f, GltfSkeletalImportError> {
	let determinant = transform.determinant();

	if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}

	Ok(transform.inverse().transpose())
}

/// Reports whether an affine transform preserves or flips tangent-space handedness.
pub(crate) fn gltf_transform_orientation(transform: maths_rs::Mat4f) -> Result<f32, GltfSkeletalImportError> {
	let determinant = transform.determinant();

	if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
		return Err(GltfSkeletalImportError::InvalidVertexDirection);
	}

	Ok(determinant.signum())
}

/// Applies only the linear matrix portion to a direction and returns a normalized result without allocating.
pub(crate) fn transform_gltf_unit_direction(
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
pub(crate) fn transform_gltf_tangent(
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
pub(crate) fn mat4_from_columns(matrix: [[f32; 4]; 4]) -> maths_rs::Mat4f {
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

/// Converts an affine maths-rs matrix into the compact column-major resource representation.
pub(crate) fn affine_matrix4x3_from_matrix4(matrix: maths_rs::Mat4f) -> AffineMatrix4x3Columns {
	[
		[matrix[(0, 0)], matrix[(1, 0)], matrix[(2, 0)]],
		[matrix[(0, 1)], matrix[(1, 1)], matrix[(2, 1)]],
		[matrix[(0, 2)], matrix[(1, 2)], matrix[(2, 2)]],
		[matrix[(0, 3)], matrix[(1, 3)], matrix[(2, 3)]],
	]
}

/// Rejects non-finite matrix components before they enter serializable skeletal resources.
pub(crate) fn validate_finite_matrix(matrix: &maths_rs::Mat4f, context: &'static str) -> Result<(), GltfSkeletalImportError> {
	if matrix.m.iter().all(|component| component.is_finite()) {
		Ok(())
	} else {
		Err(GltfSkeletalImportError::NonFinite(context))
	}
}

/// Rejects projective matrices because compact skinning matrices omit their homogeneous row.
pub(crate) fn validate_affine_matrix(matrix: &maths_rs::Mat4f, context: &'static str) -> Result<(), GltfSkeletalImportError> {
	const AFFINE_EPSILON: f32 = 0.00001;

	if matrix[(3, 0)].abs() <= AFFINE_EPSILON
		&& matrix[(3, 1)].abs() <= AFFINE_EPSILON
		&& matrix[(3, 2)].abs() <= AFFINE_EPSILON
		&& (matrix[(3, 3)] - 1.0).abs() <= AFFINE_EPSILON
	{
		Ok(())
	} else {
		Err(GltfSkeletalImportError::NonAffine(context))
	}
}

#[derive(Debug, PartialEq, Eq)]

pub(crate) enum GltfSkeletalImportError {
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
	NonAffine(&'static str),
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
			Self::NonAffine(context) => write!(formatter, "glTF skin transform is projective. The most likely cause is a skin inverse-bind {context} that cannot use the compact affine matrix format."),
		}
	}
}

impl std::error::Error for GltfSkeletalImportError {}

pub(crate) fn is_gltf_animation_fragment(fragment: &str) -> bool {
	fragment == DEFAULT_ANIMATION_FRAGMENT || fragment.starts_with(ANIMATION_FRAGMENT_PREFIX)
}

pub(crate) fn generated_gltf_skeleton_id(source: ResourceId<'_>) -> String {
	format!("{}#{SKELETON_FRAGMENT}", source.get_base().as_ref())
}

/// Selects the first, indexed, or named clip addressed by a reserved glTF animation fragment.
pub(crate) fn select_gltf_animation<'a>(
	gltf: &'a gltf::Gltf,
	fragment: &str,
) -> Result<gltf::Animation<'a>, GltfSkeletalImportError> {
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
pub(crate) fn required_gltf_animation_buffers(gltf: &gltf::Gltf, fragment: &str) -> Result<Vec<bool>, GltfSkeletalImportError> {
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
pub(crate) fn mark_gltf_accessor_buffers(accessor: gltf::Accessor<'_>, required: &mut [bool]) {
	if let Some(view) = accessor.view() {
		required[view.buffer().index()] = true;
	}

	if let Some(sparse) = accessor.sparse() {
		required[sparse.indices().view().buffer().index()] = true;

		required[sparse.values().view().buffer().index()] = true;
	}
}

/// Converts one glTF clip into node-indexed curves ready for a future CPU animation graph.
pub(crate) fn import_gltf_animation(
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
pub(crate) fn validate_animation_times(times: &[f32]) -> Result<(), GltfSkeletalImportError> {
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

pub(crate) enum GltfVector3Semantic {
	Translation,
	Scale,
}

pub(crate) fn convert_gltf_vector3(
	value: [f32; 3],
	semantic: GltfVector3Semantic,
) -> Result<[f32; 3], GltfSkeletalImportError> {
	if value.iter().any(|component| !component.is_finite()) {
		return Err(GltfSkeletalImportError::NonFinite("animation vector key"));
	}

	Ok(match semantic {
		GltfVector3Semantic::Translation => [value[0], value[1], -value[2]],
		GltfVector3Semantic::Scale => value,
	})
}

pub(crate) fn convert_gltf_quaternion(value: [f32; 4]) -> Result<[f32; 4], GltfSkeletalImportError> {
	if value.iter().any(|component| !component.is_finite()) {
		return Err(GltfSkeletalImportError::NonFinite("animation quaternion key"));
	}

	Ok([-value[0], -value[1], value[2], value[3]])
}

/// Splits glTF's interleaved cubic spline triplets into graph-friendly tangent and value arrays.
pub(crate) fn make_vector3_curve(
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
pub(crate) fn make_quaternion_curve(
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
pub(crate) fn normalize_gltf_quaternion_value(mut value: [f32; 4]) -> Result<[f32; 4], GltfSkeletalImportError> {
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
pub(crate) fn import_gltf_skin_binding(
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
					[
						[1.0, 0.0, 0.0, 0.0],
						[0.0, 1.0, 0.0, 0.0],
						[0.0, 0.0, 1.0, 0.0],
						[0.0, 0.0, 0.0, 1.0],
					],
					inverse_source_global,
					handedness,
				)?,
			});
		}
	}

	Ok(SkinBinding { entries })
}

/// Converts one source inverse bind into the flattened left-handed vertex basis used by the mesh resource.
pub(crate) fn adjust_gltf_inverse_bind(
	inverse_bind: [[f32; 4]; 4],
	inverse_source_global: maths_rs::Mat4f,
	handedness: maths_rs::Mat4f,
) -> Result<AffineMatrix4x3Columns, GltfSkeletalImportError> {
	let inverse_bind = mat4_from_columns(inverse_bind);

	validate_finite_matrix(&inverse_bind, "inverse bind matrix")?;

	validate_affine_matrix(&inverse_bind, "matrix")?;

	// Vertices are flattened by S*G, so S*IBM*inverse(G)*S keeps
	// J_lh*adjustedIBM*flattenedVertex equivalent to S*J*IBM*vertex.
	let adjusted = handedness * inverse_bind * inverse_source_global * handedness;

	validate_finite_matrix(&adjusted, "adjusted inverse bind matrix")?;

	validate_affine_matrix(&adjusted, "matrix after coordinate conversion")?;

	Ok(affine_matrix4x3_from_matrix4(adjusted))
}

/// The `GltfVertexSkinIterator` struct normalizes borrowed glTF influence sets without staging per-primitive vectors.
pub(crate) struct GltfVertexSkinIterator<'a> {
	set0_joints: gltf::mesh::util::joints::CastingIter<'a, gltf::mesh::util::joints::U16>,
	set0_weights: gltf::mesh::util::weights::CastingIter<'a, gltf::mesh::util::weights::F32>,
	set1: Option<(
		gltf::mesh::util::joints::CastingIter<'a, gltf::mesh::util::joints::U16>,
		gltf::mesh::util::weights::CastingIter<'a, gltf::mesh::util::weights::F32>,
	)>,
	joint_count: usize,
}

impl<'a> GltfVertexSkinIterator<'a> {
	/// Creates an influence iterator after validating that every accessor can yield one value per vertex.
	pub(crate) fn new<'document, F>(
		reader: &gltf::mesh::Reader<'document, 'a, F>,
		vertex_count: usize,
		joint_count: usize,
	) -> Result<Self, GltfSkeletalImportError>
	where
		F: Clone + Fn(gltf::Buffer<'document>) -> Option<&'a [u8]>,
	{
		let set0_joints = reader.read_joints(0);
		let set0_weights = reader.read_weights(0);
		if set0_joints.is_some() != set0_weights.is_some() {
			return Err(GltfSkeletalImportError::UnpairedSkinAttributes(0));
		}
		let (Some(set0_joints), Some(set0_weights)) = (set0_joints, set0_weights) else {
			return Err(GltfSkeletalImportError::MissingSkinAttributes);
		};
		let set0_joints = set0_joints.into_u16();
		let set0_weights = set0_weights.into_f32();

		let set1_joints = reader.read_joints(1);
		let set1_weights = reader.read_weights(1);
		if set1_joints.is_some() != set1_weights.is_some() {
			return Err(GltfSkeletalImportError::UnpairedSkinAttributes(1));
		}
		let set1 = match (set1_joints, set1_weights) {
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

		Ok(Self {
			set0_joints,
			set0_weights,
			set1,
			joint_count,
		})
	}
}

impl ExactSizeIterator for GltfVertexSkinIterator<'_> {}

impl Iterator for GltfVertexSkinIterator<'_> {
	type Item = Result<VertexSkin, GltfSkeletalImportError>;

	fn next(&mut self) -> Option<Self::Item> {
		let joints = self.set0_joints.next()?;
		let weights = match self.set0_weights.next() {
			Some(weights) => weights,
			None => return Some(Err(GltfSkeletalImportError::MismatchedSkinAttributeCount)),
		};
		let mut influences = [(0u16, 0.0f32); 8];
		for influence in 0..4 {
			influences[influence] = (joints[influence], weights[influence]);
		}
		let influence_count = if let Some((joints, weights)) = &mut self.set1 {
			let Some(joints) = joints.next() else {
				return Some(Err(GltfSkeletalImportError::MismatchedSkinAttributeCount));
			};
			let Some(weights) = weights.next() else {
				return Some(Err(GltfSkeletalImportError::MismatchedSkinAttributeCount));
			};
			for influence in 0..4 {
				influences[influence + 4] = (joints[influence], weights[influence]);
			}
			8
		} else {
			4
		};

		for &(joint, weight) in &influences[..influence_count] {
			if joint as usize >= self.joint_count {
				return Some(Err(GltfSkeletalImportError::SkinJointOutOfRange));
			}
			if !weight.is_finite() || weight < 0.0 {
				return Some(Err(GltfSkeletalImportError::InvalidSkinWeight));
			}
		}
		influences[..influence_count]
			.sort_unstable_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
		let total = influences[..4].iter().map(|(_, weight)| *weight).sum::<f32>();
		if !total.is_finite() || total <= f32::EPSILON {
			return Some(Err(GltfSkeletalImportError::InvalidSkinWeight));
		}
		let mut vertex_skin = VertexSkin {
			joints: [0; 4],
			weights: [0.0; 4],
		};
		for influence in 0..4 {
			vertex_skin.joints[influence] = influences[influence].0;
			vertex_skin.weights[influence] = influences[influence].1 / total;
		}
		Some(Ok(vertex_skin))
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		self.set0_joints.size_hint()
	}
}

/// Reads both supported glTF influence sets into owned values for callers that need retained skin data.
#[cfg(test)]
pub(crate) fn import_gltf_vertex_skin<'a, 's, F>(
	reader: &gltf::mesh::Reader<'a, 's, F>,
	vertex_count: usize,
	joint_count: usize,
) -> Result<(Vec<[u16; 4]>, Vec<[f32; 4]>), GltfSkeletalImportError>
where
	F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
	let vertex_skin = GltfVertexSkinIterator::new(reader, vertex_count, joint_count)?.collect::<Result<Vec<_>, _>>()?;
	Ok(vertex_skin.into_iter().map(|value| (value.joints, value.weights)).unzip())
}

/// Validates the paired influence sets consumed by skinned instances while allowing a shared mesh to be instanced rigidly.
pub(crate) fn validate_gltf_skin_attribute_sets(
	primitive: &gltf::Primitive<'_>,
	is_skinned: bool,
) -> Result<(), GltfSkeletalImportError> {
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
pub(crate) fn include_skin_vertex_layout(
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
