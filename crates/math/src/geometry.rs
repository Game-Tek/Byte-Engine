use std::{
	cmp::Ordering,
	fmt,
	marker::PhantomData,
	ops::{Add, AddAssign, Div, Mul, Neg, Sub},
};

use maths_rs::Vec3f;

/// The `WorldSpace` struct brands positions and directions that use the engine's world coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WorldSpace;

/// The `Unnormalized` struct marks a [`Vector`] whose length has not been validated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Unnormalized;

/// Describes why a vector cannot provide a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizationError {
	/// The vector has no direction because all of its components are zero.
	ZeroLength,
	/// The vector has no valid direction because at least one component is not finite.
	NonFinite,
}

impl fmt::Display for NormalizationError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::ZeroLength => formatter.write_str("Cannot normalize a zero-length vector. The input has no direction."),
			Self::NonFinite => formatter.write_str("Cannot normalize a non-finite vector. The input contains NaN or infinity."),
		}
	}
}

impl std::error::Error for NormalizationError {}

/// The `Point` struct represents a location in one coordinate space without treating it as a direction.
#[repr(transparent)]
pub struct Point<Space = WorldSpace> {
	value: Vec3f,
	space: PhantomData<Space>,
}

impl<Space> Copy for Point<Space> {}

impl<Space> Clone for Point<Space> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<Space> fmt::Debug for Point<Space> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.debug_tuple("Point").field(&self.value).finish()
	}
}

impl<Space> PartialEq for Point<Space> {
	fn eq(&self, other: &Self) -> bool {
		self.value == other.value
	}
}

impl<Space> Point<Space> {
	/// Creates a point from its coordinates in `Space`.
	pub fn new(x: f32, y: f32, z: f32) -> Self {
		Self::from_maths(Vec3f::new(x, y, z))
	}

	/// Creates the coordinate-space origin.
	pub fn origin() -> Self {
		Self::new(0.0, 0.0, 0.0)
	}

	/// Creates a branded point from an explicit `maths-rs` value.
	pub fn from_maths(value: Vec3f) -> Self {
		Self {
			value,
			space: PhantomData,
		}
	}

	/// Returns this point as an explicit `maths-rs` value for boundary integrations.
	pub fn into_maths(self) -> Vec3f {
		self.value
	}

	/// Returns the x coordinate.
	pub fn x(self) -> f32 {
		self.value.x
	}

	/// Returns the y coordinate.
	pub fn y(self) -> f32 {
		self.value.y
	}

	/// Returns the z coordinate.
	pub fn z(self) -> f32 {
		self.value.z
	}

	/// Returns the distance to `other` in this point's coordinate space.
	pub fn distance_to(self, other: Self) -> f32 {
		(self - other).length()
	}
}

impl<Space> Default for Point<Space> {
	fn default() -> Self {
		Self::origin()
	}
}

/// Returns whether all coordinates in `point` are finite.
pub fn is_finite<Space>(point: Point<Space>) -> bool {
	point.x().is_finite() && point.y().is_finite() && point.z().is_finite()
}

/// Returns the distance between two points after projecting them onto XZ.
pub fn distance_xz<Space>(first: Point<Space>, second: Point<Space>) -> f32 {
	let x = second.x() - first.x();
	let z = second.z() - first.z();
	x.hypot(z)
}

/// Returns twice the signed area of the XZ triangle formed by three points.
///
/// A positive result means `third` lies counterclockwise from the directed `first`-to-`second`
/// edge. A negative result means clockwise, and zero means collinear.
pub fn signed_area_xz<Space>(first: Point<Space>, second: Point<Space>, third: Point<Space>) -> f32 {
	(second.x() - first.x()) * (third.z() - first.z()) - (second.z() - first.z()) * (third.x() - first.x())
}

