use math::{Orientation, Scale, Vector};

use crate::{
	Reference, ReferenceModel, Solver, resource,
	resources::ParentSpace,
	resources::skeleton::{LocalTransform, Skeleton, SkeletonModel, SkeletonNode},
	solver::SolveError,
};

/// The `Curve` enum provides the keyframes of one animated local-pose component for CPU pose evaluation.
///
/// Values have type `V` and cubic spline tangents have type `T`. Translation uses [`TranslationCurve`], scale uses
/// [`ScaleCurve`], and rotation uses [`RotationCurve`]. Importers build one, and consumers read it through
/// [`Self::times`] and [`Self::values`] or match its interpolation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum Curve<V, T = V> {
	Step {
		times: Vec<f32>,
		values: Vec<V>,
	},
	Linear {
		times: Vec<f32>,
		values: Vec<V>,
	},
	CubicSpline {
		times: Vec<f32>,
		values: Vec<V>,
		in_tangents: Vec<T>,
		out_tangents: Vec<T>,
	},
}

/// Translation keyframes.
pub type TranslationCurve = Curve<Vector<ParentSpace>>;

/// Scale keyframes.
pub type ScaleCurve = Curve<Scale>;

/// Rotation keyframes. Cubic spline tangents are quaternion derivatives, so they are raw `[x, y, z, w]` components
/// rather than rotations.
pub type RotationCurve = Curve<Orientation, [f32; 4]>;

impl<V, T> Curve<V, T> {
	/// Returns the key times shared by every interpolation form.
	pub fn times(&self) -> &[f32] {
		match self {
			Self::Step { times, .. } | Self::Linear { times, .. } | Self::CubicSpline { times, .. } => times,
		}
	}

	/// Returns the key values shared by every interpolation form.
	pub fn values(&self) -> &[V] {
		match self {
			Self::Step { values, .. } | Self::Linear { values, .. } | Self::CubicSpline { values, .. } => values,
		}
	}

	/// Splits glTF-style interleaved `[in_tangent, value, out_tangent]` triplets into a cubic spline curve.
	///
	/// `map_value` converts each key value, such as normalizing rotations; tangents are kept as authored.
	pub fn cubic_spline_from_triplets<E>(
		times: Vec<f32>,
		triplets: &[[T; 3]],
		mut map_value: impl FnMut(T) -> Result<V, E>,
	) -> Result<Self, E>
	where
		T: Copy,
	{
		let mut in_tangents = Vec::with_capacity(triplets.len());
		let mut values = Vec::with_capacity(triplets.len());
		let mut out_tangents = Vec::with_capacity(triplets.len());

		for [incoming, value, outgoing] in triplets {
			in_tangents.push(*incoming);
			values.push(map_value(*value)?);
			out_tangents.push(*outgoing);
		}

		Ok(Self::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		})
	}

	/// Counts the heap bytes this curve's key storage owns.
	fn estimated_bytes(&self) -> usize {
		let (times, key_bytes) = match self {
			Self::Step { times, values } | Self::Linear { times, values } => (times, heap_bytes(values)),
			Self::CubicSpline {
				times,
				values,
				in_tangents,
				out_tangents,
			} => (
				times,
				heap_bytes(values)
					.saturating_add(heap_bytes(in_tangents))
					.saturating_add(heap_bytes(out_tangents)),
			),
		};

		heap_bytes(times).saturating_add(key_bytes)
	}
}

/// Counts the heap bytes a vector's allocation reserves.
fn heap_bytes<X>(items: &Vec<X>) -> usize {
	items.capacity().saturating_mul(std::mem::size_of::<X>())
}

/// The `CurveComponents` trait lets validation and samplers read a curve value or tangent as its `N` raw components.
pub trait CurveComponents<const N: usize>: Copy {
	fn components(self) -> [f32; N];
}

impl CurveComponents<3> for Vector<ParentSpace> {
	fn components(self) -> [f32; 3] {
		self.to_array()
	}
}

impl CurveComponents<3> for Scale {
	fn components(self) -> [f32; 3] {
		self.to_array()
	}
}

impl CurveComponents<4> for Orientation {
	fn components(self) -> [f32; 4] {
		self.to_array()
	}
}

/// Rotation tangents are quaternion derivatives, which are already raw components.
impl CurveComponents<4> for [f32; 4] {
	fn components(self) -> [f32; 4] {
		self
	}
}

