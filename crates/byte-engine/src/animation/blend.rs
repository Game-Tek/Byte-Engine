//! Allocation-free local-pose and blend-space utilities.

use resource_management::resources::skeleton::LocalTransform;

use super::math::{dot_quaternion, nlerp_quaternion, normalize_quaternion};

/// Blends two local transforms while preserving the shortest quaternion path.
pub fn blend_local_transform(left: LocalTransform, right: LocalTransform, factor: f32) -> LocalTransform {
	let factor = factor.clamp(0.0, 1.0);
	LocalTransform {
		translation: lerp_vector3(left.translation, right.translation, factor),
		rotation: nlerp_quaternion(left.rotation, right.rotation, factor),
		scale: lerp_vector3(left.scale, right.scale, factor),
	}
}

/// Blends two complete local poses into caller-owned storage.
pub fn blend_local_pose(
	left: &[LocalTransform],
	right: &[LocalTransform],
	factor: f32,
	output: &mut [LocalTransform],
) -> Result<(), BlendError> {
	validate_pose_lengths(&[left, right], output.len())?;
	for ((left, right), output) in left.iter().zip(right).zip(output) {
		*output = blend_local_transform(*left, *right, factor);
	}
	Ok(())
}

/// Blends any number of complete local poses with normalized non-negative weights.
///
/// Quaternion components use the first positively weighted pose as their
/// hemisphere reference. This keeps antipodal representations from cancelling.
pub fn blend_local_poses(
	poses: &[&[LocalTransform]],
	weights: &[f32],
	output: &mut [LocalTransform],
) -> Result<(), BlendError> {
	if poses.len() != weights.len() || poses.is_empty() {
		return Err(BlendError::InputCount {
			poses: poses.len(),
			weights: weights.len(),
		});
	}
	validate_pose_lengths(poses, output.len())?;
	if weights.iter().any(|weight| !weight.is_finite() || *weight < 0.0) {
		return Err(BlendError::InvalidWeight);
	}
	let weight_sum = weights.iter().sum::<f32>();
	if weight_sum <= f32::EPSILON {
		return Err(BlendError::ZeroWeight);
	}

	for node in 0..output.len() {
		let reference_rotation = poses
			.iter()
			.zip(weights)
			.find(|(_, weight)| **weight > 0.0)
			.map(|(pose, _)| pose[node].rotation)
			.expect("a positive total weight guarantees one reference rotation");
		let mut translation = [0.0; 3];
		let mut rotation = [0.0; 4];
		let mut scale = [0.0; 3];
		for (pose, weight) in poses.iter().zip(weights) {
			let normalized_weight = *weight / weight_sum;
			let transform = pose[node];
			for component in 0..3 {
				translation[component] += transform.translation[component] * normalized_weight;
				scale[component] += transform.scale[component] * normalized_weight;
			}
			let sign = if dot_quaternion(reference_rotation, transform.rotation) < 0.0 {
				-1.0
			} else {
				1.0
			};
			for (sum, component) in rotation.iter_mut().zip(transform.rotation) {
				*sum += component * sign * normalized_weight;
			}
		}
		output[node] = LocalTransform {
			translation,
			rotation: normalize_quaternion(rotation),
			scale,
		};
	}
	Ok(())
}

/// The `BlendSpace1D` struct stores validated scalar sample positions for allocation-free weight evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace1D {
	positions: Vec<f32>,
}

impl BlendSpace1D {
	/// Creates a blend space with finite, strictly ascending sample positions.
	///
	/// Next, call [`Self::write_weights`] during evaluation and pass the weights
	/// to [`blend_local_poses`].
	pub fn new(positions: impl Into<Vec<f32>>) -> Result<Self, BlendSpaceError> {
		let positions = positions.into();
		if positions.is_empty() {
			return Err(BlendSpaceError::NotEnoughSamples { minimum: 1 });
		}
		if positions.iter().any(|position| !position.is_finite()) {
			return Err(BlendSpaceError::NonFiniteSample);
		}
		if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
			return Err(BlendSpaceError::SamplesNotStrictlyAscending);
		}
		Ok(Self { positions })
	}

	/// Returns the number of samples and weights required by [`Self::write_weights`].
	pub fn len(&self) -> usize {
		self.positions.len()
	}

	/// Returns whether the blend space contains no samples.
	pub fn is_empty(&self) -> bool {
		self.positions.is_empty()
	}

	/// Writes normalized weights for a scalar parameter, clamping outside the sample range.
	pub fn write_weights(&self, value: f32, output: &mut [f32]) -> Result<(), BlendSpaceError> {
		if output.len() != self.positions.len() {
			return Err(BlendSpaceError::OutputLength {
				expected: self.positions.len(),
				actual: output.len(),
			});
		}
		if !value.is_finite() {
			return Err(BlendSpaceError::NonFiniteParameter);
		}
		output.fill(0.0);
		let upper = self.positions.partition_point(|position| *position <= value);
		match upper {
			0 => output[0] = 1.0,
			upper if upper == self.positions.len() => output[upper - 1] = 1.0,
			upper => {
				let lower = upper - 1;
				let factor = (value - self.positions[lower]) / (self.positions[upper] - self.positions[lower]);
				output[lower] = 1.0 - factor;
				output[upper] = factor;
			}
		}
		Ok(())
	}
}