/// Returns barycentric weights for `point` inside the XZ projection of a triangle.
///
/// Returns [`None`] for a degenerate triangle or a point outside the projected triangle. The
/// returned weights correspond to `first`, `second`, and `third` and sum to one within `f32`
/// precision.
pub fn barycentric_xz<Space>(
	point: Point<Space>,
	first: Point<Space>,
	second: Point<Space>,
	third: Point<Space>,
) -> Option<[f32; 3]> {
	let denominator = signed_area_xz(first, second, third);
	if denominator == 0.0 {
		return None;
	}

	let weights = [
		signed_area_xz(point, second, third) / denominator,
		signed_area_xz(point, third, first) / denominator,
		signed_area_xz(point, first, second) / denominator,
	];
	weights
		.iter()
		.all(|&weight| weight >= -orientation_tolerance(weight))
		.then_some(weights)
}

/// Returns whether `point` lies on the closed XZ line segment from `start` to `end`.
///
/// The caller must establish collinearity first, for example with [`signed_area_xz`].
pub fn point_on_segment_xz<Space>(point: Point<Space>, start: Point<Space>, end: Point<Space>) -> bool {
	let x = point.x();
	let z = point.z();
	x >= start.x().min(end.x()) && x <= start.x().max(end.x()) && z >= start.z().min(end.z()) && z <= start.z().max(end.z())
}

/// Returns whether two closed line segments intersect after projection onto XZ.
pub fn segments_intersect_xz<Space>(
	first: Point<Space>,
	second: Point<Space>,
	third: Point<Space>,
	fourth: Point<Space>,
) -> bool {
	let first_side = signed_area_xz(first, second, third);
	let second_side = signed_area_xz(first, second, fourth);
	let third_side = signed_area_xz(third, fourth, first);
	let fourth_side = signed_area_xz(third, fourth, second);

	if first_side == 0.0 && point_on_segment_xz(third, first, second)
		|| second_side == 0.0 && point_on_segment_xz(fourth, first, second)
		|| third_side == 0.0 && point_on_segment_xz(first, third, fourth)
		|| fourth_side == 0.0 && point_on_segment_xz(second, third, fourth)
	{
		return true;
	}

	((first_side > 0.0 && second_side < 0.0) || (first_side < 0.0 && second_side > 0.0))
		&& ((third_side > 0.0 && fourth_side < 0.0) || (third_side < 0.0 && fourth_side > 0.0))
}

fn orientation_tolerance(value: f32) -> f32 {
	f32::EPSILON * 16.0 * value.abs().max(1.0)
}

/// The `Vector` struct represents a displacement in one coordinate space and tracks whether its length is validated.
#[repr(transparent)]
pub struct Vector<Space = WorldSpace, State = Unnormalized> {
	value: Vec3f,
	space: PhantomData<(Space, State)>,
}

impl<Space, State> Copy for Vector<Space, State> {}

impl<Space, State> Clone for Vector<Space, State> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<Space, State> fmt::Debug for Vector<Space, State> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.debug_tuple("Vector").field(&self.value).finish()
	}
}

impl<Space, State> PartialEq for Vector<Space, State> {
	fn eq(&self, other: &Self) -> bool {
		self.value == other.value
	}
}

impl<Space> Vector<Space, Unnormalized> {
	/// Creates an unnormalized vector from coordinates in `Space`.
	pub fn new(x: f32, y: f32, z: f32) -> Self {
		Self::from_maths(Vec3f::new(x, y, z))
	}

	/// Creates the zero displacement.
	pub fn zero() -> Self {
		Self::new(0.0, 0.0, 0.0)
	}

	/// Creates a branded vector from an explicit `maths-rs` value.
	pub fn from_maths(value: Vec3f) -> Self {
		Self {
			value,
			space: PhantomData,
		}
	}

