use crate::{
	resource,
	resources::skeleton::{Skeleton, SkeletonModel},
	solver::SolveErrors,
	Model, Reference, ReferenceModel, Resource, Solver,
};

/// Describes translation and scale keyframes in a representation ready for CPU pose evaluation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum Vector3Curve {
	Step {
		times: Vec<f32>,
		values: Vec<[f32; 3]>,
	},
	Linear {
		times: Vec<f32>,
		values: Vec<[f32; 3]>,
	},
	CubicSpline {
		times: Vec<f32>,
		values: Vec<[f32; 3]>,
		in_tangents: Vec<[f32; 3]>,
		out_tangents: Vec<[f32; 3]>,
	},
}

impl Vector3Curve {
	/// Validates key timing, cardinality, and finite tangent data before CPU graph evaluation.
	fn validate(&self, duration: f32, track: usize, path: &'static str) -> Result<(), SolveErrors> {
		match self {
			Self::Step { times, values } | Self::Linear { times, values } => {
				validate_times_and_values(times, values, duration, track, path)
			}
			Self::CubicSpline {
				times,
				values,
				in_tangents,
				out_tangents,
			} => {
				validate_times_and_values(times, values, duration, track, path)?;
				if in_tangents.len() != times.len() || out_tangents.len() != times.len() {
					return invalid_animation(format!("track {track} {path} cubic tangents do not match its key count"));
				}
				if !in_tangents
					.iter()
					.flatten()
					.chain(out_tangents.iter().flatten())
					.all(|value| value.is_finite())
				{
					return invalid_animation(format!("track {track} {path} contains a non-finite cubic tangent"));
				}
				Ok(())
			}
		}
	}
}

/// Describes rotation keyframes in a representation ready for CPU pose evaluation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum QuaternionCurve {
	Step {
		times: Vec<f32>,
		values: Vec<[f32; 4]>,
	},
	Linear {
		times: Vec<f32>,
		values: Vec<[f32; 4]>,
	},
	CubicSpline {
		times: Vec<f32>,
		values: Vec<[f32; 4]>,
		in_tangents: Vec<[f32; 4]>,
		out_tangents: Vec<[f32; 4]>,
	},
}

impl QuaternionCurve {
	/// Validates rotation keys as unit quaternions while preserving finite cubic derivative magnitudes.
	fn validate(&self, duration: f32, track: usize) -> Result<(), SolveErrors> {
		match self {
			Self::Step { times, values } | Self::Linear { times, values } => {
				validate_times_and_values(times, values, duration, track, "rotation")?;
				validate_quaternion_values(values, track)
			}
			Self::CubicSpline {
				times,
				values,
				in_tangents,
				out_tangents,
			} => {
				validate_times_and_values(times, values, duration, track, "rotation")?;
				validate_quaternion_values(values, track)?;
				if in_tangents.len() != times.len() || out_tangents.len() != times.len() {
					return invalid_animation(format!("track {track} rotation cubic tangents do not match its key count"));
				}
				if !in_tangents
					.iter()
					.flatten()
					.chain(out_tangents.iter().flatten())
					.all(|value| value.is_finite())
				{
					return invalid_animation(format!("track {track} rotation contains a non-finite cubic tangent"));
				}
				Ok(())
			}
		}
	}
}

/// Validates rotation values while leaving cubic derivative tangents free to use arbitrary finite magnitudes.
fn validate_quaternion_values(values: &[[f32; 4]], track: usize) -> Result<(), SolveErrors> {
	if values.iter().any(|value| {
		let length_squared = value.iter().map(|component| component * component).sum::<f32>();
		(length_squared - 1.0).abs() > 1.0e-3
	}) {
		return invalid_animation(format!(
			"track {track} rotation contains a zero-length or non-unit quaternion value"
		));
	}
	Ok(())
}