/// The `BlendTriangle` struct identifies three sample indices forming one non-degenerate 2D blend region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlendTriangle(
	/// Sample indices ordered around the triangle.
	pub [usize; 3],
);

/// The `BlendSpace2D` struct stores validated points and triangles for allocation-free directional blending.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace2D {
	positions: Vec<[f32; 2]>,
	triangles: Vec<BlendTriangle>,
}

impl BlendSpace2D {
	/// Creates a triangulated blend space.
	///
	/// Triangles may share edges but must not be degenerate. Next, call
	/// [`Self::write_weights`] to find the containing triangle or the closest
	/// point on the triangulated boundary.
	pub fn new(positions: impl Into<Vec<[f32; 2]>>, triangles: impl Into<Vec<BlendTriangle>>) -> Result<Self, BlendSpaceError> {
		let positions = positions.into();
		let triangles = triangles.into();
		if positions.len() < 3 {
			return Err(BlendSpaceError::NotEnoughSamples { minimum: 3 });
		}
		if positions.iter().flatten().any(|component| !component.is_finite()) {
			return Err(BlendSpaceError::NonFiniteSample);
		}
		if triangles.is_empty() {
			return Err(BlendSpaceError::NoTriangles);
		}
		for (triangle_index, BlendTriangle(indices)) in triangles.iter().enumerate() {
			if indices.iter().any(|index| *index >= positions.len()) {
				return Err(BlendSpaceError::TriangleSampleOutOfRange {
					triangle: triangle_index,
				});
			}
			if barycentric_coordinates(
				positions[indices[0]],
				positions[indices[1]],
				positions[indices[2]],
				positions[indices[0]],
			)
			.is_none()
			{
				return Err(BlendSpaceError::DegenerateTriangle {
					triangle: triangle_index,
				});
			}
		}
		Ok(Self { positions, triangles })
	}

	/// Returns the number of samples and weights required by [`Self::write_weights`].
	pub fn len(&self) -> usize {
		self.positions.len()
	}

	/// Returns whether the blend space contains no samples.
	pub fn is_empty(&self) -> bool {
		self.positions.is_empty()
	}

	/// Writes normalized weights for a 2D parameter without allocating.
	///
	/// Values outside the triangulated region clamp to the closest point on any
	/// triangle, which provides stable behavior at directional blend-space edges.
	pub fn write_weights(&self, value: [f32; 2], output: &mut [f32]) -> Result<(), BlendSpaceError> {
		if output.len() != self.positions.len() {
			return Err(BlendSpaceError::OutputLength {
				expected: self.positions.len(),
				actual: output.len(),
			});
		}
		if value.iter().any(|component| !component.is_finite()) {
			return Err(BlendSpaceError::NonFiniteParameter);
		}
		output.fill(0.0);

		let mut closest = None;
		for BlendTriangle(indices) in &self.triangles {
			let points = [
				self.positions[indices[0]],
				self.positions[indices[1]],
				self.positions[indices[2]],
			];
			let weights = barycentric_coordinates(points[0], points[1], points[2], value)
				.expect("blend-space construction rejects degenerate triangles");
			if weights.iter().all(|weight| *weight >= -1.0e-5) {
				write_triangle_weights(*indices, weights, output);
				return Ok(());
			}

			let (clamped, distance_squared) = closest_triangle_weights(points, value);
			if closest.is_none_or(|(_, closest_distance)| distance_squared < closest_distance) {
				closest = Some(((*indices, clamped), distance_squared));
			}
		}

		let ((indices, weights), _) = closest.expect("blend-space construction requires at least one triangle");
		write_triangle_weights(indices, weights, output);
		Ok(())
	}
}

/// Errors returned when local poses cannot be blended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendError {
	/// The pose and weight counts differ, or no poses were supplied.
	InputCount {
		/// Number of supplied poses.
		poses: usize,
		/// Number of supplied weights.
		weights: usize,
	},
	/// An input pose does not match the output node count.
	PoseLength {
		/// Required node count.
		expected: usize,
		/// Supplied node count.
		actual: usize,
	},
	/// A weight is negative or non-finite.
	InvalidWeight,
	/// All supplied weights are zero.
	ZeroWeight,
}

