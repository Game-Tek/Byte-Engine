//! Allocation-free local-pose and blend-space utilities.

use math::{Orientation, Quaternion, Scale, Vector};
use resource_management::resources::skeleton::LocalTransform;

/// Blends two local transforms while preserving the shortest quaternion path.
pub fn blend_local_transform(left: LocalTransform, right: LocalTransform, factor: f32) -> LocalTransform {
	let factor = factor.clamp(0.0, 1.0);
	LocalTransform {
		translation: left.translation.lerp(right.translation, factor),
		rotation: left.rotation.nlerp(right.rotation, factor),
		scale: left.scale.lerp(right.scale, factor),
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

	let reference = weights
		.iter()
		.position(|weight| *weight > 0.0)
		.expect("a positive total weight guarantees one reference rotation");
	// Each pose's share is the same for every node, so it's divided out once. A blend weighs at most a triangle of
	// clips, so the shares stay on the stack.
	let normalized_weights: smallvec::SmallVec<[f32; 4]> = weights.iter().map(|weight| weight / weight_sum).collect();
	for node in 0..output.len() {
		let reference_rotation = poses[reference][node].rotation;
		let mut translation = Vector::zero();
		let mut rotation = Quaternion::new(0.0, 0.0, 0.0, 0.0);
		let mut scale = Scale::new(0.0, 0.0, 0.0);
		for (pose, &normalized_weight) in poses.iter().zip(&normalized_weights) {
			let transform = pose[node];
			translation += transform.translation * normalized_weight;
			scale = scale + transform.scale * normalized_weight;
			let sign = if reference_rotation.dot(transform.rotation) < 0.0 {
				-1.0
			} else {
				1.0
			};
			rotation += transform.rotation.into_maths() * (sign * normalized_weight);
		}
		output[node] = LocalTransform {
			translation,
			// Weights are positive in the reference hemisphere, so only rounding could leave a zero sum.
			rotation: Orientation::try_from_maths(rotation).unwrap_or_default(),
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
	/// Next, call [`Self::weights`] during evaluation and blend the weighted
	/// samples' poses with [`blend_local_poses`].
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

	/// Returns the number of samples.
	pub fn len(&self) -> usize {
		self.positions.len()
	}

	/// Returns whether the blend space contains no samples.
	pub fn is_empty(&self) -> bool {
		self.positions.is_empty()
	}

	/// Returns the normalized weights for a scalar parameter, clamping outside the sample range.
	///
	/// At most the two samples around the parameter weigh above zero.
	pub fn weights(&self, value: f32) -> Result<BlendWeights, BlendSpaceError> {
		if !value.is_finite() {
			return Err(BlendSpaceError::NonFiniteParameter);
		}
		let upper = self.positions.partition_point(|position| *position <= value);
		Ok(match upper {
			0 => BlendWeights::single(0),
			upper if upper == self.positions.len() => BlendWeights::single(upper - 1),
			upper => {
				let lower = upper - 1;
				let factor = (value - self.positions[lower]) / (self.positions[upper] - self.positions[lower]);
				BlendWeights::from_samples([(lower, 1.0 - factor), (upper, factor)])
			}
		})
	}
}

/// The `BlendSpace2D` struct stores validated points and triangles for allocation-free directional blending.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace2D {
	positions: Vec<[f32; 2]>,
	/// Sample indices ordered around each non-degenerate triangle.
	triangles: Vec<[usize; 3]>,
}

impl BlendSpace2D {
	/// Creates a triangulated blend space.
	///
	/// Each triangle lists three sample indices in order around it. Triangles may
	/// share edges but must not be degenerate. Next, call [`Self::weights`]
	/// to find the containing triangle or the closest point on the triangulated
	/// boundary.
	pub fn new(positions: impl Into<Vec<[f32; 2]>>, triangles: impl Into<Vec<[usize; 3]>>) -> Result<Self, BlendSpaceError> {
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
		for (triangle_index, indices) in triangles.iter().enumerate() {
			if indices.iter().any(|index| *index >= positions.len()) {
				return Err(BlendSpaceError::TriangleSampleOutOfRange {
					triangle: triangle_index,
				});
			}
			let [first, second, third] = indices.map(|index| positions[index]);
			if barycentric_coordinates(first, second, third, first).is_none() {
				return Err(BlendSpaceError::DegenerateTriangle {
					triangle: triangle_index,
				});
			}
		}
		Ok(Self { positions, triangles })
	}

	/// Returns the number of samples.
	pub fn len(&self) -> usize {
		self.positions.len()
	}

	/// Returns whether the blend space contains no samples.
	pub fn is_empty(&self) -> bool {
		self.positions.is_empty()
	}

	/// Returns the normalized weights for a 2D parameter without allocating.
	///
	/// At most the three samples of one triangle weigh above zero. Values outside the triangulated region clamp to
	/// the closest point on any triangle, which provides stable behavior at directional blend-space edges.
	pub fn weights(&self, value: [f32; 2]) -> Result<BlendWeights, BlendSpaceError> {
		if value.iter().any(|component| !component.is_finite()) {
			return Err(BlendSpaceError::NonFiniteParameter);
		}

		let mut closest = None;
		for indices in &self.triangles {
			let points = indices.map(|index| self.positions[index]);
			let weights = barycentric_coordinates(points[0], points[1], points[2], value)
				.expect("blend-space construction rejects degenerate triangles");
			if weights.iter().all(|weight| *weight >= -1.0e-5) {
				return Ok(triangle_weights(*indices, weights));
			}

			let (clamped, distance_squared) = closest_triangle_weights(points, value);
			if closest.is_none_or(|(_, closest_distance)| distance_squared < closest_distance) {
				closest = Some(((*indices, clamped), distance_squared));
			}
		}

		let ((indices, weights), _) = closest.expect("blend-space construction requires at least one triangle");
		Ok(triangle_weights(indices, weights))
	}
}

/// The `BlendWeights` struct holds the samples a blend space weighs above zero for one parameter, at most three, in
/// ascending sample order, so a blend samples only those and mixes them in a stable order.
///
/// Get it from [`BlendSpace1D::weights`] or [`BlendSpace2D::weights`], then sample each weighted clip and mix the
/// poses with [`blend_local_poses`] in the order [`Self::iter`] gives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendWeights {
	samples: [(u32, f32); Self::MAX],
	len: u8,
}

impl BlendWeights {
	/// The most samples a blend weighs at once: the corners of one triangle of a planar blend space.
	pub const MAX: usize = 3;

	/// Returns the weights of a blend that plays one sample alone.
	pub fn single(sample: usize) -> Self {
		Self::from_samples([(sample, 1.0)])
	}

	/// Collects the samples with a weight above zero, in ascending sample order. Samples must not repeat.
	fn from_samples<const N: usize>(samples: [(usize, f32); N]) -> Self {
		debug_assert!(N <= Self::MAX, "a blend weighs at most a triangle of samples");
		let mut weights = Self::default();
		for (sample, weight) in samples {
			if weight > 0.0 {
				weights.samples[usize::from(weights.len)] = (sample as u32, weight);
				weights.len += 1;
			}
		}
		weights.samples[..usize::from(weights.len)].sort_unstable_by_key(|(sample, _)| *sample);
		weights
	}

	/// Returns each weighted sample and its weight, in ascending sample order.
	pub fn iter(&self) -> impl ExactSizeIterator<Item = (usize, f32)> + '_ {
		self.samples[..usize::from(self.len)]
			.iter()
			.map(|&(sample, weight)| (sample as usize, weight))
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

/// Clamps a triangle's barycentric weights to `[0, 1]` and renormalizes them.
fn triangle_weights(indices: [usize; 3], weights: [f32; 3]) -> BlendWeights {
	let clamped = weights.map(|weight| weight.clamp(0.0, 1.0));
	let sum = clamped.iter().sum::<f32>();
	BlendWeights::from_samples(std::array::from_fn::<_, 3, _>(|corner| (indices[corner], clamped[corner] / sum)))
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

	use super::{BlendSpace1D, BlendSpace2D, blend_local_pose, blend_local_poses};

	fn transform(translation: f32, rotation: [f32; 4]) -> LocalTransform {
		LocalTransform {
			translation: math::Vector::new(translation, 0.0, 0.0),
			rotation: math::Orientation::try_from_array(rotation).expect("test rotations are unit quaternions"),
			scale: math::Scale::new(1.0 + translation, 1.0, 1.0),
		}
	}

	#[test]
	fn two_pose_blend_uses_shortest_rotation_path() {
		let left = [transform(0.0, [0.0, 0.0, 0.0, 1.0])];
		let right = [transform(2.0, [0.0, 0.0, 0.0, -1.0])];
		let mut output = [LocalTransform::identity()];
		blend_local_pose(&left, &right, 0.5, &mut output).expect("expected test value");

		assert_eq!(output[0].translation, math::Vector::new(1.0, 0.0, 0.0));
		assert_eq!(output[0].rotation, math::Orientation::identity());
		assert_eq!(output[0].scale, math::Scale::new(2.0, 1.0, 1.0));
	}

	#[test]
	fn weighted_pose_blend_normalizes_weights() {
		let first = [transform(0.0, [0.0, 0.0, 0.0, 1.0])];
		let second = [transform(4.0, [0.0, 0.0, 0.0, 1.0])];
		let mut output = [LocalTransform::identity()];
		blend_local_poses(&[&first, &second], &[3.0, 1.0], &mut output).expect("expected test value");

		assert_eq!(output[0].translation, math::Vector::new(1.0, 0.0, 0.0));
	}

	#[test]
	fn one_dimensional_weights_interpolate_and_clamp() {
		let space = BlendSpace1D::new(vec![0.0, 2.0, 6.0]).expect("expected test value");
		let weights = |value| space.weights(value).expect("expected test value").iter().collect::<Vec<_>>();

		assert_eq!(weights(4.0), [(1, 0.5), (2, 0.5)]);
		assert_eq!(weights(-1.0), [(0, 1.0)]);
		assert_eq!(weights(8.0), [(2, 1.0)]);
	}

	#[test]
	fn two_dimensional_weights_use_triangle_barycentrics() {
		let space = BlendSpace2D::new(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![[0, 1, 2]]).expect("expected test value");
		let weights = space.weights([0.25, 0.25]).expect("expected test value");

		assert_eq!(weights.iter().collect::<Vec<_>>(), [(0, 0.5), (1, 0.25), (2, 0.25)]);
	}

	#[test]
	fn two_dimensional_weights_clamp_outside_to_closest_edge() {
		let space = BlendSpace2D::new(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], vec![[0, 1, 2]]).expect("expected test value");
		let weights = space.weights([0.75, 0.75]).expect("expected test value");
		let weight = |sample| {
			weights
				.iter()
				.find(|(weighted, _)| *weighted == sample)
				.map_or(0.0, |(_, weight)| weight)
		};

		assert!((weight(0) - 0.0).abs() < 1.0e-6);
		assert!((weight(1) - 0.5).abs() < 1.0e-6);
		assert!((weight(2) - 0.5).abs() < 1.0e-6);
	}
}