	/// Checks the vector and returns a [`UnitVector`] and its original length.
	///
	/// Use the returned length when an operation needs both a direction and distance. This avoids
	/// measuring the vector again and remains accurate for finite vectors that would overflow or
	/// underflow during an unscaled length calculation.
	pub fn normalize_with_length(self) -> Result<(UnitVector<Space>, f32), NormalizationError> {
		let value = self.value;
		if !value.x.is_finite() || !value.y.is_finite() || !value.z.is_finite() {
			return Err(NormalizationError::NonFinite);
		}

		// Scaling before measuring avoids overflow for large finite values and underflow for tiny ones.
		let scale = value.x.abs().max(value.y.abs()).max(value.z.abs());
		if scale == 0.0 {
			return Err(NormalizationError::ZeroLength);
		}
		let scaled = Vec3f::new(value.x / scale, value.y / scale, value.z / scale);
		let scaled_length = dot_values(scaled, scaled).sqrt();
		let normalized = Vec3f::new(scaled.x / scaled_length, scaled.y / scaled_length, scaled.z / scaled_length);
		let length = scale * scaled_length;

		Ok((
			UnitVector {
				value: normalized,
				space: PhantomData,
			},
			length,
		))
	}

	/// Checks the vector and returns a [`UnitVector`] suitable for a normal or direction.
	pub fn normalized(self) -> Result<UnitVector<Space>, NormalizationError> {
		self.normalize_with_length().map(|(unit_vector, _)| unit_vector)
	}

	/// Checks the vector and returns a [`UnitVector`] suitable for a normal or direction.
	pub fn unit(self) -> Result<UnitVector<Space>, NormalizationError> {
		self.normalize_with_length().map(|(unit_vector, _)| unit_vector)
	}
}

impl<Space, State> Vector<Space, State> {
	/// Returns this vector as an explicit `maths-rs` value for boundary integrations.
	pub fn into_maths(self) -> Vec3f {
		self.value
	}

	/// Returns the x component.
	pub fn x(self) -> f32 {
		self.value.x
	}

	/// Returns the y component.
	pub fn y(self) -> f32 {
		self.value.y
	}

	/// Returns the z component.
	pub fn z(self) -> f32 {
		self.value.z
	}

	/// Returns the squared length without a square-root operation.
	pub fn length_squared(self) -> f32 {
		self.value.x * self.value.x + self.value.y * self.value.y + self.value.z * self.value.z
	}

	/// Returns the vector length.
	pub fn length(self) -> f32 {
		self.length_squared().sqrt()
	}

	/// Compares this vector's magnitude with `other` without calculating a square root.
	///
	/// Returns [`None`] when either vector has a non-finite component. Finite inputs are
	/// compared with scaled squared magnitudes, so the comparison remains valid even when
	/// [`Self::length_squared`] would overflow or underflow.
	pub fn partial_cmp_magnitude<OtherState>(self, other: Vector<Space, OtherState>) -> Option<Ordering> {
		let left = scaled_magnitude_squared(self.value)?;
		let right = scaled_magnitude_squared(other.value)?;

		left.partial_cmp(&right)
	}

	/// Returns the scalar projection of this vector onto `other`.
	pub fn dot<OtherState>(self, other: Vector<Space, OtherState>) -> f32 {
		dot_values(self.value, other.value)
	}

	/// Returns a vector perpendicular to this vector and `other`.
	pub fn cross<OtherState>(self, other: Vector<Space, OtherState>) -> Vector<Space> {
		Vector::from_maths(cross_values(self.value, other.value))
	}
}

impl<Space> Default for Vector<Space, Unnormalized> {
	fn default() -> Self {
		Self::zero()
	}
}

impl<Space> From<UnitVector<Space>> for Vector<Space> {
	fn from(value: UnitVector<Space>) -> Self {
		Vector::from_maths(value.value)
	}
}

