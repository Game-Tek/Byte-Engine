//! Branded, allocation-free three-dimensional geometry for Byte-Engine.
//!
//! Use [`Point`] for locations, [`Vector`] for displacements, and [`UnitVector`] for normals and directions. Space brands prevent accidental operations across coordinate systems; game domains define their own brands and use them as the `Space` parameter.

mod geometry;

pub mod aabb;
pub mod collision;
pub mod plane;
pub mod ray;
pub mod sphere;

pub use aabb::AABB;
pub use geometry::{NormalizationError, Point, UnitVector, Unnormalized, Vector, WorldSpace};
use maths_rs::mat::{MatNew4, MatTranspose as _};
pub use maths_rs::{Mat4f as Matrix, Quatf as Quaternion};
pub use plane::Plane;
pub use ray::Ray;
pub use sphere::Sphere;

/// Asserts that two floating-point values differ by no more than an explicit epsilon.
///
/// The expressions are evaluated once. The assertion fails for non-finite values so tests do not silently accept `NaN`.
#[macro_export]
macro_rules! assert_float_eq_with_epsilon {
	($left:expr, $right:expr, $epsilon:expr $(,)?) => {
		match (&$left, &$right, &$epsilon) {
			(left, right, epsilon) => {
				let difference = (*left as f64 - *right as f64).abs();
				let epsilon = *epsilon as f64;
				if !difference.is_finite() || !epsilon.is_finite() || epsilon < 0.0 || difference > epsilon {
					panic!(
						"assertion failed: values are not within epsilon\n  left: `{:?}`,\n right: `{:?}`,\n difference: `{difference:?}`,\n epsilon: `{epsilon:?}`",
						*left, *right,
					);
				}
			}
		}
	};
	($left:expr, $right:expr, $epsilon:expr, $($arg:tt)+) => {
		match (&$left, &$right, &$epsilon) {
			(left, right, epsilon) => {
				let difference = (*left as f64 - *right as f64).abs();
				let epsilon = *epsilon as f64;
				if !difference.is_finite() || !epsilon.is_finite() || epsilon < 0.0 || difference > epsilon {
					panic!(
						"assertion failed: values are not within epsilon\n  left: `{:?}`,\n right: `{:?}`,\n difference: `{difference:?}`,\n epsilon: `{epsilon:?}`\n{}",
						*left, *right, format_args!($($arg)+),
					);
				}
			}
		}
	};
}

/// Asserts that two floating-point values differ by no more than `0.001`.
#[macro_export]
macro_rules! assert_float_eq {
	($left:expr, $right:expr $(,)?) => {
		$crate::assert_float_eq_with_epsilon!($left, $right, 0.001)
	};
	($left:expr, $right:expr, $($arg:tt)+) => {
		$crate::assert_float_eq_with_epsilon!($left, $right, 0.001, $($arg)+)
	};
}

/// Asserts that two branded geometry values have near-equal x, y, and z components.
///
/// The values can be [`Point`], [`Vector`], or [`UnitVector`] values in the same space.
#[macro_export]
macro_rules! assert_geometry_near {
	($left:expr, $right:expr $(,)?) => {
		match (&$left, &$right) {
			(left, right) => {
				$crate::assert_float_eq!(left.x(), right.x(), "x component differs");
				$crate::assert_float_eq!(left.y(), right.y(), "y component differs");
				$crate::assert_float_eq!(left.z(), right.z(), "z component differs");
			}
		}
	};
	($left:expr, $right:expr, $($arg:tt)+) => {
		match (&$left, &$right) {
			(left, right) => {
				$crate::assert_float_eq!(left.x(), right.x(), $($arg)+);
				$crate::assert_float_eq!(left.y(), right.y(), $($arg)+);
				$crate::assert_float_eq!(left.z(), right.z(), $($arg)+);
			}
		}
	};
}

/// The `ShaderMatrix` struct provides the aligned matrix layout graphics backends use for GPU uploads.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderMatrix(pub [f32; 16]);

/// The `AffineShaderMatrix` struct provides the compact affine matrix layout for GPU transform uploads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AffineShaderMatrix(pub [f32; 12]);

impl From<Matrix> for ShaderMatrix {
	fn from(mut value: Matrix) -> Self {
		#[cfg(target_os = "macos")]
		{
			value = value.transpose();
		}

		Self([
			value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7], value[8], value[9], value[10],
			value[11], value[12], value[13], value[14], value[15],
		])
	}
}

impl From<Matrix> for AffineShaderMatrix {
	fn from(mut value: Matrix) -> Self {
		value = value.transpose();
		Self([
			value[0], value[1], value[2], value[4], value[5], value[6], value[8], value[9], value[10], value[12], value[13],
			value[14],
		])
	}
}

