use std::fmt;

use maths_rs::Quatf;

use crate::serialization::{ArrayForm, serialize_as_array};
use crate::{Matrix, Radians, UnitVector, Vector, orientation_from_direction};

/// Rotation vectors shorter than this are treated as no rotation, where the axis is undefined.
const ROTATION_VECTOR_EPSILON: f32 = 1.0e-8;

/// The `Orientation` struct provides a normalized, finite rotation for engine transforms.
///
/// Create one from an axis and angle with [`Self::try_from_axis_angle`], from a facing
/// [`UnitVector`] with [`orientation_from_direction`] or [`Self::from`], or from a raw quaternion
/// with [`Self::try_from_maths`]. Use [`Self::rotate_vector`] to rotate a displacement,
/// [`Self::into_matrix`] at a matrix boundary, or [`crate::direction_from_orientation`] to extract
/// the +Z facing direction.
///
/// Keep an `Orientation` when roll matters. A [`UnitVector`] contains only a direction, so a
/// direction round trip cannot preserve roll.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
	value: Quatf,
}

/// Describes why raw data cannot represent an [`Orientation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrientationError {
	/// The quaternion contains NaN or infinity. The input must contain only finite components.
	NonFiniteQuaternion,
	/// The quaternion has no rotation because every component is zero. Use [`Orientation::identity`] instead.
	ZeroLengthQuaternion,
	/// The angle contains NaN or infinity. The input angle must be finite radians.
	NonFiniteAngle,
}

impl fmt::Display for OrientationError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NonFiniteQuaternion => formatter
				.write_str("Cannot create an orientation from a non-finite quaternion. The input contains NaN or infinity."),
			Self::ZeroLengthQuaternion => formatter.write_str(
				"Cannot create an orientation from a zero-length quaternion. Use Orientation::identity for no rotation.",
			),
			Self::NonFiniteAngle => formatter
				.write_str("Cannot create an orientation from a non-finite angle. The input angle contains NaN or infinity."),
		}
	}
}

impl std::error::Error for OrientationError {}

impl Orientation {
	/// Creates the orientation that preserves every vector.
	pub const fn identity() -> Self {
		Self {
			value: Quatf {
				x: 0.0,
				y: 0.0,
				z: 0.0,
				w: 1.0,
			},
		}
	}

	/// Validates and normalizes an explicit raw [`crate::Quaternion`] from a maths integration boundary.
	///
	/// Use [`Self::into_maths`] for the reverse conversion. If the source is a facing direction
	/// rather than quaternion components, use [`orientation_from_direction`].
	pub fn try_from_maths(value: Quatf) -> Result<Self, OrientationError> {
		Ok(Self {
			value: normalize(value)?,
		})
	}

	/// Validates and normalizes `[x, y, z, w]` quaternion components.
	pub fn try_from_array([x, y, z, w]: [f32; 4]) -> Result<Self, OrientationError> {
		Self::try_from_maths(Quatf::new(x, y, z, w))
	}

	/// Returns the `[x, y, z, w]` components of this orientation's unit quaternion.
	pub fn to_array(self) -> [f32; 4] {
		[self.value.x, self.value.y, self.value.z, self.value.w]
	}

	/// Creates a rotation around a checked axis by a finite [`Radians`] value.
	///
	/// Use [`crate::from_rotation`] only when the destination specifically requires a [`Matrix`].
	pub fn try_from_axis_angle<Space>(axis: UnitVector<Space>, angle: Radians) -> Result<Self, OrientationError> {
		if !angle.is_finite() {
			return Err(OrientationError::NonFiniteAngle);
		}

		// The axis is already finite and unit length; normalization protects the invariant from rounding.
		Self::try_from_maths(Quatf::from_axis_angle(axis.into_maths(), angle.value()))
	}