/// Validates key timing, cardinality, and finite values and tangents before CPU graph evaluation.
fn validate_curve<V: CurveComponents<N>, T: CurveComponents<N>, const N: usize>(
	curve: &Curve<V, T>,
	duration: f32,
	track: usize,
	path: &'static str,
) -> Result<(), SolveError> {
	let (times, values) = (curve.times(), curve.values());
	if times.is_empty() || times.len() != values.len() {
		return invalid_animation(format!(
			"track {track} {path} key times and values do not have the same non-zero length"
		));
	}
	if times.iter().any(|time| !time.is_finite() || *time < 0.0 || *time > duration) {
		return invalid_animation(format!("track {track} {path} contains a time outside the clip duration"));
	}
	if times.windows(2).any(|pair| pair[0] >= pair[1]) {
		return invalid_animation(format!("track {track} {path} times are not strictly increasing"));
	}
	if !values.iter().flat_map(|value| value.components()).all(f32::is_finite) {
		return invalid_animation(format!("track {track} {path} contains a non-finite value"));
	}

	if let Curve::CubicSpline {
		in_tangents,
		out_tangents,
		..
	} = curve
	{
		if in_tangents.len() != times.len() || out_tangents.len() != times.len() {
			return invalid_animation(format!("track {track} {path} cubic tangents do not match its key count"));
		}
		if !in_tangents
			.iter()
			.chain(out_tangents)
			.flat_map(|tangent| tangent.components())
			.all(f32::is_finite)
		{
			return invalid_animation(format!("track {track} {path} contains a non-finite cubic tangent"));
		}
	}

	Ok(())
}

/// The `RootMotionDelta` struct carries a change in the root bone's translation and rotation from one moment to
/// another.
///
/// Clips store their whole travel in one as [`Animation::root_displacement`], and the animation graph player reports
/// each frame's travel to gameplay in one. Its default is [`Self::IDENTITY`].
#[derive(
	Clone,
	Copy,
	Debug,
	Default,
	PartialEq,
	serde::Serialize,
	serde::Deserialize,
	rkyv::Archive,
	rkyv::Serialize,
	rkyv::Deserialize,
)]
pub struct RootMotionDelta {
	/// Translation to apply in the skeleton root's parent space.
	pub translation: Vector<ParentSpace>,
	/// Rotation to apply after the previous root rotation.
	pub rotation: Orientation,
}

impl RootMotionDelta {
	/// The delta that preserves the current gameplay transform.
	pub const IDENTITY: Self = Self {
		translation: Vector::zero(),
		rotation: Orientation::identity(),
	};

	/// Calculates the shortest local transform delta between two sampled root poses.
	pub fn between(previous: LocalTransform, current: LocalTransform) -> Self {
		Self {
			translation: current.translation - previous.translation,
			rotation: current.rotation.compose(previous.rotation.inverse()),
		}
	}

	/// Composes this delta followed by `next`.
	///
	/// Translation remains in the skeleton root's parent space, so segment
	/// translations add directly. Use this to join root-motion segments across
	/// a looping clip boundary.
	pub fn then(self, next: Self) -> Self {
		Self {
			translation: self.translation + next.translation,
			rotation: next.rotation.compose(self.rotation),
		}
	}

	/// Blends two root-motion deltas for pose blending.
	pub fn blend(self, other: Self, factor: f32) -> Self {
		let factor = factor.clamp(0.0, 1.0);
		Self {
			translation: self.translation.lerp(other.translation, factor),
			rotation: self.rotation.nlerp(other.rotation, factor),
		}
	}
}

/// Calculates how far `root_bone` travels in the skeleton's object space from a clip's start to its end.
///
/// `nodes` is the skeleton the `tracks` target, and the tracks must be validated. Curves hold their first key before it
/// and their last key after it, and every interpolation passes through its keys, so the clip's start pose is each
/// curve's first value and its end pose is each curve's last value. Channels without a curve keep the rest pose. The
/// bone's ancestors are composed in, so a parent that moves the whole rig counts as travel.
fn root_displacement(nodes: &[SkeletonNode], root_bone: u32, tracks: &[NodeTrack]) -> RootMotionDelta {
	let start = object_space_key_pose(nodes, root_bone, tracks, |_| 0);
	let end = object_space_key_pose(nodes, root_bone, tracks, |count| count - 1);
	RootMotionDelta::between(start, end)
}