/// Converts a camera-relative movement command into a horizontal world-space displacement.
pub fn plane_navigation<Space>(direction: UnitVector<Space>, command: Vector<Space>) -> Vector<Space> {
	Vector::new(direction.x(), 0.0, direction.z()) * command.z() + Vector::new(direction.z(), 0.0, -direction.x()) * command.x()
}

/// Returns whether two directions are nearly parallel or anti-parallel.
pub fn are_colinear<Space>(first: UnitVector<Space>, second: UnitVector<Space>) -> bool {
	first.dot(second.into_vector()).abs() > 0.99
}

/// Returns an orthonormal transform whose forward basis follows `normal`.
pub fn from_normal<Space>(normal: UnitVector<Space>) -> Matrix {
	let up = UnitVector::y_axis();
	let reference = if are_colinear(normal, up) { UnitVector::z_axis() } else { up };
	// `UnitVector` values are finite and unit length; the non-colinear reference makes both cross products nonzero.
	let x_basis = maths_rs::normalize(maths_rs::cross(normal.into_maths(), reference.into_maths()));
	let y_basis = maths_rs::normalize(maths_rs::cross(normal.into_maths(), x_basis));

	Matrix::from((
		maths_rs::Vec4f::from((x_basis, 0.0)),
		maths_rs::Vec4f::from((y_basis, 0.0)),
		maths_rs::Vec4f::from((normal.into_maths(), 0.0)),
		maths_rs::Vec4f::from((0.0, 0.0, 0.0, 1.0)),
	))
}

/// Builds a rotation matrix around a checked axis.
pub fn from_rotation<Space>(axis: UnitVector<Space>, theta: f32) -> Matrix {
	let c = theta.cos();
	let s = -theta.sin();
	let one_minus_c = 1.0 - c;
	let x = axis.x();
	let y = axis.y();
	let z = axis.z();

	Matrix::new(
		c + x * x * one_minus_c,
		x * y * one_minus_c - z * s,
		x * z * one_minus_c + y * s,
		0.0,
		y * x * one_minus_c + z * s,
		c + y * y * one_minus_c,
		y * z * one_minus_c - x * s,
		0.0,
		z * x * one_minus_c - y * s,
		z * y * one_minus_c + x * s,
		c + z * z * one_minus_c,
		0.0,
		0.0,
		0.0,
		0.0,
		1.0,
	)
}

/// Returns the shortest orientation from the engine forward axis to `direction`.
pub fn orientation_from_direction<Space>(direction: UnitVector<Space>) -> Quaternion {
	let forward = UnitVector::z_axis();
	let alignment = forward.dot(direction.into_vector()).clamp(-1.0, 1.0);

	// Opposite directions have no unique rotation axis, so use x for deterministic output.
	if alignment <= -1.0 + f32::EPSILON {
		return Quaternion::from_axis_angle(UnitVector::<Space>::x_axis().into_maths(), std::f32::consts::PI);
	}

	let axis = forward.cross(direction.into_vector());
	maths_rs::normalize(Quaternion::new(axis.x(), axis.y(), axis.z(), 1.0 + alignment))
}

/// Returns the checked direction produced by rotating the engine forward axis.
///
/// The method returns [`NormalizationError`] if `orientation` produces a non-finite direction.
pub fn direction_from_orientation(orientation: Quaternion) -> Result<UnitVector, NormalizationError> {
	let rotated_forward = orientation * UnitVector::<WorldSpace>::z_axis().into_maths();
	Vector::from_maths(rotated_forward).normalize()
}

/// Returns a left-handed perspective projection matrix with a zero-to-one depth range.
pub fn projection_matrix(fov: f32, aspect_ratio: f32, near_plane: f32, far_plane: f32) -> Matrix {
	let h = 1.0 / (fov * 0.5).to_radians().tan();
	let w = h / aspect_ratio;
	let range = far_plane - near_plane;
	let a = -near_plane / range;
	let b = near_plane * far_plane / range;

	Matrix::from((
		maths_rs::Vec4f::from((w, 0.0, 0.0, 0.0)),
		maths_rs::Vec4f::from((0.0, h, 0.0, 0.0)),
		maths_rs::Vec4f::from((0.0, 0.0, a, b)),
		maths_rs::Vec4f::from((0.0, 0.0, 1.0, 0.0)),
	))
}

/// Returns an orthographic projection matrix centered on the origin.
pub fn orthographic_matrix_centered(width: f32, height: f32, near_plane: f32, far_plane: f32) -> Matrix {
	let range = far_plane - near_plane;
	Matrix::from((
		maths_rs::Vec4f::from((2.0 / width, 0.0, 0.0, 0.0)),
		maths_rs::Vec4f::from((0.0, 2.0 / height, 0.0, 0.0)),
		maths_rs::Vec4f::from((0.0, 0.0, -1.0 / range, far_plane / range)),
		maths_rs::Vec4f::from((0.0, 0.0, 0.0, 1.0)),
	))
}