/// The `NodeTrack` struct groups all animated local-pose curves for one skeleton node.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct NodeTrack {
	pub node: u32,
	pub translation: Option<Vector3Curve>,
	pub rotation: Option<QuaternionCurve>,
	pub scale: Option<Vector3Curve>,
}

/// The `Animation` struct supplies a validated clip and target skeleton to a CPU animation graph.
#[derive(Debug, serde::Serialize)]
pub struct Animation {
	pub name: Option<String>,
	pub skeleton: Reference<Skeleton>,
	pub duration: f32,
	pub tracks: Vec<NodeTrack>,
}

/// The `AnimationModel` struct preserves a serializable pose-oriented clip and its skeleton dependency.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AnimationModel {
	pub name: Option<String>,
	pub skeleton: ReferenceModel<SkeletonModel>,
	pub duration: f32,
	pub tracks: Vec<NodeTrack>,
}

impl Resource for Animation {
	type Model = AnimationModel;

	fn get_class(&self) -> &'static str {
		"Animation"
	}
}

impl Model for AnimationModel {
	fn get_class() -> &'static str {
		"Animation"
	}
}

impl<'de> Solver<'de, Animation> for AnimationModel {
	/// Resolves the target skeleton and rejects clip data that a CPU graph could not evaluate deterministically.
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Animation, SolveErrors> {
		let skeleton = self.skeleton.solve(storage_backend)?;
		validate_animation(self.duration, &self.tracks, skeleton.resource().nodes.len())?;
		Ok(Animation {
			name: self.name,
			skeleton,
			duration: self.duration,
			tracks: self.tracks,
		})
	}
}

impl<'de> Solver<'de, Reference<Animation>> for ReferenceModel<AnimationModel> {
	/// Resolves a stored clip and its skeleton dependency for CPU pose sampling and blending.
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Animation>, SolveErrors> {
		let (stored, reader) = storage_backend.read(self.id()).ok_or(SolveErrors::StorageError)?;
		let model: AnimationModel = crate::from_slice(stored.resource()).map_err(|error| {
			SolveErrors::DeserializationFailed(format!(
				"Animation resource could not be deserialized. The most likely cause is incompatible or corrupted clip data: {error}."
			))
		})?;
		let animation = model.solve(storage_backend)?;
		Ok(Reference::from_model(self, animation, reader))
	}
}

/// Validates clip-wide ordering, target, timing, cardinality, and numeric invariants.
fn validate_animation(duration: f32, tracks: &[NodeTrack], skeleton_nodes: usize) -> Result<(), SolveErrors> {
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
			curve.validate(duration, track_index, "translation")?;
		}
		if let Some(curve) = &track.rotation {
			curve.validate(duration, track_index)?;
		}
		if let Some(curve) = &track.scale {
			curve.validate(duration, track_index, "scale")?;
		}
		previous_node = Some(track.node);
	}

	Ok(())
}

/// Validates a key sequence shared by step, linear, and cubic curve representations.
fn validate_times_and_values<const N: usize>(
	times: &[f32],
	values: &[[f32; N]],
	duration: f32,
	track: usize,
	path: &'static str,
) -> Result<(), SolveErrors> {
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
	if !values.iter().flatten().all(|value| value.is_finite()) {
		return invalid_animation(format!("track {track} {path} contains a non-finite value"));
	}
	Ok(())
}

fn invalid_animation(reason: impl std::fmt::Display) -> Result<(), SolveErrors> {
	Err(SolveErrors::DeserializationFailed(format!(
		"Animation clip is invalid. The most likely cause is malformed imported animation data: {reason}."
	)))
}

#[cfg(test)]
mod tests {
	use super::{Animation, AnimationModel, NodeTrack, QuaternionCurve, Vector3Curve};
	use crate::{
		asset::ResourceId,
		resource::{storage_backend::tests::TestStorageBackend, WriteStorageBackend},
		resources::skeleton::{LocalTransform, SkeletonModel, SkeletonNode},
		ProcessedAsset, ReferenceModel, Solver,
	};