	/// Returns this orientation as an explicit raw [`crate::Quaternion`] for a maths integration boundary.
	///
	/// Use [`Self::try_from_maths`] for the reverse checked conversion.
	pub fn into_maths(self) -> Quatf {
		self.value
	}

	/// Returns this orientation as a homogeneous rotation [`Matrix`] for rendering or physics boundaries.
	///
	/// Keep the `Orientation` for further rotation composition. Use
	/// [`crate::direction_from_orientation`] instead when the destination only needs facing.
	/// There is no checked [`Matrix`] → `Orientation` conversion, so retain this value if you will
	/// need the rotation after crossing the matrix boundary.
	pub fn into_matrix(self) -> Matrix {
		Matrix::from(self.value)
	}

	/// Combines this orientation with `other`, applying `other` first and this orientation second.
	pub fn compose(self, other: Self) -> Self {
		// Products of normalized finite quaternions are finite; normalize to remove accumulated rounding drift.
		Self {
			value: normalize(self.value * other.value).expect("normalized finite quaternion products remain valid"),
		}
	}

	/// Returns the rotation that undoes this one.
	pub fn inverse(self) -> Self {
		let Quatf { x, y, z, w } = self.value;
		Self {
			value: Quatf::new(-x, -y, -z, w),
		}
	}

	/// Returns the four-dimensional dot product of both unit quaternions.
	///
	/// A negative result means the quaternions lie in opposite hemispheres, so blending them directly would take the
	/// longer way around.
	pub fn dot(self, other: Self) -> f32 {
		let (left, right) = (self.value, other.value);
		left.x * right.x + left.y * right.y + left.z * right.z + left.w * right.w
	}

	/// Moves `factor` of the way to `other` along the shorter arc by normalizing a linear quaternion blend.
	///
	/// A non-finite `factor` returns this orientation unchanged.
	pub fn nlerp(self, other: Self, factor: f32) -> Self {
		let left = self.value;
		let right = if self.dot(other) < 0.0 { -other.value } else { other.value };
		let blended = Quatf::new(
			left.x + (right.x - left.x) * factor,
			left.y + (right.y - left.y) * factor,
			left.z + (right.z - left.z) * factor,
			left.w + (right.w - left.w) * factor,
		);
		Self::try_from_maths(blended).unwrap_or(self)
	}

	/// Returns this rotation as the shortest rotation vector: its axis scaled by its angle in radians.
	///
	/// Use [`Self::try_from_rotation_vector`] for the reverse conversion. Rotation vectors add and scale linearly, which
	/// suits decaying or differentiating rotations.
	pub fn to_rotation_vector<Space>(self) -> Vector<Space> {
		let value = if self.value.w < 0.0 { -self.value } else { self.value };
		let vector_length = (value.x * value.x + value.y * value.y + value.z * value.z).sqrt();
		if vector_length <= ROTATION_VECTOR_EPSILON {
			return Vector::zero();
		}
		let angle = 2.0 * vector_length.atan2(value.w.clamp(-1.0, 1.0));
		Vector::new(value.x, value.y, value.z) * (angle / vector_length)
	}

	/// Creates the rotation around a rotation vector's axis by its length in radians.
	pub fn try_from_rotation_vector<Space>(vector: Vector<Space>) -> Result<Self, OrientationError> {
		let angle = vector.length();
		if !angle.is_finite() {
			return Err(OrientationError::NonFiniteAngle);
		}
		let [x, y, z] = vector.to_array();
		if angle <= ROTATION_VECTOR_EPSILON {
			return Self::try_from_maths(Quatf::new(x * 0.5, y * 0.5, z * 0.5, 1.0));
		}
		let half_angle = angle * 0.5;
		let scale = half_angle.sin() / angle;
		Self::try_from_maths(Quatf::new(x * scale, y * scale, z * scale, half_angle.cos()))
	}

	/// Rotates a displacement while preserving its coordinate-space brand.
	pub fn rotate_vector<Space>(self, vector: Vector<Space>) -> Vector<Space> {
		Vector::from_maths(self.value * vector.into_maths())
	}
}