/// Composes `node` and its ancestors into the skeleton's object space from the keys `key` picks, given each curve's key
/// count.
fn object_space_key_pose(nodes: &[SkeletonNode], node: u32, tracks: &[NodeTrack], key: fn(usize) -> usize) -> LocalTransform {
	let local = |node: u32| {
		let mut transform = nodes[node as usize].rest_local;
		// Tracks ascend by node, so a binary search finds the node's track without a lookup table.
		if let Ok(track) = tracks.binary_search_by_key(&node, |track| track.node) {
			let track = &tracks[track];
			if let Some(values) = track.translation.as_ref().map(Curve::values) {
				transform.translation = values[key(values.len())];
			}
			if let Some(values) = track.rotation.as_ref().map(Curve::values) {
				transform.rotation = values[key(values.len())];
			}
			if let Some(values) = track.scale.as_ref().map(Curve::values) {
				transform.scale = values[key(values.len())];
			}
		}
		transform
	};
	let mut transform = local(node);
	let mut ancestor = nodes[node as usize].parent;
	while let Some(parent) = ancestor {
		transform = local(parent).compose(transform);
		ancestor = nodes[parent as usize].parent;
	}
	transform
}

/// The `NodeTrack` struct groups all animated local-pose curves for one skeleton node.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct NodeTrack {
	pub node: u32,
	pub translation: Option<TranslationCurve>,
	pub rotation: Option<RotationCurve>,
	pub scale: Option<ScaleCurve>,
}

/// The `Animation` struct supplies a validated clip and target skeleton to a CPU animation graph.
#[derive(Debug, serde::Serialize)]
pub struct Animation {
	pub name: Option<String>,
	pub skeleton: Reference<Skeleton>,
	pub duration: f32,
	/// The frames per second the clip was authored at, so code can address its moments by frame number. `None` when
	/// the source format does not record one, like glTF.
	pub frame_rate: Option<f32>,
	pub tracks: Vec<NodeTrack>,
	/// How far the skeleton's [root bone](Skeleton::root_bone) travels from the clip's start to its end, in the
	/// skeleton's object space. Divide its translation by [`Self::duration`] for the clip's average speed. `None`
	/// when the skeleton declares no root bone.
	pub root_displacement: Option<RootMotionDelta>,
}

impl Animation {
	/// Estimates the heap storage retained while this clip is resident in an animation pool.
	///
	/// The estimate includes decoded curve, string, and skeleton storage owned by
	/// this resource. It deliberately excludes allocator bookkeeping and mapped
	/// reader internals, which are not part of the current animation payload.
	/// Use this value with a pool byte budget instead of [`Reference::size`],
	/// because imported animation clips currently store their curves as resource
	/// metadata rather than binary payload bytes.
	pub fn estimated_resident_bytes(&self) -> usize {
		let track_bytes = self
			.tracks
			.iter()
			.map(estimated_track_bytes)
			.fold(0usize, usize::saturating_add);
		let skeleton = self.skeleton.resource();
		// `Animation` already contains the `Reference<Skeleton>` and `Skeleton`
		// headers. Count only the allocations they own here.
		let skeleton_bytes = self
			.skeleton
			.id
			.capacity()
			.saturating_add(heap_bytes(&skeleton.nodes))
			.saturating_add(
				skeleton
					.nodes
					.iter()
					.filter_map(|node| node.name.as_ref())
					.map(|name| name.capacity())
					.fold(0usize, usize::saturating_add),
			);

		std::mem::size_of::<Self>()
			.saturating_add(self.name.as_ref().map_or(0, String::capacity))
			.saturating_add(heap_bytes(&self.tracks))
			.saturating_add(track_bytes)
			.saturating_add(skeleton_bytes)
	}
}

/// Counts curve allocations beyond the [`NodeTrack`] storage counted by its parent vector.
fn estimated_track_bytes(track: &NodeTrack) -> usize {
	track
		.translation
		.as_ref()
		.map_or(0, Curve::estimated_bytes)
		.saturating_add(track.rotation.as_ref().map_or(0, Curve::estimated_bytes))
		.saturating_add(track.scale.as_ref().map_or(0, Curve::estimated_bytes))
}

/// The `AnimationModel` struct preserves a serializable pose-oriented clip and its skeleton dependency.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AnimationModel {
	pub name: Option<String>,
	pub skeleton: ReferenceModel<SkeletonModel>,
	pub duration: f32,
	/// The frames per second the clip was authored at, as [`Animation::frame_rate`].
	pub frame_rate: Option<f32>,
	pub tracks: Vec<NodeTrack>,
	/// The root bone's travel over the clip, as [`Animation::root_displacement`]. Importers fill it with
	/// [`Self::measure_root_displacement`].
	pub root_displacement: Option<RootMotionDelta>,
}

