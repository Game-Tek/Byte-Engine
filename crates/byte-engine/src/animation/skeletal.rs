use math::Matrix;
use resource_management::resources::{
	animation::{Animation, QuaternionCurve, Vector3Curve},
	skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
};

use super::math::{hermite, nlerp_quaternion, normalize_quaternion};

/// Samples one clip into a complete source-skeleton local pose.
///
/// Missing curves retain their node's rest-local component. The caller owns
/// the output so retained animation state can reuse its capacity each frame.
/// Next, blend or inertialize the local pose before calling
/// [`write_global_pose`].
pub fn sample_local_pose(animation: &Animation, time: f32, output: &mut Vec<LocalTransform>) {
	let source_skeleton = animation.skeleton.resource();
	output.clear();
	output.extend(source_skeleton.nodes.iter().map(|node| node.rest_local));

	for track in &animation.tracks {
		let Some(local) = output.get_mut(track.node as usize) else {
			continue;
		};
		if let Some(value) = track.translation.as_ref().map(|curve| sample_vector3(curve, time)) {
			local.translation = value;
		}
		if let Some(value) = track.rotation.as_ref().map(|curve| sample_rotation(curve, time)) {
			local.rotation = value;
		}
		if let Some(value) = track.scale.as_ref().map(|curve| sample_vector3(curve, time)) {
			local.scale = value;
		}
	}
}

/// Builds global skeleton matrices from a complete local pose.
///
/// Skeleton validation guarantees that every parent matrix precedes its
/// children. The output retains capacity across calls.
pub fn write_global_pose(
	skeleton: &Skeleton,
	local_pose: &[LocalTransform],
	output: &mut Vec<Matrix>,
) -> Result<(), PoseError> {
	if local_pose.len() != skeleton.nodes.len() {
		return Err(PoseError::LocalPoseLength {
			expected: skeleton.nodes.len(),
			actual: local_pose.len(),
		});
	}
	output.clear();
	for (index, node) in skeleton.nodes.iter().enumerate() {
		let local = local_matrix(local_pose[index]);
		output.push(match node.parent {
			Some(parent) => output[parent as usize] * local,
			None => local,
		});
	}
	Ok(())
}

/// Samples an animation clip into global matrices for a compatible target skeleton.
///
/// The caller owns all pose storage so a retained animation player can reuse it each
/// frame without allocating. Next, send the resulting matrices through the renderer's
/// `UpdatePose` message for the renderable that owns the target skeleton.
pub fn sample_pose(
	animation: &Animation,
	target_skeleton: &Skeleton,
	pose_map: &SkeletonPoseMap,
	time: f32,
	source_locals: &mut Vec<LocalTransform>,
	target_locals: &mut Vec<LocalTransform>,
	output: &mut Vec<Matrix>,
) {
	sample_local_pose(animation, time, source_locals);

	pose_map
		.write_target_local_pose(source_locals, target_skeleton, target_locals)
		.expect("Animation pose map must receive the source skeleton it was built for");
	write_global_pose(target_skeleton, target_locals, output)
		.expect("A skeleton pose map always writes one local transform per target node");
}

/// The `BonePositionDifference` struct reports one bone's global-position difference between two sampled poses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePositionDifference {
	/// Identifies the bone in the shared skeleton node order.
	pub node: usize,
	/// Stores the first animation's global bone position.
	pub first: [f32; 3],
	/// Stores the second animation's global bone position.
	pub second: [f32; 3],
	/// Stores the Euclidean distance between [`Self::first`] and [`Self::second`].
	pub distance: f32,
}

/// The `AnimationBonePositionComparison` struct reports global bone-position differences for two sampled animations.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationBonePositionComparison {
	/// Stores one position comparison for every bone, in skeleton node order.
	pub differences: Vec<BonePositionDifference>,
}

impl AnimationBonePositionComparison {
	/// Returns the bone with the largest global-position difference.
	pub fn largest_difference(&self) -> Option<&BonePositionDifference> {
		self.differences
			.iter()
			.max_by(|first, second| first.distance.total_cmp(&second.distance))
	}
}