impl std::fmt::Display for BlendError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::InputCount { poses, weights } => write!(
				formatter,
				"Blend inputs do not match. The most likely cause is providing {poses} poses and {weights} weights, or no poses."
			),
			Self::PoseLength { expected, actual } => write!(
				formatter,
				"Blend pose has the wrong node count. The most likely cause is blending skeletons with {expected} and {actual} nodes."
			),
			Self::InvalidWeight => write!(
				formatter,
				"Blend weight is invalid. The most likely cause is a negative or non-finite weight."
			),
			Self::ZeroWeight => write!(
				formatter,
				"Blend has no effective input. The most likely cause is that every supplied weight is zero."
			),
		}
	}
}

impl std::error::Error for BlendError {}

/// Errors returned when constructing or evaluating a blend space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendSpaceError {
	/// The blend space has fewer samples than its topology requires.
	NotEnoughSamples {
		/// Minimum supported sample count.
		minimum: usize,
	},
	/// A sample position contains a non-finite component.
	NonFiniteSample,
	/// One-dimensional sample positions are unsorted or duplicated.
	SamplesNotStrictlyAscending,
	/// A two-dimensional blend space has no triangles.
	NoTriangles,
	/// A triangle references a sample outside the position list.
	TriangleSampleOutOfRange {
		/// Index of the invalid triangle.
		triangle: usize,
	},
	/// A triangle uses repeated or collinear sample positions.
	DegenerateTriangle {
		/// Index of the degenerate triangle.
		triangle: usize,
	},
	/// The output slice cannot hold one weight per sample.
	OutputLength {
		/// Required output length.
		expected: usize,
		/// Supplied output length.
		actual: usize,
	},
	/// The evaluation parameter contains a non-finite component.
	NonFiniteParameter,
}

impl std::fmt::Display for BlendSpaceError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotEnoughSamples { minimum } => write!(
				formatter,
				"Blend space has too few samples. The most likely cause is providing fewer than {minimum} sample positions."
			),
			Self::NonFiniteSample => write!(
				formatter,
				"Blend-space sample is invalid. The most likely cause is a position containing infinity or not-a-number."
			),
			Self::SamplesNotStrictlyAscending => write!(
				formatter,
				"One-dimensional blend samples are not strictly ascending. The most likely cause is an unsorted or duplicate position."
			),
			Self::NoTriangles => write!(
				formatter,
				"Two-dimensional blend space has no triangles. The most likely cause is omitting its sample topology."
			),
			Self::TriangleSampleOutOfRange { triangle } => write!(
				formatter,
				"Blend triangle references a missing sample. The most likely cause is an invalid index in triangle {triangle}."
			),
			Self::DegenerateTriangle { triangle } => write!(
				formatter,
				"Blend triangle has no area. The most likely cause is repeated or collinear points in triangle {triangle}."
			),
			Self::OutputLength { expected, actual } => write!(
				formatter,
				"Blend-weight output has the wrong length. The most likely cause is reserving {actual} weights for {expected} samples."
			),
			Self::NonFiniteParameter => write!(
				formatter,
				"Blend parameter is invalid. The most likely cause is an input containing infinity or not-a-number."
			),
		}
	}
}

impl std::error::Error for BlendSpaceError {}

fn validate_pose_lengths(poses: &[&[LocalTransform]], output_len: usize) -> Result<(), BlendError> {
	for pose in poses {
		if pose.len() != output_len {
			return Err(BlendError::PoseLength {
				expected: output_len,
				actual: pose.len(),
			});
		}
	}
	Ok(())
}

fn lerp_vector3(left: [f32; 3], right: [f32; 3], factor: f32) -> [f32; 3] {
	std::array::from_fn(|component| left[component] + (right[component] - left[component]) * factor)
}

fn barycentric_coordinates(a: [f32; 2], b: [f32; 2], c: [f32; 2], point: [f32; 2]) -> Option<[f32; 3]> {
	let ab = subtract2(b, a);
	let ac = subtract2(c, a);
	let ap = subtract2(point, a);
	let denominator = cross2(ab, ac);
	if denominator.abs() <= f32::EPSILON {
		return None;
	}
	let second = cross2(ap, ac) / denominator;
	let third = cross2(ab, ap) / denominator;
	Some([1.0 - second - third, second, third])
}

