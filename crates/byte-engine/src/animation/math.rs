//! Quaternion and curve operations shared by animation utilities.

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

pub(crate) fn multiply_quaternion([ax, ay, az, aw]: [f32; 4], [bx, by, bz, bw]: [f32; 4]) -> [f32; 4] {
	normalize_quaternion([
		aw * bx + ax * bw + ay * bz - az * by,
		aw * by - ax * bz + ay * bw + az * bx,
		aw * bz + ax * by - ay * bx + az * bw,
		aw * bw - ax * bx - ay * by - az * bz,
	])
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