/// Compares global bone positions from two animations sampled at independent times.
///
/// The clips must use skeletons with the same node order, names, and parent links.
/// Sampling follows [`sample_local_pose`], so times outside authored key ranges use
/// the sampler's endpoint behavior. Use [`AnimationBonePositionComparison::largest_difference`]
/// to inspect the most discontinuous bone, or [`AnimationBonePositionComparison::differences`]
/// to inspect every bone.
pub fn compare_animation_bone_positions(
	first_animation: &Animation,
	first_time: f32,
	second_animation: &Animation,
	second_time: f32,
) -> Result<AnimationBonePositionComparison, AnimationComparisonError> {
	if !first_time.is_finite() || !second_time.is_finite() {
		return Err(AnimationComparisonError::NonFiniteTime);
	}

	let first_skeleton = first_animation.skeleton.resource();
	let second_skeleton = second_animation.skeleton.resource();
	validate_skeleton_layout(first_skeleton, second_skeleton)?;

	let node_count = first_skeleton.nodes.len();
	let mut first_local_pose = Vec::with_capacity(node_count);
	let mut second_local_pose = Vec::with_capacity(node_count);
	let mut first_global_pose = Vec::with_capacity(node_count);
	let mut second_global_pose = Vec::with_capacity(node_count);

	sample_local_pose(first_animation, first_time, &mut first_local_pose);
	sample_local_pose(second_animation, second_time, &mut second_local_pose);
	write_global_pose(first_skeleton, &first_local_pose, &mut first_global_pose)
		.expect("sampled first animation pose must match its skeleton");
	write_global_pose(second_skeleton, &second_local_pose, &mut second_global_pose)
		.expect("sampled second animation pose must match its skeleton");

	let differences = first_global_pose
		.iter()
		.zip(&second_global_pose)
		.enumerate()
		.map(|(node, (first, second))| {
			let first = matrix_translation(first);
			let second = matrix_translation(second);
			let delta = [first[0] - second[0], first[1] - second[1], first[2] - second[2]];
			BonePositionDifference {
				node,
				first,
				second,
				distance: (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt(),
			}
		})
		.collect();

	Ok(AnimationBonePositionComparison { differences })
}

fn validate_skeleton_layout(first: &Skeleton, second: &Skeleton) -> Result<(), AnimationComparisonError> {
	if first.nodes.len() != second.nodes.len() {
		return Err(AnimationComparisonError::SkeletonNodeCountMismatch {
			first: first.nodes.len(),
			second: second.nodes.len(),
		});
	}

	for (node, (first, second)) in first.nodes.iter().zip(&second.nodes).enumerate() {
		if first.name != second.name || first.parent != second.parent {
			return Err(AnimationComparisonError::SkeletonLayoutMismatch { node });
		}
	}
	Ok(())
}

/// Extracts translation from the row-major matrix slots used by [`local_matrix`].
fn matrix_translation(matrix: &Matrix) -> [f32; 3] {
	[matrix[3], matrix[7], matrix[11]]
}

/// Converts a blendable local transform into the matrix convention used by render pose updates.
fn local_matrix(local: LocalTransform) -> Matrix {
	let [x, y, z, w] = local.rotation;
	let [sx, sy, sz] = local.scale;
	let [tx, ty, tz] = local.translation;
	let columns = [
		[
			(1.0 - 2.0 * (y * y + z * z)) * sx,
			(2.0 * (x * y + z * w)) * sx,
			(2.0 * (x * z - y * w)) * sx,
			0.0,
		],
		[
			(2.0 * (x * y - z * w)) * sy,
			(1.0 - 2.0 * (x * x + z * z)) * sy,
			(2.0 * (y * z + x * w)) * sy,
			0.0,
		],
		[
			(2.0 * (x * z + y * w)) * sz,
			(2.0 * (y * z - x * w)) * sz,
			(1.0 - 2.0 * (x * x + y * y)) * sz,
			0.0,
		],
		[tx, ty, tz, 1.0],
	];
	Matrix::from((
		columns[0][0],
		columns[1][0],
		columns[2][0],
		columns[3][0],
		columns[0][1],
		columns[1][1],
		columns[2][1],
		columns[3][1],
		columns[0][2],
		columns[1][2],
		columns[2][2],
		columns[3][2],
		columns[0][3],
		columns[1][3],
		columns[2][3],
		columns[3][3],
	))
}

/// Samples every validated translation or scale interpolation form.
fn sample_vector3(curve: &Vector3Curve, time: f32) -> [f32; 3] {
	match curve {
		Vector3Curve::Step { times, values } => values[step_key(times, time)],
		Vector3Curve::Linear { times, values } => {
			let (lower, upper, factor, _) = interpolation_segment(times, time);
			std::array::from_fn(|component| {
				values[lower][component] + (values[upper][component] - values[lower][component]) * factor
			})
		}
		Vector3Curve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			let (lower, upper, factor, span) = interpolation_segment(times, time);
			hermite(
				values[lower],
				out_tangents[lower],
				values[upper],
				in_tangents[upper],
				factor,
				span,
			)
		}
	}
}

