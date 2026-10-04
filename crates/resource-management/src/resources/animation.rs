use math::{Orientation, Scale, Vector};

use crate::{
	Reference, ReferenceModel, Solver, resource,
	resources::ParentSpace,
	resources::skeleton::{Skeleton, SkeletonModel},
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
	pub tracks: Vec<NodeTrack>,
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
	pub tracks: Vec<NodeTrack>,
}

super::impl_resource_model!(Animation, AnimationModel, "Animation");

impl<'de> Solver<'de, Animation> for AnimationModel {
	/// Resolves the target skeleton and rejects clip data that a CPU graph could not evaluate deterministically.
	fn solve(
		self,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Animation, SolveError>> {
		crate::r#async::future(async move {
			let skeleton = self.skeleton.solve(storage_backend).await?;
			validate_animation(self.duration, &self.tracks, skeleton.resource().nodes.len())?;
			Ok(Animation {
				name: self.name,
				skeleton,
				duration: self.duration,
				tracks: self.tracks,
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

fn invalid_animation(reason: impl std::fmt::Display) -> Result<(), SolveError> {
	Err(SolveError::DeserializationFailed(format!(
		"Animation clip is invalid. The most likely cause is malformed imported animation data: {reason}."
	)))
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Scale, Vector};

	use super::{AnimationModel, NodeTrack, RotationCurve, ScaleCurve, TranslationCurve};
	use crate::{
		ProcessedAsset, ReferenceModel, Solver,
		asset::ResourceId,
		resource::{WriteStorageBackend, storage_backend::tests::TestStorageBackend},
		resources::skeleton::{SkeletonModel, tests::node},
	};

	async fn skeleton_reference(storage: &TestStorageBackend, node_count: usize) -> ReferenceModel<SkeletonModel> {
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
		};
		storage
			.store(ProcessedAsset::new(ResourceId::new("test.skeleton"), skeleton), &[])
			.await
			.expect("Test skeleton should store")
			.into()
	}

	async fn valid_model(storage: &TestStorageBackend) -> AnimationModel {
		AnimationModel {
			name: Some("walk".into()),
			skeleton: skeleton_reference(storage, 2).await,
			duration: 1.0,
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
		}
	}

	#[crate::r#async::test]
	async fn solving_rejects_unsorted_duplicate_and_out_of_range_tracks() {
		let storage = TestStorageBackend::new();
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
		let storage = TestStorageBackend::new();
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
		let storage = TestStorageBackend::new();
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
}
