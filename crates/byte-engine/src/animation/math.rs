//! Vector, quaternion, and curve operations shared by animation utilities and skinning.

const QUATERNION_EPSILON: f32 = 1.0e-8;

pub(crate) fn dot_quaternion(left: [f32; 4], right: [f32; 4]) -> f32 {
	left.iter().zip(right).map(|(left, right)| left * right).sum()
}

pub(crate) fn normalize_quaternion(mut value: [f32; 4]) -> [f32; 4] {
	let length_squared = dot_quaternion(value, value);
	if length_squared <= QUATERNION_EPSILON {
		return [0.0, 0.0, 0.0, 1.0];
	}
	let inverse_length = length_squared.sqrt().recip();
	for component in &mut value {
		*component *= inverse_length;
	}
	value
}

pub(crate) fn conjugate_quaternion([x, y, z, w]: [f32; 4]) -> [f32; 4] {
	[-x, -y, -z, w]
}

/// Multiplies two rotations and normalizes the product.
pub(crate) fn multiply_quaternion(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
	normalize_quaternion(quaternion_product(left, right))
}

/// Multiplies two quaternions without normalizing the product.
///
/// Use it where the product is not a rotation, such as the translation part of a dual quaternion; use
/// [`multiply_quaternion`] for rotations.
pub(crate) fn quaternion_product([ax, ay, az, aw]: [f32; 4], [bx, by, bz, bw]: [f32; 4]) -> [f32; 4] {
	[
		aw * bx + ax * bw + ay * bz - az * by,
		aw * by - ax * bz + ay * bw + az * bx,
		aw * bz + ax * by - ay * bx + az * bw,
		aw * bw - ax * bx - ay * by - az * bz,
	]
}

/// Rotates `vector` by the unit xyzw quaternion.
pub(crate) fn rotate_vector([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
	let quaternion_vector = [x, y, z];
	let twice_cross = cross3(quaternion_vector, vector).map(|component| 2.0 * component);
	let cross_again = cross3(quaternion_vector, twice_cross);
	std::array::from_fn(|component| vector[component] + w * twice_cross[component] + cross_again[component])
}

pub(crate) fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] + right[component])
}

pub(crate) fn sub3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] - right[component])
}

/// Moves `factor` of the way from `left` to `right`.
pub(crate) fn lerp3(left: [f32; 3], right: [f32; 3], factor: f32) -> [f32; 3] {
	std::array::from_fn(|component| left[component] + (right[component] - left[component]) * factor)
}

pub(crate) fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
	left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

pub(crate) fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

pub(crate) fn nlerp_quaternion(left: [f32; 4], mut right: [f32; 4], factor: f32) -> [f32; 4] {
	if dot_quaternion(left, right) < 0.0 {
		for component in &mut right {
			*component = -*component;
		}
	}
	normalize_quaternion(std::array::from_fn(|component| {
		left[component] + (right[component] - left[component]) * factor
	}))
}

/// Converts a unit quaternion to its shortest axis-angle rotation vector.
pub(crate) fn quaternion_log(mut value: [f32; 4]) -> [f32; 3] {
	value = normalize_quaternion(value);
	if value[3] < 0.0 {
		for component in &mut value {
			*component = -*component;
		}
	}
	let vector_length = value[..3].iter().map(|component| component * component).sum::<f32>().sqrt();
	if vector_length <= QUATERNION_EPSILON {
		return [0.0; 3];
	}
	let angle = 2.0 * vector_length.atan2(value[3].clamp(-1.0, 1.0));
	std::array::from_fn(|component| value[component] * angle / vector_length)
}

/// Converts an axis-angle rotation vector to a unit quaternion.
pub(crate) fn quaternion_exp(value: [f32; 3]) -> [f32; 4] {
	let angle = value.iter().map(|component| component * component).sum::<f32>().sqrt();
	if angle <= QUATERNION_EPSILON {
		return normalize_quaternion([value[0] * 0.5, value[1] * 0.5, value[2] * 0.5, 1.0]);
	}
	let half_angle = angle * 0.5;
	let scale = half_angle.sin() / angle;
	[value[0] * scale, value[1] * scale, value[2] * scale, half_angle.cos()]
}

/// Evaluates one cubic Hermite span, scaling the tangents by the span duration.
///
/// Both the packed and the unpacked samplers use this to interpolate a keyframe pair.
pub(crate) fn hermite<const N: usize>(
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

/// The `CurveInterpolation` enum names how a curve moves between keys, for both resource and packed curves.
///
/// The discriminants are the packed encoding, so [`super::packed`] writes them as-is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CurveInterpolation {
	Step = 0,
	Linear = 1,
	CubicSpline = 2,
}

/// The `CurveValue` trait lets [`sample_curve`] blend translations and scales component by component and
/// rotations as unit quaternions.
pub(crate) trait CurveValue: Copy {
	/// Blends two linear keys.
	fn lerp(self, other: Self, factor: f32) -> Self;
	/// Adjusts a cubic Hermite result, which is how rotations stay unit length.
	fn finish_cubic(self) -> Self;
}

impl CurveValue for [f32; 3] {
	fn lerp(self, other: Self, factor: f32) -> Self {
		lerp3(self, other, factor)
	}

	fn finish_cubic(self) -> Self {
		self
	}
}

impl CurveValue for [f32; 4] {
	fn lerp(self, other: Self, factor: f32) -> Self {
		nlerp_quaternion(self, other, factor)
	}

	fn finish_cubic(self) -> Self {
		normalize_quaternion(self)
	}
}

/// Samples one curve at `time` through key accessors, so resource and packed curves share one interpolation path.
///
/// `time_at` reads the time of a key, `value_at` its value, and `tangents_at` its `(in, out)` tangents, which only
/// cubic curves read. Keys must be sorted by time. Times before the first key or after the last one hold that key.
pub(crate) fn sample_curve<const N: usize>(
	interpolation: CurveInterpolation,
	key_count: usize,
	time: f32,
	time_at: impl Fn(usize) -> f32,
	value_at: impl Fn(usize) -> [f32; N],
	tangents_at: impl Fn(usize) -> ([f32; N], [f32; N]),
) -> [f32; N]
where
	[f32; N]: CurveValue,
{
	// Binary search for the first key after `time` through the accessor, so packed words need no typed slice.
	let (mut first_after, mut end) = (0, key_count);
	while first_after < end {
		let middle = first_after + (end - first_after) / 2;
		if time_at(middle) <= time {
			first_after = middle + 1;
		} else {
			end = middle;
		}
	}
	if interpolation == CurveInterpolation::Step {
		return value_at(first_after.saturating_sub(1));
	}

	let upper = first_after.min(key_count.saturating_sub(1));
	let lower = upper.saturating_sub(1);
	let span = time_at(upper) - time_at(lower);
	let factor = if span > 0.0 { (time - time_at(lower)) / span } else { 0.0 }.clamp(0.0, 1.0);
	match interpolation {
		CurveInterpolation::Linear => value_at(lower).lerp(value_at(upper), factor),
		_ => {
			let (_, start_tangent) = tangents_at(lower);
			let (end_tangent, _) = tangents_at(upper);
			hermite(value_at(lower), start_tangent, value_at(upper), end_tangent, factor, span).finish_cubic()
		}
	}
}