/// Samples every validated rotation interpolation form and returns a unit quaternion.
fn sample_rotation(curve: &QuaternionCurve, time: f32) -> [f32; 4] {
	match curve {
		QuaternionCurve::Step { times, values } => values[step_key(times, time)],
		QuaternionCurve::Linear { times, values } => {
			let (lower, upper, factor, _) = interpolation_segment(times, time);
			nlerp_quaternion(values[lower], values[upper], factor)
		}
		QuaternionCurve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			let (lower, upper, factor, span) = interpolation_segment(times, time);
			let value = hermite(
				values[lower],
				out_tangents[lower],
				values[upper],
				in_tangents[upper],
				factor,
				span,
			);
			normalize_quaternion(value)
		}
	}
}

fn step_key(times: &[f32], time: f32) -> usize {
	times.partition_point(|key_time| *key_time <= time).saturating_sub(1)
}

fn interpolation_segment(times: &[f32], time: f32) -> (usize, usize, f32, f32) {
	let upper = times
		.partition_point(|key_time| *key_time <= time)
		.min(times.len().saturating_sub(1));
	let lower = upper.saturating_sub(1);
	let span = times[upper] - times[lower];
	let factor = if span > 0.0 { (time - times[lower]) / span } else { 0.0 }.clamp(0.0, 1.0);
	(lower, upper, factor, span)
}

/// The `PoseError` enum identifies local poses that cannot be converted to global matrices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoseError {
	/// The local pose and skeleton contain different numbers of nodes.
	LocalPoseLength {
		/// The number of transforms required by the skeleton.
		expected: usize,
		/// The number of transforms supplied by the local pose.
		actual: usize,
	},
}

impl std::fmt::Display for PoseError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::LocalPoseLength { expected, actual } => write!(
				formatter,
				"Local pose has the wrong node count. The most likely cause is providing {actual} transforms for a skeleton with {expected} nodes."
			),
		}
	}
}

impl std::error::Error for PoseError {}

/// The `AnimationComparisonError` enum identifies invalid inputs to bone-position comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationComparisonError {
	/// A sample time is NaN or infinite.
	NonFiniteTime,
	/// The animations use skeletons with different node counts.
	SkeletonNodeCountMismatch {
		/// The first animation's node count.
		first: usize,
		/// The second animation's node count.
		second: usize,
	},
	/// The skeletons use different names or parent links at one node.
	SkeletonLayoutMismatch {
		/// The index of the first incompatible node.
		node: usize,
	},
}

impl std::fmt::Display for AnimationComparisonError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NonFiniteTime => write!(
				formatter,
				"Animation comparison time is invalid. The most likely cause is passing a non-finite sample time."
			),
			Self::SkeletonNodeCountMismatch { first, second } => write!(
				formatter,
				"Animation comparison skeletons have different node counts. The most likely cause is comparing clips from different rigs: {first} versus {second}."
			),
			Self::SkeletonLayoutMismatch { node } => write!(
				formatter,
				"Animation comparison skeleton layouts differ. The most likely cause is that node {node} has a different name or parent link."
			),
		}
	}
}