	fn skeleton_reference(storage: &TestStorageBackend, node_count: usize) -> ReferenceModel<SkeletonModel> {
		let skeleton = SkeletonModel {
			nodes: (0..node_count)
				.map(|index| SkeletonNode {
					name: Some(format!("node-{index}")),
					parent: index.checked_sub(1).map(|parent| parent as u32),
					rest_local: LocalTransform::identity(),
				})
				.collect(),
		};
		storage
			.store(ProcessedAsset::new(ResourceId::new("test.skeleton"), skeleton), &[])
			.expect("Test skeleton should store")
			.into()
	}

	fn valid_model(storage: &TestStorageBackend) -> AnimationModel {
		AnimationModel {
			name: Some("walk".into()),
			skeleton: skeleton_reference(storage, 2),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]],
				}),
				rotation: Some(QuaternionCurve::Step {
					times: vec![0.0],
					values: vec![[0.0, 0.0, 0.0, 1.0]],
				}),
				scale: None,
			}],
		}
	}

	#[test]
	fn solving_preserves_pose_tracks_and_resolves_their_skeleton() {
		let storage = TestStorageBackend::new();
		let animation: Animation = valid_model(&storage).solve(&storage).expect("Valid animation should solve");

		assert_eq!(animation.name.as_deref(), Some("walk"));
		assert_eq!(animation.skeleton.resource().nodes.len(), 2);
		assert_eq!(animation.tracks[0].node, 1);
		assert!(matches!(animation.tracks[0].translation, Some(Vector3Curve::Linear { .. })));
	}

	#[test]
	fn solving_rejects_unsorted_duplicate_and_out_of_range_tracks() {
		let storage = TestStorageBackend::new();
		let mut model = valid_model(&storage);
		model.tracks.insert(
			0,
			NodeTrack {
				node: 1,
				translation: None,
				rotation: None,
				scale: Some(Vector3Curve::Step {
					times: vec![0.0],
					values: vec![[1.0; 3]],
				}),
			},
		);
		assert!(model.solve(&storage).is_err());

		let mut model = valid_model(&storage);
		model.tracks[0].node = 2;
		assert!(model.solve(&storage).is_err());
	}

	#[test]
	fn solving_rejects_invalid_curve_cardinality_timing_and_numbers() {
		let storage = TestStorageBackend::new();
		let mut model = valid_model(&storage);
		model.tracks[0].translation = Some(Vector3Curve::CubicSpline {
			times: vec![0.0, 0.0],
			values: vec![[0.0; 3], [1.0; 3]],
			in_tangents: vec![[0.0; 3]],
			out_tangents: vec![[0.0; 3], [f32::NAN; 3]],
		});
		assert!(model.solve(&storage).is_err());

		let mut model = valid_model(&storage);
		model.duration = f32::INFINITY;
		assert!(model.solve(&storage).is_err());
	}

	#[test]
	fn solving_rejects_non_unit_rotation_values_but_accepts_arbitrary_finite_cubic_tangents() {
		let storage = TestStorageBackend::new();
		let mut model = valid_model(&storage);
		model.tracks[0].rotation = Some(QuaternionCurve::Linear {
			times: vec![0.0],
			values: vec![[0.0; 4]],
		});
		assert!(model.solve(&storage).is_err());

		let mut model = valid_model(&storage);
		model.tracks[0].rotation = Some(QuaternionCurve::CubicSpline {
			times: vec![0.0, 1.0],
			values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]],
			in_tangents: vec![[50.0, -20.0, 4.0, 0.5], [-2.0, 3.0, 7.0, 11.0]],
			out_tangents: vec![[-8.0, 9.0, 10.0, 12.0], [4.0, 3.0, 2.0, 1.0]],
		});

		assert!(model.solve(&storage).is_ok());
	}
}
