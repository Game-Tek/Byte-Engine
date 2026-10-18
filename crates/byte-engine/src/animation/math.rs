//! Curve sampling shared by resource and packed animation clips.

use math::{Orientation, Scale, Vector};
use resource_management::resources::{ParentSpace, animation::CurveComponents};

/// Evaluates one cubic Hermite span, scaling the tangents by the span duration.
///
/// [`interpolate`] uses this to interpolate a cubic keyframe pair for both packed and unpacked clips.
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

/// The `CurveValue` trait lets [`interpolate`] blend translations and scales component by component and
/// rotations as unit quaternions.
pub(crate) trait CurveValue<const N: usize>: CurveComponents<N> {
	/// Builds a value from interpolated or stored components, which is how rotations stay unit length.
	fn from_components(components: [f32; N]) -> Self;
	/// Rebuilds a stored key whose components already passed through [`Self::from_components`].
	fn from_key(components: [f32; N]) -> Self {
		Self::from_components(components)
	}
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

	/// Packed keys are stored normalized, so reading one back skips a second normalization.
	fn from_key(components: [f32; 4]) -> Self {
		Self::from_unit_array_unchecked(components)
	}

	fn lerp(self, other: Self, factor: f32) -> Self {
		self.nlerp(other, factor)
	}
}

/// The `KeySpan` struct places a time among a curve's keys: the key a step curve holds and the pair a linear or cubic
/// curve interpolates between.
///
/// [`KeySpan::find`] defines the search once, so resource curves and packed timelines place a time the same way. The
/// keys are indices into the curve's keys, or wherever the caller's key data starts, like a packed timeline's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct KeySpan {
	/// The last key at or before the time, or the first key before every key.
	pub(crate) step: usize,
	pub(crate) lower: usize,
	pub(crate) upper: usize,
	/// How far the time lies from `lower` toward `upper`, in `[0, 1]`.
	pub(crate) factor: f32,
	/// The seconds between `lower` and `upper`, which scale cubic tangents.
	pub(crate) span: f32,
}

impl KeySpan {
	/// Finds where `time` lies among `keys`, whose times `seconds` reads. Keys must ascend, and a curve has at least
	/// one.
	///
	/// Times before the first key or after the last one hold that key.
	pub(crate) fn find<K>(keys: &[K], time: f32, seconds: impl Fn(&K) -> f32) -> Self {
		let first_after = keys.partition_point(|key| seconds(key) <= time);
		let upper = first_after.min(keys.len().saturating_sub(1));
		let lower = upper.saturating_sub(1);
		let (lower_time, upper_time) = (seconds(&keys[lower]), seconds(&keys[upper]));
		let span = upper_time - lower_time;
		let factor = if span > 0.0 { (time - lower_time) / span } else { 0.0 }.clamp(0.0, 1.0);
		Self {
			step: first_after.saturating_sub(1),
			lower,
			upper,
			factor,
			span,
		}
	}
}

/// Interpolates between the keys a [`KeySpan`] selected, so resource and packed curves share one interpolation
/// path.
///
/// `value_at` reads a key's value and `tangents_at` its `(in, out)` tangents, which only cubic curves read.
pub(crate) fn interpolate<V: CurveValue<N>, const N: usize>(
	interpolation: CurveInterpolation,
	keys: KeySpan,
	value_at: impl Fn(usize) -> V,
	tangents_at: impl Fn(usize) -> ([f32; N], [f32; N]),
) -> V {
	match interpolation {
		CurveInterpolation::Step => value_at(keys.step),
		CurveInterpolation::Linear => value_at(keys.lower).lerp(value_at(keys.upper), keys.factor),
		CurveInterpolation::CubicSpline => {
			let (_, start_tangent) = tangents_at(keys.lower);
			let (end_tangent, _) = tangents_at(keys.upper);
			V::from_components(hermite(
				value_at(keys.lower).components(),
				start_tangent,
				value_at(keys.upper).components(),
				end_tangent,
				keys.factor,
				keys.span,
			))
		}
	}
}