/// Returns barycentric weights for the closest point on a triangle.
fn closest_triangle_weights(points: [[f32; 2]; 3], value: [f32; 2]) -> ([f32; 3], f32) {
	let mut closest_weights = [1.0, 0.0, 0.0];
	let mut closest_distance = distance_squared2(points[0], value);
	for (start, end, start_weights, end_weights) in [
		(points[0], points[1], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
		(points[1], points[2], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
		(points[2], points[0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
	] {
		let edge = subtract2(end, start);
		let edge_length_squared = dot2(edge, edge);
		let factor = (dot2(subtract2(value, start), edge) / edge_length_squared).clamp(0.0, 1.0);
		let closest_point = [start[0] + edge[0] * factor, start[1] + edge[1] * factor];
		let distance = distance_squared2(closest_point, value);
		if distance < closest_distance {
			closest_distance = distance;
			closest_weights =
				std::array::from_fn(|component| start_weights[component] * (1.0 - factor) + end_weights[component] * factor);
		}
	}
	(closest_weights, closest_distance)
}

fn write_triangle_weights(indices: [usize; 3], weights: [f32; 3], output: &mut [f32]) {
	let clamped = weights.map(|weight| weight.clamp(0.0, 1.0));
	let sum = clamped.iter().sum::<f32>();
	for (index, weight) in indices.into_iter().zip(clamped) {
		output[index] = weight / sum;
	}
}

fn subtract2(left: [f32; 2], right: [f32; 2]) -> [f32; 2] {
	[left[0] - right[0], left[1] - right[1]]
}

fn dot2(left: [f32; 2], right: [f32; 2]) -> f32 {
	left[0] * right[0] + left[1] * right[1]
}

fn cross2(left: [f32; 2], right: [f32; 2]) -> f32 {
	left[0] * right[1] - left[1] * right[0]
}

fn distance_squared2(left: [f32; 2], right: [f32; 2]) -> f32 {
	let difference = subtract2(left, right);
	dot2(difference, difference)
}

#[cfg(test)]
mod tests {
	use resource_management::resources::skeleton::LocalTransform;

	use super::{BlendSpace1D, BlendSpace2D, BlendSpaceError, BlendTriangle, blend_local_pose, blend_local_poses};

	fn transform(translation: f32, rotation: [f32; 4]) -> LocalTransform {
		LocalTransform {
			translation: [translation, 0.0, 0.0],
			rotation,
			scale: [1.0 + translation, 1.0, 1.0],
		}
	}

	#[test]
	fn two_pose_blend_uses_shortest_rotation_path() {
		let left = [transform(0.0, [0.0, 0.0, 0.0, 1.0])];
		let right = [transform(2.0, [0.0, 0.0, 0.0, -1.0])];
		let mut output = [LocalTransform::identity()];
		blend_local_pose(&left, &right, 0.5, &mut output).expect("expected test value");

		assert_eq!(output[0].translation, [1.0, 0.0, 0.0]);
		assert_eq!(output[0].rotation, [0.0, 0.0, 0.0, 1.0]);
		assert_eq!(output[0].scale, [2.0, 1.0, 1.0]);
	}

	#[test]
	fn weighted_pose_blend_normalizes_weights() {
		let first = [transform(0.0, [0.0, 0.0, 0.0, 1.0])];
		let second = [transform(4.0, [0.0, 0.0, 0.0, 1.0])];
		let mut output = [LocalTransform::identity()];
		blend_local_poses(&[&first, &second], &[3.0, 1.0], &mut output).expect("expected test value");

		assert_eq!(output[0].translation, [1.0, 0.0, 0.0]);
	}

	#[test]
	fn one_dimensional_weights_interpolate_and_clamp() {
		let space = BlendSpace1D::new(vec![0.0, 2.0, 6.0]).expect("expected test value");
		let mut weights = [0.0; 3];
		space.write_weights(4.0, &mut weights).expect("expected test value");

		assert_eq!(weights, [0.0, 0.5, 0.5]);
		space.write_weights(-1.0, &mut weights).expect("expected test value");

		assert_eq!(weights, [1.0, 0.0, 0.0]);
		space.write_weights(8.0, &mut weights).expect("expected test value");

		assert_eq!(weights, [0.0, 0.0, 1.0]);
	}

	#[test]
	fn one_dimensional_samples_must_be_strictly_ascending() {
		assert_eq!(
			BlendSpace1D::new(vec![0.0, 1.0, 1.0]),
			Err(BlendSpaceError::SamplesNotStrictlyAscending)
		);
	}

	#[test]
	fn two_dimensional_weights_use_triangle_barycentrics() {
		let space = BlendSpace2D::new(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![BlendTriangle([0, 1, 2])])
			.expect("expected test value");
		let mut weights = [0.0; 3];
		space.write_weights([0.25, 0.25], &mut weights).expect("expected test value");

		assert_eq!(weights, [0.5, 0.25, 0.25]);
	}

	#[test]
	fn two_dimensional_weights_clamp_outside_to_closest_edge() {
		let space = BlendSpace2D::new(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![BlendTriangle([0, 1, 2])])
			.expect("expected test value");
		let mut weights = [0.0; 3];
		space.write_weights([0.75, 0.75], &mut weights).expect("expected test value");

		assert!((weights[0] - 0.0).abs() < 1.0e-6);
		assert!((weights[1] - 0.5).abs() < 1.0e-6);
		assert!((weights[2] - 0.5).abs() < 1.0e-6);
	}
}