impl std::error::Error for AnimationComparisonError {}

#[cfg(test)]
mod tests {
	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode},
		},
	};

	use super::{AnimationComparisonError, compare_animation_bone_positions, sample_rotation, sample_vector3};

	fn comparison_skeleton(child_name: &str) -> Skeleton {
		Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("root".to_string()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some(child_name.to_string()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
		}
	}

	fn comparison_animation(skeleton: Skeleton, child_end_x: f32) -> Animation {
		Animation {
			name: None,
			skeleton: Reference::in_memory("comparison-skeleton", skeleton),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0; 3], [child_end_x, 0.0, 0.0]],
				}),
				rotation: None,
				scale: None,
			}],
		}
	}

	#[test]
	fn linear_vector_sampling_clamps_and_interpolates() {
		let curve = Vector3Curve::Linear {
			times: vec![1.0, 3.0],
			values: vec![[2.0, 4.0, 6.0], [6.0, 8.0, 10.0]],
		};

		assert_eq!(sample_vector3(&curve, 0.0), [2.0, 4.0, 6.0]);
		assert_eq!(sample_vector3(&curve, 2.0), [4.0, 6.0, 8.0]);
		assert_eq!(sample_vector3(&curve, 4.0), [6.0, 8.0, 10.0]);
	}

	#[test]
	fn rotation_sampling_uses_the_shortest_quaternion_path() {
		let curve = QuaternionCurve::Linear {
			times: vec![0.0, 1.0],
			values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]],
		};

		assert_eq!(sample_rotation(&curve, 0.5), [0.0, 0.0, 0.0, 1.0]);
	}

	#[test]
	fn cubic_vector_sampling_applies_time_scaled_tangents() {
		let curve = Vector3Curve::CubicSpline {
			times: vec![0.0, 2.0],
			values: vec![[0.0; 3], [2.0, 0.0, 0.0]],
			in_tangents: vec![[0.0; 3], [1.0, 0.0, 0.0]],
			out_tangents: vec![[1.0, 0.0, 0.0], [0.0; 3]],
		};

		assert_eq!(sample_vector3(&curve, 1.0), [1.0, 0.0, 0.0]);
	}

	#[test]
	fn step_sampling_clamps_before_the_first_key() {
		let curve = Vector3Curve::Step {
			times: vec![1.0, 2.0],
			values: vec![[3.0; 3], [4.0; 3]],
		};

		assert_eq!(sample_vector3(&curve, 0.0), [3.0; 3]);
	}

	#[test]
	fn compares_global_bone_positions_at_independent_times() {
		let first = comparison_animation(comparison_skeleton("child"), 2.0);
		let second = comparison_animation(comparison_skeleton("child"), 0.0);

		let comparison = compare_animation_bone_positions(&first, 0.5, &second, 0.0).expect("comparison should succeed");
		let child = comparison.largest_difference().expect("the child should differ");

		assert_eq!(child.node, 1);
		assert_eq!(child.first, [1.0, 0.0, 0.0]);
		assert_eq!(child.second, [0.0, 0.0, 0.0]);
		assert_eq!(child.distance, 1.0);
	}

	#[test]
	fn rejects_incompatible_skeleton_layouts_and_non_finite_times() {
		let first = comparison_animation(comparison_skeleton("child"), 1.0);
		let second = comparison_animation(comparison_skeleton("other"), 1.0);

		assert_eq!(
			compare_animation_bone_positions(&first, 0.0, &second, 0.0),
			Err(AnimationComparisonError::SkeletonLayoutMismatch { node: 1 })
		);
		assert_eq!(
			compare_animation_bone_positions(&first, f32::NAN, &first, 0.0),
			Err(AnimationComparisonError::NonFiniteTime)
		);
	}
}
