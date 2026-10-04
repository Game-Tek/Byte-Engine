//! Curve sampling shared by resource and packed animation clips.

use math::{Orientation, Scale, Vector};
use resource_management::resources::{ParentSpace, animation::CurveComponents};

/// Evaluates one cubic Hermite span, scaling the tangents by the span duration.
///
/// [`sample_curve`] uses this to interpolate a cubic keyframe pair for both packed and unpacked clips.
fn hermite<const N: usize>(
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
pub(crate) trait CurveValue<const N: usize>: CurveComponents<N> {
	/// Builds a value from interpolated or stored components, which is how rotations stay unit length.
	fn from_components(components: [f32; N]) -> Self;
	/// Blends two linear keys.
	fn lerp(self, other: Self, factor: f32) -> Self;
}

impl CurveValue<3> for Vector<ParentSpace> {
	fn from_components(components: [f32; 3]) -> Self {
		Self::from_array(components)
	}

	fn lerp(self, other: Self, factor: f32) -> Self {
		Vector::lerp(self, other, factor)
	}
}

impl CurveValue<3> for Scale {
	fn from_components(components: [f32; 3]) -> Self {
		Self::from_array(components)
	}

	fn lerp(self, other: Self, factor: f32) -> Self {
		Scale::lerp(self, other, factor)
	}
}

impl CurveValue<4> for Orientation {
	/// A cubic blend can pass through zero length only for degenerate tangents, which fall back to identity.
	fn from_components(components: [f32; 4]) -> Self {
		Self::try_from_array(components).unwrap_or_default()
	}

	fn lerp(self, other: Self, factor: f32) -> Self {
		self.nlerp(other, factor)
	}
}

/// Samples one curve at `time` through key accessors, so resource and packed curves share one interpolation path.
///
/// `time_at` reads the time of a key, `value_at` its value, and `tangents_at` its `(in, out)` tangents, which only
/// cubic curves read. Keys must be sorted by time. Times before the first key or after the last one hold that key.
pub(crate) fn sample_curve<V: CurveValue<N>, const N: usize>(
	interpolation: CurveInterpolation,
	key_count: usize,
	time: f32,
	time_at: impl Fn(usize) -> f32,
	value_at: impl Fn(usize) -> V,
	tangents_at: impl Fn(usize) -> ([f32; N], [f32; N]),
) -> V {
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
			V::from_components(hermite(
				value_at(lower).components(),
				start_tangent,
				value_at(upper).components(),
				end_tangent,
				factor,
				span,
			))
		}
	}
}
