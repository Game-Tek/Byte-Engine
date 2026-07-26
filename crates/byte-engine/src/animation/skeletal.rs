use math::{mat::MatNew4 as _, Matrix4};
use resource_management::resources::{
	animation::{Animation, QuaternionCurve, Vector3Curve},
	skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
};

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
	let source_skeleton = animation.skeleton.resource();
	source_locals.clear();
	source_locals.extend(source_skeleton.nodes.iter().map(|node| node.rest_local));

	for track in &animation.tracks {
		let Some(local) = source_locals.get_mut(track.node as usize) else {
			continue;
		};
		if let Some(value) = track.translation.as_ref().and_then(|curve| sample_vector3(curve, time)) {
			local.translation = value;
		}
		if let Some(value) = track.rotation.as_ref().and_then(|curve| sample_rotation(curve, time)) {
			local.rotation = value;
		}
		if let Some(value) = track.scale.as_ref().and_then(|curve| sample_vector3(curve, time)) {
			local.scale = value;
		}
	}

	pose_map
		.write_target_local_pose(source_locals, target_skeleton, target_locals)
		.expect("Animation pose map must receive the source skeleton it was built for");

	output.clear();
	// Skeleton validation guarantees parents precede children, so each parent matrix is ready here.
	for (index, node) in target_skeleton.nodes.iter().enumerate() {
		let local = local_matrix(target_locals[index]);
		output.push(match node.parent {
			Some(parent) => output[parent as usize] * local,
			None => local,
		});
	}
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

/// Samples translation or scale curves, returning no value for curve forms not yet supported by runtime evaluation.
fn sample_vector3(curve: &Vector3Curve, time: f32) -> Option<[f32; 3]> {
	let (times, values) = match curve {
		Vector3Curve::Step { times, values } => {
			return times
				.iter()
				.zip(values)
				.rev()
				.find(|(key_time, _)| **key_time <= time)
				.map(|(_, value)| *value)
		}
		Vector3Curve::Linear { times, values } => (times, values),
		Vector3Curve::CubicSpline { .. } => return None,
	};
	let upper = times
		.partition_point(|key_time| *key_time <= time)
		.min(times.len().saturating_sub(1));
	let lower = upper.saturating_sub(1);
	let span = times[upper] - times[lower];
	let factor = if span > 0.0 { (time - times[lower]) / span } else { 0.0 }.clamp(0.0, 1.0);
	Some(std::array::from_fn(|component| {
		values[lower][component] + (values[upper][component] - values[lower][component]) * factor
	}))
}

/// Samples a rotation curve with shortest-path normalized linear interpolation.
fn sample_rotation(curve: &QuaternionCurve, time: f32) -> Option<[f32; 4]> {
	let QuaternionCurve::Linear { times, values } = curve else {
		return None;
	};
	let upper = times
		.partition_point(|key_time| *key_time <= time)
		.min(times.len().saturating_sub(1));
	let lower = upper.saturating_sub(1);
	let span = times[upper] - times[lower];
	let factor = if span > 0.0 { (time - times[lower]) / span } else { 0.0 }.clamp(0.0, 1.0);
	let sign = if values[lower]
		.iter()
		.zip(values[upper])
		.map(|(left, right)| left * right)
		.sum::<f32>()
		< 0.0
	{
		-1.0
	} else {
		1.0
	};
	let mut value = std::array::from_fn(|component| {
		values[lower][component] + (values[upper][component] * sign - values[lower][component]) * factor
	});
	let length = value.iter().map(|component| component * component).sum::<f32>().sqrt();
	if length > 0.0 {
		for component in &mut value {
			*component /= length;
		}
	}
	Some(value)
}

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
		assert_eq!(sample_vector3(&curve, 0.0), Some([2.0, 4.0, 6.0]));
		assert_eq!(sample_vector3(&curve, 2.0), Some([4.0, 6.0, 8.0]));
		assert_eq!(sample_vector3(&curve, 4.0), Some([6.0, 8.0, 10.0]));
	}

	#[test]
	fn rotation_sampling_uses_the_shortest_quaternion_path() {
		let curve = QuaternionCurve::Linear {
			times: vec![0.0, 1.0],
			values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]],
		};
		assert_eq!(sample_rotation(&curve, 0.5), Some([0.0, 0.0, 0.0, 1.0]));
	}
}
