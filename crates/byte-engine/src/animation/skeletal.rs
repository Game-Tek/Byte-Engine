use math::{mat::MatNew4 as _, Matrix4};
use resource_management::resources::{
	animation::{Animation, QuaternionCurve, Vector3Curve},
	skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
};

use super::math::{nlerp_quaternion, normalize_quaternion};

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
	output: &mut Vec<Matrix4>,
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
/// frame without allocating. Next, send the resulting matrices through
/// [`crate::rendering::UpdatePose`] for the renderable that owns the target skeleton.
pub fn sample_pose(
	animation: &Animation,
	target_skeleton: &Skeleton,
	pose_map: &SkeletonPoseMap,
	time: f32,
	source_locals: &mut Vec<LocalTransform>,
	target_locals: &mut Vec<LocalTransform>,
	output: &mut Vec<Matrix4>,
) {
	sample_local_pose(animation, time, source_locals);

	pose_map
		.write_target_local_pose(source_locals, target_skeleton, target_locals)
		.expect("Animation pose map must receive the source skeleton it was built for");
	write_global_pose(target_skeleton, target_locals, output)
		.expect("A skeleton pose map always writes one local transform per target node");
}

/// Converts a blendable local transform into the matrix convention used by render pose updates.
fn local_matrix(local: LocalTransform) -> Matrix4 {
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
	Matrix4::new(
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
	)
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
			hermite_vector(
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
			let value = hermite_vector(
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

fn hermite_vector<const N: usize>(
	start: [f32; N],
	start_tangent: [f32; N],
	end: [f32; N],
	end_tangent: [f32; N],
	factor: f32,
	span: f32,
) -> [f32; N] {
	let factor_squared = factor * factor;
	let factor_cubed = factor_squared * factor;
	let start_value_weight = 2.0 * factor_cubed - 3.0 * factor_squared + 1.0;
	let start_tangent_weight = factor_cubed - 2.0 * factor_squared + factor;
	let end_value_weight = -2.0 * factor_cubed + 3.0 * factor_squared;
	let end_tangent_weight = factor_cubed - factor_squared;
	std::array::from_fn(|component| {
		start[component] * start_value_weight
			+ start_tangent[component] * span * start_tangent_weight
			+ end[component] * end_value_weight
			+ end_tangent[component] * span * end_tangent_weight
	})
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoseError {
	LocalPoseLength { expected: usize, actual: usize },
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

#[cfg(test)]
mod tests {
	use resource_management::resources::animation::{QuaternionCurve, Vector3Curve};

	use super::{sample_rotation, sample_vector3};

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
}