super::impl_resource_model!(Animation, AnimationModel, "Animation", references = animation_references);

/// Lists the skeleton the clip animates.
fn animation_references(animation: &AnimationModel) -> Vec<&str> {
	vec![animation.skeleton.id().as_str()]
}

impl AnimationModel {
	/// Stores how far the referenced skeleton's root bone travels over the clip in [`Self::root_displacement`], or
	/// clears it when the skeleton declares no root bone.
	///
	/// Importers call it once [`Self::tracks`] target [`Self::skeleton`], after the skeleton is stored, so a clip that
	/// shares another file's skeleton measures that skeleton's root bone.
	pub(crate) fn measure_root_displacement(&mut self) -> Result<(), String> {
		let skeleton = crate::from_slice::<SkeletonModel>(&self.skeleton.resource).map_err(|error| {
			format!(
				"Animation skeleton could not be read. The most likely cause is that '{}' contains invalid skeleton metadata: {error}. Bake the skeleton again.",
				self.skeleton.id().as_str()
			)
		})?;
		self.root_displacement = skeleton
			.root_bone
			.map(|root_bone| root_displacement(&skeleton.nodes, root_bone, &self.tracks));
		Ok(())
	}
}

impl<'de> Solver<'de, Animation> for AnimationModel {
	/// Resolves the target skeleton and rejects clip data that a CPU graph could not evaluate deterministically.
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Animation, SolveError>> {
		crate::r#async::future(async move {
			let skeleton = self.skeleton.solve(storage_backend).await?;
			validate_animation(self.duration, &self.tracks, skeleton.resource().nodes.len())?;
			if self.frame_rate.is_some_and(|rate| !rate.is_finite() || rate <= 0.0) {
				invalid_animation("the frame rate is not a finite positive number")?;
			}
			validate_root_displacement(self.root_displacement, skeleton.resource().root_bone)?;
			Ok(Animation {
				name: self.name,
				skeleton,
				duration: self.duration,
				frame_rate: self.frame_rate,
				tracks: self.tracks,
				root_displacement: self.root_displacement,
			})
		})
	}
}

impl crate::StoredModel for AnimationModel {
	type Resource = Animation;

	/// Resolves a stored clip and its skeleton dependency for CPU pose sampling and blending.
	fn solve_stored<'de>(
		stored: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Animation>, SolveError>> {
		crate::r#async::future(async move {
			let model: AnimationModel = crate::from_slice(stored.resource()).map_err(|error| {
				SolveError::DeserializationFailed(format!(
					"Animation resource could not be deserialized. The most likely cause is incompatible or corrupted clip data: {error}."
				))
			})?;
			let animation = model.solve(storage_backend).await?;
			Ok(Reference::from_stored(stored, animation, reader))
		})
	}
}

/// Validates clip-wide ordering, target, timing, cardinality, and numeric invariants.
fn validate_animation(duration: f32, tracks: &[NodeTrack], skeleton_nodes: usize) -> Result<(), SolveError> {
	if !duration.is_finite() || duration < 0.0 {
		return invalid_animation("the duration is not a finite non-negative number");
	}

	let mut previous_node = None;
	for (track_index, track) in tracks.iter().enumerate() {
		if track.node as usize >= skeleton_nodes {
			return invalid_animation(format!(
				"track {track_index} targets node {} but the skeleton has {skeleton_nodes} nodes",
				track.node
			));
		}
		if previous_node.is_some_and(|previous| track.node <= previous) {
			return invalid_animation(format!("track {track_index} does not follow strict ascending node order"));
		}
		if track.translation.is_none() && track.rotation.is_none() && track.scale.is_none() {
			return invalid_animation(format!("track {track_index} contains no pose curves"));
		}

		if let Some(curve) = &track.translation {
			validate_curve(curve, duration, track_index, "translation")?;
		}
		if let Some(curve) = &track.rotation {
			validate_curve(curve, duration, track_index, "rotation")?;
		}
		if let Some(curve) = &track.scale {
			validate_curve(curve, duration, track_index, "scale")?;
		}
		previous_node = Some(track.node);
	}

	Ok(())
}

