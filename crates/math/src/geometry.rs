use std::{
	fmt,
	marker::PhantomData,
	ops::{Add, Div, Mul, Neg, Sub},
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
	pub fn normalize(self) -> Result<UnitVector<Space>, NormalizationError> {
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

/// The `UnitVector` struct represents a checked unit-length direction for normals, rays, and orientation APIs.
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
	pub fn try_from_vector(vector: Vector<Space>) -> Result<Self, NormalizationError> {
		vector.normalize()
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

	/// Returns this direction as an unnormalized vector when an affine operation needs a displacement.
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
	pub fn cross<OtherState>(self, other: Vector<Space, OtherState>) -> Vector<Space> {
		Vector::from_maths(cross_values(self.value, other.value))
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

	#[test]
	fn affine_operations_keep_points_and_vectors_distinct() {
		let point = Point::<LocalSpace>::new(1.0, 2.0, 3.0);
		let displacement = Vector::<LocalSpace>::new(4.0, -2.0, 1.0);

		assert_eq!(point + displacement, Point::new(5.0, 0.0, 4.0));
		assert_eq!(Point::<LocalSpace>::new(5.0, 0.0, 4.0) - point, displacement);
	}

	#[test]
	fn normalization_rejects_zero_and_non_finite_vectors() {
		assert_eq!(Vector::<WorldSpace>::zero().normalize(), Err(NormalizationError::ZeroLength));
		assert_eq!(
			Vector::<WorldSpace>::new(f32::NAN, 0.0, 0.0).normalize(),
			Err(NormalizationError::NonFinite)
		);
		assert_eq!(
			Vector::<WorldSpace>::new(f32::INFINITY, 0.0, 0.0).normalize(),
			Err(NormalizationError::NonFinite)
		);
	}

	#[test]
	fn normalization_with_length_returns_both_results_from_the_checked_pass() {
		let vector = Vector::<WorldSpace>::new(3.0, 4.0, 0.0);
		let (unit_vector, length) = vector.normalize_with_length().unwrap();

		assert_eq!(unit_vector, Vector::new(0.6, 0.8, 0.0).normalize().unwrap());
		assert_eq!(length, 5.0);
		assert_eq!(vector.normalize().unwrap(), unit_vector);
		assert_eq!(UnitVector::try_from_vector(vector).unwrap(), unit_vector);
	}

	#[test]
	fn normalization_handles_tiny_and_large_finite_vectors() {
		let tiny = Vector::<WorldSpace>::new(f32::MIN_POSITIVE, 0.0, 0.0).normalize().unwrap();
		let large = Vector::<WorldSpace>::new(f32::MAX, f32::MAX, 0.0).normalize().unwrap();

		assert_eq!(tiny, UnitVector::x_axis());
		assert!((large.into_vector().length_squared() - 1.0).abs() < 0.0001);
	}
}