impl Default for Orientation {
	fn default() -> Self {
		Self::identity()
	}
}

fn normalize(value: Quatf) -> Result<Quatf, OrientationError> {
	if !value.x.is_finite() || !value.y.is_finite() || !value.z.is_finite() || !value.w.is_finite() {
		return Err(OrientationError::NonFiniteQuaternion);
	}

	// Scaling first prevents overflow and underflow when measuring finite input components.
	let scale = value.x.abs().max(value.y.abs()).max(value.z.abs()).max(value.w.abs());
	if scale == 0.0 {
		return Err(OrientationError::ZeroLengthQuaternion);
	}
	let x = value.x / scale;
	let y = value.y / scale;
	let z = value.z / scale;
	let w = value.w / scale;
	let length = (x * x + y * y + z * z + w * w).sqrt();

	Ok(Quatf::new(x / length, y / length, z / length, w / length))
}

impl From<UnitVector> for Orientation {
	fn from(direction: UnitVector) -> Self {
		orientation_from_direction(direction)
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Quatf;

	use super::{Orientation, OrientationError};
	use crate::{Radians, UnitVector, Vector, WorldSpace};

	#[test]
	fn raw_construction_normalizes_finite_quaternions() {
		let orientation = Orientation::try_from_maths(Quatf::new(0.0, 0.0, 0.0, 2.0)).unwrap();

		assert_eq!(orientation, Orientation::identity());
	}

	#[test]
	fn raw_construction_rejects_invalid_quaternions() {
		assert_eq!(
			Orientation::try_from_maths(Quatf::new(f32::NAN, 0.0, 0.0, 1.0)),
			Err(OrientationError::NonFiniteQuaternion)
		);
		assert_eq!(
			Orientation::try_from_maths(Quatf::new(0.0, 0.0, 0.0, 0.0)),
			Err(OrientationError::ZeroLengthQuaternion)
		);
	}

	#[test]
	fn composition_matches_sequential_rotation() {
		let around_x =
			Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::x_axis(), Radians::new(std::f32::consts::FRAC_PI_2))
				.unwrap();
		let around_z =
			Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::z_axis(), Radians::new(std::f32::consts::FRAC_PI_2))
				.unwrap();
		let vector = Vector::<WorldSpace>::new(0.0, 1.0, 0.0);

		let composed = around_z.compose(around_x).rotate_vector(vector);
		let sequential = around_z.rotate_vector(around_x.rotate_vector(vector));

		assert!((composed.x() - sequential.x()).abs() < 0.0001);
		assert!((composed.y() - sequential.y()).abs() < 0.0001);
		assert!((composed.z() - sequential.z()).abs() < 0.0001);
	}

	#[test]
	fn rotation_vectors_round_trip_through_the_shorter_arc() {
		let vector = Vector::<WorldSpace>::new(0.0, 1.25, 0.0);
		let orientation = Orientation::try_from_rotation_vector(vector).unwrap();

		crate::assert_geometry_near!(
			orientation.to_rotation_vector::<WorldSpace>(),
			vector,
			"rotation vectors must round trip"
		);
		// The antipodal quaternion is the same rotation, so it must give the same rotation vector.
		let antipodal = Orientation::try_from_array(orientation.to_array().map(|component| -component)).unwrap();
		crate::assert_geometry_near!(
			antipodal.to_rotation_vector::<WorldSpace>(),
			vector,
			"antipodes must give the shorter arc"
		);
	}
}

impl ArrayForm<4> for Orientation {
	type Array = [f32; 4];
	type Error = OrientationError;

	fn to_array(&self) -> Self::Array {
		Orientation::to_array(*self)
	}

	fn try_from_array(array: Self::Array) -> Result<Self, Self::Error> {
		Self::try_from_array(array)
	}
}

serialize_as_array!(Orientation, 4);