/// Requires a finite displacement exactly when the skeleton declares a root bone, so a clip baked before its
/// skeleton gained or lost one fails instead of reporting no travel.
fn validate_root_displacement(displacement: Option<RootMotionDelta>, root_bone: Option<u32>) -> Result<(), SolveError> {
	match (displacement, root_bone) {
		(None, None) => Ok(()),
		(Some(displacement), Some(_)) => {
			let mut components = displacement.translation.to_array().into_iter().chain(displacement.rotation.to_array());
			if components.all(f32::is_finite) {
				Ok(())
			} else {
				invalid_animation("the root displacement contains a non-finite value")
			}
		}
		(None, Some(_)) | (Some(_), None) => Err(SolveError::DeserializationFailed(
			"Animation clip's root displacement does not match its skeleton. The most likely cause is that the clip was baked before its skeleton's `root_bone` changed. Bake the clip again.".into(),
		)),
	}
}

fn invalid_animation(reason: impl std::fmt::Display) -> Result<(), SolveError> {
	Err(SolveError::DeserializationFailed(format!(
		"Animation clip is invalid. The most likely cause is malformed imported animation data: {reason}."
	)))
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Scale, Vector};

	use super::{AnimationModel, NodeTrack, RootMotionDelta, RotationCurve, ScaleCurve, TranslationCurve, root_displacement};
	use crate::{
		ProcessedAsset, ReferenceModel, Solver,
		asset::ResourceId,
		resource::{WriteStorageBackend, storage_backend::MemoryStorageBackend},
		resources::{
			ParentSpace,
			skeleton::{SkeletonModel, tests::node},
		},
	};

	async fn skeleton_reference(storage: &MemoryStorageBackend, node_count: usize) -> ReferenceModel<SkeletonModel> {
		let skeleton = SkeletonModel {
			nodes: (0..node_count)
				.map(|index| {
					node(
						Some(&format!("node-{index}")),
						index.checked_sub(1).map(|parent| parent as u32),
						[0.0; 3],
					)
				})
				.collect(),
			root_bone: None,
		};
		storage
			.store(ProcessedAsset::new(ResourceId::new("test.skeleton"), skeleton), &[])
			.await
			.expect("Test skeleton should store")
			.into()
	}

	async fn valid_model(storage: &MemoryStorageBackend) -> AnimationModel {
		AnimationModel {
			name: Some("walk".into()),
			skeleton: skeleton_reference(storage, 2).await,
			duration: 1.0,
			frame_rate: Some(30.0),
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(TranslationCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![Vector::zero(), Vector::new(1.0, 2.0, 3.0)],
				}),
				rotation: Some(RotationCurve::Step {
					times: vec![0.0],
					values: vec![Orientation::identity()],
				}),
				scale: None,
			}],
			root_displacement: None,
		}
	}

	#[crate::r#async::test]
	async fn solving_rejects_unsorted_duplicate_and_out_of_range_tracks() {
		let storage = MemoryStorageBackend::new();
		let mut model = valid_model(&storage).await;
		model.tracks.insert(
			0,
			NodeTrack {
				node: 1,
				translation: None,
				rotation: None,
				scale: Some(ScaleCurve::Step {
					times: vec![0.0],
					values: vec![Scale::identity()],
				}),
			},
		);

		assert!(model.solve(&storage).await.is_err());

		let mut model = valid_model(&storage).await;
		model.tracks[0].node = 2;

		assert!(model.solve(&storage).await.is_err());
	}

	#[crate::r#async::test]
	async fn solving_rejects_invalid_curve_cardinality_timing_and_numbers() {
		let storage = MemoryStorageBackend::new();
		let mut model = valid_model(&storage).await;
		model.tracks[0].translation = Some(TranslationCurve::CubicSpline {
			times: vec![0.0, 0.0],
			values: vec![Vector::zero(), Vector::new(1.0, 1.0, 1.0)],
			in_tangents: vec![Vector::zero()],
			out_tangents: vec![Vector::zero(), Vector::new(f32::NAN, f32::NAN, f32::NAN)],
		});

		assert!(model.solve(&storage).await.is_err());

		let mut model = valid_model(&storage).await;
		model.duration = f32::INFINITY;

		assert!(model.solve(&storage).await.is_err());
	}

	#[crate::r#async::test]
	async fn solving_accepts_arbitrary_finite_rotation_cubic_tangents() {
		let storage = MemoryStorageBackend::new();
		let mut model = valid_model(&storage).await;
		model.tracks[0].rotation = Some(RotationCurve::CubicSpline {
			times: vec![0.0, 1.0],
			values: vec![
				Orientation::identity(),
				Orientation::try_from_array([0.0, 0.0, 1.0, 0.0]).unwrap(),
			],
			in_tangents: vec![[50.0, -20.0, 4.0, 0.5], [-2.0, 3.0, 7.0, 11.0]],
			out_tangents: vec![[-8.0, 9.0, 10.0, 12.0], [4.0, 3.0, 2.0, 1.0]],
		});

		assert!(model.solve(&storage).await.is_ok());
	}

	fn yaw(radians: f32) -> Orientation {
		Orientation::try_from_rotation_vector(Vector::<ParentSpace>::new(0.0, radians, 0.0)).unwrap()
	}

	fn assert_near(actual: impl IntoIterator<Item = f32>, expected: impl IntoIterator<Item = f32>) {
		let (actual, expected): (Vec<_>, Vec<_>) = (actual.into_iter().collect(), expected.into_iter().collect());
		assert!(
			actual
				.iter()
				.zip(&expected)
				.all(|(actual, expected)| (actual - expected).abs() < 1.0e-5),
			"{actual:?} is not near {expected:?}"
		);
	}

	#[test]
	fn root_displacement_measures_end_keys_through_the_bone_ancestors() {
		// An untracked grandparent offsets the rig, a parent turns it a quarter, and the root bone walks along its
		// parent's +Z between keys that start after the clip begins and end before it ends.
		let nodes = vec![
			node(Some("Offset"), None, [5.0, 0.0, 0.0]),
			node(Some("Rig"), Some(0), [0.0; 3]),
			node(Some("Root"), Some(1), [0.0, 1.0, 0.0]),
		];
		let quarter = yaw(std::f32::consts::FRAC_PI_2);
		let tracks = vec![
			NodeTrack {
				node: 1,
				translation: None,
				rotation: Some(RotationCurve::Step {
					times: vec![0.0],
					values: vec![quarter],
				}),
				scale: None,
			},
			NodeTrack {
				node: 2,
				translation: Some(TranslationCurve::Linear {
					times: vec![0.25, 0.75],
					values: vec![Vector::new(0.0, 1.0, 0.0), Vector::new(0.0, 1.0, 2.0)],
				}),
				rotation: Some(RotationCurve::Linear {
					times: vec![0.25, 0.75],
					values: vec![Orientation::identity(), yaw(0.5)],
				}),
				scale: None,
			},
		];

		let displacement = root_displacement(&nodes, 2, &tracks);

		// The walk turns with the parent, and the grandparent's fixed offset cancels out.
		assert_near(
			displacement.translation.to_array(),
			quarter.rotate_vector(Vector::<ParentSpace>::new(0.0, 0.0, 2.0)).to_array(),
		);
		assert_near(displacement.rotation.to_array(), yaw(0.5).to_array());
	}

	#[test]
	fn root_displacement_of_an_unanimated_bone_is_the_identity() {
		let nodes = vec![node(Some("Root"), None, [1.0, 2.0, 3.0])];

		assert_eq!(root_displacement(&nodes, 0, &[]), RootMotionDelta::IDENTITY);
	}

	#[crate::r#async::test]
	async fn solving_requires_a_displacement_exactly_when_the_skeleton_has_a_root_bone() {
		let storage = MemoryStorageBackend::new();
		let mut model = valid_model(&storage).await;
		model.root_displacement = Some(RootMotionDelta::IDENTITY);

		assert!(model.solve(&storage).await.is_err());

		let skeleton = SkeletonModel {
			nodes: vec![node(Some("node-0"), None, [0.0; 3]), node(Some("node-1"), Some(0), [0.0; 3])],
			root_bone: Some(1),
		};
		let rooted: ReferenceModel<SkeletonModel> = storage
			.store(ProcessedAsset::new(ResourceId::new("rooted.skeleton"), skeleton), &[])
			.await
			.expect("Test skeleton should store")
			.into();
		let mut model = valid_model(&storage).await;
		model.skeleton = rooted.clone();

		assert!(model.solve(&storage).await.is_err());

		let mut model = valid_model(&storage).await;
		model.skeleton = rooted;
		model.measure_root_displacement().unwrap();

		assert_near(model.root_displacement.unwrap().translation.to_array(), [1.0, 2.0, 3.0]);
		assert!(model.solve(&storage).await.is_ok());
	}
}