/// Returns an orthographic projection matrix for the supplied extents.
pub fn orthographic_matrix(left: f32, right: f32, bottom: f32, top: f32, near_plane: f32, far_plane: f32) -> Matrix {
	let range = far_plane - near_plane;
	Matrix::from((
		maths_rs::Vec4f::from((2.0 / (right - left), 0.0, 0.0, -(right + left) / (right - left))),
		maths_rs::Vec4f::from((0.0, 2.0 / (top - bottom), 0.0, -(top + bottom) / (top - bottom))),
		maths_rs::Vec4f::from((0.0, 0.0, -1.0 / range, far_plane / range)),
		maths_rs::Vec4f::from((0.0, 0.0, 0.0, 1.0)),
	))
}

/// Returns the inverse of `matrix`.
pub fn inverse(matrix: Matrix) -> Matrix {
	use maths_rs::mat::MatInverse as _;

	matrix.inverse()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn float_assertions_accept_near_values_and_reject_nan() {
		crate::assert_float_eq!(2.0 + 2.0, 4.0);
		crate::assert_geometry_near!(
			Vector::<WorldSpace>::new(1.0, 2.0, 3.0),
			Vector::<WorldSpace>::new(1.0, 2.0, 3.00005)
		);

		let result = std::panic::catch_unwind(|| crate::assert_float_eq!(f32::NAN, 0.0));
		assert!(result.is_err());
	}

	#[test]
	fn from_normal_builds_a_right_handed_orthonormal_basis() {
		for normal in [
			UnitVector::<WorldSpace>::z_axis(),
			UnitVector::<WorldSpace>::y_axis(),
			UnitVector::<WorldSpace>::x_axis(),
			-UnitVector::<WorldSpace>::x_axis(),
		] {
			let basis = from_normal(normal);
			let x = Vector::<WorldSpace>::new(basis[0], basis[1], basis[2]);
			let y = Vector::<WorldSpace>::new(basis[4], basis[5], basis[6]);
			let z = Vector::<WorldSpace>::new(basis[8], basis[9], basis[10]);

			crate::assert_float_eq_with_epsilon!(x.length(), 1.0, 0.0001);
			crate::assert_float_eq_with_epsilon!(y.length(), 1.0, 0.0001);
			crate::assert_float_eq_with_epsilon!(z.length(), 1.0, 0.0001);
			crate::assert_float_eq_with_epsilon!(x.dot(y), 0.0, 0.0001);
			crate::assert_float_eq_with_epsilon!(x.cross(y).dot(z), 1.0, 0.0001);
			crate::assert_geometry_near!(z, normal.into_vector());
		}
	}

	#[test]
	fn inverse_preserves_identity_and_inverts_a_scale_matrix() {
		let identity = Matrix::new(1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0);
		assert_eq!(inverse(identity), identity);

		let scale = Matrix::new(1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 1.0);
		let inverse_scale = inverse(scale);
		crate::assert_float_eq_with_epsilon!(inverse_scale[0], 1.0, 0.0001);
		crate::assert_float_eq_with_epsilon!(inverse_scale[5], 0.5, 0.0001);
		crate::assert_float_eq_with_epsilon!(inverse_scale[10], 1.0 / 3.0, 0.0001);
	}

	#[test]
	fn orientation_round_trip_preserves_representative_checked_directions() {
		for direction in [
			UnitVector::<WorldSpace>::z_axis(),
			-UnitVector::<WorldSpace>::z_axis(),
			UnitVector::<WorldSpace>::x_axis(),
			UnitVector::<WorldSpace>::y_axis(),
			Vector::<WorldSpace>::new(0.3, -0.4, 0.5).normalize().unwrap(),
		] {
			let resolved = direction_from_orientation(orientation_from_direction(direction)).unwrap();
			crate::assert_geometry_near!(resolved, direction, "orientation round trip must preserve direction");
		}
	}

	#[test]
	fn orientation_from_near_forward_direction_preserves_small_mouse_motion() {
		let yaw = (2.0 / 1024.0) * std::f32::consts::PI;
		let direction = Vector::<WorldSpace>::new(yaw.sin(), 0.0, yaw.cos()).normalize().unwrap();
		let resolved = direction_from_orientation(orientation_from_direction(direction)).unwrap();

		crate::assert_geometry_near!(resolved, direction, "near-forward rotations must retain small input changes");
	}

	#[test]
	fn shader_matrix_layouts_are_stable() {
		let matrix = Matrix::new(
			1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
		);
		let affine = AffineShaderMatrix::from(matrix);

		assert_eq!(affine.0, [1.0, 5.0, 9.0, 2.0, 6.0, 10.0, 3.0, 7.0, 11.0, 4.0, 8.0, 12.0]);
		assert_eq!(std::mem::size_of::<AffineShaderMatrix>(), 48);
	}
}
