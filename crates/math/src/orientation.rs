use std::fmt;

use maths_rs::Quatf;

use crate::{orientation_from_direction, Matrix, UnitVector, Vector};

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
	pub fn identity() -> Self {
		Self {
			value: Quatf::identity(),
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

	/// Creates a rotation around a checked axis by a finite angle in radians.
	///
	/// Use [`crate::from_rotation`] only when the destination specifically requires a [`Matrix`].
	pub fn try_from_axis_angle<Space>(axis: UnitVector<Space>, angle: f32) -> Result<Self, OrientationError> {
		if !angle.is_finite() {
			return Err(OrientationError::NonFiniteAngle);
		}

		// The axis is already finite and unit length; normalization protects the invariant from rounding.
		Self::try_from_maths(Quatf::from_axis_angle(axis.into_maths(), angle))
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
	use crate::{UnitVector, Vector, WorldSpace};

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
	fn axis_angle_construction_rotates_a_world_vector() {
		let orientation =
			Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::z_axis(), std::f32::consts::FRAC_PI_2).unwrap();
		let rotated = orientation.rotate_vector(Vector::<WorldSpace>::new(1.0, 0.0, 0.0));

		assert!(rotated.x().abs() < 0.0001);
		assert!((rotated.y() - 1.0).abs() < 0.0001);
		assert!(rotated.z().abs() < 0.0001);
	}

	#[test]
	fn composition_matches_sequential_rotation() {
		let around_x =
			Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::x_axis(), std::f32::consts::FRAC_PI_2).unwrap();
		let around_z =
			Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::z_axis(), std::f32::consts::FRAC_PI_2).unwrap();
		let vector = Vector::<WorldSpace>::new(0.0, 1.0, 0.0);

		let composed = around_z.compose(around_x).rotate_vector(vector);
		let sequential = around_z.rotate_vector(around_x.rotate_vector(vector));

		assert!((composed.x() - sequential.x()).abs() < 0.0001);
		assert!((composed.y() - sequential.y()).abs() < 0.0001);
		assert!((composed.z() - sequential.z()).abs() < 0.0001);
	}
}
