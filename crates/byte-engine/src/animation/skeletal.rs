use math::AffineMatrix;
use resource_management::resources::{
	animation::{Animation, Curve, CurveComponents},
	skeleton::{LocalTransform, Skeleton},
};

use super::math::{CurveInterpolation, CurveValue, KeySpan, interpolate};

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
		if let Some(curve) = &track.translation {
			local.translation = sample_resource_curve(curve, time);
		}
		if let Some(curve) = &track.rotation {
			local.rotation = sample_resource_curve(curve, time);
		}
		if let Some(curve) = &track.scale {
			local.scale = sample_resource_curve(curve, time);
		}
	}
}

/// Builds global skeleton transforms from a complete local pose, one transform for each of `skeleton`'s nodes.
///
/// Skeleton validation guarantees that every parent transform precedes its
/// children. The transforms are affine, as rendering stores skinned poses, so a
/// pose update carries them as they are. The output retains capacity across calls.
///
/// # Panics
///
/// Panics when `local_pose` has another node count than `skeleton`.
pub fn write_global_pose(skeleton: &Skeleton, local_pose: &[LocalTransform], output: &mut Vec<AffineMatrix>) {
	assert_eq!(
		local_pose.len(),
		skeleton.nodes.len(),
		"Local pose has the wrong node count. The most likely cause is a pose sampled for another skeleton. Sample the pose with this skeleton's clips."
	);
	output.clear();
	for (index, node) in skeleton.nodes.iter().enumerate() {
		let local = local_pose[index];
		let local = AffineMatrix::from_scale_rotation_translation(local.scale, local.rotation, local.translation);
		output.push(match node.parent {
			Some(parent) => output[parent as usize] * local,
			None => local,
		});
	}
}

/// Samples one validated resource curve at `time`.
fn sample_resource_curve<V: CurveValue<N>, T: CurveComponents<N>, const N: usize>(curve: &Curve<V, T>, time: f32) -> V {
	let (interpolation, tangents) = match curve {
		Curve::Step { .. } => (CurveInterpolation::Step, None),
		Curve::Linear { .. } => (CurveInterpolation::Linear, None),
		Curve::CubicSpline {
			in_tangents,
			out_tangents,
			..
		} => (CurveInterpolation::CubicSpline, Some((in_tangents, out_tangents))),
	};
	let (times, values) = (curve.times(), curve.values());
	interpolate(
		interpolation,
		KeySpan::find(times, time, |key| *key),
		|key| values[key],
		|key| {
			let (in_tangents, out_tangents) = tangents.expect("Only cubic curves read tangents.");
			(in_tangents[key].components(), out_tangents[key].components())
		},
	)
}

#[cfg(test)]
mod tests {
	use resource_management::resources::animation::{RotationCurve, TranslationCurve};

	use super::sample_resource_curve;

	#[test]
	fn linear_vector_sampling_clamps_and_interpolates() {
		let curve = TranslationCurve::Linear {
			times: vec![1.0, 3.0],
			values: vec![math::Vector::new(2.0, 4.0, 6.0), math::Vector::new(6.0, 8.0, 10.0)],
		};

		assert_eq!(sample_resource_curve(&curve, 0.0), math::Vector::new(2.0, 4.0, 6.0));
		assert_eq!(sample_resource_curve(&curve, 2.0), math::Vector::new(4.0, 6.0, 8.0));
		assert_eq!(sample_resource_curve(&curve, 4.0), math::Vector::new(6.0, 8.0, 10.0));
	}

	#[test]
	fn rotation_sampling_uses_the_shortest_quaternion_path() {
		let curve = RotationCurve::Linear {
			times: vec![0.0, 1.0],
			values: vec![
				math::Orientation::identity(),
				math::Orientation::try_from_array([0.0, 0.0, 0.0, -1.0]).unwrap(),
			],
		};

		assert_eq!(sample_resource_curve(&curve, 0.5), math::Orientation::identity());
	}

	#[test]
	fn cubic_vector_sampling_applies_time_scaled_tangents() {
		let curve = TranslationCurve::CubicSpline {
			times: vec![0.0, 2.0],
			values: vec![math::Vector::zero(), math::Vector::new(2.0, 0.0, 0.0)],
			in_tangents: vec![math::Vector::zero(), math::Vector::new(1.0, 0.0, 0.0)],
			out_tangents: vec![math::Vector::new(1.0, 0.0, 0.0), math::Vector::zero()],
		};

		assert_eq!(sample_resource_curve(&curve, 1.0), math::Vector::new(1.0, 0.0, 0.0));
	}
}