/// The `UnitVector` struct provides a checked unit-length direction for normals, rays, and orientation APIs.
///
/// Create one from an arbitrary [`Vector`] with [`Self::try_from_vector`] or
/// [`Vector::normalized`]. Convert it to a facing [`crate::Orientation`] with
/// [`crate::orientation_from_direction`] or [`crate::Orientation::from`], or to an orthonormal
/// [`crate::Matrix`] with [`crate::from_normal`]. Use [`Self::into_vector`] when an operation needs
/// a displacement instead of a checked direction.
///
/// A direction does not contain roll. Keep an [`crate::Orientation`] when you need the complete
/// rotation.
#[repr(transparent)]
pub struct UnitVector<Space = WorldSpace> {
	value: Vec3f,
	space: PhantomData<Space>,
}

impl<Space> Copy for UnitVector<Space> {}

impl<Space> Clone for UnitVector<Space> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<Space> fmt::Debug for UnitVector<Space> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.debug_tuple("UnitVector").field(&self.value).finish()
	}
}

impl<Space> PartialEq for UnitVector<Space> {
	fn eq(&self, other: &Self) -> bool {
		self.value == other.value
	}
}

impl<Space> UnitVector<Space> {
	/// Validates and normalizes `vector` so it can be used as a direction or normal.
	///
	/// Use [`crate::orientation_from_direction`] next when the direction must become a facing
	/// [`crate::Orientation`].
	pub fn try_from_vector(vector: Vector<Space>) -> Result<Self, NormalizationError> {
		vector.normalized()
	}

	/// Returns the positive x axis.
	pub fn x_axis() -> Self {
		Self {
			value: Vec3f::new(1.0, 0.0, 0.0),
			space: PhantomData,
		}
	}

	/// Returns the positive y axis.
	pub fn y_axis() -> Self {
		Self {
			value: Vec3f::new(0.0, 1.0, 0.0),
			space: PhantomData,
		}
	}

	/// Returns the positive z axis.
	pub fn z_axis() -> Self {
		Self {
			value: Vec3f::new(0.0, 0.0, 1.0),
			space: PhantomData,
		}
	}

	/// Returns this direction as an unnormalized [`Vector`] when an affine operation needs a displacement.
	///
	/// Call [`Vector::normalized`] to validate an arbitrary vector in the reverse direction.
	pub fn into_vector(self) -> Vector<Space> {
		Vector::from_maths(self.value)
	}

	/// Returns this direction as an explicit `maths-rs` value for boundary integrations.
	pub fn into_maths(self) -> Vec3f {
		self.value
	}

	/// Returns the x component.
	pub fn x(self) -> f32 {
		self.value.x
	}

	/// Returns the y component.
	pub fn y(self) -> f32 {
		self.value.y
	}

	/// Returns the z component.
	pub fn z(self) -> f32 {
		self.value.z
	}

	/// Returns the scalar projection of this direction onto `other`.
	pub fn dot<OtherState>(self, other: Vector<Space, OtherState>) -> f32 {
		dot_values(self.value, other.value)
	}

	/// Returns a perpendicular unnormalized vector.
	pub fn cross(self, other: impl Into<Vector<Space>>) -> Vector<Space> {
		Vector::from_maths(cross_values(self.value, other.into().value))
	}
}

impl<Space, State> Add<Vector<Space, State>> for Point<Space> {
	type Output = Self;

	fn add(self, rhs: Vector<Space, State>) -> Self::Output {
		Self::from_maths(self.value + rhs.value)
	}
}

impl<Space> Add<UnitVector<Space>> for Point<Space> {
	type Output = Self;

	fn add(self, rhs: UnitVector<Space>) -> Self::Output {
		self + rhs.into_vector()
	}
}

impl<Space> AddAssign<Vector<Space>> for Point<Space> {
	fn add_assign(&mut self, rhs: Vector<Space>) {
		self.value += rhs.value;
	}
}

impl<Space, State> Sub<Vector<Space, State>> for Point<Space> {
	type Output = Self;

	fn sub(self, rhs: Vector<Space, State>) -> Self::Output {
		Self::from_maths(self.value - rhs.value)
	}
}

impl<Space> Sub<UnitVector<Space>> for Point<Space> {
	type Output = Self;

	fn sub(self, rhs: UnitVector<Space>) -> Self::Output {
		self - rhs.into_vector()
	}
}

impl<Space> Sub for Point<Space> {
	type Output = Vector<Space>;

	fn sub(self, rhs: Self) -> Self::Output {
		Vector::from_maths(self.value - rhs.value)
	}
}

impl<Space, LeftState, RightState> Add<Vector<Space, RightState>> for Vector<Space, LeftState> {
	type Output = Vector<Space>;

	fn add(self, rhs: Vector<Space, RightState>) -> Self::Output {
		Vector::from_maths(self.value + rhs.value)
	}
}

impl<Space, LeftState, RightState> AddAssign<Vector<Space, RightState>> for Vector<Space, LeftState> {
	fn add_assign(&mut self, rhs: Vector<Space, RightState>) {
		self.value += rhs.value;
	}
}

impl<Space, LeftState, RightState> Sub<Vector<Space, RightState>> for Vector<Space, LeftState> {
	type Output = Vector<Space>;

	fn sub(self, rhs: Vector<Space, RightState>) -> Self::Output {
		Vector::from_maths(self.value - rhs.value)
	}
}

impl<Space, State> Add<UnitVector<Space>> for Vector<Space, State> {
	type Output = Vector<Space>;

	fn add(self, rhs: UnitVector<Space>) -> Self::Output {
		self + rhs.into_vector()
	}
}

impl<Space, State> Sub<UnitVector<Space>> for Vector<Space, State> {
	type Output = Vector<Space>;

	fn sub(self, rhs: UnitVector<Space>) -> Self::Output {
		self - rhs.into_vector()
	}
}

impl<Space, State> Mul<f32> for Vector<Space, State> {
	type Output = Vector<Space>;

	fn mul(self, rhs: f32) -> Self::Output {
		Vector::from_maths(self.value * rhs)
	}
}

impl<Space, State> Mul<Vector<Space, State>> for f32 {
	type Output = Vector<Space>;

	fn mul(self, rhs: Vector<Space, State>) -> Self::Output {
		rhs * self
	}
}

impl<Space, State> Div<f32> for Vector<Space, State> {
	type Output = Vector<Space>;

	fn div(self, rhs: f32) -> Self::Output {
		Vector::from_maths(self.value / rhs)
	}
}

impl<Space, State> Neg for Vector<Space, State> {
	type Output = Vector<Space>;

	fn neg(self) -> Self::Output {
		Vector::from_maths(-self.value)
	}
}

impl<Space> Add<Vector<Space>> for UnitVector<Space> {
	type Output = Vector<Space>;

	fn add(self, rhs: Vector<Space>) -> Self::Output {
		self.into_vector() + rhs
	}
}

impl<Space> Sub<Vector<Space>> for UnitVector<Space> {
	type Output = Vector<Space>;

	fn sub(self, rhs: Vector<Space>) -> Self::Output {
		self.into_vector() - rhs
	}
}

impl<Space> Mul<f32> for UnitVector<Space> {
	type Output = Vector<Space>;

	fn mul(self, rhs: f32) -> Self::Output {
		self.into_vector() * rhs
	}
}

impl<Space> Mul<UnitVector<Space>> for f32 {
	type Output = Vector<Space>;

	fn mul(self, rhs: UnitVector<Space>) -> Self::Output {
		rhs * self
	}
}

impl<Space> Neg for UnitVector<Space> {
	type Output = Self;

	fn neg(self) -> Self::Output {
		Self {
			value: -self.value,
			space: PhantomData,
		}
	}
}

pub(crate) fn dot_values(left: Vec3f, right: Vec3f) -> f32 {
	left.x * right.x + left.y * right.y + left.z * right.z
}

// `f64` can represent the square of every finite `f32`, including subnormal values.
// Scaling first keeps the component sum bounded and avoids the `f32` overflow and underflow
// that make `Vector::length_squared` unsuitable for magnitude comparisons.
fn scaled_magnitude_squared(value: Vec3f) -> Option<f64> {
	if !value.x.is_finite() || !value.y.is_finite() || !value.z.is_finite() {
		return None;
	}

	let scale = value.x.abs().max(value.y.abs()).max(value.z.abs()) as f64;
	if scale == 0.0 {
		return Some(0.0);
	}

	let x = value.x as f64 / scale;
	let y = value.y as f64 / scale;
	let z = value.z as f64 / scale;
	Some(scale * scale * (x * x + y * y + z * z))
}

pub(crate) fn cross_values(left: Vec3f, right: Vec3f) -> Vec3f {
	Vec3f::new(
		left.y * right.z - left.z * right.y,
		left.z * right.x - left.x * right.z,
		left.x * right.y - left.y * right.x,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	struct LocalSpace;

	fn point(x: f32, z: f32) -> Point<LocalSpace> {
		Point::new(x, 0.0, z)
	}

	#[test]
	fn xz_queries_report_finiteness_distance_and_orientation() {
		assert!(is_finite(point(1.0, 2.0)));
		assert!(!is_finite(Point::<LocalSpace>::new(f32::NAN, 0.0, 0.0)));
		assert_eq!(
			distance_xz(Point::<LocalSpace>::new(0.0, -10.0, 0.0), Point::new(3.0, 20.0, 4.0)),
			5.0
		);
		assert!(signed_area_xz(point(0.0, 0.0), point(2.0, 0.0), point(1.0, 1.0)) > 0.0);
		assert!(signed_area_xz(point(0.0, 0.0), point(2.0, 0.0), point(1.0, -1.0)) < 0.0);
	}

	#[test]
	fn barycentric_xz_returns_weights_only_inside_a_projected_triangle() {
		let first = point(0.0, 0.0);
		let second = point(2.0, 0.0);
		let third = point(0.0, 2.0);

		assert_eq!(barycentric_xz(point(0.5, 0.5), first, second, third), Some([0.5, 0.25, 0.25]));
		assert_eq!(barycentric_xz(point(2.0, 2.0), first, second, third), None);
		assert_eq!(barycentric_xz(point(0.0, 0.0), first, first, first), None);
	}

	#[test]
	fn xz_segment_queries_include_crossings_and_collinear_boundaries() {
		let start = point(0.0, 0.0);
		let end = point(2.0, 2.0);

		assert!(point_on_segment_xz(point(1.0, 1.0), start, end));
		assert!(!point_on_segment_xz(point(3.0, 3.0), start, end));
		assert!(segments_intersect_xz(start, end, point(0.0, 2.0), point(2.0, 0.0)));
		assert!(segments_intersect_xz(start, end, point(1.0, 1.0), point(3.0, 3.0)));
		assert!(!segments_intersect_xz(start, end, point(3.0, 2.0), point(4.0, 3.0)));
	}

	#[test]
	fn affine_operations_keep_points_and_vectors_distinct() {
		let point = Point::<LocalSpace>::new(1.0, 2.0, 3.0);
		let displacement = Vector::<LocalSpace>::new(4.0, -2.0, 1.0);

		assert_eq!(point + displacement, Point::new(5.0, 0.0, 4.0));
		assert_eq!(Point::<LocalSpace>::new(5.0, 0.0, 4.0) - point, displacement);
	}

	#[test]
	fn normalization_rejects_zero_and_non_finite_vectors() {
		assert_eq!(Vector::<WorldSpace>::zero().normalized(), Err(NormalizationError::ZeroLength));
		assert_eq!(
			Vector::<WorldSpace>::new(f32::NAN, 0.0, 0.0).normalized(),
			Err(NormalizationError::NonFinite)
		);
		assert_eq!(
			Vector::<WorldSpace>::new(f32::INFINITY, 0.0, 0.0).normalized(),
			Err(NormalizationError::NonFinite)
		);
	}

	#[test]
	fn normalization_with_length_returns_both_results_from_the_checked_pass() {
		let vector = Vector::<WorldSpace>::new(3.0, 4.0, 0.0);
		let (unit_vector, length) = vector.normalize_with_length().unwrap();

		assert_eq!(unit_vector, Vector::new(0.6, 0.8, 0.0).normalized().unwrap());
		assert_eq!(length, 5.0);
		assert_eq!(vector.normalized().unwrap(), unit_vector);
		assert_eq!(UnitVector::try_from_vector(vector).unwrap(), unit_vector);
	}

	#[test]
	fn normalization_handles_tiny_and_large_finite_vectors() {
		let tiny = Vector::<WorldSpace>::new(f32::MIN_POSITIVE, 0.0, 0.0).normalized().unwrap();
		let large = Vector::<WorldSpace>::new(f32::MAX, f32::MAX, 0.0).normalized().unwrap();

		assert_eq!(tiny, UnitVector::x_axis());
		assert!((large.into_vector().length_squared() - 1.0).abs() < 0.0001);
	}

	#[test]
	fn magnitude_comparison_orders_normal_vectors_and_accepts_any_state() {
		struct Checked;

		let shorter = Vector::<LocalSpace>::new(3.0, 4.0, 0.0);
		let longer = Vector::<LocalSpace, Checked> {
			value: Vec3f::new(6.0, 8.0, 0.0),
			space: PhantomData,
		};

		assert_eq!(shorter.partial_cmp_magnitude(longer), Some(Ordering::Less));
		assert_eq!(longer.partial_cmp_magnitude(shorter), Some(Ordering::Greater));
	}

	#[test]
	fn magnitude_comparison_recognizes_equal_and_zero_magnitudes() {
		let first = Vector::<WorldSpace>::new(3.0, 4.0, 0.0);
		let equal = Vector::<WorldSpace>::new(-4.0, 3.0, 0.0);
		let zero = Vector::<WorldSpace>::zero();

		assert_eq!(first.partial_cmp_magnitude(equal), Some(Ordering::Equal));
		assert_eq!(zero.partial_cmp_magnitude(Vector::zero()), Some(Ordering::Equal));
		assert_eq!(zero.partial_cmp_magnitude(first), Some(Ordering::Less));
		assert_eq!(first.partial_cmp_magnitude(zero), Some(Ordering::Greater));
	}

	#[test]
	fn magnitude_comparison_handles_finite_values_that_break_raw_squared_lengths() {
		let large = Vector::<WorldSpace>::new(f32::MAX, 0.0, 0.0);
		let smaller_large = Vector::<WorldSpace>::new(f32::MAX / 2.0, 0.0, 0.0);
		let tiny = Vector::<WorldSpace>::new(f32::from_bits(1), 0.0, 0.0);
		let larger_tiny = Vector::<WorldSpace>::new(f32::from_bits(2), 0.0, 0.0);

		assert!(large.length_squared().is_infinite());
		assert!(smaller_large.length_squared().is_infinite());
		assert_eq!(large.partial_cmp_magnitude(smaller_large), Some(Ordering::Greater));
		assert_eq!(tiny.length_squared(), 0.0);
		assert_eq!(larger_tiny.length_squared(), 0.0);
		assert_eq!(tiny.partial_cmp_magnitude(larger_tiny), Some(Ordering::Less));
	}

	#[test]
	fn magnitude_comparison_rejects_non_finite_components() {
		let finite = Vector::<WorldSpace>::new(1.0, 0.0, 0.0);

		assert_eq!(finite.partial_cmp_magnitude(Vector::new(f32::NAN, 0.0, 0.0)), None);
		assert_eq!(Vector::new(f32::INFINITY, 0.0, 0.0).partial_cmp_magnitude(finite), None);
		assert_eq!(Vector::new(0.0, f32::NEG_INFINITY, 0.0).partial_cmp_magnitude(finite), None);
	}
}
